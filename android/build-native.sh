#!/bin/bash
# Builds the Rust half of the Android app and puts it where Gradle packs it
# into the APK:
#   android/app/src/main/jniLibs/<abi>/libaudiovis.so
# for phones and tablets (arm64-v8a) and for the emulator (x86_64).
# Needs the Android NDK (ANDROID_NDK_HOME) and both Rust targets:
#   rustup target add aarch64-linux-android x86_64-linux-android
# Then, to make the APK:
#   gradle -p android assembleRelease
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/.." && pwd)"

ndk="${ANDROID_NDK_HOME:-${ANDROID_NDK_LATEST_HOME:-}}"
if [ ! -d "$ndk/toolchains/llvm/prebuilt" ]; then
    echo "Set ANDROID_NDK_HOME to the folder of an installed Android NDK." >&2
    exit 1
fi
host="$(ls "$ndk/toolchains/llvm/prebuilt" | head -n 1)"
bin="$ndk/toolchains/llvm/prebuilt/$host/bin"

# Android 10, the oldest version the app runs on (minSdk in app/build.gradle).
api=29
target="${CARGO_TARGET_DIR:-$root/visualiser/target}"

# Leave out the symbol tables, which would triple the size of the download.
export CARGO_PROFILE_RELEASE_STRIP=symbols

cd "$root/visualiser"
for pair in aarch64-linux-android:arm64-v8a x86_64-linux-android:x86_64; do
    triple="${pair%%:*}"
    abi="${pair##*:}"
    upper="$(echo "$triple" | tr 'a-z-' 'A-Z_')"
    lower="${triple//-/_}"
    export "CARGO_TARGET_${upper}_LINKER=$bin/$triple$api-clang"
    # Newer phones load libraries in 16 KB pages; this suits those and the older 4 KB ones.
    export "CARGO_TARGET_${upper}_RUSTFLAGS=-C link-arg=-Wl,-z,max-page-size=16384"
    export "CC_$lower=$bin/$triple$api-clang"
    export "AR_$lower=$bin/llvm-ar"

    cargo rustc --lib --crate-type cdylib --release --target "$triple"

    mkdir -p "$here/app/src/main/jniLibs/$abi"
    cp "$target/$triple/release/libaudiovis.so" "$here/app/src/main/jniLibs/$abi/"
done

ls -l "$here"/app/src/main/jniLibs/*/libaudiovis.so
