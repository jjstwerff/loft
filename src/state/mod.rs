// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(dead_code)]

mod codegen;
mod debug;
mod io;
mod text;

use crate::data::{Data, Type};
pub use crate::database::Call;
use crate::database::{ParallelCtx, Stores};
use crate::fill::OPERATORS;
use crate::keys::{DbRef, Str};
use crate::log_config::LogConfig;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{Error, Write};
use std::sync::Arc;

pub const STRING_NULL: &str = "\0";

/// One entry in the shadow call-frame vector (TR1.1).
/// Pushed by `fn_call`, popped by `fn_return`.  Stores enough information for
/// `stack_trace()` to reconstruct function names, source lines, and argument
/// values without walking the raw bytecode stack.
#[derive(Clone, Debug)]
pub struct CallFrame {
    /// Definition number of the called function.
    pub d_nr: u32,
    /// Bytecode position of the call instruction (for line-number lookup).
    pub call_pos: u32,
    /// Absolute stack position of the first argument byte.
    pub args_base: u32,
    /// Total byte size of all parameters.
    pub args_size: u16,
    /// Source line number of the call site (TR1.4).  0 if unknown.
    pub line: u32,
}

/// Reserved store number for coroutine `DbRef` encoding (CO1.1).
/// Cannot clash with real Stores allocations (limited by `Stores::max`).
pub const COROUTINE_STORE: u16 = u16::MAX;

/// Lifecycle state of a coroutine frame (CO1.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoroutineStatus {
    Created,
    Suspended,
    Running,
    Exhausted,
}

/// Runtime state of a single coroutine instance (CO1.1).
/// Holds the serialised stack and metadata needed to suspend and resume.
#[derive(Clone, Debug)]
pub struct CoroutineFrame {
    /// Generator function definition number.
    pub d_nr: u32,
    /// Current lifecycle state.
    pub status: CoroutineStatus,
    /// Bytecode position to resume from (set by yield).
    pub code_pos: u32,
    /// Absolute stack position during execution.
    pub stack_base: u32,
    /// Return address in the consumer.
    pub caller_return_pos: u32,
    /// Serialised stack locals (copied on suspend, restored on resume).
    pub stack_bytes: Vec<u8>,
    /// Owned text slot copies (offset, content) taken on suspend.
    pub text_owned: Vec<(u32, String)>,
    /// Saved call stack entries from the generator's call frames.
    pub call_frames: Vec<CallFrame>,
    /// Call depth baseline when the coroutine was last running.
    pub call_depth: usize,
    /// S27 (debug-only): `text_positions` entries for this frame's locals, saved at
    /// yield and restored at resume.  Prevents stale entries from masking
    /// double-free or missing-free bugs in the consumer while the frame is suspended.
    #[cfg(debug_assertions)]
    pub saved_text_positions: std::collections::BTreeSet<u32>,
}

/// Internal State of the interpreter to run bytecode.
pub struct State {
    pub(crate) bytecode: Arc<Vec<u8>>,
    pub(crate) text_code: Arc<Vec<u8>>,
    pub(crate) stack_cur: DbRef,
    pub stack_pos: u32,
    pub code_pos: u32,
    pub(crate) def_pos: u32,
    pub(crate) source: u16,
    // The current source during the generation of code.
    pub database: Stores,
    // Stack size of the arguments
    pub arguments: u16,
    // Local function stack positions of individual byte-code statements.
    pub stack: HashMap<u32, u16>,
    // Variables from byte code, used to also gain stack position
    pub vars: HashMap<u32, u16>,
    // Calls of function definitions from byte code.
    pub calls: HashMap<u32, Vec<u32>>,
    // Information for enumerate-types and database (record, vectors and fields) types.
    pub types: HashMap<u32, u16>,
    pub library: Arc<Vec<Call>>,
    pub library_names: HashMap<String, u16>,
    pub(crate) text_positions: BTreeSet<u32>,
    pub(crate) line_numbers: BTreeMap<u32, u32>,
    pub(crate) fn_positions: Vec<u32>,
    /// Shadow call-frame vector (TR1.1).  One entry per active loft function call.
    pub call_stack: Vec<CallFrame>,
    /// TR1.3: raw pointer to `Data`, valid only during `execute_argv`.
    pub(crate) data_ptr: *const crate::data::Data,
    /// Fix #87: cached library index for `n_stack_trace`.  `u16::MAX` = not yet resolved.
    pub(crate) stack_trace_lib_nr: u16,
    /// Coroutine frame storage (CO1.1).  Index 0 is always `None` (null sentinel).
    pub coroutines: Vec<Option<Box<CoroutineFrame>>>,
    /// Indices of currently-running coroutines in `coroutines`.
    pub active_coroutines: Vec<usize>,
    /// Recursion depth counter for `generate`; reset to 0 when code generation starts.
    pub(crate) generate_depth: usize,
}

pub(crate) fn new_ref(data: &DbRef, pos: u32, arg: u16) -> DbRef {
    DbRef {
        store_nr: data.store_nr,
        rec: pos,
        pos: u32::from(arg),
    }
}

impl State {
    /**
    Create a new interpreter state
    # Panics
    When the statically defined alignment is not correct.
    */
    #[must_use]
    pub fn new(mut db: Stores) -> State {
        State {
            bytecode: Arc::new(Vec::new()),
            text_code: Arc::new(Vec::new()),
            stack_cur: db.database(1000),
            stack_pos: 4,
            code_pos: 0,
            def_pos: 0,
            source: u16::MAX,
            database: db,
            arguments: 0,
            stack: HashMap::new(),
            vars: HashMap::new(),
            calls: HashMap::new(),
            types: HashMap::new(),
            library: Arc::new(Vec::new()),
            library_names: HashMap::new(),
            text_positions: BTreeSet::new(),
            line_numbers: BTreeMap::new(),
            fn_positions: Vec::new(),
            call_stack: Vec::new(),
            data_ptr: std::ptr::null(),
            stack_trace_lib_nr: u16::MAX,
            coroutines: vec![None], // index 0 = null sentinel
            active_coroutines: Vec::new(),
            generate_depth: 0,
        }
    }

