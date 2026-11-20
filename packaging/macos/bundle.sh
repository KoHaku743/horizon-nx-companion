#!/bin/sh
# Builds "Horizon NX Companion.app" (Apple silicon and Intel in one) and zips it.
# Usage: packaging/macos/bundle.sh [version]
set -eu
cd "$(dirname "$0")/../.."
version="${1:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)}"
for target in aarch64-apple-darwin x86_64-apple-darwin; do
  rustup target add "$target" >/dev/null 2>&1 || true
  cargo build --release --locked -p horizon-nx-companion --target "$target"
done
app="dist/Horizon NX Companion.app"
rm -rf dist && mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
lipo -create -output "$app/Contents/MacOS/horizon-nx-companion" \
  target/aarch64-apple-darwin/release/horizon-nx-companion \
  target/x86_64-apple-darwin/release/horizon-nx-companion
sed "s/VERSION/$version/g" packaging/macos/Info.plist > "$app/Contents/Info.plist"
iconset="dist/AppIcon.iconset"
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
  sips -z $size $size packaging/macos/icon.png --out "$iconset/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z $double $double packaging/macos/icon.png --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/AppIcon.icns"
rm -rf "$iconset"
# Not notarized: an ad-hoc signature, so macOS runs it after "Open" from the right-click menu.
codesign --force --deep --sign - "$app"
(cd dist && ditto -c -k --keepParent "Horizon NX Companion.app" "horizon-nx-companion-macos.zip")
echo "dist/horizon-nx-companion-macos.zip"
