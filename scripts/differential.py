"""差分测试驱动:新旧两个 funget 实现跑同一套场景,逐项比对。

这是本次 Rust 改造**最关键的安全网** —— 因为没有保留纯 Python 回退,
一旦行为漂移就是下游 16 个仓库的线上问题。

做法:建两个 venv,一个装改造前的 ``funget==1.1.70``(纯 Python),一个装新
wheel;各自起一个全新的测试服务(服务端带状态,比如 flaky 的失败配额,所以
必须一边一个,不能共用),在各自的解释器里跑 ``differential_probe.py``,
比对三项可观测行为:**返回值 / 产物 sha256 / 文件是否创建**。

用法::

    python scripts/differential.py                       # 自动找 dist/ 下的新 wheel
    python scripts/differential.py --new-wheel path.whl
    python scripts/differential.py --baseline funget==1.1.70   # 从 PyPI 取基线
"""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SERVER = ROOT / "scripts" / "differential_server.py"
PROBE = ROOT / "scripts" / "differential_probe.py"

GREEN, RED, YELLOW, DIM, RESET = (
    "\033[32m",
    "\033[31m",
    "\033[33m",
    "\033[2m",
    "\033[0m",
)


def run(cmd: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
    return subprocess.run(cmd, check=True, text=True, **kwargs)  # type: ignore[arg-type]


def make_venv(path: Path, spec: str) -> Path:
    """建 venv 并装一个 funget,返回解释器路径。``spec`` 是 wheel 路径或版本约束。"""
    if path.exists():
        shutil.rmtree(path)
    run(["uv", "venv", "--python", "3.12", str(path)], capture_output=True)
    python = path / "bin" / "python"
    run(
        ["uv", "pip", "install", "--python", str(python), spec],
        capture_output=True,
    )
    return python


def start_server(python: Path) -> tuple[subprocess.Popen[str], str]:
    """起测试服务,返回 ``(进程, base_url)``。

    服务跑在 venv 的解释器里纯属方便 —— 它只用标准库,和装了哪个 funget 无关。
    """
    process = subprocess.Popen(
        [str(python), str(SERVER), "0"],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    line = process.stdout.readline().strip() if process.stdout else ""
    if not line.startswith("http://"):
        process.kill()
        stderr = process.stderr.read() if process.stderr else ""
        raise RuntimeError(f"测试服务没能启动: {line!r}\n{stderr}")
    return process, line


def probe(python: Path, workdir: Path, label: str) -> dict[str, dict[str, object]]:
    """在给定解释器里跑一遍场景矩阵。"""
    server, base_url = start_server(python)
    try:
        result = subprocess.run(
            [
                str(python),
                str(PROBE),
                "--base-url",
                base_url,
                "--workdir",
                str(workdir),
            ],
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            raise RuntimeError(f"{label} 探测进程退出码 {result.returncode}\n{result.stderr}")
        payload = json.loads(result.stdout)
    finally:
        server.terminate()
        server.wait(timeout=10)

    # 端口号随机,别让它漏进比对项里。
    host = base_url.removeprefix("http://")
    for entry in payload["results"].values():
        if isinstance(entry.get("error"), str):
            entry["error"] = entry["error"].replace(host, "<server>")
    return payload["results"]


def compare(old: dict[str, dict], new: dict[str, dict]) -> int:
    """逐场景比对,返回不一致的条数。"""
    names = sorted(set(old) | set(new))
    width = max(len(name) for name in names)
    mismatches = 0

    for name in names:
        before, after = old.get(name), new.get(name)
        if before is None or after is None:
            missing = "旧实现" if before is None else "新实现"
            print(f"{RED}MISSING{RESET} {name:<{width}}  {missing}没有这个场景")
            mismatches += 1
            continue

        diffs = [
            f"{field}: {before[field]!r} → {after[field]!r}"
            for field in ("returned", "created", "sha256")
            if before[field] != after[field]
        ]
        if diffs:
            mismatches += 1
            print(f"{RED}DIFF{RESET}    {name:<{width}}")
            for line in diffs:
                print(f"          {RED}{line}{RESET}")
            for label, entry in (("旧", before), ("新", after)):
                if entry["error"]:
                    print(f"          {DIM}{label}异常: {entry['error']}{RESET}")
        else:
            detail = f"returned={before['returned']!r}"
            if before["sha256"]:
                detail += f" sha={before['sha256'][:12]}"
            print(f"{GREEN}OK{RESET}      {name:<{width}}  {DIM}{detail}{RESET}")
            # 两边都抛异常但类型不同,不算致命(异常文本本就不是契约),但要提示。
            if before["error"] != after["error"] and (before["error"] or after["error"]):
                print(f"          {YELLOW}异常文本不同{RESET}")
                print(f"          {DIM}旧: {before['error']}{RESET}")
                print(f"          {DIM}新: {after['error']}{RESET}")

    return mismatches


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--baseline",
        default=str(ROOT / "dist" / "funget-1.1.70-py3-none-any.whl"),
        help="基线实现:wheel 路径或 pip 约束(默认用 dist 下改造前的纯 Python wheel)",
    )
    parser.add_argument("--new-wheel", default=None, help="新 wheel 路径,默认自动在 dist/ 找")
    parser.add_argument("--workdir", default=str(ROOT / ".differential"))
    parser.add_argument("--keep", action="store_true", help="保留 venv 和产物,便于排查")
    args = parser.parse_args()

    new_wheel = args.new_wheel
    if new_wheel is None:
        candidates = sorted(
            path
            for path in (ROOT / "dist").glob("funget-*.whl")
            if "py3-none-any" not in path.name
        )
        if not candidates:
            print("dist/ 下没有找到编译产出的 wheel,先跑 maturin build", file=sys.stderr)
            return 2
        new_wheel = str(candidates[-1])

    workdir = Path(args.workdir)
    workdir.mkdir(parents=True, exist_ok=True)

    print(f"{DIM}基线  : {args.baseline}{RESET}")
    print(f"{DIM}新实现: {new_wheel}{RESET}\n")

    started = time.monotonic()
    old_python = make_venv(workdir / "venv-old", args.baseline)
    new_python = make_venv(workdir / "venv-new", new_wheel)
    print(f"{DIM}venv 就绪,用时 {time.monotonic() - started:.1f}s{RESET}\n")

    old = probe(old_python, workdir / "out-old", "基线")
    new = probe(new_python, workdir / "out-new", "新实现")

    mismatches = compare(old, new)
    total = len(set(old) | set(new))
    print()
    if mismatches:
        print(f"{RED}{mismatches}/{total} 个场景行为不一致{RESET}")
    else:
        print(f"{GREEN}{total}/{total} 个场景行为完全一致{RESET}")

    if not args.keep:
        shutil.rmtree(workdir, ignore_errors=True)

    return 1 if mismatches else 0


if __name__ == "__main__":
    raise SystemExit(main())
