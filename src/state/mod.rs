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
use crate::keys::DbRef;
use crate::log_config::LogConfig;
use std::collections::{BTreeSet, HashMap};
use std::io::{Error, Write};
use std::sync::Arc;

pub const STRING_NULL: &str = "\0";

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
    pub(crate) line_numbers: HashMap<u32, u32>,
    pub(crate) fn_positions: Vec<u32>,
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
            line_numbers: HashMap::new(),
            fn_positions: Vec::new(),
        }
    }

    pub fn static_fn(&mut self, name: &str, call: Call) {
        let lib = Arc::make_mut(&mut self.library);
        let nr = lib.len() as u16;
        self.library_names.insert(name.to_string(), nr);
        lib.push(call);
    }

    /// Call a function, remember the current code position on the stack.
    ///
    /// * `size` - the amount of stack space maximally needed for the new function.
    /// * `to` - the code position where the called function resides.
    pub fn fn_call(&mut self, _size: u16, to: i32) {
        self.put_stack(self.code_pos);
        // TODO allow to switch stacks
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
        self.fn_call(arg_size, code_pos);
    }

    pub fn static_call(&mut self) {
        let call = *self.code::<u16>();
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
        debug_assert!(
            self.text_positions
                .range(self.stack_pos..=pos)
                .next()
                .is_none(),
            "Not freed texts on return: {}",
            self.text_positions
                .range(self.stack_pos..=pos)
                .next()
                .unwrap()
        );
        let fn_stack = self.stack_pos;
        self.stack_pos += u32::from(ret);
        self.code_pos = *self.get_var::<u32>(0);
        self.copy_result(value, pos, fn_stack);
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
        debug_assert!(
            self.text_positions
                .range(self.stack_pos..=pos)
                .next()
                .is_none(),
            "Not freed texts"
        );
        self.copy_result(value, pos, self.stack_pos);
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

    pub fn get_var<T>(&mut self, pos: u16) -> &T {
        self.database.store(&self.stack_cur).addr::<T>(
            self.stack_cur.rec,
            self.stack_cur.pos + self.stack_pos - u32::from(pos),
        )
    }

    pub fn mut_var<T>(&mut self, pos: u16) -> &mut T {
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
    pub fn execute_argv(&mut self, name: &str, data: &Data, argv: &[String]) {
        let d_nr = data.def_nr(&format!("n_{name}"));
        let pos = data.def(d_nr).code_position;

        // Expose bytecode, text_code, library, and Data to native functions
        // that need to spawn worker threads (e.g. n_parallel_for_int).
        let bc_ptr = &raw const self.bytecode;
        let tc_ptr = &raw const self.text_code;
        let lib_ptr = &raw const self.library;
        let data_ptr = std::ptr::from_ref::<Data>(data);
        self.database.parallel_ctx = Some(Box::new(ParallelCtx {
            bytecode: bc_ptr,
            text_code: tc_ptr,
            library: lib_ptr,
            data: data_ptr,
        }));

        // Drop all temporary strings from the previous execute call before starting a new one.
        // After execute() returns, stack_pos is reset, so no Str pointer can still reference them.
        self.database.scratch.clear();
        self.fn_positions = data.definitions.iter().map(|d| d.code_position).collect();
        self.code_pos = pos;
        self.stack_pos = 4;
        // If fn main declares a vector<text> parameter, push argv before the return address.
        let attrs = &data.def(d_nr).attributes;
        if attrs.len() == 1 && matches!(attrs[0].typedef, Type::Vector(_, _)) {
            let args_vec = self.database.text_vector(argv);
            self.put_stack(args_vec);
        }
        self.put_stack(u32::MAX);
        let mut step = 0;
        let bytecode_len = self.bytecode.len() as u32;
        while self.code_pos < bytecode_len {
            let op = *self.code::<u8>();
            OPERATORS[op as usize](self);
            step += 1;
            debug_assert!(step < 10_000_000, "Too many operations");
            if self.code_pos == u32::MAX {
                break;
            }
        }

        self.database.parallel_ctx = None;
    }

    /// Snapshot the bytecode, text segment, and native-function library for
    /// use in a parallel worker thread.  All three are `Arc`-cloned — O(1).
    #[must_use]
    pub fn worker_program(&self) -> crate::parallel::WorkerProgram {
        crate::parallel::WorkerProgram {
            bytecode: Arc::clone(&self.bytecode),
            text_code: Arc::clone(&self.text_code),
            library: Arc::clone(&self.library),
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
            line_numbers: HashMap::new(),
            fn_positions: Vec::new(),
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

    /// Execute the bytecode function at `fn_pos` passing one `DbRef` argument,
    /// then return the raw result bits in a `u64`.  The caller must supply the
    /// `return_size` (in bytes: 1, 4, or 8) to select the right pop width.
    ///
    /// Stack layout built here:
    /// ```text
    ///   [arg: DbRef (12 bytes)][return-addr u32::MAX (4 bytes)]
    /// ```
    ///
    /// # Panics
    /// Panics if the worker executes more than 10 000 000 operations.
    pub fn execute_at_raw(&mut self, fn_pos: u32, arg: &DbRef, return_size: u32) -> u64 {
        self.stack_pos = 4;
        self.put_stack(*arg); // 12 bytes → stack_pos = 16
        self.put_stack(u32::MAX); // 4 bytes → stack_pos = 20
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
