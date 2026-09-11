#!/usr/bin/env bash
# Builds "apps/macos/dist/SJTU Canvas Downloader.app" for one architecture.
#
#   1. sjtu-canvas-engine (Rust, release) for the target architecture
#   2. the SwiftUI app (swift build -c release)
#   3. SJTU Canvas Downloader.app/Contents/MacOS/{SJTUCanvasDownloader,sjtu-canvas-engine}
#      SJTU Canvas Downloader.app/Contents/Resources/AppIcon.icns
#   4. a code signature: ad-hoc by default, Developer ID with --sign
#   5. optionally notarization + stapling (--notarize) and packages:
#      --pkg  an installer package (recommended without Developer ID)
#      --dmg  a drag-to-Applications disk image
#      --zip  a zip of the app
#
# Without a Developer ID, distribute the .pkg: the user allows the installer
# once ("仍要打开" in System Settings, or Control-click → 打开), and the app
# that macOS Installer puts into /Applications carries no quarantine flag, so
# Gatekeeper never assesses it — it opens directly, and a download or unzip
# tool can never leave it looking "damaged". Every binary is signed and the
# bundle sealed either way; a Developer ID signature that is notarized and
# stapled opens without any prompt.
#
# Requirements: Xcode (or the Command Line Tools) with the macOS 14 SDK or
# newer, and Rust. The x86_64 installer refuses Apple silicon Macs and the
# arm64 one Intel Macs, each naming the right download.
# Notarization needs a notarytool keychain profile, created once:
#   xcrun notarytool store-credentials sjtu-canvas --apple-id … --team-id … --password <app-specific password>
#
# Usage: apps/macos/build.sh [--arch arm64|x86_64]
#                            [--sign "Developer ID Application: Name (TEAMID)"]
#                            [--installer-sign "Developer ID Installer: Name (TEAMID)"]
#                            [--notarize <keychain-profile>] [--pkg] [--dmg] [--zip]
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
ARCH="$(uname -m)"
ZIP=0
DMG=0
PKG=0
IDENTITY="${SJTU_CANVAS_SIGN_IDENTITY:--}"
INSTALLER_IDENTITY="${SJTU_CANVAS_INSTALLER_IDENTITY:-}"
NOTARY_PROFILE="${SJTU_CANVAS_NOTARY_PROFILE:-}"
BUNDLE_ID="io.github.uniseem.sjtu-canvas-downloader"
APP_NAME="SJTU Canvas Downloader"
EXECUTABLE="SJTUCanvasDownloader"

while [ $# -gt 0 ]; do
  case "$1" in
    --arch) ARCH="$2"; shift 2 ;;
    --zip) ZIP=1; shift ;;
    --dmg) DMG=1; shift ;;
    --pkg) PKG=1; shift ;;
    --sign) IDENTITY="$2"; shift 2 ;;
    --installer-sign) INSTALLER_IDENTITY="$2"; shift 2 ;;
    --notarize) NOTARY_PROFILE="$2"; shift 2 ;;
    -h|--help) sed -n '2,31p' "$0"; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

case "$ARCH" in
  arm64|aarch64) ARCH=arm64; RUST_TARGET=aarch64-apple-darwin; WANTS_ARM=true; OTHER_KIND="Intel 芯片的 Mac 请下载 x86_64 版" ;;
  x86_64) RUST_TARGET=x86_64-apple-darwin; WANTS_ARM=false; OTHER_KIND="Apple 芯片的 Mac 请下载 arm64 版" ;;
  *) echo "unsupported architecture: $ARCH" >&2; exit 2 ;;
esac
if [ -n "$NOTARY_PROFILE" ] && [ "$IDENTITY" = "-" ]; then
  echo "--notarize needs a Developer ID signature (--sign)" >&2
  exit 2
fi
if [ -n "$INSTALLER_IDENTITY" ] && [ "$IDENTITY" = "-" ]; then
  echo "--installer-sign needs the app signed with a Developer ID as well (--sign)" >&2
  exit 2
fi

VERSION="$(sed -n 's/^version = "\(.*\)"$/\1/p' "$REPO/engine/Cargo.toml" | head -n 1)"
BUILD="$(git -C "$REPO" rev-list --count HEAD 2>/dev/null || echo 1)"
DIST="$HERE/dist"
APP="$DIST/$APP_NAME.app"
export MACOSX_DEPLOYMENT_TARGET=14.0

echo "==> Engine ($RUST_TARGET, release)"
if command -v rustup >/dev/null 2>&1; then
  rustup target add "$RUST_TARGET" >/dev/null
fi
cargo build --release --locked --target "$RUST_TARGET" --manifest-path "$REPO/engine/Cargo.toml"
ENGINE="$REPO/engine/target/$RUST_TARGET/release/sjtu-canvas-engine"

