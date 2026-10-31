#!/usr/bin/env bash
# Build dist/wt++.app and dist/wtpp-<version>-macos-<arch>.zip. Run on a Mac.
set -euo pipefail

cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
arch="$(uname -m)"
cargo build --release

app="dist/wt++.app"
rm -rf "$app" && mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/wtpp "$app/Contents/MacOS/wtpp"
cp LICENSE assets/fonts/LICENSE-Cascadia.txt "$app/Contents/Resources/"

# Icon: every size macOS asks for, from the 1024px PNG (which has the margin
# macOS icons have).
iconset="$(mktemp -d)/wtpp.iconset"
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
    sips -z $size $size assets/wtpp-1024.png --out "$iconset/icon_${size}x${size}.png" >/dev/null
    sips -z $((size * 2)) $((size * 2)) assets/wtpp-1024.png --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/wtpp.icns"

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>wt++</string>
    <key>CFBundleDisplayName</key><string>wt++</string>
    <key>CFBundleIdentifier</key><string>dev.wtpp.terminal</string>
    <key>CFBundleExecutable</key><string>wtpp</string>
    <key>CFBundleIconFile</key><string>wtpp</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundleVersion</key><string>$version</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

# Not notarized (that needs an Apple developer ID): signed ad hoc so it runs;
# a downloaded copy needs "Open" from the Finder context menu the first time.
codesign --force --deep --sign - "$app"
(cd dist && rm -f "wtpp-$version-macos-$arch.zip" && ditto -c -k --keepParent "wt++.app" "wtpp-$version-macos-$arch.zip")
ls -la dist
