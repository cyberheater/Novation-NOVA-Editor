#!/usr/bin/env bash
# Build "dist/Nova Editor.app" (ad-hoc signed). Run from anywhere.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH"

cargo build --release

VERSION=$(sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -1)
APP="dist/Nova Editor.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/novaedit "$APP/Contents/MacOS/novaedit"

# icon (optional: skipped if anything fails)
ICON_KEY=""
TMP=$(mktemp -d)
if python3 scripts/make_icon.py "$TMP/icon.png" 2>/dev/null; then
  mkdir "$TMP/icon.iconset"
  ok=1
  for s in 16 32 128 256 512; do
    sips -z $s $s "$TMP/icon.png" --out "$TMP/icon.iconset/icon_${s}x${s}.png" >/dev/null || ok=0
    d=$((s * 2))
    sips -z $d $d "$TMP/icon.png" --out "$TMP/icon.iconset/icon_${s}x${s}@2x.png" >/dev/null || ok=0
  done
  if [ $ok = 1 ] && iconutil -c icns "$TMP/icon.iconset" -o "$APP/Contents/Resources/AppIcon.icns"; then
    ICON_KEY="<key>CFBundleIconFile</key><string>AppIcon</string>"
  fi
fi
rm -rf "$TMP"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Nova Editor</string>
  <key>CFBundleDisplayName</key><string>Nova Editor</string>
  <key>CFBundleIdentifier</key><string>com.local.novaedit</string>
  <key>CFBundleExecutable</key><string>novaedit</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  $ICON_KEY
</dict>
</plist>
PLIST
plutil -lint "$APP/Contents/Info.plist"

codesign --force --deep -s - "$APP"
echo "Built: $(pwd)/$APP"