echo "==> SwiftUI app"
swift build -c release --arch "$ARCH" --package-path "$HERE"
BIN="$(swift build -c release --arch "$ARCH" --package-path "$HERE" --show-bin-path)"

echo "==> $APP"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources/zh-Hans.lproj"
cp "$BIN/$EXECUTABLE" "$APP/Contents/MacOS/$EXECUTABLE"
cp "$ENGINE" "$APP/Contents/MacOS/sjtu-canvas-engine"
sed -e "s/__VERSION__/$VERSION/g" -e "s/__BUILD__/$BUILD/g" "$HERE/Resources/Info.plist" > "$APP/Contents/Info.plist"
plutil -lint "$APP/Contents/Info.plist" >/dev/null
printf 'APPL????' > "$APP/Contents/PkgInfo"
iconutil -c icns "$HERE/Resources/AppIcon.iconset" -o "$APP/Contents/Resources/AppIcon.icns"
cp "$REPO/LICENSE" "$REPO/THIRD_PARTY_NOTICES.md" "$APP/Contents/Resources/"

# Every binary must run on the app's minimum macOS version.
for binary in "$APP/Contents/MacOS/$EXECUTABLE" "$APP/Contents/MacOS/sjtu-canvas-engine"; do
  minos="$(otool -l "$binary" | awk '/LC_BUILD_VERSION/{found=1} found && /minos/{print $2; exit}')"
  case "$minos" in
    ""|1[0-3].*|14.0|14) ;;
    *) echo "error: $(basename "$binary") needs macOS $minos (the app supports 14.0)" >&2; exit 1 ;;
  esac
done
# Extended attributes (quarantine, Finder info) are not allowed in a sealed bundle.
xattr -cr "$APP"

echo "==> Code signature ($IDENTITY)"
if [ "$IDENTITY" = "-" ]; then
  # Ad-hoc, inside out: arm64 binaries are only linker-signed; every binary
  # gets a real signature and the bundle a sealed one, so it verifies strictly.
  codesign --force --sign - --timestamp=none "$APP/Contents/MacOS/sjtu-canvas-engine"
  codesign --force --sign - --timestamp=none "$APP"
else
  # Developer ID with the hardened runtime and a secure timestamp, inside out.
  codesign --force --options runtime --timestamp --sign "$IDENTITY" "$APP/Contents/MacOS/sjtu-canvas-engine"
  codesign --force --options runtime --timestamp --sign "$IDENTITY" "$APP"
fi
codesign --verify --deep --strict --verbose=2 "$APP"
codesign --verify --strict "$APP/Contents/MacOS/sjtu-canvas-engine"

notarize() {
  # notarize <file to submit> <what to staple>
  local submission="$1" target="$2" result
  result="$(mktemp)"
  echo "==> Notarizing $(basename "$submission") (this can take a few minutes)"
  xcrun notarytool submit "$submission" --keychain-profile "$NOTARY_PROFILE" --wait --output-format json > "$result"
  local status id
  status="$(plutil -extract status raw -o - "$result" 2>/dev/null || echo unknown)"
  id="$(plutil -extract id raw -o - "$result" 2>/dev/null || echo "")"
  rm -f "$result"
  if [ "$status" != "Accepted" ]; then
    echo "notarization failed ($status); details: xcrun notarytool log $id --keychain-profile $NOTARY_PROFILE" >&2
    exit 1
  fi
  xcrun stapler staple "$target"
  xcrun stapler validate "$target"
}

if [ -n "$NOTARY_PROFILE" ]; then
  SUBMISSION="$DIST/.notarize-$ARCH.zip"
  ditto -c -k --sequesterRsrc --keepParent "$APP" "$SUBMISSION"
  notarize "$SUBMISSION" "$APP"
  rm -f "$SUBMISSION"
  spctl --assess --type execute --verbose=2 "$APP"
fi

NOTE="$HERE/Resources/InstallNote.txt"

if [ "$PKG" = 1 ]; then
  PACKAGE="$DIST/SJTUCanvasDownloader-macos-$ARCH.pkg"
  rm -f "$PACKAGE"
  echo "==> $PACKAGE"
  WORK="$(mktemp -d)"
  mkdir -p "$WORK/root" "$WORK/resources"
  ditto "$APP" "$WORK/root/$APP_NAME.app"
  # Always install into /Applications, replacing an older copy entirely.
  pkgbuild --analyze --root "$WORK/root" "$WORK/components.plist" >/dev/null
  plutil -replace 0.BundleIsRelocatable -bool NO "$WORK/components.plist"
  plutil -replace 0.BundleIsVersionChecked -bool NO "$WORK/components.plist"
  plutil -replace 0.BundleOverwriteAction -string upgrade "$WORK/components.plist"
  pkgbuild --root "$WORK/root" --component-plist "$WORK/components.plist" \
    --identifier "$BUNDLE_ID" --version "$VERSION" --install-location /Applications \
    "$WORK/component.pkg" >/dev/null
  cp "$HERE/Resources/installer/welcome.html" "$HERE/Resources/installer/conclusion.html" "$WORK/resources/"
  cat > "$WORK/distribution.xml" <<XML
