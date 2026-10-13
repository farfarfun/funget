#!/usr/bin/env bash
# 把一个已经构建好的静态二进制打成 .deb。
#
#   build-deb.sh <二进制> <版本号> <amd64|arm64> [输出目录]
#
# 只接受 musl 静态二进制 —— control 里没有 Depends，拿 gnu 动态二进制打出来的包
# 在 glibc 更老的机器上装得上、跑不起来。
set -euo pipefail

BIN="${1:-}"
VERSION="${2:-}"
ARCH="${3:-}"
OUTPUT_DIR="${4:-dist}"

die() {
  echo "错误: $*" >&2
  exit 1
}

[[ -f "$BIN" ]] || die "给的二进制不存在: ${BIN:-<空>}"
VERSION="${VERSION#v}"
[[ -n "$VERSION" && "$VERSION" =~ ^[0-9][0-9A-Za-z.+:~-]*$ ]] || die "版本号无效: ${VERSION:-<空>}"
case "$ARCH" in
  amd64 | arm64) ;;
  *) die "架构只认 amd64 / arm64，给的是: ${ARCH:-<空>}" ;;
esac
command -v dpkg-deb >/dev/null 2>&1 || die "缺少 dpkg-deb"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mkdir -p "$OUTPUT_DIR"
OUTPUT_DIR="$(cd "$OUTPUT_DIR" && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf -- "$WORK"' EXIT
STAGE="${WORK}/root"

install -D -m 0755 "$BIN" "${STAGE}/usr/bin/funget"
install -D -m 0644 "${ROOT}/LICENSE" "${STAGE}/usr/share/doc/funget/copyright"

INSTALLED_SIZE="$(du -sk "${STAGE}/usr" | awk '{print $1}')"
mkdir -p "${STAGE}/DEBIAN"
# 没有 Depends：二进制是 musl 静态链接的，装上就能跑。
#
# ca-certificates 是 Recommends 而不是 Depends：系统证书库一张都读不到时 reqwest
# 不报错（只在「读到了但全解析失败」时才让客户端构造失败），编进二进制的 webpki
# 根证书照样验得了公网站点。所以没装 ca-certificates 的最小镜像里 funget 依然
# 可用，只是认不了企业私有 CA——那是「建议装」的语义，不是「必须装」。
cat >"${STAGE}/DEBIAN/control" <<EOF
Package: funget
Version: ${VERSION}
Section: net
Priority: optional
Architecture: ${ARCH}
Maintainer: farfarfun <farfarfun@qq.com>
Installed-Size: ${INSTALLED_SIZE}
Recommends: ca-certificates
Homepage: https://github.com/farfarfun/funget
Description: HTTP file transfer tool with ranged concurrency and resume
 funget downloads files over HTTP using ranged concurrent chunks, resumes
 interrupted transfers and uploads via PUT or multipart POST.
 Statically linked; no runtime dependencies.
EOF

PACKAGE="${OUTPUT_DIR}/funget_${VERSION}_${ARCH}.deb"
dpkg-deb --root-owner-group --build "${STAGE}" "${PACKAGE}" >/dev/null
echo "$PACKAGE"