    pub fn static_fn(&mut self, name: &str, call: Call) {
        let lib = Arc::make_mut(&mut self.library);
        let nr = lib.len() as u16;
        self.library_names.insert(name.to_string(), nr);
        lib.push(call);
    }

    /// Register a native Rust function under `symbol` for use by `#native "symbol"` loft
    /// functions.  Alias for `static_fn` with an external-extension naming convention.
    pub fn register_native(&mut self, symbol: &str, call: Call) {
        self.static_fn(symbol, call);
    }

    /// Call a function, remember the current code position on the stack.
    ///
    /// * `d_nr` - definition number of the called function.
    /// * `args_size` - total byte size of all parameters.
    /// * `to` - the code position where the called function resides.
    pub fn fn_call(&mut self, d_nr: u32, args_size: u16, to: i32) {
        let args_base = self.stack_pos - u32::from(args_size);
        // Find the nearest source line at or before the current code position.
        // line_numbers entries are emitted before the first instruction on each line,
        // so after consuming a Call instruction code_pos is past the entry — use
        // range(..=code_pos).next_back() to recover the most recent line.
        let line = self
            .line_numbers
            .range(..=self.code_pos)
            .next_back()
            .map_or(0, |(_, &v)| v);
        self.call_stack.push(CallFrame {
            d_nr,
            call_pos: self.code_pos,
            args_base,
            args_size,
            line,
        });
        self.put_stack(self.code_pos);
        self.code_pos = to as u32;
    }

    /// Call a function through a runtime function reference.
    ///
    /// Reads the definition number stored in the fn-ref variable at `fn_var` bytes below the
    /// current stack top, looks up its bytecode position, then delegates to `fn_call`.
    pub fn fn_call_ref(&mut self, fn_var: u16, arg_size: u16) {
        let d_nr = *self.get_var::<i32>(fn_var) as usize;
        debug_assert!(
            d_nr < self.fn_positions.len(),
            "fn_call_ref: d_nr {d_nr} out of range"
        );
        let code_pos = self.fn_positions[d_nr] as i32;
        self.fn_call(d_nr as u32, arg_size, code_pos);
    }

    pub fn static_call(&mut self) {
        let call = *self.code::<u16>();
        // Fix #87: resolve n_stack_trace index lazily, then only snapshot for that call.
        if self.stack_trace_lib_nr == u16::MAX
            && let Some(&nr) = self.library_names.get("n_stack_trace")
        {
            self.stack_trace_lib_nr = nr;
        }
        // TR1.3: snapshot call_stack only when n_stack_trace is being called.
        // Fix #92: also works in parallel workers where data_ptr may be null;
        // frames with d_nr == u32::MAX (synthetic worker frame) get a placeholder name.
        if call == self.stack_trace_lib_nr && !self.call_stack.is_empty() {
            // SAFETY: data_ptr is set in execute_argv and valid during execution.
            let data_opt: Option<&Data> = if self.data_ptr.is_null() {
                None
            } else {
                Some(unsafe { &*self.data_ptr })
            };
            self.database.call_stack_snapshot = self
                .call_stack
                .iter()
                .map(|f| {
                    if let Some(data) = data_opt
                        && f.d_nr != u32::MAX
                        && (f.d_nr as usize) < data.definitions.len()
                    {
                        let def = &data.definitions[f.d_nr as usize];
                        let name = if def.name.starts_with("n_") {
                            def.name[2..].to_string()
                        } else {
                            def.name.clone()
                        };
                        let file = def.position.file.clone();
                        (name, file, f.line)
                    } else {
                        // Worker frame without Data context — use placeholder.
                        ("<worker>".to_string(), String::new(), f.line)
                    }
                })
                .collect();
        }
        let mut stack = self.stack_cur;
        stack.pos = 8 + self.stack_pos;
        self.library[call as usize](&mut self.database, &mut stack);
        self.stack_pos = stack.pos - 8;
    }

    /**
    Returns from a function, the data structures that went out of scope should already have
    been freed at this point.
    * `ret` - Size of the parameters to get the return address after it.
    * `value` - Size of the return value.
    * `discard` - The amount of space claimed on the stack at this point.
    # Panics
    When there are claimed texts that are not freed yet.
    */
    pub fn fn_return(&mut self, ret: u16, value: u8, discard: u16) {
        let pos = self.stack_pos;
        self.stack_pos -= u32::from(discard);
        // Clean up any text positions in the discarded range.  This can happen
        // when conditional match arms with field bindings produce text values —
        // the scope analysis may not emit OpFreeText for all branches.
        if cfg!(debug_assertions) {
            let orphans: Vec<u32> = self
                .text_positions
                .range(self.stack_pos..=pos)
                .copied()
                .collect();
            for p in orphans {
                self.text_positions.remove(&p);
            }
        }
        let fn_stack = self.stack_pos;
        self.stack_pos += u32::from(ret);
        self.code_pos = *self.get_var::<u32>(0);
        self.copy_result(value, pos, fn_stack);
        self.call_stack.pop();
    }

    // ── CO1.1 — Coroutine frame helpers ─────────────────────────────────────

    /// Allocate a coroutine frame. Returns the index (always >= 1).
    pub fn allocate_coroutine(&mut self, frame: CoroutineFrame) -> usize {
        // Reuse the first free slot (index >= 1).
        for (i, slot) in self.coroutines.iter_mut().enumerate().skip(1) {
            if slot.is_none() {
                *slot = Some(Box::new(frame));
                return i;
            }
        }
        let idx = self.coroutines.len();
        self.coroutines.push(Some(Box::new(frame)));
        idx
    }

