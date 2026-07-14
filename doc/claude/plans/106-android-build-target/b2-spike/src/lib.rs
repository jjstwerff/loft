//! B2 spike — wrap a loft program in an Android NativeActivity.
//!
//! `android-activity` (native-activity feature) exports `ANativeActivity_onCreate`
//! and calls our `#[no_mangle] android_main`. We run the loft program's generated
//! `main` (emitted by `loft --native-emit`, included below as a module) inside it,
//! forwarding its stdout into logcat so the loft output is visible on-device.
//!
//! The real B2 will have loft emit this `android_main` entry itself (the runtime-
//! entry descriptor field); this crate is the throwaway spike that proves the
//! packaging + launch pipeline end to end.

// Crate-level allows hoisted out of the loft-generated prog.rs (its `#![...]`
// inner attrs can't survive an `include!` inside a module).
#![allow(unused_imports)]
#![allow(unused_parens)]
#![allow(unused_variables)]
#![allow(unreachable_code)]
#![allow(unused_mut)]
#![allow(non_snake_case)]
#![allow(dead_code)]
#![allow(redundant_semicolons)]
#![allow(unused_assignments)]
#![allow(unused_labels)]
#![allow(unused_braces)]
#![allow(unused_unsafe)]

// The loft-emitted program as a module. `sed` made its `fn main` -> `pub fn main`
// so we can call it; everything else (the loft runtime glue) it pulls from the
// `loft` crate dependency.
mod prog {
    include!("prog.rs");
}

use android_activity::{AndroidApp, MainEvent, PollEvent};
use std::time::Duration;

/// Forward the process stdout/stderr into logcat (tag `loft-stdout`) so the loft
/// program's `print()` output shows up in `adb logcat`. Best-effort: any failure
/// just leaves output going to the (invisible) default fds.
fn redirect_stdio_to_logcat() {
    #[link(name = "log")]
    unsafe extern "C" {
        fn __android_log_write(prio: i32, tag: *const libc::c_char, text: *const libc::c_char)
        -> i32;
    }
    let mut fds = [0i32; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return;
    }
    let (rd, wr) = (fds[0], fds[1]);
    unsafe {
        libc::dup2(wr, libc::STDOUT_FILENO);
        libc::dup2(wr, libc::STDERR_FILENO);
        libc::close(wr);
    }
    std::thread::spawn(move || {
        const TAG: &[u8] = b"loft-stdout\0";
        let mut buf = [0u8; 1024];
        let mut line: Vec<u8> = Vec::new();
        loop {
            let n = unsafe { libc::read(rd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
            if n <= 0 {
                break;
            }
            for &b in &buf[..n as usize] {
                if b == b'\n' {
                    line.push(0);
                    unsafe {
                        __android_log_write(
                            4, // ANDROID_LOG_INFO
                            TAG.as_ptr() as *const libc::c_char,
                            line.as_ptr() as *const libc::c_char,
                        );
                    }
                    line.clear();
                } else {
                    line.push(b);
                }
            }
        }
    });
}

#[unsafe(no_mangle)]
extern "C" fn android_main(app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("loft"),
    );
    log::info!("android_main reached");
    redirect_stdio_to_logcat();

    // Run the loft program. Its print() output is now piped into logcat.
    log::info!("running loft program");
    prog::main();
    log::info!("loft program returned");

    // Keep the activity alive briefly so the app doesn't crash-loop, and drain
    // lifecycle events (a real app would render here). Exit on Destroy or after a
    // few seconds — enough to prove launch without a persistent UI.
    let mut ticks = 0;
    loop {
        let mut destroy = false;
        app.poll_events(Some(Duration::from_millis(100)), |event| {
            if let PollEvent::Main(MainEvent::Destroy) = event {
                destroy = true;
            }
        });
        ticks += 1;
        if destroy || ticks > 40 {
            break;
        }
    }
    log::info!("android_main exiting");
}
