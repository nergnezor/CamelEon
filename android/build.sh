#!/usr/bin/env bash
# Builds the Android app into dist/camel-eon.apk (arm64, Android 9+).
#
# Needs the Android SDK and NDK (ANDROID_HOME and ANDROID_NDK_ROOT), the
# aarch64-linux-android Rust target and cargo-apk:
#   rustup target add aarch64-linux-android
#   cargo install cargo-apk --locked
# Install with `adb install -r dist/camel-eon.apk`.
set -euo pipefail
cd "$(dirname "$0")/.."

# Signed with the standard Android debug key, which is fine for sideloading.
# An APK signed with a different key has to be uninstalled before installing.
keystore="${CARGO_APK_RELEASE_KEYSTORE:-$HOME/.android/debug.keystore}"
if [ ! -f "$keystore" ]; then
    mkdir -p "$(dirname "$keystore")"
    keytool -genkeypair -keystore "$keystore" -storepass android -keypass android \
        -alias androiddebugkey -dname "CN=Android Debug,O=Android,C=US" \
        -keyalg RSA -keysize 2048 -validity 10000 -noprompt
fi
export CARGO_APK_RELEASE_KEYSTORE="$keystore"
export CARGO_APK_RELEASE_KEYSTORE_PASSWORD="${CARGO_APK_RELEASE_KEYSTORE_PASSWORD:-android}"

# Android 15+ devices can use 16 KB memory pages, and a library aligned to
# 4 KB pages fails to load there (Play requires 16 KB too). cargo-apk keeps
# RUSTFLAGS but overrides the rustflags in .cargo/config.toml.
export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-C link-arg=-Wl,-z,max-page-size=16384"

# Same dependency versions as the other builds.
cp Cargo.lock android/Cargo.lock
(cd android && cargo apk build --release)
mkdir -p dist
cp android/target/release/apk/camel-eon.apk dist/
