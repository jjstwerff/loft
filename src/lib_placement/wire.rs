//! The Linux transport for @PLN119 placement: the shared mapping, the
//! spin-then-sleep handshake, the frame codec, and the two ends of a call.
//!
//! See the parent module for why the handshake spins before it sleeps and what
//! arc A does and does not carry across the boundary.

use super::arena::Arena;
use std::io;
use std::path::{Path, PathBuf};

/// The two arena files that sit beside a wire, derived from its path rather than
/// passed separately: three paths that must agree are three chances to disagree.
fn arena_paths(wire: &Path) -> (PathBuf, PathBuf) {
    let mut arg = wire.as_os_str().to_os_string();
    arg.push(".arg");
    let mut ret = wire.as_os_str().to_os_string();
    ret.push(".ret");
    (PathBuf::from(arg), PathBuf::from(ret))
}

/// `"LOFW"` — the first word of the mapping, so a stale or foreign file is
/// rejected rather than read as a call frame.
const MAGIC: u32 = 0x4C4F_4657;

/// Bumped whenever the frame encoding below changes. Caller and worker are
/// normally the same binary, but a stale worker executable is exactly the case
/// this catches.
const PROTOCOL: u32 = 2;

/// Total mapping size. Arc A's frames are scalars and text, so this is
/// generous; arc B sizes it from the argument graph instead.
const WIRE_BYTES: usize = 1 << 20;

// Header offsets. The two sequence words sit on separate 64-byte lines: they
// are written by opposite sides on every call, and sharing a line would trade
// the syscall we just removed for cache-line ping-pong.
const OFF_MAGIC: usize = 0;
const OFF_PROTOCOL: usize = 4;
const OFF_EPOCH: usize = 8;
const OFF_STORE_BASE: usize = 12;
const OFF_REQ_SEQ: usize = 64;
const OFF_REQ_SLEEPERS: usize = 68;
const OFF_RESP_SEQ: usize = 128;
const OFF_RESP_SLEEPERS: usize = 132;
const OFF_PAYLOAD: usize = 192;

/// How many times a side re-reads the sequence word before it sleeps.
///
/// Chosen from the Q4 measurement: a served call turns round in ~130 ns, so a
/// budget this size covers a worker that is answering promptly and expires
/// quickly on one that is not.
const SPIN_BUDGET: u32 = 2000;

/// How long a caller sleeps before looking to see the worker is still there.
///
/// Only a call that has already spun past its budget ever sleeps, so this is not
/// on the path of a busy exchange; what it bounds is how long a caller waits on
/// a worker that will never answer. Short enough that a death is reported
/// promptly, long enough that a genuinely slow library call costs a handful of
/// wakeups rather than a spin.
const LIVENESS_POLL: std::time::Duration = std::time::Duration::from_millis(50);

/// A request kind, the first word of every request frame.
const REQ_CALL: u32 = 0;
const REQ_SHUTDOWN: u32 = 1;
/// Ask the worker how IT lays out a named function's compound types (@PLN119
/// arc B's layout gate). Answered once per function at install, never on a call.
const REQ_LAYOUT: u32 = 2;

/// Response status, the first word of every response frame.
const RESP_OK: u32 = 0;
const RESP_ERR: u32 = 1;

/// Value tags on the wire. These mirror [`crate::host::Value`] rather than
/// loft's `Type`, because the host marshaller is what both ends use.
const TAG_VOID: u32 = 0;
const TAG_BOOL: u32 = 1;
const TAG_INT: u32 = 2;
const TAG_FLOAT: u32 = 3;
const TAG_TEXT: u32 = 4;
/// A struct / vector: the `(rec, pos)` of its record in the call arena.
///
/// The store NUMBER is deliberately not on the wire. Each side registers the
/// arena at whatever slot its own `Stores` had free, and nothing inside a record
/// graph names a store (interior pointers are plain `u32` record ids), so the
/// receiver fills in its own — see [`super::arena`].
const TAG_REF: u32 = 5;
/// An absent struct / vector (`DbRef::NULL`).  A separate tag rather than a
/// reserved `rec`: record 0 is a real address, and an absent value that decoded
/// to one would read the arena's own header as a struct.
const TAG_NULLREF: u32 = 6;

/// The shared mapping both processes address the call through.
///
/// Not a [`crate::store::Store`]: arc A carries scalars, so it needs a frame
/// buffer rather than a heap. Arc B replaces the payload region with a real
/// mmap-backed store so a value graph can be built in place, which is why the
/// header already reserves the epoch and store-base words.
pub struct Wire {
    base: *mut u8,
    /// Kept so the file outlives the mapping and unlinking is the owner's call.
    path: PathBuf,
    /// Only the process that created the file removes it.
    owner: bool,
}

// The mapping is shared by definition; what makes it safe is the sequence
// protocol, which hands ownership of the payload region to exactly one side at
// a time.
unsafe impl Send for Wire {}

