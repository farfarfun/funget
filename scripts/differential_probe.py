"""在**当前**解释器里跑一遍场景矩阵,把结果以 JSON 打到 stdout。

这个脚本会被跑两次:一次在装了 PyPI ``funget==1.1.70``(旧的纯 Python 实现)
的 venv 里,一次在装了新 wheel 的 venv 里。``differential.py`` 负责比对两份输出。

每个场景记录三件事:**返回值**、**产物文件的 sha256**、**文件是否被创建**。
这三项就是下游真正依赖的全部可观测行为。
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import sys
import traceback
from pathlib import Path

MIB = 1024 * 1024


def digest(path: Path) -> str | None:
    if not path.is_file():
        return None
    sha = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            sha.update(block)
    return sha.hexdigest()


def scenarios(base: str, workdir: Path) -> list[tuple[str, callable]]:
    """返回 ``(名称, 可调用)``,可调用返回 ``(结果, 产物路径或 None)``。"""
    import funget

    def out(name: str) -> Path:
        return workdir / f"{name}.bin"

    cases: list[tuple[str, callable]] = []

    def case(name: str):
        def register(fn):
            cases.append((name, fn))
            return fn

        return register

    # ---- simple_download --------------------------------------------------

    @case("single/small")
    def _():
        target = out("single_small")
        return funget.simple_download(f"{base}/plain/4096", str(target)), target

    @case("single/empty-ish")
    def _():
        target = out("single_tiny")
        return funget.simple_download(f"{base}/plain/1", str(target)), target

    @case("single/404")
    def _():
        target = out("single_404")
        return funget.simple_download(f"{base}/missing", str(target)), target

    @case("single/skip-existing")
    def _():
        target = out("single_skip")
        funget.simple_download(f"{base}/plain/8192", str(target))
        # 第二次:大小一致、overwrite=False,应当跳过且不改动文件
        return funget.simple_download(f"{base}/plain/8192", str(target)), target

    @case("single/overwrite")
    def _():
        target = out("single_overwrite")
        target.write_bytes(b"\x00" * 8192)
        return (
            funget.simple_download(f"{base}/plain/8192", str(target), overwrite=True),
            target,
        )

    @case("single/nested-dirs")
    def _():
        target = workdir / "a" / "b" / "c" / "nested.bin"
        return funget.simple_download(f"{base}/plain/4096", str(target)), target

    @case("single/chunk-size-zero")
    def _():
        target = out("single_chunk0")
        return (
            funget.simple_download(f"{base}/plain/4096", str(target), chunk_size=0),
            target,
        )

    @case("single/auth")
    def _():
        target = out("single_auth")
        return (
            funget.simple_download(
                f"{base}/auth/4096", str(target), auth=("alice", "s3cret")
            ),
            target,
        )

    @case("single/auth-missing")
    def _():
        target = out("single_auth_missing")
        return funget.simple_download(f"{base}/auth/4096", str(target)), target

    # ---- multi_thread_download -------------------------------------------

    @case("multi/20mib")
    def _():
        target = out("multi_20")
        return (
            funget.multi_thread_download(
                f"{base}/plain/{20 * MIB}", str(target), worker_num=4, block_size=1
            ),
            target,
        )

    @case("multi/no-range-server")
    def _():
        target = out("multi_norange")
        return (
            funget.multi_thread_download(
                f"{base}/norange/{2 * MIB}", str(target), worker_num=4, block_size=1
            ),
            target,
        )

    @case("multi/worker-zero")
    def _():
        target = out("multi_worker0")
        return (
            funget.multi_thread_download(
                f"{base}/plain/{2 * MIB}", str(target), worker_num=0
            ),
            target,
        )

    @case("multi/smaller-than-block")
    def _():
        target = out("multi_small")
        return (
            funget.multi_thread_download(
                f"{base}/plain/4096", str(target), worker_num=4, block_size=100
            ),
            target,
        )

    @case("multi/404")
    def _():
        target = out("multi_404")
        return funget.multi_thread_download(f"{base}/missing", str(target)), target

    @case("multi/custom-headers")
    def _():
        target = out("multi_headers")
        return (
            funget.multi_thread_download(
                f"{base}/plain/{2 * MIB}",
                str(target),
                worker_num=2,
                headers={"X-Trace": "differential"},
            ),
            target,
        )

    # ---- download(自动选择) ----------------------------------------------

    @case("auto/exactly-10mib")
    def _():
        target = out("auto_10")
        return funget.download(f"{base}/plain/{10 * MIB}", str(target)), target

    @case("auto/10mib-plus-one")
    def _():
        target = out("auto_10p1")
        return funget.download(f"{base}/plain/{10 * MIB + 1}", str(target)), target

    @case("auto/forced-single")
    def _():
        target = out("auto_forced_single")
        return (
            funget.download(f"{base}/plain/{12 * MIB}", str(target), multi=False),
            target,
        )

    @case("auto/forced-multi")
    def _():
        target = out("auto_forced_multi")
        return (
            funget.download(f"{base}/plain/{2 * MIB}", str(target), multi=True),
            target,
        )

    @case("auto/no-content-length")
    def _():
        target = out("auto_nolen")
        return funget.download(f"{base}/nolen/{64 * 1024}", str(target)), target

    @case("auto/flaky-retries")
    def _():
        target = out("auto_flaky")
        return (
            funget.download(f"{base}/flaky/{128 * 1024}", str(target), max_retries=5),
            target,
        )

    # ---- single_upload ----------------------------------------------------

    source = workdir / "upload_source.bin"
    source.write_bytes(bytes((i * 17) % 241 for i in range(300_000)))

    @case("upload/put")
    def _():
        return funget.single_upload(f"{base}/upload", str(source)), None

    @case("upload/post")
    def _():
        return (
            funget.single_upload(f"{base}/upload", str(source), method="POST"),
            None,
        )

    @case("upload/bad-method")
    def _():
        return (
            funget.single_upload(f"{base}/upload", str(source), method="PATCH"),
            None,
        )

    @case("upload/chunk-size-zero")
    def _():
        return (
            funget.single_upload(f"{base}/upload", str(source), chunk_size=0),
            None,
        )

    @case("upload/negative-retries")
    def _():
        return (
            funget.single_upload(f"{base}/upload", str(source), max_retries=-1),
            None,
        )

    @case("upload/zero-timeout")
    def _():
        return funget.single_upload(f"{base}/upload", str(source), timeout=0), None

    @case("upload/missing-file")
    def _():
        return funget.single_upload(f"{base}/upload", str(workdir / "nope.bin")), None

    @case("upload/empty-file")
    def _():
        empty = workdir / "empty.bin"
        empty.write_bytes(b"")
        return funget.single_upload(f"{base}/upload", str(empty)), None

    return cases


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base-url", required=True)
    parser.add_argument("--workdir", required=True)
    parser.add_argument("--only", default=None, help="只跑名字里含该子串的场景")
    args = parser.parse_args()

    workdir = Path(args.workdir)
    if workdir.exists():
        shutil.rmtree(workdir)
    workdir.mkdir(parents=True)

    results: dict[str, object] = {}
    for name, run in scenarios(args.base_url, workdir):
        if args.only and args.only not in name:
            continue
        try:
            returned, target = run()
            results[name] = {
                "returned": returned,
                "created": bool(target and target.exists()),
                "sha256": digest(target) if target else None,
                "error": None,
            }
        except Exception:
            results[name] = {
                "returned": "<raised>",
                "created": None,
                "sha256": None,
                "error": traceback.format_exc(limit=1).strip().splitlines()[-1],
            }

    json.dump(
        {"python": sys.version.split()[0], "pid": os.getpid(), "results": results},
        sys.stdout,
        indent=2,
        sort_keys=True,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
