//! The Linux transport for @PLN119 placement: the shared mapping, the
//! spin-then-sleep handshake, the frame codec, and the two ends of a call.
//!
//! See the parent module for why the handshake spins before it sleeps and what
//! arc A does and does not carry across the boundary.

use std::io;
use std::path::{Path, PathBuf};

/// `"LOFW"` — the first word of the mapping, so a stale or foreign file is
/// rejected rather than read as a call frame.
const MAGIC: u32 = 0x4C4F_4657;

/// Bumped whenever the frame encoding below changes. Caller and worker are
/// normally the same binary, but a stale worker executable is exactly the case
/// this catches.
const PROTOCOL: u32 = 1;

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

/// A request kind, the first word of every request frame.
const REQ_CALL: u32 = 0;
const REQ_SHUTDOWN: u32 = 1;

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
    fn await_past(&self, seq_off: usize, sleepers_off: usize, last: u32) -> u32 {
        let seq = self.atomic(seq_off);
        let sleepers = self.atomic(sleepers_off);
        let mut spun = 0u32;
        loop {
            let cur = seq.load(std::sync::atomic::Ordering::SeqCst);
            if cur != last {
                return cur;
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
                return cur;
            }
            futex_wait(seq, cur);
            sleepers.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            spun = 0;
        }
    }

    fn send_request(&self, seq: u32) {
        self.publish(OFF_REQ_SEQ, OFF_REQ_SLEEPERS, seq);
    }

    fn await_request(&self, last: u32) -> u32 {
        self.await_past(OFF_REQ_SEQ, OFF_REQ_SLEEPERS, last)
    }

    fn send_response(&self, seq: u32) {
        self.publish(OFF_RESP_SEQ, OFF_RESP_SLEEPERS, seq);
    }

    fn await_response(&self, last: u32) -> u32 {
        self.await_past(OFF_RESP_SEQ, OFF_RESP_SLEEPERS, last)
    }
}

// ── futex ───────────────────────────────────────────────────────────────────
//
// Shared (no `FUTEX_PRIVATE_FLAG`): the word lives in a file mapping that spans
// two processes, and the private variant hashes on the mm, so the two sides
// would queue on different keys and every wake would be lost.

fn futex_wait(a: &std::sync::atomic::AtomicU32, expect: u32) {
    unsafe {
        libc::syscall(
            libc::SYS_futex,
            std::ptr::from_ref(a),
            libc::FUTEX_WAIT,
            expect,
            std::ptr::null::<libc::timespec>(),
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
        }
    }

    fn get_value(&mut self) -> Option<crate::host::Value> {
        use crate::host::Value;
        match self.get_u32()? {
            TAG_VOID => Some(Value::Void),
            TAG_BOOL => Some(Value::Bool(self.get_i64()? != 0)),
            TAG_INT => Some(Value::Int(self.get_i64()?)),
            TAG_FLOAT => Some(Value::Float(f64::from_bits(self.get_i64()? as u64))),
            TAG_TEXT => Some(Value::Text(self.get_str()?)),
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
    child: std::process::Child,
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
        let _ = self.child.kill();
        let _ = self.child.wait();
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
    pub fn spawn(name: &str, pkg_dir: &Path, stdlib_dir: &Path) -> io::Result<Worker> {
        let path = std::env::temp_dir().join(format!(
            "loft-place-{}-{}.wire",
            std::process::id(),
            name.replace(|c: char| !c.is_ascii_alphanumeric(), "_")
        ));
        let wire = Wire::create(&path)?;
        // Normally the worker is this same binary. `LOFT_WORKER_EXE` names a
        // different one, which is what lets a test drive a real worker without
        // being the `loft` executable itself — and what lets an embedder whose
        // process is not `loft` place a library at all.
        let exe = match std::env::var_os("LOFT_WORKER_EXE") {
            Some(p) => PathBuf::from(p),
            None => std::env::current_exe()?,
        };
        let child = std::process::Command::new(exe)
            .arg("--lib-worker")
            .arg(&path)
            .arg(pkg_dir)
            .arg("--default")
            .arg(stdlib_dir)
            .spawn()?;

        let w = Worker {
            wire,
            child,
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

    /// Call `func` in the worker.
    ///
    /// # Errors
    /// The worker's own error text for a fault inside the library, or a
    /// transport failure (a dead worker, an oversized frame).
    pub fn call(
        &self,
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
        &self,
        func: &str,
        args: &[crate::host::Value],
    ) -> Result<crate::host::Value, String> {
        if self.dead.get() {
            return Err(format!("library '{}' worker is gone", self.name));
        }
        let mut f = Frame::new(self.wire.payload());
        let built = f.put_u32(REQ_CALL)
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
        let seq = self.seq.get() + 1;
        self.seq.set(seq);
        self.wire.send_request(seq);
        self.wire.await_response(seq - 1);

        let mut f = Frame::new(self.wire.payload());
        match f.get_u32() {
            Some(RESP_OK) => f
                .get_value()
                .ok_or_else(|| format!("malformed response frame from '{}'", self.name)),
            Some(RESP_ERR) => Err(f
                .get_str()
                .unwrap_or_else(|| "malformed error frame".to_string())),
            _ => Err(format!("malformed response frame from '{}'", self.name)),
        }
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
        Ok(p) => {
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

    let mut last = 0u32;
    loop {
        last = wire.await_request(last);
        let mut f = Frame::new(wire.payload());
        let kind = f.get_u32();
        if kind == Some(REQ_SHUTDOWN) || kind.is_none() {
            std::process::exit(0);
        }
        let reply = read_and_run(&mut f, program.as_mut());
        let mut out = Frame::new(wire.payload());
        match reply {
            Ok(v) => {
                if !(out.put_u32(RESP_OK) && out.put_value(&v)) {
                    let mut out = Frame::new(wire.payload());
                    let _ = out.put_u32(RESP_ERR)
                        && out.put_str("return value does not fit the placement wire");
                }
            }
            Err(e) => {
                let mut out = Frame::new(wire.payload());
                let _ = out.put_u32(RESP_ERR) && out.put_str(&e);
            }
        }
        wire.send_response(last);
    }
}

/// Decode one request frame and run it against the loaded library.
fn read_and_run(
    f: &mut Frame,
    program: Option<&mut crate::host::Program>,
) -> Result<crate::host::Value, String> {
    let func = f.get_str().ok_or("malformed request frame")?;
    let n_args = f.get_u32().ok_or("malformed request frame")? as usize;
    let mut args = Vec::with_capacity(n_args);
    for _ in 0..n_args {
        args.push(f.get_value().ok_or("malformed argument in request frame")?);
    }
    let program = program.ok_or("library failed to load")?;
    // The handshake: the caller only needs to know we got here.
    if func.is_empty() {
        return Ok(crate::host::Value::Void);
    }
    // A fault inside the library is the caller's error, not the worker's death
    // — arc D extends this to a worker that dies outright.
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| program.call(&func, &args))) {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err(format!("library panicked in '{func}'")),
    }
}