impl Drop for Wire {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.base.cast::<libc::c_void>(), WIRE_BYTES);
        }
        if self.owner {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

impl Wire {
    /// Create the backing file and map it. Used by the caller, which owns the
    /// file's lifetime.
    ///
    /// # Errors
    /// Any failure to create, size, or map the file.
    pub fn create(path: &Path) -> io::Result<Wire> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        file.set_len(WIRE_BYTES as u64)?;
        let w = Wire::map(&file, path, true)?;
        unsafe {
            w.base.write_bytes(0, WIRE_BYTES);
        }
        w.put_u32(OFF_PROTOCOL, PROTOCOL);
        // Magic last: it is what the worker waits on, so publishing it before
        // the rest is initialised would let the worker read a half-built header.
        w.publish_magic();
        Ok(w)
    }

    /// Map a file another process created. Used by the worker.
    ///
    /// # Errors
    /// A missing or unmappable file, a wrong magic (not our file), or a
    /// protocol mismatch (a stale worker executable against a newer caller).
    pub fn attach(path: &Path) -> io::Result<Wire> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)?;
        let w = Wire::map(&file, path, false)?;
        if w.get_u32(OFF_MAGIC) != MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{} is not a loft placement wire", path.display()),
            ));
        }
        let proto = w.get_u32(OFF_PROTOCOL);
        if proto != PROTOCOL {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "placement wire protocol {proto} but this worker speaks {PROTOCOL} — \
                     the worker executable is a different build from the caller"
                ),
            ));
        }
        Ok(w)
    }

    fn map(file: &std::fs::File, path: &Path, owner: bool) -> io::Result<Wire> {
        use std::os::fd::AsRawFd;
        let base = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                WIRE_BYTES,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                file.as_raw_fd(),
                0,
            )
        };
        if base == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        Ok(Wire {
            base: base.cast::<u8>(),
            path: path.to_path_buf(),
            owner,
        })
    }

    fn publish_magic(&self) {
        self.atomic(OFF_MAGIC)
            .store(MAGIC, std::sync::atomic::Ordering::SeqCst);
    }

    /// `mmap` returns page-aligned memory and every header offset above is a
    /// multiple of 4, so the `AtomicU32` alignment this cast needs is a
    /// property of the constants rather than a hope.
    #[allow(clippy::cast_ptr_alignment)]
    fn atomic(&self, off: usize) -> &std::sync::atomic::AtomicU32 {
        debug_assert_eq!(off % 4, 0, "header offset {off} is not u32-aligned");
        unsafe { &*self.base.add(off).cast::<std::sync::atomic::AtomicU32>() }
    }

    fn get_u32(&self, off: usize) -> u32 {
        self.atomic(off).load(std::sync::atomic::Ordering::SeqCst)
    }

    fn put_u32(&self, off: usize, v: u32) {
        self.atomic(off)
            .store(v, std::sync::atomic::Ordering::SeqCst);
    }

    /// The caller's store-numbering base, handed to the worker at attach so a
    /// `DbRef` it mints can be translated into the caller's numbering.
    ///
    /// Arc A never sends a reference, so nothing reads this yet. It is written
    /// at handshake anyway because the alternative — adding it when arc B needs
    /// it — is a protocol change that would silently mismatch a running worker.
    pub fn set_store_base(&self, base: u32) {
        self.put_u32(OFF_STORE_BASE, base);
    }

    /// See [`Wire::set_store_base`].
    #[must_use]
    pub fn store_base(&self) -> u32 {
        self.get_u32(OFF_STORE_BASE)
    }

    /// Stamp the caller's store generation, so a worker holding a mapping from
    /// before a structural change can tell that it is stale.
    pub fn set_epoch(&self, epoch: u32) {
        self.put_u32(OFF_EPOCH, epoch);
    }

    /// See [`Wire::set_epoch`].
    #[must_use]
    pub fn epoch(&self) -> u32 {
        self.get_u32(OFF_EPOCH)
    }

    fn payload(&self) -> *mut u8 {
        unsafe { self.base.add(OFF_PAYLOAD) }
    }

    // ── the two directions ──────────────────────────────────────────────

    /// Publish `seq` on a channel and wake the other side if it is asleep.
    fn publish(&self, seq_off: usize, sleepers_off: usize, seq: u32) {
        self.atomic(seq_off)
            .store(seq, std::sync::atomic::Ordering::SeqCst);
        // The SeqCst store above pairs with the waiter's SeqCst re-read after it
        // announces itself: whichever order the two run in, one of them observes
        // the other, so a wakeup cannot be lost.
        if self.get_u32(sleepers_off) != 0 {
            futex_wake(self.atomic(seq_off));
        }
    }

    /// Block until the channel's sequence passes `last`, spinning first.
    ///
    /// `poll` bounds each individual sleep, and `keep_waiting` is asked after
    /// every sleep that expires; answering `false` abandons the wait and returns
    /// `None`. Pass `None` to sleep until woken, which is what a side with an
    /// independent way of noticing the other has gone should do.
    ///
    /// The polling exists because a `FUTEX_WAIT` on a word nobody will ever
    /// write again does not fail — it waits forever. Only a wait that has
    /// already spun past its budget ever reaches a sleep, so a busy exchange
    /// never pays for this.
    fn await_past<F: FnMut() -> bool>(
        &self,
        seq_off: usize,
        sleepers_off: usize,
        last: u32,
        poll: Option<std::time::Duration>,
        mut keep_waiting: F,
    ) -> Option<u32> {
        let seq = self.atomic(seq_off);
        let sleepers = self.atomic(sleepers_off);
        let mut spun = 0u32;
        loop {
            let cur = seq.load(std::sync::atomic::Ordering::SeqCst);
            if cur != last {
                return Some(cur);
            }
            if spun < SPIN_BUDGET {
                spun += 1;
                std::hint::spin_loop();
                continue;
            }
            sleepers.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            std::sync::atomic::fence(std::sync::atomic::Ordering::SeqCst);
            let cur = seq.load(std::sync::atomic::Ordering::SeqCst);
            if cur != last {
                sleepers.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                return Some(cur);
            }
            futex_wait(seq, cur, poll);
            sleepers.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            // Re-read before consulting the check: a wake and a timeout are
            // indistinguishable here, and reporting the counterpart gone while
            // its answer is already published would be a race, not a diagnosis.
            if seq.load(std::sync::atomic::Ordering::SeqCst) == last && !keep_waiting() {
                return None;
            }
            spun = 0;
        }
    }

    fn send_request(&self, seq: u32) {
        self.publish(OFF_REQ_SEQ, OFF_REQ_SLEEPERS, seq);
    }

    /// The worker's wait for the next call. Untimed: a worker has its own way of
    /// noticing the caller is gone (`PR_SET_PDEATHSIG`), so waking it on a timer
    /// would burn a wakeup per idle period to learn nothing.
    fn await_request(&self, last: u32) -> u32 {
        self.await_past(OFF_REQ_SEQ, OFF_REQ_SLEEPERS, last, None, || true)
            .expect("an untimed wait never abandons")
    }

    fn send_response(&self, seq: u32) {
        self.publish(OFF_RESP_SEQ, OFF_RESP_SLEEPERS, seq);
    }

    /// The caller's wait for an answer, abandoned when `alive` reports the
    /// worker has gone. The caller has no signal for a child's death — it has to
    /// look — so this is the side that polls.
    fn await_response<F: FnMut() -> bool>(&self, last: u32, alive: F) -> Option<u32> {
        self.await_past(
            OFF_RESP_SEQ,
            OFF_RESP_SLEEPERS,
            last,
            Some(LIVENESS_POLL),
            alive,
        )
    }
}