    /// Free a coroutine frame, making the slot available for reuse.
    pub fn free_coroutine(&mut self, idx: usize) {
        if idx > 0 && idx < self.coroutines.len() {
            self.coroutines[idx] = None;
        }
    }

    /// Get a mutable reference to a coroutine frame.
    ///
    /// # Panics
    /// Panics if `idx` is 0 (null), out of range, or the slot is empty.
    pub fn coroutine_frame_mut(&mut self, idx: usize) -> &mut CoroutineFrame {
        assert!(idx > 0, "coroutine_frame_mut: null index");
        self.coroutines[idx]
            .as_mut()
            .expect("coroutine_frame_mut: empty slot")
    }

    // CO1.2: Create a coroutine frame — copy arguments into the frame without
    // entering the function body.
    pub fn coroutine_create(&mut self, d_nr: u32, args_size: u32, entry_pos: u32) {
        let args_base = self.stack_pos - args_size;
        let mut stack_bytes = vec![0u8; args_size as usize];
        let store = self.database.store(&self.stack_cur);
        let src = store.addr::<u8>(self.stack_cur.rec, self.stack_cur.pos + args_base);
        unsafe {
            std::ptr::copy_nonoverlapping(src, stack_bytes.as_mut_ptr(), args_size as usize);
        }
        // CO1.3d: append the 4-byte return-address slot expected by the function body.
        // fn_call pushes this slot for regular calls; coroutines must include it so that
        // get_var offsets computed at codegen time remain valid after resume.
        stack_bytes.extend_from_slice(&[0u8; 4]);
        self.stack_pos = args_base;

        let frame = CoroutineFrame {
            d_nr,
            status: CoroutineStatus::Created,
            code_pos: entry_pos,
            stack_base: 0,
            caller_return_pos: 0,
            stack_bytes,
            text_owned: Vec::new(),
            call_frames: Vec::new(),
            call_depth: 0,
            #[cfg(debug_assertions)]
            saved_text_positions: std::collections::BTreeSet::new(),
        };
        let idx = self.allocate_coroutine(frame);

        let db_ref = DbRef {
            store_nr: COROUTINE_STORE,
            rec: idx as u32,
            pos: 0,
        };
        self.put_stack(db_ref);
    }

    /// CO1.2: Advance a coroutine — restore stack, resume execution.
    /// # Panics
    /// Panics on re-entrant advance (coroutine already running).
    pub fn coroutine_next(&mut self, value_size: u32) {
        let gen_ref = *self.get_stack::<DbRef>();

        if gen_ref.store_nr != COROUTINE_STORE || gen_ref.rec == 0 {
            // CO1.6c: push typed null sentinel.
            self.push_null_value(value_size);
            return;
        }
        let idx = gen_ref.rec as usize;
        // S23: defense-in-depth runtime guard — coroutine DbRefs must not cross
        // thread boundaries.  Worker State instances have only a null slot at index
        // 0; a rec from the main thread would be out-of-bounds here.
        assert!(
            idx < self.coroutines.len(),
            "coroutine DbRef (rec={idx}) out of range — \
             iterator<T> values must not cross thread boundaries \
             (use a non-generator worker function in par())"
        );
        // S26: slot may be None — freed on exhaustion by coroutine_return.
        // Treat as exhausted (same as the Exhausted variant).
        if self.coroutines[idx].is_none() {
            self.push_null_value(value_size);
            return;
        }
        let status = self.coroutine_frame_mut(idx).status;

        match status {
            CoroutineStatus::Exhausted => {
                self.push_null_value(value_size);
            }
            CoroutineStatus::Running => {
                panic!("re-entrant advance on coroutine {idx}");
            }
            CoroutineStatus::Created | CoroutineStatus::Suspended => {
                let caller_return_pos = self.code_pos;
                let call_depth = self.call_stack.len();
                let stack_base = self.stack_pos;
                {
                    let f = self.coroutine_frame_mut(idx);
                    f.caller_return_pos = caller_return_pos;
                    f.call_depth = call_depth;
                    f.stack_base = stack_base;
                    f.status = CoroutineStatus::Running;
                }

                let bytes = self.coroutine_frame_mut(idx).stack_bytes.clone();
                let code_pos = self.coroutine_frame_mut(idx).code_pos;
                let saved_frames: Vec<_> = self
                    .coroutine_frame_mut(idx)
                    .call_frames
                    .drain(..)
                    .collect();

                // S27 (debug-only): restore the generator's text_positions entries
                // that were removed at yield.  The generator's locals are live again
                // once the stack bytes are copied back below.
                #[cfg(debug_assertions)]
                {
                    let saved: std::collections::BTreeSet<u32> =
                        std::mem::take(&mut self.coroutine_frame_mut(idx).saved_text_positions);
                    self.text_positions.extend(saved);
                }

                let dest = self
                    .database
                    .store_mut(&self.stack_cur)
                    .addr_mut::<u8>(self.stack_cur.rec, self.stack_cur.pos + self.stack_pos);
                unsafe {
                    std::ptr::copy_nonoverlapping(bytes.as_ptr(), dest, bytes.len());
                }
                self.stack_pos += bytes.len() as u32;
                self.call_stack.extend(saved_frames);
                self.active_coroutines.push(idx);
                self.code_pos = code_pos;
            }
        }
    }

    // CO1.6: check if a coroutine is exhausted.
    #[must_use]
    pub fn coroutine_exhausted(&self, gen_ref: &DbRef) -> bool {
        if gen_ref.store_nr != COROUTINE_STORE || gen_ref.rec == 0 {
            return true; // null iterator is exhausted
        }
        let idx = gen_ref.rec as usize;
        if idx >= self.coroutines.len() {
            return true;
        }
        match &self.coroutines[idx] {
            Some(frame) => frame.status == CoroutineStatus::Exhausted,
            None => true,
        }
    }

