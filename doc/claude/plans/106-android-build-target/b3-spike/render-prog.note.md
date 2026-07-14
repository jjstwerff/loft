# render-prog.loft — the B3 end-to-end test

An unchanged loft graphics program that clears the screen orange. Build + run:

```sh
# a loft.toml beside it points `graphics` at the local fixture:
#   [dependencies]
#   graphics = { path = "<loft>/tests/fixtures/libs/graphics" }
ANDROID_NDK_HOME=<ndk> ANDROID_HOME=<sdk> \
  LOFT_ANDROID_TARGET=x86_64-linux-android \
  loft --native-android app.apk render-prog.loft
# then ../b2-spike/run_emulator_test.sh (APK=app.apk), and:
#   adb exec-out screencap -p > screen.png
```

Golden: `b3_render_golden.png` — center pixel (255,128,0), 99.6% #ff8000.