// ── futex ───────────────────────────────────────────────────────────────────
//
// Shared (no `FUTEX_PRIVATE_FLAG`): the word lives in a file mapping that spans
// two processes, and the private variant hashes on the mm, so the two sides
// would queue on different keys and every wake would be lost.

fn futex_wait(a: &std::sync::atomic::AtomicU32, expect: u32, limit: Option<std::time::Duration>) {
    let ts = limit.map(|d| libc::timespec {
        tv_sec: d.as_secs() as libc::time_t,
        tv_nsec: libc::c_long::from(d.subsec_nanos()),
    });
    unsafe {
        libc::syscall(
            libc::SYS_futex,
            std::ptr::from_ref(a),
            libc::FUTEX_WAIT,
            expect,
            ts.as_ref()
                .map_or(std::ptr::null(), std::ptr::from_ref::<libc::timespec>),
        );
    }
}

fn futex_wake(a: &std::sync::atomic::AtomicU32) {
    unsafe {
        libc::syscall(
            libc::SYS_futex,
            std::ptr::from_ref(a),
            libc::FUTEX_WAKE,
            1i32,
        );
    }
}

// ── frame codec ─────────────────────────────────────────────────────────────

/// A bounds-checked cursor over the payload region.
///
/// Every read is checked against the region end: a worker must not be
/// trustable into reading past the mapping because a length word was wrong, and
/// a length word is exactly what a mismatched build gets wrong.
struct Frame {
    base: *mut u8,
    at: usize,
}

impl Frame {
    fn new(base: *mut u8) -> Frame {
        Frame { base, at: 0 }
    }

    fn room(&self, n: usize) -> bool {
        self.at + n <= WIRE_BYTES - OFF_PAYLOAD
    }

    fn put_u32(&mut self, v: u32) -> bool {
        if !self.room(4) {
            return false;
        }
        unsafe {
            self.base.add(self.at).cast::<u32>().write_unaligned(v);
        }
        self.at += 4;
        true
    }

