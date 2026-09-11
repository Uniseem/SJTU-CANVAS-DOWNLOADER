#!/bin/bash
# SJTU Canvas Downloader for macOS: one-line install from Terminal.
#
#   curl -fsSL https://raw.githubusercontent.com/Uniseem/SJTU-CANVAS-DOWNLOADER/main/install.sh | bash
#
# Downloads the installer package of the latest release for this Mac's chip
# (or SJTU_CANVAS_VERSION, e.g. v1.0.0), checks it against the release's
# SHA256SUMS.txt and installs it into /Applications with macOS Installer.
# Running it again updates the app; downloads and settings are kept.
set -euo pipefail

REPO="Uniseem/SJTU-CANVAS-DOWNLOADER"
APP="/Applications/SJTU Canvas Downloader.app"

fail() {
  printf '错误：%s\n' "$1" >&2
  exit 1
}

[ "$(uname -s)" = "Darwin" ] || fail "这个脚本只用于 macOS。Windows 请从 https://github.com/$REPO/releases 下载安装程序。"
version="$(sw_vers -productVersion)"
[ "${version%%.*}" -ge 14 ] || fail "需要 macOS 14 或更新版本，这台 Mac 是 $version。"

# Set on Apple silicon even when this shell runs under Rosetta.
if [ "$(sysctl -n hw.optional.arm64 2>/dev/null || echo 0)" = "1" ]; then
  arch=arm64
else
  arch=x86_64
fi

if [ -n "${SJTU_CANVAS_VERSION:-}" ]; then
  base="https://github.com/$REPO/releases/download/$SJTU_CANVAS_VERSION"
else
  base="https://github.com/$REPO/releases/latest/download"
fi
package="SJTUCanvasDownloader-macos-$arch.pkg"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

echo "==> 下载 SJTU Canvas Downloader（$arch）"
curl -fL --retry 3 --progress-bar -o "$work/$package" "$base/$package" ||
  fail "无法下载安装包。请检查网络，或从 https://github.com/$REPO/releases 手动下载 $package。"
curl -fsSL --retry 3 -o "$work/SHA256SUMS.txt" "$base/SHA256SUMS.txt" || fail "无法下载校验文件 SHA256SUMS.txt。"

expected="$(awk -v name="$package" '$2 == name || $2 == "*" name { print $1 }' "$work/SHA256SUMS.txt")"
actual="$(shasum -a 256 "$work/$package" | awk '{ print $1 }')"
[ -n "$expected" ] || fail "SHA256SUMS.txt 中没有 $package。"
[ "$expected" = "$actual" ] || fail "安装包校验失败（SHA-256 不一致），请重新运行。"
echo "==> 校验通过"

if pgrep -x SJTUCanvasDownloader >/dev/null 2>&1; then
  echo "提示：SJTU Canvas Downloader 正在运行，安装完成后请退出并重新打开。"
fi

echo "==> 安装到“应用程序”文件夹（需要输入这台 Mac 的登录密码）"
sudo installer -pkg "$work/$package" -target / >/dev/null ||
  fail "安装没有完成。"

echo "==> 已安装：$APP"
if ! pgrep -x SJTUCanvasDownloader >/dev/null 2>&1; then
  open "$APP" || true
fi