    // CO1.6c: push a typed null sentinel onto the stack.
    fn push_null_value(&mut self, value_size: u32) {
        match value_size {
            4 => self.put_stack(i32::MIN), // integer null sentinel
            8 => self.put_stack(i64::MIN), // long null sentinel
            _ => {
                for _ in 0..value_size {
                    self.put_stack(0u8);
                }
            }
        }
    }

    /// CO1.3b: suspend a running coroutine — serialise stack, return yielded value.
    /// # Panics
    /// Panics if no coroutine is currently active.
    pub fn coroutine_yield(&mut self, value_size: u32) {
        let idx = *self
            .active_coroutines
            .last()
            .expect("OpYield outside active coroutine");

        // Compute regions.
        let stack_top = self.stack_pos;
        let frame = self.coroutine_frame_mut(idx);
        let base = frame.stack_base;
        let value_start = stack_top - value_size;
        let locals_len = (value_start - base) as usize;

        // Serialise locals (integer-only path — no text_owned handling yet).
        let mut locals_bytes = vec![0u8; locals_len];
        let store = self.database.store(&self.stack_cur);
        let src = store.addr::<u8>(self.stack_cur.rec, self.stack_cur.pos + base);
        unsafe {
            std::ptr::copy_nonoverlapping(src, locals_bytes.as_mut_ptr(), locals_len);
        }

        // Copy yielded value bytes separately (for the slide step).
        let vs = value_size as usize;
        let mut value_bytes = vec![0u8; vs];
        let val_src = store.addr::<u8>(self.stack_cur.rec, self.stack_cur.pos + value_start);
        unsafe {
            std::ptr::copy_nonoverlapping(val_src, value_bytes.as_mut_ptr(), vs);
        }

        // Extract frame fields before mutable borrow conflicts.
        let call_depth = self.coroutine_frame_mut(idx).call_depth;
        let caller_return_pos = self.coroutine_frame_mut(idx).caller_return_pos;

        // Save call frames above the base depth.
        let saved_frames = self.call_stack[call_depth..].to_vec();
        self.call_stack.truncate(call_depth);

        let code_pos = self.code_pos;
        {
            let frame = self.coroutine_frame_mut(idx);
            frame.stack_bytes = locals_bytes;
            // text_owned stays empty — CO1.3d will handle text serialisation.
            frame.call_frames = saved_frames;
            frame.code_pos = code_pos;
            frame.status = CoroutineStatus::Suspended;
        }

        // S27 (debug-only): remove text_positions entries for the generator's locals
        // [base, value_start) and save them in the frame.  While suspended, the consumer
        // may create text values at the same absolute stack positions; keeping the
        // generator's entries would mask missing or double OpFreeText calls.
        #[cfg(debug_assertions)]
        {
            let locals_range = base..value_start;
            let to_save: std::collections::BTreeSet<u32> =
                self.text_positions.range(locals_range).copied().collect();
            for p in &to_save {
                self.text_positions.remove(p);
            }
            self.coroutine_frame_mut(idx).saved_text_positions = to_save;
        }

        self.active_coroutines.pop();

        // Slide the yielded value to stack_base.
        let dest = self
            .database
            .store_mut(&self.stack_cur)
            .addr_mut::<u8>(self.stack_cur.rec, self.stack_cur.pos + base);
        unsafe {
            std::ptr::copy_nonoverlapping(value_bytes.as_ptr(), dest, vs);
        }
        self.stack_pos = base + value_size;

        // Return to consumer.
        self.code_pos = caller_return_pos;
    }

    /// CO1.3a: exhaust a running coroutine — cleanup and return null to consumer.
    /// # Panics
    /// Panics if no coroutine is currently active.
    pub fn coroutine_return(&mut self, value_size: u32) {
        let idx = *self
            .active_coroutines
            .last()
            .expect("OpCoroutineReturn outside active coroutine");
        let frame = self.coroutine_frame_mut(idx);

        // Drop serialised state.
        frame.text_owned.clear();
        frame.stack_bytes.clear();

        let call_depth = frame.call_depth;
        let stack_base = frame.stack_base;
        let caller_return_pos = frame.caller_return_pos;

        // Exhaust and immediately free the slot (S26).
        // Setting the slot to None prevents unbounded growth of the coroutines table
        // when many generators are created over a program's lifetime.
        // coroutine_exhausted() treats None as exhausted, so callers see no difference.
        frame.status = CoroutineStatus::Exhausted;
        self.active_coroutines.pop();
        // Free the slot: coroutine_exhausted() returns true for None entries.
        self.coroutines[idx] = None;

        // Restore call stack to consumer depth.
        self.call_stack.truncate(call_depth);

        // Rewind stack to frame base; push typed null.
        self.stack_pos = stack_base;
        // Zero-fill value_size bytes as the null return value.
        for _ in 0..value_size {
            self.put_stack(0u8);
        }

        // Return to consumer.
        self.code_pos = caller_return_pos;
    }

    /**
    Clear the stack of local variables, possibly return a value.
    * `value` - Size of the return value.
    * `discard` - The amount of space claimed on the stack at this point.
    # Panics
    When texts are not freed from the stack beforehand.
    */
    pub fn free_stack(&mut self, value: u8, discard: u16) {
        let pos = self.stack_pos;
        self.stack_pos -= u32::from(discard);
        if cfg!(debug_assertions) {
            let orphans: Vec<u32> = self
                .text_positions
                .range(self.stack_pos..=pos)
                .copied()
                .collect();
            for p in orphans {
                self.text_positions.remove(&p);
            }
        }
        self.copy_result(value, pos, self.stack_pos);
    }

