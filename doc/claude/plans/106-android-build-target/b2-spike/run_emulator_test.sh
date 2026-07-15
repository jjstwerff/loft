#!/bin/bash
# Boot a headless KVM emulator, install the APK, launch it, prove android_main
# ran via logcat. Self-contained: kills the emulator at the end.
# Env: SDK (Android SDK root), JAVA_HOME, APK (signed APK path).
set -u
: "${SDK:?set SDK to the Android SDK root}"
: "${JAVA_HOME:?set JAVA_HOME}"
export ANDROID_SDK_ROOT="$SDK" ANDROID_HOME="$SDK"
export PATH="$JAVA_HOME/bin:$SDK/platform-tools:$SDK/emulator:$SDK/cmdline-tools/latest/bin:$PATH"
APK="${APK:?set APK to the signed APK path}"
AVD="${AVD:-loft_spike}"
IMG="system-images;android-34;google_apis;x86_64"

echo "no" | avdmanager create avd -n "$AVD" -k "$IMG" --force 2>&1 | tail -1
"$SDK/emulator/emulator" -avd "$AVD" -no-window -no-audio -no-boot-anim \
  -gpu swiftshader_indirect -no-snapshot -accel on -wipe-data > /tmp/loft_emu.log 2>&1 &
EMU_PID=$!

adb wait-for-device
BOOT=0
for i in $(seq 1 90); do
  BOOT=$(adb shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')
  [ "$BOOT" = "1" ] && { echo "booted after ~$((i*3))s"; break; }
  sleep 3
done
[ "$BOOT" = "1" ] || { echo "BOOT FAILED"; tail -20 /tmp/loft_emu.log; adb emu kill; exit 1; }

adb install -r "$APK" | tail -1
# Derive the launchable component from the APK (loft names the package after the
# .apk stem, e.g. com.loft.<name>), so this works with any loft-built APK.
BT="$SDK/build-tools/$(ls "$SDK/build-tools" | sort -V | tail -1)"
PKG=$("$BT/aapt2" dump badging "$APK" 2>/dev/null | sed -n "s/^package: name='\([^']*\)'.*/\1/p")
ACT=$("$BT/aapt2" dump badging "$APK" 2>/dev/null | sed -n "s/^launchable-activity: name='\([^']*\)'.*/\1/p")
adb logcat -c
adb shell am start -n "$PKG/$ACT" | tail -1
sleep 8
echo "=== LOGCAT (loft markers) ==="
adb logcat -d 2>/dev/null | grep -iE "loft|android_main|sum of squares|FATAL|AndroidRuntime" | tail -40
adb emu kill 2>/dev/null; kill "$EMU_PID" 2>/dev/null
echo DONE
