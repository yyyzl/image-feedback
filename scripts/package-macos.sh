#!/usr/bin/env bash
set -euo pipefail
[[ "$(uname -s)" == Darwin ]] || { echo 'Run this script on macOS' >&2; exit 1; }
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-13.0}"
export SDKROOT="$(xcrun --sdk macosx --show-sdk-path)"
export SLINT_EMBED_RESOURCES=true
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
VERSION="$(cargo metadata --no-deps --locked --format-version 1 | python3 -c 'import json,sys;print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="image-feedback-slint"))')"
WORK="$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/imagefeedback-package.XXXXXX")"
STAGE="$WORK/stage"
APP="$STAGE/Image Feedback.app"
RESOURCES="$APP/Contents/Resources"
BINARY="$APP/Contents/MacOS/ImageFeedback"
ICONSET="$WORK/AppIcon.iconset"
mkdir -p "$APP/Contents/MacOS" "$RESOURCES/licenses" "$ICONSET" "$ROOT/dist/macos"
for TARGET in aarch64-apple-darwin x86_64-apple-darwin; do
  cargo build --release --locked --bin image-feedback-slint --target "$TARGET"
done
xcrun lipo -create target/aarch64-apple-darwin/release/image-feedback-slint target/x86_64-apple-darwin/release/image-feedback-slint -output "$BINARY"
chmod 755 "$BINARY"
xcrun lipo "$BINARY" -verify_arch arm64 x86_64
for SIZE in 16 32 128 256 512; do
  sips -z "$SIZE" "$SIZE" assets/app-icon.png --out "$ICONSET/icon_${SIZE}x${SIZE}.png" >/dev/null
  DOUBLE=$((SIZE * 2))
  sips -z "$DOUBLE" "$DOUBLE" assets/app-icon.png --out "$ICONSET/icon_${SIZE}x${SIZE}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$RESOURCES/AppIcon.icns"
cp LICENSE THIRD_PARTY_NOTICES.md Cargo.lock "$RESOURCES/licenses/"
cp -R licenses/. "$RESOURCES/licenses/"
python3 scripts/collect-licenses.py --target aarch64-apple-darwin --output "$RESOURCES/licenses/THIRD_PARTY_LICENSES.html"
# Include the x86 dependency inventory as well; the bundle has both executable slices.
python3 scripts/collect-licenses.py --target x86_64-apple-darwin --output "$RESOURCES/licenses/THIRD_PARTY_LICENSES-x86_64.html"
cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>io.github.yyyzl.imagefeedback</string>
<key>CFBundleName</key><string>Image Feedback</string>
<key>CFBundleDisplayName</key><string>Image Feedback</string>
<key>CFBundleExecutable</key><string>ImageFeedback</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>$VERSION</string>
<key>CFBundleVersion</key><string>$VERSION</string>
<key>CFBundleIconFile</key><string>AppIcon.icns</string>
<key>LSMinimumSystemVersion</key><string>$MACOSX_DEPLOYMENT_TARGET</string>
<key>LSApplicationCategoryType</key><string>public.app-category.graphics-design</string>
<key>LSUIElement</key><true/>
<key>NSHighResolutionCapable</key><true/>
<key>NSSupportsAutomaticGraphicsSwitching</key><true/>
<key>NSHumanReadableCopyright</key><string>Copyright 2026 Image Feedback contributors</string>
</dict></plist>
EOF
plutil -lint "$APP/Contents/Info.plist"
for ARCH in arm64 x86_64; do
  xcrun otool -arch "$ARCH" -L "$BINARY" > "$WORK/dependencies-$ARCH.txt"
  cat "$WORK/dependencies-$ARCH.txt"
  while IFS= read -r DEPENDENCY; do
    case "$DEPENDENCY" in /usr/lib/*|/System/Library/*) ;; *) echo "Unbundled non-system dependency: $DEPENDENCY" >&2; exit 1 ;; esac
  done < <(awk 'NR>1 && NF {print $1}' "$WORK/dependencies-$ARCH.txt")
  xcrun vtool -arch "$ARCH" -show-build "$BINARY"
done
# No Developer ID credentials are configured. Integrity signing is NOT Apple notarization.
codesign --force --sign - --timestamp=none "$APP"
codesign --verify --deep --strict --verbose=2 "$APP"
ln -s /Applications "$STAGE/Applications"
cat > "$STAGE/READ-ME-FIRST.txt" <<'EOF'
Image Feedback
Drag Image Feedback.app to Applications, then run it from Applications.
This open-source build is ad-hoc signed, not Apple-notarized.
On first launch, macOS may require Open Anyway in System Settings > Privacy & Security.
The app contains its runtime; Rust, Qt, Node.js and Homebrew are not required.
The red native close button closes the editor; the menu-bar entry is independent.
EOF
DMG="$ROOT/dist/macos/ImageFeedback-$VERSION-macos-universal-adhoc.dmg"
[[ ! -e "$DMG" ]] || { echo "Refusing to replace existing DMG: $DMG" >&2; exit 1; }
hdiutil create -volname 'Image Feedback' -srcfolder "$STAGE" -format UDZO "$DMG"
hdiutil verify "$DMG"
(cd "$(dirname "$DMG")" && shasum -a 256 "$(basename "$DMG")" > "$(basename "$DMG").sha256")
printf 'DMG ready (not notarized): %s\n' "$DMG"