    /// Advance the stack pointer by `size` bytes, reserving space for pre-claimed variables.
    pub fn reserve_frame(&mut self, size: u16) {
        self.stack_pos += u32::from(size);
    }

    pub(crate) fn copy_result(&mut self, value: u8, pos: u32, fn_stack: u32) {
        let size = u32::from(value);
        if value > 0 {
            let from_pos = self.stack_cur.plus(pos).min(size);
            let to_pos = self.stack_cur.plus(fn_stack);
            self.database.copy_block(&from_pos, &to_pos, size);
        }
        self.stack_pos = fn_stack + size;
    }

    /**
    Write to the byte code.
    # Panics
    When that was problematic
    */
    pub fn code_put<T>(&mut self, on: u32, value: T) {
        unsafe {
            let off = Arc::make_mut(&mut self.bytecode)
                .as_mut_ptr()
                .offset(on as isize)
                .cast::<T>();
            *off.as_mut().expect("code") = value;
        }
    }

    /** Remember the stack position for the current code. */
    pub fn remember_stack(&mut self, position: u16) {
        self.stack.insert(self.code_pos, position);
    }

    /**
    Add to the byte code.
    # Panics
    When that was problematic
    */
    pub fn code_add<T: std::fmt::Display>(&mut self, value: T) {
        let bc = Arc::make_mut(&mut self.bytecode);
        if self.code_pos as usize + size_of::<T>() > bc.len() {
            bc.resize(self.code_pos as usize + size_of::<T>(), 0);
        }
        unsafe {
            let off = bc.as_mut_ptr().offset(self.code_pos as isize).cast::<T>();
            self.code_pos += u32::try_from(size_of::<T>()).expect("Problem");
            *off.as_mut().expect("code") = value;
        }
    }

    pub fn code_add_str(&mut self, value: &str) {
        self.code_add(value.len() as u8);
        let bc = Arc::make_mut(&mut self.bytecode);
        if self.code_pos as usize + value.len() > bc.len() {
            bc.resize(self.code_pos as usize + value.len(), 0);
        }
        unsafe {
            let off = bc.as_mut_ptr().offset(self.code_pos as isize);
            value.as_ptr().copy_to(off, value.len());
        }
        self.code_pos += value.len() as u32;
    }

    /** Get a value from the byte-code increasing the position to after this value
    # Panics
    When the position is outside the byte-code
    */
    pub fn code<T>(&mut self) -> &T {
        assert!(
            self.code_pos + (size_of::<T>() as u32) <= self.bytecode.len() as u32,
            "Position {} + {} outside generated code {}",
            self.code_pos,
            size_of::<T>(),
            self.bytecode.len()
        );
        unsafe {
            let off = self
                .bytecode
                .as_ptr()
                .offset(self.code_pos as isize)
                .cast::<T>();
            self.code_pos += size_of::<T>() as u32;
            off.as_ref().expect("code")
        }
    }

    pub fn code_str(&mut self) -> &str {
        let len = *self.code::<u8>();
        unsafe {
            let off = self.bytecode.as_ptr().offset(self.code_pos as isize);
            self.code_pos += u32::from(len);
            std::str::from_utf8_unchecked(std::slice::from_raw_parts(off, len as usize))
        }
    }

    pub fn static_str(&mut self) -> &str {
        let from = *self.code::<u32>() as usize;
        let len = *self.code::<u32>() as usize;
        std::str::from_utf8(&self.text_code[from..from + len]).unwrap_or_default()
    }

    /**
    Pull a value from stack
    # Panics
    When the stack has no values left
    */
    #[must_use]
    pub fn get_stack<T>(&mut self) -> &T {
        assert!(
            (size_of::<T>() as u32) < self.stack_pos,
            "No elements left on the stack {} < {}",
            self.stack_pos,
            size_of::<T>() as u32
        );
        self.stack_pos -= size_of::<T>() as u32;
        self.database
            .store(&self.stack_cur)
            .addr::<T>(self.stack_cur.rec, self.stack_cur.pos + self.stack_pos)
    }

    /// superinstruction stubs — delegated from fill.rs.
    /// These are placeholders; the peephole pass is not yet active.
    #[allow(clippy::unused_self)]
    pub fn nop(&mut self) {}
    pub fn si_load2_add_store(&mut self) {
        self.nop();
    }
    pub fn si_load_const_add_store(&mut self) {
        self.nop();
    }
    pub fn si_load_const_cmp_branch(&mut self) {
        self.nop();
    }
    pub fn si_load2_cmp_branch(&mut self) {
        self.nop();
    }
    pub fn si_load_const_mul_store(&mut self) {
        self.nop();
    }
    pub fn si_load2_mul_store(&mut self) {
        self.nop();
    }

    pub fn get_var<T>(&mut self, pos: u16) -> &T {
        // get_var reads T at (stack_pos - pos); pos > stack_pos would underflow.
        // pos < size_of::<T>() is also invalid (read extends before the frame base).
        // Note: pos == 0 is valid when accessing a pre-reserved frame slot above the
        // current evaluation stack (e.g. immediately after ReserveFrame).
        debug_assert!(
            u32::from(pos) <= self.stack_pos,
            "get_var: pos={pos} exceeds stack_pos={} (frame underflow)",
            self.stack_pos
        );
        self.database.store(&self.stack_cur).addr::<T>(
            self.stack_cur.rec,
            self.stack_cur.pos + self.stack_pos - u32::from(pos),
        )
    }

    pub fn mut_var<T>(&mut self, pos: u16) -> &mut T {
        debug_assert!(
            u32::from(pos) <= self.stack_pos,
            "mut_var: pos={pos} exceeds stack_pos={} (frame underflow)",
            self.stack_pos
        );
        self.database.store_mut(&self.stack_cur).addr_mut::<T>(
            self.stack_cur.rec,
            self.stack_cur.pos + self.stack_pos - u32::from(pos),
        )
    }