    fn get_u32(&mut self) -> Option<u32> {
        if !self.room(4) {
            return None;
        }
        let v = unsafe { self.base.add(self.at).cast::<u32>().read_unaligned() };
        self.at += 4;
        Some(v)
    }

    fn put_i64(&mut self, v: i64) -> bool {
        if !self.room(8) {
            return false;
        }
        unsafe {
            self.base.add(self.at).cast::<i64>().write_unaligned(v);
        }
        self.at += 8;
        true
    }

    fn get_i64(&mut self) -> Option<i64> {
        if !self.room(8) {
            return None;
        }
        let v = unsafe { self.base.add(self.at).cast::<i64>().read_unaligned() };
        self.at += 8;
        Some(v)
    }

    fn put_str(&mut self, s: &str) -> bool {
        let b = s.as_bytes();
        if !self.put_u32(b.len() as u32) || !self.room(b.len()) {
            return false;
        }
        unsafe {
            std::ptr::copy_nonoverlapping(b.as_ptr(), self.base.add(self.at), b.len());
        }
        self.at += b.len();
        true
    }

    fn get_str(&mut self) -> Option<String> {
        let n = self.get_u32()? as usize;
        if !self.room(n) {
            return None;
        }
        let s = unsafe { std::slice::from_raw_parts(self.base.add(self.at), n) };
        self.at += n;
        // Lossy rather than fatal: the bytes came from a loft `text`, which is
        // already UTF-8, so a failure here means a corrupt frame — and reporting
        // it as a garbled name gives a better error than a decode panic.
        Some(String::from_utf8_lossy(s).into_owned())
    }

    fn put_value(&mut self, v: &crate::host::Value) -> bool {
        use crate::host::Value;
        match v {
            Value::Void => self.put_u32(TAG_VOID),
            Value::Bool(b) => self.put_u32(TAG_BOOL) && self.put_i64(i64::from(*b)),
            Value::Int(i) => self.put_u32(TAG_INT) && self.put_i64(*i),
            Value::Float(f) => self.put_u32(TAG_FLOAT) && self.put_i64(f.to_bits() as i64),
            Value::Text(s) => self.put_u32(TAG_TEXT) && self.put_str(s),
            Value::Ref(r) if r.is_null() => self.put_u32(TAG_NULLREF),
            Value::Ref(r) => self.put_u32(TAG_REF) && self.put_u32(r.rec) && self.put_u32(r.pos),
        }
    }

    /// Decode one value.  `arena_nr` is the store number THIS side registered the
    /// call arena under, and is what a `TAG_REF` is completed with.
    fn get_value(&mut self, arena_nr: u16) -> Option<crate::host::Value> {
        use crate::host::Value;
        match self.get_u32()? {
            TAG_VOID => Some(Value::Void),
            TAG_BOOL => Some(Value::Bool(self.get_i64()? != 0)),
            TAG_INT => Some(Value::Int(self.get_i64()?)),
            TAG_FLOAT => Some(Value::Float(f64::from_bits(self.get_i64()? as u64))),
            TAG_TEXT => Some(Value::Text(self.get_str()?)),
            TAG_NULLREF => Some(Value::Ref(crate::keys::DbRef::NULL)),
            TAG_REF => {
                let rec = self.get_u32()?;
                let pos = self.get_u32()?;
                Some(Value::Ref(crate::keys::DbRef {
                    store_nr: arena_nr,
                    rec,
                    pos,
                }))
            }
            _ => None,
        }
    }
}

// ── caller side ─────────────────────────────────────────────────────────────

/// A library running in a worker process, and the wire to reach it.
///
/// One per process-placed library, created at load and kept for the run: the
/// worker holds the library's stores across calls, which is what lets a placed
/// library keep state exactly as an in-process one does.
pub struct Worker {
    wire: Wire,
    /// The call arena, one store per direction (@PLN119 arc B).
    ///
    /// `arg` is written by the caller and `ret` by the worker — one writer each,
    /// because a `Store` caches its free list on the Rust side and two
    /// processes allocating out of one would hand out the same block twice.
    arg: Arena,
    ret: Arena,
    /// Behind a `RefCell` so a call — which holds `&self` — can ask whether the
    /// child is still running. `try_wait` reaps, which needs `&mut Child`.
    child: std::cell::RefCell<std::process::Child>,
    /// Last request sequence sent. Requests and responses share the counter so
    /// a response can be matched to its request.
    seq: std::cell::Cell<u32>,
    /// Set once the worker has been seen to die, so every later call reports
    /// the death instead of blocking forever on a wire nobody is serving.
    dead: std::cell::Cell<bool>,
    name: String,
}