<?xml version="1.0" encoding="utf-8"?>
<installer-gui-script minSpecVersion="2">
    <title>$APP_NAME</title>
    <welcome file="welcome.html" mime-type="text/html"/>
    <conclusion file="conclusion.html" mime-type="text/html"/>
    <options customize="never" require-scripts="false" hostArchitectures="x86_64,arm64"/>
    <domains enable_localSystem="true"/>
    <installation-check script="matchingMac()"/>
    <script><![CDATA[
function matchingMac() {
    // Set on Apple silicon even when asked from a Rosetta process; Intel
    // Macs do not have the key.
    var isArm = false;
    try {
        isArm = system.sysctl('hw.optional.arm64') == 1;
    } catch (error) {
    }
    if (isArm == $WANTS_ARM) {
        return true;
    }
    my.result.type = 'Fatal';
    my.result.title = '这个安装包不适用于这台 Mac';
    my.result.message = '${OTHER_KIND}。';
    return false;
}
]]></script>
    <volume-check>
        <allowed-os-versions>
            <os-version min="14.0"/>
        </allowed-os-versions>
    </volume-check>
    <choices-outline>
        <line choice="default">
            <line choice="$BUNDLE_ID"/>
        </line>
    </choices-outline>
    <choice id="default"/>
    <choice id="$BUNDLE_ID" visible="false">
        <pkg-ref id="$BUNDLE_ID"/>
    </choice>
    <pkg-ref id="$BUNDLE_ID" version="$VERSION" onConclusion="none">component.pkg</pkg-ref>
</installer-gui-script>
XML
  if [ -n "$INSTALLER_IDENTITY" ]; then
    productbuild --distribution "$WORK/distribution.xml" --resources "$WORK/resources" --package-path "$WORK" \
      --sign "$INSTALLER_IDENTITY" --timestamp "$PACKAGE"
    pkgutil --check-signature "$PACKAGE"
    if [ -n "$NOTARY_PROFILE" ]; then
      notarize "$PACKAGE" "$PACKAGE"
    fi
  else
    productbuild --distribution "$WORK/distribution.xml" --resources "$WORK/resources" --package-path "$WORK" "$PACKAGE"
  fi
  rm -rf "$WORK"
  # The package must contain the app exactly as signed.
  CHECK="$(mktemp -d)"
  pkgutil --expand-full "$PACKAGE" "$CHECK/expanded"
  codesign --verify --deep --strict "$CHECK/expanded/component.pkg/Payload/$APP_NAME.app"
  rm -rf "$CHECK"
fi

if [ "$ZIP" = 1 ]; then
  ARCHIVE="$DIST/SJTUCanvasDownloader-macos-$ARCH.zip"
  rm -f "$ARCHIVE"
  echo "==> $ARCHIVE"
  if [ "$IDENTITY" = "-" ]; then
    # The zip holds the app and the note for opening an unsigned app.
    STAGE="$(mktemp -d)"
    ditto "$APP" "$STAGE/$APP_NAME.app"
    cp "$NOTE" "$STAGE/安装说明.txt"
    ditto -c -k --sequesterRsrc "$STAGE" "$ARCHIVE"
    rm -rf "$STAGE"
  else
    ditto -c -k --sequesterRsrc --keepParent "$APP" "$ARCHIVE"
  fi
fi

if [ "$DMG" = 1 ]; then
  IMAGE="$DIST/SJTUCanvasDownloader-macos-$ARCH.dmg"
  rm -f "$IMAGE"
  echo "==> $IMAGE"
  STAGE="$(mktemp -d)"
  ditto "$APP" "$STAGE/$APP_NAME.app"
  ln -s /Applications "$STAGE/Applications"
  if [ "$IDENTITY" = "-" ]; then
    cp "$NOTE" "$STAGE/安装说明.txt"
  fi
  hdiutil create -volname "$APP_NAME" -srcfolder "$STAGE" -fs HFS+ -format UDZO -ov "$IMAGE" >/dev/null
  rm -rf "$STAGE"
  if [ "$IDENTITY" != "-" ]; then
    codesign --force --timestamp --sign "$IDENTITY" "$IMAGE"
    if [ -n "$NOTARY_PROFILE" ]; then
      notarize "$IMAGE" "$IMAGE"
    fi
  fi
fi

echo "Done: $APP ($(du -sh "$APP" | cut -f1))"