    pub fn put_var<T>(&mut self, pos: u16, value: T) {
        *self.database.store_mut(&self.stack_cur).addr_mut::<T>(
            self.stack_cur.rec,
            self.stack_cur.pos + self.stack_pos + size_of::<T>() as u32 - u32::from(pos),
        ) = value;
    }

    pub fn put_stack<T>(&mut self, val: T) {
        let m = self
            .database
            .store_mut(&self.stack_cur)
            .addr_mut::<T>(self.stack_cur.rec, self.stack_cur.pos + self.stack_pos);
        *m = val;
        self.stack_pos += size_of::<T>() as u32;
    }

    /**
    Execute a function inside the `byte_code`.
    # Panics
    When too many steps were taken, this might indicate an unending loop.
    */
    pub fn execute(&mut self, name: &str, data: &Data) {
        self.execute_argv(name, data, &[]);
    }

    /// Execute entry-point `name`, optionally passing `argv` as a `vector<text>` argument.
    ///
    /// If the named function has exactly one `vector<…>` parameter, the strings in `argv`
    /// are built into a `vector<text>` and pushed onto the stack before the return address.
    /// If the function takes no parameters, `argv` is ignored.
    ///
    /// # Panics
    /// Panics if the program executes more than 10 000 000 operations (infinite-loop guard).
    pub fn execute_argv(&mut self, name: &str, data: &Data, argv: &[String]) {
        let d_nr = data.def_nr(&format!("n_{name}"));
        let pos = data.def(d_nr).code_position;

        // Expose bytecode, text_code, library, and Data to native functions
        // that need to spawn worker threads (e.g. n_parallel_for_int).
        let bc_ptr = &raw const self.bytecode;
        let tc_ptr = &raw const self.text_code;
        let lib_ptr = &raw const self.library;
        let data_ptr = std::ptr::from_ref::<Data>(data);
        self.data_ptr = data_ptr;
        let stk_lib_nr = self
            .library_names
            .get("n_stack_trace")
            .copied()
            .unwrap_or(u16::MAX);
        self.database.parallel_ctx = Some(Box::new(ParallelCtx {
            bytecode: bc_ptr,
            text_code: tc_ptr,
            library: lib_ptr,
            data: data_ptr,
            stack_trace_lib_nr: stk_lib_nr,
        }));

        self.fn_positions = data.definitions.iter().map(|d| d.code_position).collect();
        self.code_pos = pos;
        self.stack_pos = 4;
        // Fix #88: push a synthetic CallFrame for the entry function so it
        // appears in stack_trace() output.
        self.call_stack.push(CallFrame {
            d_nr,
            call_pos: 0,
            args_base: 4,
            args_size: 0,
            line: 0,
        });
        // If fn main declares a vector<text> parameter, push argv before the return address.
        let attrs = &data.def(d_nr).attributes;
        if attrs.len() == 1 && matches!(attrs[0].typedef, Type::Vector(_, _)) {
            let args_vec = self.database.text_vector(argv);
            self.put_stack(args_vec);
        }
        self.put_stack(u32::MAX);
        #[cfg(debug_assertions)]
        let mut step = 0;
        #[cfg(debug_assertions)]
        let mut trail_pos = [u32::MAX; 16usize];
        #[cfg(debug_assertions)]
        let mut trail_op = [0u8; 16usize];
        #[cfg(debug_assertions)]
        let mut trail_head: usize = 0;
        let bytecode_len = self.bytecode.len() as u32;
        while self.code_pos < bytecode_len {
            #[cfg(debug_assertions)]
            let op_pos = self.code_pos;
            let op = *self.code::<u8>();
            #[cfg(debug_assertions)]
            {
                trail_pos[trail_head] = op_pos;
                trail_op[trail_head] = op;
                trail_head = (trail_head + 1) % 16;
            }
            OPERATORS[op as usize](self);
            #[cfg(debug_assertions)]
            {
                step += 1;
            }
            #[cfg(debug_assertions)]
            if step >= 10_000_000 {
                use std::fmt::Write as _;
                let mut msg = String::from("Too many operations (infinite loop?). Last 16 ops:\n");
                for i in 0..16usize {
                    let idx = (trail_head + i) % 16;
                    if trail_pos[idx] == u32::MAX {
                        continue;
                    }
                    let pos = trail_pos[idx];
                    let fn_nr = Self::fn_d_nr_for_pos(pos, data);
                    let (label, offset) = if fn_nr == u32::MAX {
                        ("?".to_owned(), pos)
                    } else {
                        (
                            data.def(fn_nr).name.trim_start_matches("n_").to_owned(),
                            pos - data.def(fn_nr).code_position,
                        )
                    };
                    let op_name = (0..data.definitions())
                        .find(|&d| data.def(d).op_code == u16::from(trail_op[idx]))
                        .map_or("?", |d| data.def(d).name.as_str());
                    let _ = writeln!(msg, "  {label}+{offset}: {op_name}");
                }
                panic!("{msg}");
            }
            if self.code_pos == u32::MAX {
                break;
            }
        }

        // Fix #88: pop the synthetic entry-function frame.
        self.call_stack.pop();
        self.database.parallel_ctx = None;
    }

    /// Snapshot the bytecode, text segment, and native-function library for
    /// use in a parallel worker thread.  All three are `Arc`-cloned — O(1).
    #[must_use]
    pub fn worker_program(&self) -> crate::parallel::WorkerProgram {
        // Resolve n_stack_trace now so workers can call stack_trace() (fix #92).
        let stack_trace_lib_nr = self
            .library_names
            .get("n_stack_trace")
            .copied()
            .unwrap_or(u16::MAX);
        crate::parallel::WorkerProgram {
            bytecode: Arc::clone(&self.bytecode),
            text_code: Arc::clone(&self.text_code),
            library: Arc::clone(&self.library),
            stack_trace_lib_nr,
        }
    }

