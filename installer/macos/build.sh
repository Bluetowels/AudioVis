#!/bin/bash
# Builds the release app and the disk image, on a Mac:
#   installer/output/AudioVis.app
#   installer/output/AudioVis-<version>.dmg
# The app is a universal binary (Apple silicon and Intel), so both Rust
# targets must be installed:
#   rustup target add aarch64-apple-darwin x86_64-apple-darwin
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
out="$root/installer/output"

version="$(sed -n 's/^version *= *"\(.*\)"/\1/p' "$root/visualiser/Cargo.toml" | head -n 1)"
target="${CARGO_TARGET_DIR:-$root/visualiser/target}"

# System audio capture uses Core Audio taps, which need macOS 14.6.
export MACOSX_DEPLOYMENT_TARGET=14.6

cd "$root/visualiser"
binaries=()
for arch in aarch64 x86_64; do
    cargo build --release --target "$arch-apple-darwin"
    binaries+=("$target/$arch-apple-darwin/release/audiovis")
done

app="$out/AudioVis.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
lipo -create -output "$app/Contents/MacOS/audiovis" "${binaries[@]}"
sed -e "s/@VERSION@/$version/g" -e "s/@MACOS_MIN@/$MACOSX_DEPLOYMENT_TARGET/g" "$here/Info.plist" > "$app/Contents/Info.plist"

# Signed for this Mac only ("ad hoc") unless CODESIGN_IDENTITY names a
# Developer ID certificate. An ad hoc app downloaded from the internet has
# to be allowed once in System Settings > Privacy & Security.
codesign --force --sign "${CODESIGN_IDENTITY:--}" "$app"

# The disk image holds the app and a shortcut to drag it onto.
staging="$(mktemp -d)"
trap 'rm -rf "$staging"' EXIT
cp -R "$app" "$staging/"
ln -s /Applications "$staging/Applications"
dmg="$out/AudioVis-$version.dmg"
rm -f "$dmg"
# hdiutil now and then reports "Resource busy" on a busy machine; try again.
for attempt in 1 2 3; do
    hdiutil create -volname "AudioVis $version" -srcfolder "$staging" -format UDZO -ov "$dmg" && break
    [ "$attempt" = 3 ] && { echo "hdiutil failed" >&2; exit 1; }
    sleep 5
done

echo "App: $app"
echo "Disk image: $dmg"