impl Drop for Worker {
    fn drop(&mut self) {
        if !self.dead.get() {
            let seq = self.seq.get() + 1;
            let mut f = Frame::new(self.wire.payload());
            if f.put_u32(REQ_SHUTDOWN) {
                self.wire.send_request(seq);
            }
        }
        // A worker that ignores the shutdown word must not wedge the run.
        let mut child = self.child.borrow_mut();
        let _ = child.kill();
        let _ = child.wait();
    }
}

impl Worker {
    /// Start a worker holding the library at `pkg_dir` and complete the attach
    /// handshake.
    ///
    /// # Errors
    /// A failure to create the wire, spawn the worker, or hear its ready
    /// response — each reported with the library name, because the operator's
    /// next question is always which library failed to place.
    /// `cwd` is the directory the CALLER will be running in — not necessarily
    /// the one it is in now.
    ///
    /// loft anchors a program's relative file access at its own source directory
    /// and chdirs there before running, but that happens long after libraries are
    /// installed. A worker started before it would inherit the INVOCATION
    /// directory, and then every relative path a placed library touched would
    /// resolve somewhere else than the same library in-process — a divergence
    /// with no error, found by `lib/git` answering "not a git repository" in one
    /// placement and the history in the other.
    pub fn spawn(name: &str, pkg_dir: &Path, stdlib_dir: &Path, cwd: &Path) -> io::Result<Worker> {
        let path = std::env::temp_dir().join(format!(
            "loft-place-{}-{}.wire",
            std::process::id(),
            name.replace(|c: char| !c.is_ascii_alphanumeric(), "_")
        ));
        let wire = Wire::create(&path)?;
        // Both arenas exist and are initialised before the worker starts, so
        // there is no window in which it can attach a half-built store.
        let (arg_path, ret_path) = arena_paths(&path);
        let arg = Arena::create(&arg_path)?;
        let ret = Arena::create(&ret_path)?;
        // Normally the worker is this same binary. `LOFT_WORKER_EXE` names a
        // different one, which is what lets a test drive a real worker without
        // being the `loft` executable itself — and what lets an embedder whose
        // process is not `loft` place a library at all.
        let exe = match std::env::var_os("LOFT_WORKER_EXE") {
            Some(p) => PathBuf::from(p),
            None => std::env::current_exe()?,
        };
        let mut command = std::process::Command::new(exe);
        command
            .arg("--lib-worker")
            .arg(&path)
            .arg(pkg_dir)
            .arg("--default")
            .arg(stdlib_dir);
        if !cwd.as_os_str().is_empty() {
            command.current_dir(cwd);
        }
        let child = command.spawn()?;

        let mut w = Worker {
            wire,
            arg,
            ret,
            child: std::cell::RefCell::new(child),
            seq: std::cell::Cell::new(0),
            dead: std::cell::Cell::new(false),
            name: name.to_string(),
        };
        // The handshake is the first call: the worker answers it only after the
        // library has parsed and compiled, so a library that cannot load is
        // reported here rather than at whichever call happens to be first.
        match w.call_raw("", &[]) {
            Ok(_) => Ok(w),
            Err(e) => Err(io::Error::other(format!(
                "library '{name}' declares placement = \"process\" but its worker did not \
                 come up: {e}"
            ))),
        }
    }

    /// The store-numbering base the worker reported — arc B translates against
    /// it. Present here so attaching does not change when arc B lands.
    #[must_use]
    pub fn store_base(&self) -> u32 {
        self.wire.store_base()
    }

    /// The worker process's id — what a caller needs to observe or signal it.
    ///
    /// # Panics
    /// If the handle is already borrowed, which only happens while this worker
    /// is being dropped.
    #[must_use]
    pub fn child_id(&self) -> u32 {
        self.child.borrow().id()
    }

    /// Is the worker process still running?
    ///
    /// `try_wait` rather than a signal probe, because a worker that has exited
    /// but not been reaped is a zombie — still a live pid, answering `kill(0)`
    /// perfectly happily, and never going to serve another call.
    fn still_running(&self) -> bool {
        match self.child.try_borrow_mut() {
            Ok(mut c) => matches!(c.try_wait(), Ok(None)),
            // Borrowed means `Drop` is already tearing this worker down; let the
            // wait end rather than claim a liveness we cannot check.
            Err(_) => false,
        }
    }

    /// The arena a compound ARGUMENT is marshalled into. Written by this side.
    pub fn arg_arena(&mut self) -> &mut Arena {
        &mut self.arg
    }

    /// The arena a compound RETURN comes back in. Written by the worker; this
    /// side only reads it, and only until the next call resets it.
    pub fn ret_arena(&mut self) -> &mut Arena {
        &mut self.ret
    }