    /// Create a `State` for use in a parallel worker thread.
    ///
    /// `db` should be built with `Stores::clone_for_worker()`; this call
    /// allocates a fresh stack store at the next available index in `db`.
    #[must_use]
    pub fn new_worker(
        mut db: Stores,
        bytecode: Arc<Vec<u8>>,
        text_code: Arc<Vec<u8>>,
        library: Arc<Vec<Call>>,
    ) -> State {
        State {
            stack_cur: db.database(1000),
            stack_pos: 4,
            code_pos: 0,
            def_pos: 0,
            source: u16::MAX,
            database: db,
            arguments: 0,
            bytecode,
            text_code,
            library,
            library_names: HashMap::new(),
            stack: HashMap::new(),
            vars: HashMap::new(),
            calls: HashMap::new(),
            types: HashMap::new(),
            text_positions: BTreeSet::new(),
            line_numbers: BTreeMap::new(),
            fn_positions: Vec::new(),
            call_stack: Vec::new(),
            data_ptr: std::ptr::null(),
            stack_trace_lib_nr: u16::MAX,
            coroutines: vec![None],
            active_coroutines: Vec::new(),
            generate_depth: 0,
        }
    }

    /// Execute the bytecode function at `fn_pos` passing one `DbRef` argument,
    /// then return the `i32` result left on the stack.
    ///
    /// Stack layout built here:
    /// ```text
    ///   [arg: DbRef (12 bytes)][return-addr u32::MAX (4 bytes)]
    /// ```
    /// This matches what `fn_return(ret=12, value=4, discard=D)` expects.
    ///
    /// # Panics
    /// Panics if the worker executes more than 10 000 000 operations (infinite-loop guard).
    pub fn execute_at(&mut self, fn_pos: u32, arg: &DbRef) -> i32 {
        // Fix #92: propagate data_ptr, stack_trace_lib_nr, and fn_positions from
        // ParallelCtx so that stack_trace() works inside parallel workers called via
        // n_parallel_for_int.  When parallel_ctx is None (direct run_parallel_* path),
        // stack_trace_lib_nr is already set by WorkerProgram::new_state — don't clobber it.
        if let Some(ctx) = &self.database.parallel_ctx {
            self.data_ptr = ctx.data;
            self.stack_trace_lib_nr = ctx.stack_trace_lib_nr;
            if self.fn_positions.is_empty() && !ctx.data.is_null() {
                let data = unsafe { &*ctx.data };
                self.fn_positions = data.definitions.iter().map(|d| d.code_position).collect();
            }
        }
        let d_nr = self
            .fn_positions
            .iter()
            .position(|&p| p == fn_pos)
            .map_or(u32::MAX, |i| i as u32);
        self.call_stack.push(CallFrame {
            d_nr,
            call_pos: 0,
            args_base: 4,
            args_size: 12,
            line: 0,
        });
        self.stack_pos = 4;
        self.put_stack(*arg); // 12 bytes → stack_pos = 16
        self.put_stack(u32::MAX); // 4 bytes  → stack_pos = 20
        self.code_pos = fn_pos;
        let mut step = 0;
        let bytecode_len = self.bytecode.len() as u32;
        while self.code_pos < bytecode_len {
            let op = *self.code::<u8>();
            OPERATORS[op as usize](self);
            step += 1;
            debug_assert!(step < 10_000_000, "Worker: too many operations");
            if self.code_pos == u32::MAX {
                break;
            }
        }
        *self.get_stack::<i32>()
    }

    /// Execute a worker function at `fn_pos`, return raw result bits as `u64`.
    pub fn execute_at_raw(
        &mut self,
        fn_pos: u32,
        arg: &DbRef,
        extra_args: &[u64],
        return_size: u32,
    ) -> u64 {
        if let Some(ctx) = &self.database.parallel_ctx {
            self.data_ptr = ctx.data;
            self.stack_trace_lib_nr = ctx.stack_trace_lib_nr;
            if self.fn_positions.is_empty() && !ctx.data.is_null() {
                let data = unsafe { &*ctx.data };
                self.fn_positions = data.definitions.iter().map(|d| d.code_position).collect();
            }
        }
        let d_nr = self
            .fn_positions
            .iter()
            .position(|&p| p == fn_pos)
            .map_or(u32::MAX, |i| i as u32);
        self.call_stack.push(CallFrame {
            d_nr,
            call_pos: 0,
            args_base: 4,
            args_size: 12,
            line: 0,
        });
        self.stack_pos = 4;
        // Push extra context args first (they precede the element arg in the
        // function's parameter list: fn worker(element, extra1, extra2, ...)).
        // The stack grows upward; the function reads params from low to high offset.
        // Element arg (DbRef) occupies the first parameter slot; extras follow.
        self.put_stack(*arg); // 12 bytes
        for &extra in extra_args {
            // Push each extra as a raw i32 (integer context args).
            self.put_stack(extra as i32);
        }
        self.put_stack(u32::MAX); // return address sentinel
        self.code_pos = fn_pos;
        let mut step = 0;
        let bytecode_len = self.bytecode.len() as u32;
        while self.code_pos < bytecode_len {
            let op = *self.code::<u8>();
            OPERATORS[op as usize](self);
            step += 1;
            debug_assert!(step < 10_000_000, "Worker: too many operations");
            if self.code_pos == u32::MAX {
                break;
            }
        }
        match return_size {
            8 => *self.get_stack::<u64>(),
            1 => u64::from(*self.get_stack::<u8>()),
            _ => u64::from(*self.get_stack::<u32>()),
        }
    }

