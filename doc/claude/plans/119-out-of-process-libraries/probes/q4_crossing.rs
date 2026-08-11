// @PLN119 Q4 — what does one cross-process call cost?
//
// Two processes share ONE mmap-backed page (the arena the plan proposes as the
// wire) and bounce a request across it. Three waiting disciplines:
//
//   futex      — sleep on every call. One syscall pair per crossing; a worker
//                between calls costs nothing.
//   spin       — never sleep. The lower bound, and it burns a core per idle
//                worker, so it is not shippable on its own.
//   spin-then  — spin briefly, then sleep, with a SLEEPER FLAG so the waker
//                only pays the FUTEX_WAKE syscall when someone is actually
//                asleep. Back-to-back calls never reach a syscall; an idle
//                worker still gives its core back. This is the shippable one.
//
// x86_64 Linux. Build: rustc -O q4_crossing.rs -o q4_crossing

use std::sync::atomic::{fence, AtomicU32, Ordering};
use std::time::Instant;

const SYS_FUTEX: i64 = 202;
const FUTEX_WAIT: i32 = 0; // shared (no PRIVATE flag) — the arena spans processes
const FUTEX_WAKE: i32 = 1;

unsafe extern "C" {
    fn syscall(num: i64, ...) -> i64;
    fn fork() -> i32;
    fn mmap(addr: *mut u8, len: usize, prot: i32, flags: i32, fd: i32, off: i64) -> *mut u8;
    fn _exit(code: i32) -> !;
    fn waitpid(pid: i32, status: *mut i32, opts: i32) -> i32;
}

const PROT_READ: i32 = 1;
const PROT_WRITE: i32 = 2;
const MAP_SHARED: i32 = 1;
const MAP_ANONYMOUS: i32 = 0x20;

fn futex_wait(a: &AtomicU32, expect: u32) {
    unsafe {
        syscall(
            SYS_FUTEX,
            std::ptr::from_ref(a),
            FUTEX_WAIT,
            expect,
            std::ptr::null::<u8>(),
        );
    }
}

fn futex_wake(a: &AtomicU32) {
    unsafe {
        syscall(SYS_FUTEX, std::ptr::from_ref(a), FUTEX_WAKE, 1i32);
    }
}

/// How a side waits for its counterpart.
#[derive(Clone, Copy, PartialEq)]
enum Wait {
    Futex,
    Spin,
    /// Spin for this many reads before sleeping.
    SpinThen(u32),
}

/// One direction of the wire: a sequence word plus the flag that says whether
/// anyone is asleep on it.
struct Chan {
    seq: &'static AtomicU32,
    sleepers: &'static AtomicU32,
}

impl Chan {
    /// Publish `v` and wake the counterpart — but only pay the syscall if it
    /// actually went to sleep.
    fn publish(&self, v: u32, mode: Wait) {
        self.seq.store(v, Ordering::SeqCst);
        if mode == Wait::Spin {
            return;
        }
        // SeqCst store above + SeqCst load here: if the waiter set its flag
        // before re-reading seq, at least one of us sees the other's write, so
        // the wakeup cannot be lost.
        if self.sleepers.load(Ordering::SeqCst) != 0 {
            futex_wake(self.seq);
        }
    }

    /// Block until `seq` exceeds `last`, and return the value seen.
    fn await_past(&self, last: u32, mode: Wait) -> u32 {
        let mut spun = 0u32;
        loop {
            let cur = self.seq.load(Ordering::SeqCst);
            if cur > last {
                return cur;
            }
            let budget = match mode {
                Wait::Spin => u32::MAX,
                Wait::Futex => 0,
                Wait::SpinThen(n) => n,
            };
            if spun < budget {
                spun += 1;
                std::hint::spin_loop();
                continue;
            }
            // About to sleep: announce it, then re-check before committing.
            self.sleepers.fetch_add(1, Ordering::SeqCst);
            fence(Ordering::SeqCst);
            let cur = self.seq.load(Ordering::SeqCst);
            if cur > last {
                self.sleepers.fetch_sub(1, Ordering::SeqCst);
                return cur;
            }
            futex_wait(self.seq, cur);
            self.sleepers.fetch_sub(1, Ordering::SeqCst);
            spun = 0;
        }
    }
}

fn map_shared_page() -> (Chan, Chan) {
    unsafe {
        let p = mmap(
            std::ptr::null_mut(),
            4096,
            PROT_READ | PROT_WRITE,
            MAP_SHARED | MAP_ANONYMOUS,
            -1,
            0,
        );
        assert!(p as isize > 0, "mmap failed");
        p.write_bytes(0, 4096);
        // Separate cache lines: the two directions must not ping-pong one line.
        let req = Chan {
            seq: &*p.cast::<AtomicU32>(),
            sleepers: &*p.add(8).cast::<AtomicU32>(),
        };
        let resp = Chan {
            seq: &*p.add(64).cast::<AtomicU32>(),
            sleepers: &*p.add(72).cast::<AtomicU32>(),
        };
        (req, resp)
    }
}

fn run(label: &str, iters: u32, mode: Wait) {
    let (req, resp) = map_shared_page();

    let pid = unsafe { fork() };
    if pid == 0 {
        // ---- callee: the worker process ----
        let mut last = 0u32;
        loop {
            last = req.await_past(last, mode);
            if last == u32::MAX {
                unsafe { _exit(0) };
            }
            // A ping() body: nothing. We time the CROSSING, not the work.
            resp.publish(last, mode);
        }
    }

    // ---- caller ----
    let mut seq = 0u32;
    let mut call = |seq: &mut u32| {
        *seq += 1;
        req.publish(*seq, mode);
        resp.await_past(*seq - 1, mode);
    };
    for _ in 0..2000 {
        call(&mut seq);
    }
    let t = Instant::now();
    for _ in 0..iters {
        call(&mut seq);
    }
    let el = t.elapsed();

    req.publish(u32::MAX, Wait::Futex);
    futex_wake(req.seq);
    let mut st = 0;
    unsafe { waitpid(pid, &mut st, 0) };

    let per = el.as_nanos() as f64 / f64::from(iters);
    println!("{label:<40} {per:>9.0} ns/call  ({:.2} µs)", per / 1000.0);
}

fn main() {
    println!("@PLN119 Q4 — cross-process call crossing cost (shared mmap arena)\n");
    run("futex every call", 50_000, Wait::Futex);
    run("spin-then-sleep (SHIPPABLE)", 200_000, Wait::SpinThen(2000));
    run("spin only (lower bound, burns a core)", 200_000, Wait::Spin);
    // Control: a ZERO spin budget must sleep on every call, so it has to land
    // on the futex number. If it instead reads like the 2000-budget row, the
    // sleeper-flag path is never taken and the shippable row is measuring
    // nothing but a spin.
    run("CONTROL SpinThen(0) — must match futex", 50_000, Wait::SpinThen(0));
}
