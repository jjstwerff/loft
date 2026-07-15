#!/bin/bash
# Assemble + sign a NativeActivity APK around the loft-built .so.
# Env: SDK (Android SDK root), JAVA_HOME, SPIKE (this crate dir), SO (the cdylib).
set -e
: "${SDK:?set SDK to the Android SDK root}"
: "${JAVA_HOME:?set JAVA_HOME}"
: "${SPIKE:?set SPIKE to the b2-spike crate dir}"
BT="$SDK/build-tools/34.0.0"
export PATH="$JAVA_HOME/bin:$PATH"
ANDROID_JAR="$SDK/platforms/android-34/android.jar"
SO="${SO:-$SPIKE/target/x86_64-linux-android/release/libloft_android_spike.so}"
ABI="${ABI:-x86_64}"
WORK="${WORK:-$SPIKE/apk}"

mkdir -p "$WORK"; cd "$WORK"
# One-time: a debug keystore next to the APK.
[ -f debug.keystore ] || "$JAVA_HOME/bin/keytool" -genkeypair -v -keystore debug.keystore \
  -alias androiddebugkey -storepass android -keypass android -keyalg RSA -keysize 2048 \
  -validity 10000 -dname "CN=Android Debug,O=Android,C=US"

rm -f base.apk aligned.apk app-signed.apk
rm -rf staging; mkdir -p "staging/lib/$ABI"
cp "$SO" "staging/lib/$ABI/libloft_android_spike.so"

# 1. aapt2 link — manifest only (labels are literals, no resources)
"$BT/aapt2" link -o base.apk -I "$ANDROID_JAR" --manifest "$SPIKE/AndroidManifest.xml" \
  --min-sdk-version 24 --target-sdk-version 34 --version-code 1 --version-name 1.0
# 2. add the native lib, 3. align, 4. sign
( cd staging && zip -qr ../base.apk lib )
"$BT/zipalign" -f -p 4 base.apk aligned.apk
"$BT/apksigner" sign --ks debug.keystore --ks-pass pass:android --key-pass pass:android \
  --min-sdk-version 24 --out app-signed.apk aligned.apk
"$BT/apksigner" verify --min-sdk-version 24 app-signed.apk && echo "APK signature OK -> $WORK/app-signed.apk"