    /// Execute a worker function that returns a struct reference (`DbRef`).
    /// Returns the 12-byte `DbRef` from the worker's stack.  The referenced
    /// record lives in `self.database` (the worker's cloned stores).
    pub fn execute_at_ref(&mut self, fn_pos: u32, arg: &DbRef, extra_args: &[u64]) -> DbRef {
        if let Some(ctx) = &self.database.parallel_ctx {
            self.data_ptr = ctx.data;
            self.stack_trace_lib_nr = ctx.stack_trace_lib_nr;
            if self.fn_positions.is_empty() && !ctx.data.is_null() {
                let data = unsafe { &*ctx.data };
                self.fn_positions = data.definitions.iter().map(|d| d.code_position).collect();
            }
        }
        let d_nr = self
            .fn_positions
            .iter()
            .position(|&p| p == fn_pos)
            .map_or(u32::MAX, |i| i as u32);
        self.call_stack.push(CallFrame {
            d_nr,
            call_pos: 0,
            args_base: 4,
            args_size: 12,
            line: 0,
        });
        self.stack_pos = 4;
        self.put_stack(*arg);
        for &extra in extra_args {
            self.put_stack(extra as i32);
        }
        self.put_stack(u32::MAX);
        self.code_pos = fn_pos;
        let mut step = 0;
        let bytecode_len = self.bytecode.len() as u32;
        while self.code_pos < bytecode_len {
            let op = *self.code::<u8>();
            OPERATORS[op as usize](self);
            step += 1;
            debug_assert!(step < 10_000_000, "Worker: too many operations");
            if self.code_pos == u32::MAX {
                break;
            }
        }
        *self.get_stack::<DbRef>()
    }

    /// Execute a text-returning worker function; copy the `Str` result to an owned
    /// `String` before the worker state is dropped. Allocates `String` buffers in the
    /// stack store for hidden `__work_N` parameters.
    pub fn execute_at_text(
        &mut self,
        fn_pos: u32,
        arg: &DbRef,
        extra_args: &[u64],
        n_hidden_text: usize,
    ) -> String {
        if let Some(ctx) = &self.database.parallel_ctx {
            self.data_ptr = ctx.data;
            self.stack_trace_lib_nr = ctx.stack_trace_lib_nr;
            if self.fn_positions.is_empty() && !ctx.data.is_null() {
                let data = unsafe { &*ctx.data };
                self.fn_positions = data.definitions.iter().map(|d| d.code_position).collect();
            }
        }
        let d_nr = self
            .fn_positions
            .iter()
            .position(|&p| p == fn_pos)
            .map_or(u32::MAX, |i| i as u32);
        self.call_stack.push(CallFrame {
            d_nr,
            call_pos: 0,
            args_base: 4,
            args_size: 12,
            line: 0,
        });
        // Allocate String buffers for hidden RefVar(Text) params in the stack store.
        let mut work_crs: Vec<DbRef> = Vec::with_capacity(n_hidden_text);
        for _ in 0..n_hidden_text {
            let cr = self.database.claim(&self.stack_cur, 4); // 32 bytes; String needs 24
            unsafe {
                let p = self
                    .database
                    .store_mut(&self.stack_cur)
                    .addr_mut::<String>(cr.rec, cr.pos);
                let p = std::ptr::from_mut(p);
                std::ptr::write(p, String::new());
            }
            work_crs.push(cr);
        }

        self.stack_pos = 4;
        self.put_stack(*arg);
        for &extra in extra_args {
            self.put_stack(extra as i32);
        }
        // Push the work buffer DbRefs as the hidden parameters.
        for cr in &work_crs {
            self.put_stack(*cr);
        }
        self.put_stack(u32::MAX);
        self.code_pos = fn_pos;
        let mut step = 0;
        let bytecode_len = self.bytecode.len() as u32;
        while self.code_pos < bytecode_len {
            let op = *self.code::<u8>();
            OPERATORS[op as usize](self);
            step += 1;
            debug_assert!(step < 10_000_000, "Worker: too many operations");
            if self.code_pos == u32::MAX {
                break;
            }
        }
        // Pop the Str return value (16 bytes) and copy into owned String.
        let s = *self.get_stack::<Str>();
        let result = s.str().to_owned();
        // Drop the String buffers to free their heap allocations.
        for cr in work_crs.iter().rev() {
            unsafe {
                let p = self
                    .database
                    .store_mut(&self.stack_cur)
                    .addr_mut::<String>(cr.rec, cr.pos);
                let p = std::ptr::from_mut(p);
                std::ptr::drop_in_place(p);
            }
        }
        result
    }

    /**
    Execute a function inside the `byte_code` with logging each step.

    The `config` parameter controls which phases, functions, and opcodes appear
    in the output.  When `config.trace_tail` is set the execution trace is held
    in a ring buffer; if a panic occurs the buffer is flushed to `log` before
    the panic is re-raised, giving you the last N lines at the crash site.

    When `config.phases.execution` is `false`, or the function name does not
    match `config.show_functions`, the function is executed silently (same as
    [`Self::execute`]).

    # Errors
    When the log cannot be written.
    # Panics
    On too many steps or when the stack or claimed structures are not correctly
    cleared afterward.
    */
    pub fn execute_log(
        &mut self,
        log: &mut dyn Write,
        name: &str,
        config: &LogConfig,
        data: &Data,
    ) -> Result<(), Error> {
        debug::execute_log_impl(self, log, name, config, data)
    }
}

#[inline]
#[must_use]
pub fn size_ptr() -> u32 {
    size_of::<crate::keys::Str>() as u32
}

#[inline]
#[must_use]
pub fn size_str() -> u32 {
    size_of::<String>() as u32
}

#[inline]
#[must_use]
pub fn size_ref() -> u32 {
    size_of::<DbRef>() as u32
}
