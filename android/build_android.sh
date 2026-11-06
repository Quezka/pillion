#!/usr/bin/env bash
# Builds the Android app with the SDK's own tools. The finished file is Pillion.apk.
# Signing: PILLION_KEYSTORE and PILLION_KEYSTORE_PASSWORD in CI, otherwise android/pillion.jks and
# android/keystore.properties, neither of which is ever committed.
set -euo pipefail

SDK="${ANDROID_HOME:-$HOME/Android/Sdk}"
NDK="$SDK/ndk/27.2.12479018"
BUILD_TOOLS="$SDK/build-tools/35.0.0"
PLATFORM_JAR="$SDK/platforms/android-35/android.jar"
MIN_SDK=26
TARGET_SDK=34
LIB=libpillion.so

export PATH="$HOME/.cargo/bin:$PATH"
if ! command -v javac >/dev/null; then
    JAVA_HOME="$(ls -d "$HOME"/.config/craft-anything/engines/jdk/jdk-* | head -1)"
    export JAVA_HOME PATH="$JAVA_HOME/bin:$PATH"
fi

HERE="$(cd "$(dirname "$0")" && pwd)"
APP="$(dirname "$HERE")"
OUT="$HERE/build"
mkdir -p "$OUT"
find "$OUT" -mindepth 1 -maxdepth 1 ! -name 'keep' -exec rm -rf {} +
mkdir -p "$OUT/gen" "$OUT/classes" "$OUT/dex"

CLANG="$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin"
build_native() {
    local triple="$1" abi="$2" env_triple="$3"
    local linker="$CLANG/$triple$MIN_SDK-clang"
    (
        cd "$APP"
        export "CARGO_TARGET_${env_triple}_LINKER=$linker"
        export "CC_${triple//-/_}=$linker"
        export "AR_${triple//-/_}=$CLANG/llvm-ar"
        # Android 15 phones can use 16 KB memory pages.
        export "CARGO_TARGET_${env_triple}_RUSTFLAGS=-C link-arg=-Wl,-z,max-page-size=16384"
        cargo build --release --lib --target "$triple"
    )
    mkdir -p "$OUT/apk/lib/$abi"
    cp "$APP/target/$triple/release/$LIB" "$OUT/apk/lib/$abi/$LIB"
}
# The natural voice's engine: Apache 2.0, with ONNX Runtime (MIT) and espeak-ng (GPL 3) inside. It's kept out of the
# repository for its size, and is checked against this sum wherever it's fetched. Version 2 dropped espeak-ng, which these
# voices need to turn words into sounds, so this stays on the last version that has it.
SHERPA_VERSION=1.13.8
SHERPA_SUM=b22c3fc1b6a45666d28892bb2f7694beeb77a8362d7ebd77c1a5431ec9435471
SHERPA="$HERE/third_party/sherpa-onnx-$SHERPA_VERSION.aar"
if ! echo "$SHERPA_SUM  $SHERPA" | sha256sum -c --status 2>/dev/null; then
    mkdir -p "$HERE/third_party"
    curl -fsSL -o "$SHERPA" "https://github.com/k2-fsa/sherpa-onnx/releases/download/v$SHERPA_VERSION/sherpa-onnx-static-link-onnxruntime-$SHERPA_VERSION.aar"
    echo "$SHERPA_SUM  $SHERPA" | sha256sum -c --status
fi

# Phones are arm64. PILLION_ARM_ONLY leaves out the x86_64 parts that only test phones on a computer use, which saves 10 MB.
ABIS="arm64-v8a"
build_native aarch64-linux-android arm64-v8a AARCH64_LINUX_ANDROID
if [ -z "${PILLION_ARM_ONLY:-}" ]; then
    ABIS="$ABIS x86_64"
    build_native x86_64-linux-android x86_64 X86_64_LINUX_ANDROID
fi
for abi in $ABIS; do
    unzip -qoj "$SHERPA" "jni/$abi/libsherpa-onnx-jni.so" -d "$OUT/apk/lib/$abi"
done

"$BUILD_TOOLS/aapt2" compile --dir "$HERE/res" -o "$OUT/res.zip"
# The version comes from Cargo.toml, and each release must count higher for Android to accept the update.
VERSION="$(grep -m1 -oP '^version = "\K[^"]+' "$APP/Cargo.toml")"
IFS=. read -r MAJOR MINOR PATCH <<< "$VERSION"
VERSION_CODE=$((MAJOR * 10000 + MINOR * 100 + PATCH))

"$BUILD_TOOLS/aapt2" link -I "$PLATFORM_JAR" --manifest "$HERE/AndroidManifest.xml" \
    --min-sdk-version "$MIN_SDK" --target-sdk-version "$TARGET_SDK" \
    --version-code "$VERSION_CODE" --version-name "$VERSION" \
    --java "$OUT/gen" -o "$OUT/apk/unsigned.apk" "$OUT/res.zip"

javac -nowarn -Xlint:-options --release 8 -classpath "$PLATFORM_JAR" -d "$OUT/classes" \
    $(find "$HERE/java" "$OUT/gen" -name '*.java')
"$BUILD_TOOLS/d8" --lib "$PLATFORM_JAR" --min-api "$MIN_SDK" --output "$OUT/apk" \
    $(find "$OUT/classes" -name '*.class')

(
    cd "$OUT/apk"
    mkdir -p assets
    cp "$APP/LICENSE" "$APP/THIRD_PARTY_NOTICES.txt" assets/
    "$BUILD_TOOLS/aapt" add unsigned.apk classes.dex $(find lib -name '*.so') assets/LICENSE assets/THIRD_PARTY_NOTICES.txt >/dev/null
)
"$BUILD_TOOLS/zipalign" -f -p 4 "$OUT/apk/unsigned.apk" "$OUT/aligned.apk"

KEYSTORE="${PILLION_KEYSTORE:-$HERE/pillion.jks}"
if [ -z "${PILLION_KEYSTORE_PASSWORD:-}" ]; then
    PILLION_KEYSTORE_PASSWORD="$(grep -oP '^storePassword=\K.*' "$HERE/keystore.properties")"
fi
export PILLION_KEYSTORE_PASSWORD
"$BUILD_TOOLS/apksigner" sign --ks "$KEYSTORE" --ks-pass env:PILLION_KEYSTORE_PASSWORD \
    --key-pass env:PILLION_KEYSTORE_PASSWORD --out "$OUT/Pillion.apk" "$OUT/aligned.apk"

cp "$OUT/Pillion.apk" "$APP/Pillion.apk"
ls -la "$APP/Pillion.apk"
