#!/bin/bash
# SJTU Canvas Downloader for macOS: one-line install from Terminal.
#
#   curl -fsSL https://raw.githubusercontent.com/Uniseem/SJTU-CANVAS-DOWNLOADER/main/install.sh | bash
#
# Downloads the app of the latest release for this Mac's chip (or the release
# named by SJTU_CANVAS_VERSION, e.g. v2.0.0), checks it against the release's
# SHA256SUMS.txt and puts it into /Applications (or ~/Applications when
# /Applications is not writable). The app is not notarized, so this is the
# way to install it without the "damaged app" warning that a browser download
# gets. Running the script again updates the app; downloads and settings are
# kept. SJTU_CANVAS_PACKAGE=<file.zip> installs a local build instead.
set -euo pipefail

REPO="Uniseem/SJTU-CANVAS-DOWNLOADER"
APP="SJTU Canvas Downloader.app"
ENGINE="sjtu-canvas-engine"

fail() {
  printf '错误：%s\n' "$1" >&2
  exit 1
}

[ "$(uname -s)" = "Darwin" ] || fail "这个脚本只用于 macOS。Windows 请从 https://github.com/$REPO/releases 下载安装程序。"
version="$(sw_vers -productVersion)"
[ "${version%%.*}" -ge 12 ] || fail "需要 macOS 12 或更新版本，这台 Mac 是 $version。"

# Set on Apple silicon even when this shell runs under Rosetta.
if [ "$(sysctl -n hw.optional.arm64 2>/dev/null || echo 0)" = "1" ]; then
  arch=arm64
else
  arch=x86_64
fi
package="SJTUCanvasDownloader-macos-$arch.zip"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

if [ -n "${SJTU_CANVAS_PACKAGE:-}" ]; then
  [ -f "$SJTU_CANVAS_PACKAGE" ] || fail "找不到 $SJTU_CANVAS_PACKAGE。"
  echo "==> 使用本地安装包 $SJTU_CANVAS_PACKAGE"
  cp "$SJTU_CANVAS_PACKAGE" "$work/$package"
else
  if [ -n "${SJTU_CANVAS_VERSION:-}" ]; then
    base="https://github.com/$REPO/releases/download/$SJTU_CANVAS_VERSION"
  else
    base="https://github.com/$REPO/releases/latest/download"
  fi
  echo "==> 下载 SJTU Canvas Downloader（$arch）"
  curl -fL --retry 3 --progress-bar -o "$work/$package" "$base/$package" ||
    fail "无法下载安装包。请检查网络，或从 https://github.com/$REPO/releases 手动下载 $package。"
  curl -fsSL --retry 3 -o "$work/SHA256SUMS.txt" "$base/SHA256SUMS.txt" || fail "无法下载校验文件 SHA256SUMS.txt。"
  expected="$(awk -v name="$package" '$2 == name || $2 == "*" name { print $1 }' "$work/SHA256SUMS.txt")"
  actual="$(shasum -a 256 "$work/$package" | awk '{ print $1 }')"
  [ -n "$expected" ] || fail "SHA256SUMS.txt 中没有 $package。"
  [ "$expected" = "$actual" ] || fail "安装包校验失败（SHA-256 不一致），请重新运行。"
  echo "==> 校验通过"
fi

echo "==> 解压"
mkdir -p "$work/extract"
ditto -x -k "$work/$package" "$work/extract" || fail "安装包无法解压。"
[ -d "$work/extract/$APP" ] || fail "安装包里没有 $APP。"

# A running copy is asked to quit; it stops its downloads cleanly and they
# resume when the new version starts.
if pgrep -f "/$APP/Contents/MacOS/" >/dev/null 2>&1; then
  echo "==> 正在退出运行中的 SJTU Canvas Downloader"
  osascript -e 'quit app "SJTU Canvas Downloader"' >/dev/null 2>&1 || true
  for _ in $(seq 1 20); do
    pgrep -f "/$APP/Contents/MacOS/" >/dev/null 2>&1 || break
    sleep 0.5
  done
  pkill -f "/$APP/Contents/MacOS/" >/dev/null 2>&1 || true
  pkill -x "$ENGINE" >/dev/null 2>&1 || true
fi

destination="/Applications"
sudo_prefix=""
if [ ! -w "$destination" ] || { [ -e "$destination/$APP" ] && [ ! -w "$destination/$APP" ]; }; then
  if [ -e "$destination/$APP" ] || [ -t 0 ]; then
    echo "==> 安装到“应用程序”文件夹需要这台 Mac 的登录密码"
    sudo_prefix="sudo"
  else
    destination="$HOME/Applications"
    mkdir -p "$destination"
  fi
fi

echo "==> 安装到 $destination"
$sudo_prefix rm -rf "$destination/$APP"
$sudo_prefix ditto "$work/extract/$APP" "$destination/$APP" || fail "无法复制到 $destination。"
# Nothing here is a browser download, so no quarantine flag is expected; remove
# any that a manual download may have left, so Gatekeeper does not block it.
$sudo_prefix xattr -dr com.apple.quarantine "$destination/$APP" 2>/dev/null || true

echo "==> 已安装：$destination/$APP"
open "$destination/$APP" || true