    /// How the WORKER lays out `func`'s compound parameters and return.
    ///
    /// The caller compares this against its own answer and refuses to place a
    /// function the two disagree about. See [`super::dispatch`]'s layout gate.
    ///
    /// # Errors
    /// A transport failure, or the worker not knowing the function.
    pub fn layout(&mut self, func: &str) -> Result<String, String> {
        let mut f = Frame::new(self.wire.payload());
        if !(f.put_u32(REQ_LAYOUT) && f.put_str(func)) {
            return Err("layout request does not fit the wire".to_string());
        }
        match self.exchange(func)? {
            crate::host::Value::Text(s) => Ok(s),
            other => Err(format!("layout answer was {other:?}")),
        }
    }

    /// Call `func` in the worker.
    ///
    /// A COMPOUND answer comes back as a reference this side cannot complete
    /// yet — the return arena has no store number until it is bound, and it
    /// cannot be bound until it has been re-mapped, which this call is what
    /// decides. So the reference is left arena-relative here and
    /// [`reread_answer`](Worker::reread_answer) completes it once the caller has
    /// bound the arena.
    ///
    /// # Errors
    /// The worker's own error text for a fault inside the library, or a
    /// transport failure (a dead worker, an oversized frame).
    pub fn call(
        &mut self,
        func: &str,
        args: &[crate::host::Value],
    ) -> Result<crate::host::Value, String> {
        if func.is_empty() {
            return Err("empty function name".to_string());
        }
        self.call_raw(func, args)
    }

    /// The handshake and every later call take the same path; `func == ""` is
    /// the handshake, which the worker answers with void once it is loaded.
    fn call_raw(
        &mut self,
        func: &str,
        args: &[crate::host::Value],
    ) -> Result<crate::host::Value, String> {
        // The arg arena's size travels ahead of the arguments: marshalling may
        // have grown the file, and the worker has to map the new length before
        // it can read a record that lives past the old end.
        let arg_words = self.arg.words();
        let mut f = Frame::new(self.wire.payload());
        let built = f.put_u32(REQ_CALL)
            && f.put_u32(arg_words)
            && f.put_str(func)
            && f.put_u32(args.len() as u32)
            && args.iter().all(|a| f.put_value(a));
        if !built {
            return Err(format!(
                "call to '{func}' does not fit the placement wire ({} KiB) — \
                 arc B sizes the arena from the argument graph",
                WIRE_BYTES / 1024
            ));
        }
        self.exchange(func)
    }

    /// Send the request already built into the payload, wait for the answer,
    /// re-map whatever grew, and decode. Split out because the layout request
    /// and a call share every step but the frame they wrote.
    fn exchange(&mut self, func: &str) -> Result<crate::host::Value, String> {
        if self.dead.get() {
            return Err(format!("library '{}' worker is gone", self.name));
        }
        let seq = self.seq.get() + 1;
        self.seq.set(seq);
        self.wire.send_request(seq);
        if self
            .wire
            .await_response(seq - 1, || self.still_running())
            .is_none()
        {
            // The worker died with this call outstanding. Nothing will ever
            // write the response word, so the wait had to end by looking rather
            // than by being woken. Remember it: the wire is unusable from here,
            // and a later call must say so at once instead of waiting again.
            self.dead.set(true);
            return Err(format!(
                "library '{}' worker died during the call to '{}'",
                self.name,
                if func.is_empty() { "<load>" } else { func }
            ));
        }
        // Both arena sizes come back on EVERY answer, before the value. The ret
        // arena may have grown because the answer is large; the ARG arena may
        // have grown because the callee appended to a vector parameter — loft
        // passes a compound by reference, so that write is the caller's to see.
        let mut f = Frame::new(self.wire.payload());
        let status = f.get_u32();
        let ret_words = f.get_u32().unwrap_or(0);
        let arg_words = f.get_u32().unwrap_or(0);
        match status {
            Some(RESP_OK) => {}
            Some(RESP_ERR) => {
                return Err(f
                    .get_str()
                    .unwrap_or_else(|| "malformed error frame".to_string()));
            }
            _ => return Err(format!("malformed response frame from '{}'", self.name)),
        }
        self.arg
            .remap_if_grown(arg_words)
            .map_err(|e| format!("call arena (arguments) of '{}': {e}", self.name))?;
        self.ret
            .remap_if_grown(ret_words)
            .map_err(|e| format!("call arena (return) of '{}': {e}", self.name))?;
        f.get_value(u16::MAX)
            .ok_or_else(|| format!("malformed response frame from '{}'", self.name))
    }

    /// Re-read the answer that is still sitting in the payload, now that the
    /// caller knows which store number to complete a compound reference with.
    ///
    /// Only meaningful immediately after a successful [`call`](Worker::call) that
    /// asked for `u16::MAX`, and before the next request overwrites the frame.
    pub fn reread_answer(&self, arena_nr: u16) -> Option<crate::host::Value> {
        let mut f = Frame::new(self.wire.payload());
        if f.get_u32() != Some(RESP_OK) {
            return None;
        }
        f.get_u32()?; // ret words
        f.get_u32()?; // arg words
        f.get_value(arena_nr)
    }
}

