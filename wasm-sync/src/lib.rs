// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @PLN117 — browser threading: locks a page's main thread is allowed to use.

//! `Mutex` / `Condvar` that a browser main thread may use.
//!
//! A page's main thread may not block: `memory.atomic.wait32` throws
//! *"Atomics.wait cannot be called in this context"* there.  rayon reaches its
//! internal locks from the calling thread on every `par` (the join in
//! `in_worker_cold`, and `build_global`'s wait for the workers to prime), so a
//! `par` on the main thread dies on stock `std` locks.  rayon solves this with
//! its `web_spin_lock` feature, which swaps `std::sync` for the `wasm_sync`
//! crate — but that crate recognises the main thread through `web_sys::window()`
//! and so drags in wasm-bindgen, which loft's raw (`--html`) wasm must not
//! contain.
//!
//! This crate is loft's drop-in replacement, wired in through
//! `[patch.crates-io]`: same spin-instead-of-block behaviour, no wasm-bindgen.
//! A thread is assumed to be allowed to block — the plain `std` behaviour, so
//! nothing changes off the browser main thread — and loft marks that one thread
//! with [`set_can_block(false)`] when it starts the worker pool.  Failing to
//! mark it throws loudly on the first `par`; the opposite default would quietly
//! spin every worker at 100 % CPU instead.
//!
//! Only what rayon-core uses is implemented (`Mutex::lock`, `Condvar::wait` /
//! `notify_one` / `notify_all`); anything else fails to compile rather than
//! silently behaving differently.

#[cfg(target_arch = "wasm32")]
use std::cell::Cell;
use std::hint::spin_loop;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{PoisonError, TryLockError};

pub type LockResult<G> = Result<G, PoisonError<G>>;

#[cfg(target_arch = "wasm32")]
thread_local! {
    static CAN_BLOCK: Cell<bool> = const { Cell::new(true) };
}

/// Declare whether the calling thread may block.
///
/// loft calls this with `false` on the browser main thread, which is the only
/// thread that may not; a thread that never calls it keeps the `std` behaviour.
#[cfg(target_arch = "wasm32")]
pub fn set_can_block(yes: bool) {
    CAN_BLOCK.with(|c| c.set(yes));
}

/// No-op off wasm — every native thread may block.
#[cfg(not(target_arch = "wasm32"))]
pub fn set_can_block(_yes: bool) {}

#[inline]
fn can_block() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        CAN_BLOCK.with(Cell::get)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        true
    }
}

/// A mutex that spins instead of blocking on a thread that may not block.
pub struct Mutex<T: ?Sized> {
    inner: std::sync::Mutex<T>,
}

impl<T> Mutex<T> {
    pub const fn new(value: T) -> Self {
        Self {
            inner: std::sync::Mutex::new(value),
        }
    }
}

impl<T: ?Sized> Mutex<T> {
    pub fn lock(&self) -> LockResult<MutexGuard<'_, T>> {
        if can_block() {
            return match self.inner.lock() {
                Ok(g) => Ok(self.wrap(g)),
                Err(p) => Err(PoisonError::new(self.wrap(p.into_inner()))),
            };
        }
        loop {
            match self.inner.try_lock() {
                Ok(g) => return Ok(self.wrap(g)),
                Err(TryLockError::Poisoned(p)) => {
                    return Err(PoisonError::new(self.wrap(p.into_inner())));
                }
                Err(TryLockError::WouldBlock) => spin_loop(),
            }
        }
    }

    fn wrap<'a>(&'a self, guard: std::sync::MutexGuard<'a, T>) -> MutexGuard<'a, T> {
        MutexGuard {
            lock: self,
            guard: Some(guard),
        }
    }
}

impl<T: ?Sized + std::fmt::Debug> std::fmt::Debug for Mutex<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.inner.fmt(f)
    }
}

impl<T: Default> Default for Mutex<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

pub struct MutexGuard<'a, T: ?Sized> {
    lock: &'a Mutex<T>,
    /// `Some` except for the instant [`Condvar::wait`] hands the inner guard to
    /// `std` and takes the returned one back.
    guard: Option<std::sync::MutexGuard<'a, T>>,
}

impl<T: ?Sized> Deref for MutexGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.guard.as_ref().expect("wasm_sync: guard in use")
    }
}

impl<T: ?Sized> DerefMut for MutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.guard.as_mut().expect("wasm_sync: guard in use")
    }
}

/// A condition variable whose `wait` spins on a thread that may not block.
///
/// Waiters that may block park on the `std` condvar; a waiter that may not
/// watches `generation`, which every notification bumps.  One `notify_*` wakes
/// both kinds.  A spinning waiter cannot miss the notification it is about to
/// wait for, because it reads the generation while it still holds the mutex and
/// rayon always notifies under that same mutex.
#[derive(Debug, Default)]
pub struct Condvar {
    inner: std::sync::Condvar,
    generation: AtomicU64,
}

impl Condvar {
    pub const fn new() -> Self {
        Self {
            inner: std::sync::Condvar::new(),
            generation: AtomicU64::new(0),
        }
    }

    pub fn wait<'a, T>(&self, mut guard: MutexGuard<'a, T>) -> LockResult<MutexGuard<'a, T>> {
        let lock = guard.lock;
        if can_block() {
            let inner = guard.guard.take().expect("wasm_sync: guard in use");
            return match self.inner.wait(inner) {
                Ok(g) => Ok(lock.wrap(g)),
                Err(p) => Err(PoisonError::new(lock.wrap(p.into_inner()))),
            };
        }
        let seen = self.generation.load(Ordering::Relaxed);
        drop(guard);
        while self.generation.load(Ordering::Acquire) == seen {
            spin_loop();
        }
        lock.lock()
    }

    pub fn notify_one(&self) {
        self.generation.fetch_add(1, Ordering::Release);
        self.inner.notify_one();
    }

    pub fn notify_all(&self) {
        self.generation.fetch_add(1, Ordering::Release);
        self.inner.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The blocking path is the `std` one: a notification wakes a real waiter.
    #[test]
    fn blocking_wait_is_woken() {
        let pair = std::sync::Arc::new((Mutex::new(false), Condvar::new()));
        let other = std::sync::Arc::clone(&pair);
        let t = std::thread::spawn(move || {
            let (lock, cv) = &*other;
            let mut ready = lock.lock().unwrap();
            while !*ready {
                ready = cv.wait(ready).unwrap();
            }
        });
        let (lock, cv) = &*pair;
        {
            let mut ready = lock.lock().unwrap();
            *ready = true;
            cv.notify_all();
        }
        t.join().unwrap();
    }

    /// A poisoned mutex still reports poisoning through the wrapper.
    #[test]
    fn poison_survives_the_wrapper() {
        let m = std::sync::Arc::new(Mutex::new(0));
        let other = std::sync::Arc::clone(&m);
        let _ = std::thread::spawn(move || {
            let _g = other.lock().unwrap();
            panic!("poison it");
        })
        .join();
        assert!(m.lock().is_err());
    }
}
