#!/bin/sh
# Cargo runner for `pnpm tauri dev` on macOS. A bare binary has no bundle, so the Dock shows
# a blank icon and names the tile after whichever app launched it. Running the binary from a
# throwaway Bayonet.app next to it gives the Dock the product name and icon.
set -eu

bin=$1
shift
dir=$(cd "$(dirname "$bin")" && pwd)
root=$(cd "$(dirname "$0")/.." && pwd)
app="$dir/Bayonet.app"
contents="$app/Contents"
id="io.github.skuirrels.bayonet.dev.$(printf %s "$root" | shasum | cut -c1-12)"

mkdir -p "$contents/MacOS" "$contents/Resources"
# Clone on APFS (instant), plain copy elsewhere.
cp -c "$bin" "$contents/MacOS/bayonet" 2>/dev/null || cp "$bin" "$contents/MacOS/bayonet"
cp "$root/src-tauri/icons/icon.icns" "$contents/Resources/icon.icns"
# Remote development looks for the server binary beside the app's executable.
ln -sfn "$dir/openidea-server" "$contents/MacOS/openidea-server"

cat >"$contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleDisplayName</key><string>Bayonet</string>
  <key>CFBundleExecutable</key><string>bayonet</string>
  <key>CFBundleIconFile</key><string>icon.icns</string>
  <key>CFBundleIdentifier</key><string>$id</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>Bayonet</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.0.0-dev</string>
  <key>CFBundleVersion</key><string>0</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF
# WebKit keys web storage by bundle ID; the bare binary used its file name. Carry it over once.
webkit="$HOME/Library/WebKit"
if [ ! -e "$webkit/$id" ] && [ -d "$webkit/openidea" ]; then
  cp -Rc "$webkit/openidea" "$webkit/$id" 2>/dev/null || cp -R "$webkit/openidea" "$webkit/$id"
fi
# Bundle contents changed under the same path: make Launch Services re-read the icon and name.
touch "$app"

exec "$contents/MacOS/bayonet" "$@"