// ── worker side ─────────────────────────────────────────────────────────────

/// Run as the worker for one placed library: attach the wire, load the library,
/// then serve calls until the caller says stop or goes away.
///
/// Entered from `loft --lib-worker <wire> <pkg_dir> --default <stdlib>`. Never
/// returns normally — it exits the process, because a worker that fell out of
/// its loop has nothing else to be.
///
/// # Panics
/// Never deliberately; a fault inside the library is caught and returned to the
/// caller as an error, which is what makes a placed library's error behaviour
/// match an in-process one.
pub fn serve(wire_path: &Path, pkg_dir: &Path, stdlib_dir: &Path) -> ! {
    // Die with the caller. A worker is meaningless without the process that
    // started it, and the caller cannot be relied on to say goodbye: it may
    // `exit` from any of a dozen places, or be killed outright. Without this a
    // crashed run leaves a worker holding the terminal's stdout, which reads as
    // the run itself having hung.
    unsafe {
        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
    }
    // Re-check after arming: if the caller died in the window before the
    // `prctl`, the signal has already been missed.
    if unsafe { libc::getppid() } == 1 {
        std::process::exit(0);
    }
    let wire = match Wire::attach(wire_path) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("loft worker: cannot attach {}: {e}", wire_path.display());
            std::process::exit(1);
        }
    };

    let loaded = crate::host::Program::from_library_dir(pkg_dir, stdlib_dir);

    // Report the store-numbering base before answering the handshake, so the
    // caller never observes a ready worker without one.
    let mut program = match loaded {
        Ok(mut p) => {
            // Resolve a relative path the way the CALLER does. The caller spawned
            // this worker in the directory it will itself run in, so that
            // directory — not the library's own — is what `file("x")` means to
            // the code this worker serves.
            if let Ok(here) = std::env::current_dir() {
                p.anchor_paths_at(&here);
            }
            wire.set_store_base(p.store_count());
            wire.set_epoch(p.store_epoch());
            Some(p)
        }
        Err(e) => {
            // Keep serving: the handshake reply carries the load error, which
            // is how the caller reports it against the library's name.
            wire.set_store_base(0);
            let mut f = Frame::new(wire.payload());
            let _ = f.put_u32(RESP_ERR) && f.put_str(&e.to_string());
            wire.send_response(1);
            std::process::exit(0);
        }
    };

    // The arenas the caller created before starting us. A worker that cannot map
    // them answers the handshake with the reason, exactly as a library that
    // cannot parse does — a worker that came up half-attached would fail at
    // whichever call first carried a struct.
    let (arg_path, ret_path) = arena_paths(wire_path);
    let mut arenas = match (Arena::attach(&arg_path), Arena::attach(&ret_path)) {
        (Ok(a), Ok(r)) => (a, r),
        (Err(e), _) | (_, Err(e)) => {
            let mut f = Frame::new(wire.payload());
            let _ = f.put_u32(RESP_ERR)
                && f.put_u32(0)
                && f.put_u32(0)
                && f.put_str(&format!("cannot map the call arena: {e}"));
            wire.send_response(1);
            std::process::exit(0);
        }
    };

    let mut last = 0u32;
    loop {
        last = wire.await_request(last);
        let mut f = Frame::new(wire.payload());
        let kind = f.get_u32();
        if kind == Some(REQ_SHUTDOWN) || kind.is_none() {
            std::process::exit(0);
        }
        let reply = if kind == Some(REQ_LAYOUT) {
            answer_layout(&mut f, program.as_mut())
        } else {
            serve_one(&mut f, program.as_mut(), &mut arenas)
        };
        let (arg_words, ret_words) = (arenas.0.words(), arenas.1.words());
        let mut out = Frame::new(wire.payload());
        // The sizes go out on the error path too: the callee may have grown the
        // ARGUMENT arena before it faulted, and a caller that read the old size
        // would then map short and read a hole where its own value was.
        match reply {
            Ok(v) => {
                if !(out.put_u32(RESP_OK)
                    && out.put_u32(ret_words)
                    && out.put_u32(arg_words)
                    && out.put_value(&v))
                {
                    let mut out = Frame::new(wire.payload());
                    let _ = out.put_u32(RESP_ERR)
                        && out.put_u32(ret_words)
                        && out.put_u32(arg_words)
                        && out.put_str("return value does not fit the placement wire");
                }
            }
            Err(e) => {
                let mut out = Frame::new(wire.payload());
                let _ = out.put_u32(RESP_ERR)
                    && out.put_u32(ret_words)
                    && out.put_u32(arg_words)
                    && out.put_str(&e);
            }
        }
        wire.send_response(last);
    }
}

/// Report how THIS program lays out a function's compound types, for the
/// caller's layout gate.
fn answer_layout(
    f: &mut Frame,
    program: Option<&mut crate::host::Program>,
) -> Result<crate::host::Value, String> {
    let func = f.get_str().ok_or("malformed layout request")?;
    let program = program.ok_or("library failed to load")?;
    Ok(crate::host::Value::Text(
        crate::lib_placement::dispatch::signature_layout(program, &func)
            .ok_or_else(|| format!("no function '{func}'"))?,
    ))
}

/// Decode one call frame and run it against the loaded library, with the arenas
/// bound for the duration.
///
/// The order here is the contract, and each step is load-bearing:
///
/// 1. **Re-map** the argument arena if the caller's marshal grew the file — its
///    records may live past the length this process mapped.
/// 2. **Resync** it. loft passes a compound BY REFERENCE, so the callee may
///    append to a vector parameter and allocate in this arena; allocating
///    against a free list cached before the caller's claims would hand out a
///    block that is already in use.
/// 3. **Reset** the return arena. This side owns it, and last call's answer is
///    dead the moment this one starts.
fn serve_one(
    f: &mut Frame,
    program: Option<&mut crate::host::Program>,
    arenas: &mut (Arena, Arena),
) -> Result<crate::host::Value, String> {
    let arg_words = f.get_u32().ok_or("malformed request frame")?;
    let program = program.ok_or("library failed to load")?;
    arenas
        .0
        .remap_if_grown(arg_words)
        .map_err(|e| format!("cannot map the argument arena: {e}"))?;
    arenas.0.resync();
    arenas.1.reset();

    let arg_nr = arenas.0.bind(program.stores());
    let ret_nr = arenas.1.bind(program.stores());
    let out = run_bound(f, program, ret_nr, arg_nr);
    arenas.0.unbind(program.stores(), arg_nr);
    arenas.1.unbind(program.stores(), ret_nr);
    out
}

/// The part of a call that runs while both arenas are registered.
fn run_bound(
    f: &mut Frame,
    program: &mut crate::host::Program,
    ret_nr: u16,
    arg_nr: u16,
) -> Result<crate::host::Value, String> {
    let func = f.get_str().ok_or("malformed request frame")?;
    let n_args = f.get_u32().ok_or("malformed request frame")? as usize;
    let mut args = Vec::with_capacity(n_args);
    for _ in 0..n_args {
        args.push(
            f.get_value(arg_nr)
                .ok_or("malformed argument in request frame")?,
        );
    }
    // The handshake: the caller only needs to know we got here.
    if func.is_empty() {
        return Ok(crate::host::Value::Void);
    }
    for a in &args {
        if let crate::host::Value::Ref(r) = a {
            super::arena::trace(program.stores(), "worker-arg", r, u16::MAX);
        }
    }
    // A compound answer is BUILT in the return arena rather than copied into it:
    // the destination a loft function's hidden `__retbuf` parameter names is
    // simply a record over there. A callee that ignores the offer and mints its
    // own store is the other half of the same protocol, handled below.
    let ret_ty = program
        .signature(&func)
        .ok_or_else(|| format!("no function '{func}'"))?
        .1;
    let retbuf = if crate::host::is_compound(&ret_ty) {
        let (tp, _) = program.layout_of(&ret_ty);
        super::arena::alloc_value(program.stores(), ret_nr, tp)
    } else {
        crate::keys::DbRef::NULL
    };

    // A fault inside the library is the caller's error, not the worker's death
    // — arc D extends this to a worker that dies outright.
    let out = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        program.call_into(&func, &args, retbuf)
    })) {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => return Err(e.to_string()),
        Err(_) => return Err(format!("library panicked in '{func}'")),
    };

    let crate::host::Value::Ref(answer) = out else {
        return Ok(out);
    };
    if answer.is_null() || answer.store_nr == ret_nr {
        return Ok(crate::host::Value::Ref(answer));
    }
    // The callee ignored the offered destination and built its answer in a store
    // of its own — the ordinary shape for `Point { … }`, where the constructor
    // mints a store and hands ownership to its caller. Copy it into the arena,
    // then free it exactly as an in-process caller's `OpFreeRef` would: this
    // process IS that caller, and nothing else will ever free it.
    //
    // Only when the answer is genuinely OWNED, which is the analysis's verdict
    // and not a second opinion derived here (@PLN119 arc C). A `View` return
    // borrows storage the callee did not create, and freeing that would pull the
    // ground out from under the library's own state — but a `View` return is
    // refused at marking, so reaching this with one means the two sides of the
    // decision have drifted apart.
    let (tp, _) = program.layout_of(&ret_ty);
    let landed = super::arena::alloc_value(program.stores(), ret_nr, tp);
    super::arena::copy_value(program.stores(), &landed, &answer, tp);
    if program.return_delivery(&func) == crate::use_analysis::HeapDelivery::Owned {
        program.stores().free_named(&answer, "<placed return>");
    }
    Ok(crate::host::Value::Ref(landed))
}
