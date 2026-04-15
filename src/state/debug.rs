// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

use super::{STRING_NULL, State};
use crate::data::{Attribute, Context, Data, DefType, Definition, I32, Type};
use crate::fill::OPERATORS;
use crate::keys::{DbRef, Key};
use crate::log_config::{LogConfig, TailBuffer};
use crate::native::FUNCTIONS;
use crate::variables::size;
use std::collections::{BTreeMap, HashMap};
use std::io::{Error, Write};
use std::str::FromStr;

// ------------------------------------------------------------------ StackDiagLevel

/// Controls how much detail [`State::validate_stack`] writes to its output.
///
/// Variants are ordered from least to most verbose; you can compare them with `>=`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum StackDiagLevel {
    /// Count anomalies without writing anything.
    Silent,
    /// Single summary line: `stack_pos`, compile-time depth (if available),
    /// anomaly count.
    Brief,
    /// Summary + 16-byte-wide hex dump of the entire active stack region,
    /// with any caller-supplied range labels shown after each affected line.
    Hex,
    /// Hex dump + `DbRef` anomaly detail + per-variable slot annotations derived
    /// from compile-time metadata (requires `data` to be `Some`).
    Full,
}

/// Return the current call frame's `args_base`, or 4 (entry function) when
/// the call stack is empty.  Used by [`State::dump_frame_variables`].
fn frame_base_or(state: &State) -> u32 {
    state.call_stack.last().map_or(4, |cf| cf.args_base)
}

// ------------------------------------------------------------------ FrameVariable

/// One slot-assigned variable in the current call frame, with a snapshot of its
/// runtime value.  Produced by [`State::iter_frame_variables`].
#[derive(Debug, Clone)]
pub struct FrameVariable {
    pub var_nr: u16,
    pub name: String,
    pub typedef: Type,
    /// Slot offset from the frame base (compile-time, never `u16::MAX` here).
    pub slot: u16,
    /// Absolute byte offset within the stack store (slot + frame_base).
    pub abs_pos: u32,
    /// Width of the slot in bytes (`Context::Variable`).
    pub size: u16,
    pub is_argument: bool,
    pub scope: u16,
    pub value: VariableValue,
    /// True when the variable is live at the current `code_pos` — derived from
    /// the bytecode reference range in `State.vars`.  Stale variables (slot
    /// coalesced with another live variable) are reported but marked.
    pub live: bool,
    /// First bytecode position referencing this variable in the current
    /// function, or `u32::MAX` if never referenced.
    pub bc_first: u32,
    /// Last bytecode position referencing this variable, or `u32::MAX`.
    pub bc_last: u32,
}

/// A safely-read snapshot of a variable's value.  Never dereferences raw
/// pointers without bounds-checking — corrupted slots produce `Unreadable` or
/// `OutOfFrame` rather than crashing.
#[derive(Debug, Clone)]
pub enum VariableValue {
    Integer(i32),
    Long(i64),
    Single(f32),
    Float(f64),
    Boolean(bool),
    Character(char),
    /// Text variable (String storage).  `cap`/`ptr`/`len` are the raw 24-byte
    /// String layout fields; `content` is `Some(s)` only when the pointer
    /// passes `Str::str()`'s safety guard.
    Text {
        cap: u64,
        ptr: u64,
        len: u64,
        content: Option<String>,
    },
    /// `Str` view (16 bytes — used for text on the eval stack).
    StrView {
        ptr: u64,
        len: u32,
        content: Option<String>,
    },
    Reference(DbRef),
    Vector(DbRef),
    /// Slot lies above the current `stack_pos` (not yet allocated this frame).
    OutOfFrame,
    /// Slot is below `stack_pos` but the read failed (bounds, alignment, or
    /// store-not-mapped).  The string explains why.
    Unreadable(&'static str),
    /// Type variant not yet handled by the introspection framework.
    Unsupported,
}

impl State {
    /**
    Print the byte-code
    # Panics
    When unknown operators are encountered in the byte-code.
    */
    pub fn print_code(&mut self, d_nr: u32, data: &Data) {
        let mut buf = Vec::new();
        self.dump_code(&mut buf, d_nr, data, true).unwrap();
        println!("{}", String::from_utf8(buf).unwrap());
    }

    /// Validate the interpreter stack and write a diagnostic hex dump.
    /// Returns the number of anomalies (bounds, alignment, stale `DbRef`).
    ///
    /// # Errors
    /// Propagates I/O errors from the writer.
    #[allow(clippy::too_many_lines)]
    pub fn validate_stack(
        &self,
        f: &mut dyn Write,
        code_pos: u32,
        data: Option<&Data>,
        level: StackDiagLevel,
        extras: &[(u32, u32, &str)],
    ) -> Result<usize, Error> {
        let mut anomalies = 0usize;
        let store = self.database.store(&self.stack_cur);
        let store_bytes = store.byte_capacity();
        let base = self.stack_cur.pos; // base byte offset within the record
        let sp = self.stack_pos;

        // ---- 1. Bounds ----
        if u64::from(base) + u64::from(sp) > store_bytes {
            anomalies += 1;
            if level > StackDiagLevel::Silent {
                writeln!(
                    f,
                    "[STACK] OVERFLOW: stack_pos={sp} base={base} \
                     top={} store_bytes={}",
                    u64::from(base) + u64::from(sp),
                    store_bytes
                )?;
            }
        }

        // ---- 2. Alignment ----
        if !sp.is_multiple_of(4) {
            anomalies += 1;
            if level > StackDiagLevel::Silent {
                writeln!(f, "[STACK] MISALIGNED: stack_pos={sp} not 4-byte aligned")?;
            }
        }

        // ---- 3. Compile-time depth comparison ----
        let compile_sp: Option<u16> = if code_pos == u32::MAX {
            None
        } else {
            self.stack.get(&code_pos).copied()
        };
        if let Some(csp) = compile_sp
            && u32::from(csp) != sp
        {
            anomalies += 1;
        }

        // ---- 4. DbRef anomaly scan ----
        // Walk every 4-byte aligned offset and treat every 12-byte window as a
        // potential DbRef.  Flag those whose store_nr is non-zero but >= max.
        let max_store = self.database.allocations.len() as u16;
        let safe_top = sp.min((store_bytes.saturating_sub(u64::from(base))) as u32);
        // Collect anomalous DbRef positions for the hex dump annotation.
        let mut dbref_anomalies: Vec<(u32, u16, u32, u32)> = Vec::new(); // (offset, sn, rec, pos)
        if safe_top >= 12 {
            let mut off = 0u32;
            while off + 12 <= safe_top {
                // Read the three 4-byte words manually from consecutive bytes to
                // avoid any potential alignment or bounds issue.
                let b = |o: u32| -> u8 { *store.addr::<u8>(self.stack_cur.rec, base + o) };
                let w0 = u32::from_le_bytes([b(off), b(off + 1), b(off + 2), b(off + 3)]);
                let sn = (w0 & 0xFFFF) as u16; // store_nr lives in the low 2 bytes
                let rec = u32::from_le_bytes([b(off + 4), b(off + 5), b(off + 6), b(off + 7)]);
                let pos = u32::from_le_bytes([b(off + 8), b(off + 9), b(off + 10), b(off + 11)]);
                if sn != 0 && sn < u16::MAX && sn >= max_store {
                    anomalies += 1;
                    dbref_anomalies.push((off, sn, rec, pos));
                    if level > StackDiagLevel::Silent && level < StackDiagLevel::Hex {
                        // In Brief mode print the anomaly inline.
                        writeln!(
                            f,
                            "[STACK] SUSPECT DbRef @{off}: store_nr={sn} \
                             (max={max_store}) rec={rec} pos={pos}"
                        )?;
                    }
                }
                off += 4;
            }
        }

        if level == StackDiagLevel::Silent {
            return Ok(anomalies);
        }

        // ---- 5. Summary line ----
        write!(f, "[STACK] stack_pos={sp}")?;
        match compile_sp {
            Some(csp) if u32::from(csp) == sp => write!(f, " compile={csp}[ok]")?,
            Some(csp) => write!(f, " compile={csp}[MISMATCH runtime={sp}]")?,
            None => {}
        }
        write!(f, " anomalies={anomalies}")?;
        if code_pos != u32::MAX {
            write!(f, " code_pos={code_pos}")?;
        }
        writeln!(f)?;

        if level == StackDiagLevel::Brief {
            return Ok(anomalies);
        }

        // ---- 6. Hex dump ----
        // Build a combined label map: (offset, label) sorted by offset.
        let mut labels: Vec<(u32, String)> = Vec::new();
        for (off, sn, rec, pos) in &dbref_anomalies {
            labels.push((
                *off,
                format!("SUSPECT DbRef store_nr={sn}(max={max_store}) rec={rec} pos={pos}"),
            ));
        }
        for (from, to, note) in extras {
            labels.push((*from, format!("extra [{from}..{to}): {note}")));
        }
        labels.sort_by_key(|(o, _)| *o);

        if safe_top > 0 {
            const ROW: u32 = 16;
            writeln!(f, "[STACK] hex dump (stack_pos={sp}, base={base}):")?;
            let b = |o: u32| -> u8 { *store.addr::<u8>(self.stack_cur.rec, base + o) };
            let mut off = 0u32;
            while off < safe_top {
                let row_end = (off + ROW).min(safe_top);
                write!(f, "  {off:5}: ")?;
                // Hex
                for i in off..row_end {
                    write!(f, "{:02x} ", b(i))?;
                }
                // Pad short rows
                for _ in 0..(ROW - (row_end - off)) {
                    write!(f, "   ")?;
                }
                // ASCII
                write!(f, " |")?;
                for i in off..row_end {
                    let c = b(i);
                    write!(f, "{}", if c.is_ascii_graphic() { c as char } else { '.' })?;
                }
                write!(f, "|")?;
                // Labels that start in this row
                for (loff, lbl) in labels.iter().filter(|(lo, _)| *lo >= off && *lo < row_end) {
                    write!(f, "  <@{loff} {lbl}>")?;
                }
                writeln!(f)?;
                off += ROW;
            }
        }

        if level < StackDiagLevel::Full {
            return Ok(anomalies);
        }

        // ---- 7. Variable annotations (Full mode) ----
        let fn_d_nr = data.map_or(u32::MAX, |d| State::fn_d_nr_for_pos(code_pos, d));
        if fn_d_nr != u32::MAX {
            if let Some(d) = data {
                let vars = &d.def(fn_d_nr).variables;
                writeln!(
                    f,
                    "[STACK] variables for fn_d_nr={fn_d_nr} ({}):",
                    d.def(fn_d_nr).name
                )?;
                // Collect variables that have an assigned stack slot, sorted by slot.
                let mut slots: Vec<(u16, String, String, u16)> = Vec::new(); // (slot, name, type, size)
                for v_nr in 0..d.def(fn_d_nr).variables.count() {
                    let slot = vars.stack(v_nr);
                    if slot == u16::MAX {
                        continue;
                    }
                    let var_size = size(vars.tp(v_nr), &Context::Variable);
                    let type_str = vars.tp(v_nr).show(d, vars);
                    slots.push((slot, vars.name(v_nr).to_string(), type_str, var_size));
                }
                slots.sort_by_key(|(s, _, _, _)| *s);
                if slots.is_empty() {
                    writeln!(f, "  (no slot-assigned variables at this code position)")?;
                } else {
                    writeln!(f, "  {:<6} {:<6} {:<22} type", "slot", "size", "name")?;
                    for (slot, name, tp, var_size) in &slots {
                        let end = slot + var_size;
                        // Mark slot as live (within current stack frame)
                        let live = u32::from(*slot) < sp;
                        let flag = if live { "" } else { " [out-of-frame]" };
                        writeln!(f, "  [{slot}..{end}) {name:<22} {tp}{flag}")?;
                    }
                }
            }
        } else if code_pos != u32::MAX {
            writeln!(f, "[STACK] no matching function for code_pos={code_pos}")?;
        }

        Ok(anomalies)
    }

    /// Convenience wrapper: write a Full stack validation report to stderr.
    ///
    /// Useful for quick one-off diagnostics inside opcode implementations:
    /// ```ignore
    /// s.dump_stack_to_stderr(s.code_pos, None, &[]);
    /// ```
    pub fn dump_stack_to_stderr(
        &self,
        code_pos: u32,
        data: Option<&Data>,
        extras: &[(u32, u32, &str)],
    ) {
        let mut buf = Vec::<u8>::new();
        let _ = self.validate_stack(&mut buf, code_pos, data, StackDiagLevel::Full, extras);
        eprint!("{}", String::from_utf8_lossy(&buf));
    }

    /// Bounds-checked read at an absolute frame offset (not relative to TOS).
    /// Returns `None` if the read would extend beyond the current stack store
    /// buffer.  Does NOT modify `stack_pos` or any state.
    fn peek_at<T: Copy>(&self, abs_pos: u32) -> Option<T> {
        let store = self.database.store(&self.stack_cur);
        let total =
            u64::from(self.stack_cur.pos) + u64::from(abs_pos) + std::mem::size_of::<T>() as u64;
        if total > store.byte_capacity() {
            return None;
        }
        Some(*store.addr::<T>(self.stack_cur.rec, self.stack_cur.pos + abs_pos))
    }

    /// Enumerate every slot-assigned variable in the current call frame with a
    /// safe snapshot of its value.  Returns an empty `Vec` when no function
    /// matches `self.code_pos` or when the call stack is empty.
    ///
    /// This is a read-only introspection helper — it never mutates `stack_pos`,
    /// `code_pos`, or any store data.  Variables whose slot lies above the
    /// current `stack_pos` (not yet allocated) appear with
    /// [`VariableValue::OutOfFrame`].  Variables that fail bounds checks appear
    /// with [`VariableValue::Unreadable`].
    #[must_use]
    pub fn iter_frame_variables(&self, data: &Data) -> Vec<FrameVariable> {
        let fn_d_nr = State::fn_d_nr_for_pos(self.code_pos, data);
        if fn_d_nr == u32::MAX {
            return Vec::new();
        }
        let frame_base = self.call_stack.last().map_or(4u32, |cf| cf.args_base);
        self.iter_frame_variables_at(data, fn_d_nr, frame_base, self.code_pos)
    }

    /// Like [`iter_frame_variables`] but for a specific frame, identified by
    /// its `d_nr`, `args_base`, and the bytecode position to evaluate liveness
    /// against.  Used by stack-trace introspection to walk every frame in the
    /// active call chain — see `n_stack_trace`'s variables snapshot.
    #[must_use]
    pub fn iter_frame_variables_at(
        &self,
        data: &Data,
        fn_d_nr: u32,
        frame_base: u32,
        code_pos: u32,
    ) -> Vec<FrameVariable> {
        let mut out = Vec::new();
        if fn_d_nr == u32::MAX || (fn_d_nr as usize) >= data.definitions() as usize {
            return out;
        }
        let def = data.def(fn_d_nr);
        let vars = &def.variables;
        // Pre-compute bytecode reference ranges per var_nr by scanning
        // self.vars for entries within this function's bytecode range.
        let fn_bc_start = def.code_position;
        let fn_bc_end = def.code_position + def.code_length;
        let mut bc_first = vec![u32::MAX; vars.count() as usize];
        let mut bc_last = vec![u32::MAX; vars.count() as usize];
        for (&bc, &v) in &self.vars {
            if bc < fn_bc_start || bc >= fn_bc_end {
                continue;
            }
            let i = v as usize;
            if i >= bc_first.len() {
                continue;
            }
            if bc_first[i] == u32::MAX || bc < bc_first[i] {
                bc_first[i] = bc;
            }
            if bc_last[i] == u32::MAX || bc > bc_last[i] {
                bc_last[i] = bc;
            }
        }
        for v_nr in 0..vars.count() {
            let slot = vars.stack(v_nr);
            if slot == u16::MAX {
                continue;
            }
            let typedef = vars.tp(v_nr).clone();
            // Argument text variables are 16-byte Str on the stack; local text
            // variables are 24-byte String.  Match the runtime layout.
            let is_arg = vars.is_argument(v_nr);
            let ctx = if is_arg {
                &Context::Argument
            } else {
                &Context::Variable
            };
            let size_bytes = size(&typedef, ctx);
            let abs_pos = frame_base + u32::from(slot);
            // Compute liveness before reading the value — non-live slots may
            // contain uninitialized memory whose garbage pointer fields cause
            // SIGSEGV when dereferenced (e.g. text variables).
            let i = v_nr as usize;
            let first = bc_first[i];
            let last = bc_last[i];
            let live = if is_arg {
                true
            } else if first == u32::MAX {
                false
            } else {
                code_pos >= first && code_pos <= last
            };
            let value = if !live
                || u64::from(abs_pos) + u64::from(size_bytes) > u64::from(self.stack_pos)
            {
                VariableValue::OutOfFrame
            } else {
                self.read_variable_value(&typedef, abs_pos, size_bytes, is_arg)
            };
            out.push(FrameVariable {
                var_nr: v_nr,
                name: vars.name(v_nr).to_string(),
                typedef,
                slot,
                abs_pos,
                size: size_bytes,
                is_argument: is_arg,
                scope: vars.scope(v_nr),
                value,
                live,
                bc_first: first,
                bc_last: last,
            });
        }
        out
    }

    /// Read a variable's value safely at an absolute frame position.
    /// Mirrors `dump_stack`'s per-type matching but reads at `abs_pos` instead
    /// of popping from TOS.  When `is_arg` is true, text variables are read as
    /// `Str` (16 bytes); otherwise as `String` (24 bytes).
    fn read_variable_value(
        &self,
        tp: &Type,
        abs_pos: u32,
        _size: u16,
        is_arg: bool,
    ) -> VariableValue {
        match tp {
            Type::Integer(_, _, _) => self
                .peek_at::<i32>(abs_pos)
                .map_or(VariableValue::Unreadable("oob"), VariableValue::Integer),
            Type::Long => self
                .peek_at::<i64>(abs_pos)
                .map_or(VariableValue::Unreadable("oob"), VariableValue::Long),
            Type::Single => self
                .peek_at::<f32>(abs_pos)
                .map_or(VariableValue::Unreadable("oob"), VariableValue::Single),
            Type::Float => self
                .peek_at::<f64>(abs_pos)
                .map_or(VariableValue::Unreadable("oob"), VariableValue::Float),
            Type::Boolean => self
                .peek_at::<u8>(abs_pos)
                .map_or(VariableValue::Unreadable("oob"), |b| {
                    VariableValue::Boolean(b != 0)
                }),
            Type::Character => self
                .peek_at::<u32>(abs_pos)
                .map(|w| char::from_u32(w).unwrap_or('\0'))
                .map_or(VariableValue::Unreadable("oob"), VariableValue::Character),
            Type::Text(_) if is_arg => {
                // Str layout (16 bytes): ptr@0 (8 bytes), len@8 (4 bytes), pad@12 (4 bytes)
                let ptr = match self.peek_at::<u64>(abs_pos) {
                    Some(v) => v,
                    None => return VariableValue::Unreadable("oob"),
                };
                let len = match self.peek_at::<u32>(abs_pos + 8) {
                    Some(v) => v,
                    None => return VariableValue::Unreadable("oob"),
                };
                let content = if ptr == 0 || ptr < (1 << 16) {
                    if len == 0 { Some(String::new()) } else { None }
                } else if len > 10_000_000 {
                    None
                } else {
                    let slice =
                        unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
                    std::str::from_utf8(slice).ok().map(str::to_string)
                };
                VariableValue::StrView { ptr, len, content }
            }
            Type::Text(_) => {
                // String layout (24 bytes): cap@0, ptr@8, len@16
                let cap = match self.peek_at::<u64>(abs_pos) {
                    Some(v) => v,
                    None => return VariableValue::Unreadable("oob"),
                };
                let ptr = match self.peek_at::<u64>(abs_pos + 8) {
                    Some(v) => v,
                    None => return VariableValue::Unreadable("oob"),
                };
                let len = match self.peek_at::<u64>(abs_pos + 16) {
                    Some(v) => v,
                    None => return VariableValue::Unreadable("oob"),
                };
                let content = if ptr == 0 || ptr < (1 << 16) {
                    if len == 0 { Some(String::new()) } else { None }
                } else if len > 10_000_000 {
                    None
                } else {
                    let slice =
                        unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
                    std::str::from_utf8(slice).ok().map(str::to_string)
                };
                VariableValue::Text {
                    cap,
                    ptr,
                    len,
                    content,
                }
            }
            Type::Reference(_, _) | Type::Enum(_, true, _) => {
                // DbRef layout: rec@0, pos@4, store_nr@8 (Rust reorders)
                let rec = match self.peek_at::<u32>(abs_pos) {
                    Some(v) => v,
                    None => return VariableValue::Unreadable("oob"),
                };
                let pos = match self.peek_at::<u32>(abs_pos + 4) {
                    Some(v) => v,
                    None => return VariableValue::Unreadable("oob"),
                };
                let store_nr = match self.peek_at::<u16>(abs_pos + 8) {
                    Some(v) => v,
                    None => return VariableValue::Unreadable("oob"),
                };
                VariableValue::Reference(DbRef { store_nr, rec, pos })
            }
            Type::Vector(_, _)
            | Type::Sorted(_, _, _)
            | Type::Hash(_, _, _)
            | Type::Index(_, _, _) => {
                let rec = match self.peek_at::<u32>(abs_pos) {
                    Some(v) => v,
                    None => return VariableValue::Unreadable("oob"),
                };
                let pos = match self.peek_at::<u32>(abs_pos + 4) {
                    Some(v) => v,
                    None => return VariableValue::Unreadable("oob"),
                };
                let store_nr = match self.peek_at::<u16>(abs_pos + 8) {
                    Some(v) => v,
                    None => return VariableValue::Unreadable("oob"),
                };
                VariableValue::Vector(DbRef { store_nr, rec, pos })
            }
            _ => VariableValue::Unsupported,
        }
    }

    /// Format the result of [`iter_frame_variables`] as a multi-line table.
    /// One line per variable, sorted by slot offset.  Used by trace output and
    /// the `Full` level of `validate_stack`.
    ///
    /// # Errors
    /// Propagates I/O errors from the writer.
    pub fn dump_frame_variables(&self, f: &mut dyn Write, data: &Data) -> Result<(), Error> {
        let fn_d_nr = State::fn_d_nr_for_pos(self.code_pos, data);
        if fn_d_nr == u32::MAX {
            writeln!(f, "[VARS] no function for code_pos={}", self.code_pos)?;
            return Ok(());
        }
        let mut vars = self.iter_frame_variables(data);
        // Sort by slot, then by liveness so live variables come first within
        // each slot (slot coalescing groups multiple vars at the same offset).
        vars.sort_by_key(|v| (v.slot, !v.live));
        let total_vars = data.def(fn_d_nr).variables.count();
        let live_count = vars.iter().filter(|v| v.live).count();
        // When LOFT_DUMP_VARS_LIVE is set, hide stale (slot-coalesced inactive)
        // variables — only show what is actually live at this code position.
        let live_only = std::env::var("LOFT_DUMP_VARS_LIVE").is_ok();
        if live_only {
            vars.retain(|v| v.live);
        }
        writeln!(
            f,
            "[VARS] fn={} code_pos={} stack_pos={} ({} live, {} slot-assigned, {} total)",
            data.def(fn_d_nr).name,
            self.code_pos,
            self.stack_pos,
            live_count,
            vars.len(),
            total_vars
        )?;
        for v in &vars {
            let arg = if v.is_argument { "arg " } else { "    " };
            let live_marker = if v.live { " " } else { "X" };
            write!(
                f,
                "  {live_marker}v{:<3} [{:4}+{:3}={:4}..{:4}) {arg}{:<24} ",
                v.var_nr,
                frame_base_or(self),
                v.slot,
                v.abs_pos,
                v.abs_pos + u32::from(v.size),
                v.name
            )?;
            match &v.value {
                VariableValue::Integer(n) => writeln!(f, "i32  = {n}")?,
                VariableValue::Long(n) => writeln!(f, "i64  = {n}")?,
                VariableValue::Single(n) => writeln!(f, "f32  = {n}")?,
                VariableValue::Float(n) => writeln!(f, "f64  = {n}")?,
                VariableValue::Boolean(b) => writeln!(f, "bool = {b}")?,
                VariableValue::Character(c) => writeln!(f, "char = {c:?}")?,
                VariableValue::Text {
                    cap,
                    ptr,
                    len,
                    content,
                } => {
                    write!(f, "text = String{{cap={cap:#x}, ptr={ptr:#x}, len={len}}}")?;
                    if let Some(s) = content {
                        if s.len() > 32 {
                            writeln!(f, " {:?}...", &s[..32])?;
                        } else {
                            writeln!(f, " {s:?}")?;
                        }
                    } else {
                        writeln!(f, " <unreadable>")?;
                    }
                }
                VariableValue::StrView { ptr, len, content } => {
                    write!(f, "str  = Str{{ptr={ptr:#x}, len={len}}}")?;
                    match content {
                        Some(s) => writeln!(f, " {s:?}")?,
                        None => writeln!(f, " <unreadable>")?,
                    }
                }
                VariableValue::Reference(r) => {
                    writeln!(f, "ref  = ({},{},{})", r.store_nr, r.rec, r.pos)?;
                }
                VariableValue::Vector(r) => {
                    writeln!(f, "vec  = ({},{},{})", r.store_nr, r.rec, r.pos)?;
                }
                VariableValue::OutOfFrame => writeln!(f, "<out-of-frame>")?,
                VariableValue::Unreadable(why) => writeln!(f, "<unreadable: {why}>")?,
                VariableValue::Unsupported => writeln!(f, "<unsupported type>")?,
            }
        }
        Ok(())
    }

    pub(super) fn dump_fn_signature(
        f: &mut dyn Write,
        d_nr: u32,
        data: &Data,
    ) -> Result<u16, Error> {
        write!(f, "{}(", data.def(d_nr).name)?;
        let mut stack_pos = 0;
        for a_nr in 0..data.attributes(d_nr) {
            if a_nr > 0 {
                write!(f, ", ")?;
            }
            write!(
                f,
                "{}: {}[{stack_pos}]",
                data.attr_name(d_nr, a_nr),
                data.attr_type(d_nr, a_nr)
                    .show(data, &data.def(d_nr).variables)
            )?;
            stack_pos += size(&data.attr_type(d_nr, a_nr), &Context::Argument);
        }
        write!(f, ")")?;
        if data.def(d_nr).returned != Type::Void {
            write!(
                f,
                " -> {}",
                data.def(d_nr)
                    .returned
                    .show(data, &data.def(d_nr).variables)
            )?;
        }
        writeln!(f)?;
        Ok(stack_pos)
    }

    // dump helper threading opcode metadata alongside &mut self; no sensible grouping
    #[allow(clippy::too_many_arguments)]
    pub(super) fn dump_op_arg(
        &mut self,
        f: &mut dyn Write,
        def: &Definition,
        p: u32,
        start_pos: u32,
        d_nr: u32,
        data: &Data,
        a_nr: usize,
        a: &Attribute,
    ) -> Result<(), Error> {
        if (def.name == "OpGotoFalseWord" || def.name == "OpGotoWord") && a_nr == 0 {
            let to = i64::from(p) + 3 + i64::from(*self.code::<i16>()) - i64::from(start_pos);
            write!(f, "jump={to}")?;
        } else if def.name == "OpCall" && a_nr == 2 {
            self.fn_name(f, data)?;
        } else if def.name == "OpStaticCall" {
            let v = *self.code::<u16>();
            for (n, val) in &self.library_names {
                if *val == v {
                    write!(f, "{n}")?;
                }
            }
        } else if a_nr == 0
            && !a.mutable
            && a.name == "pos"
            && a.typedef == Type::Integer(0, 65535, false)
            && self.stack.contains_key(&p)
        {
            let pos = i32::from(*self.code::<u16>());
            write!(f, "var[{}]", i32::from(self.stack[&p]) - pos)?;
        } else if a.mutable {
            write!(
                f,
                "{}: {}",
                a.name,
                a.typedef.show(data, &data.def(d_nr).variables)
            )?;
        } else {
            write!(f, "{}={}", a.name, self.dump_attribute(a))?;
        }
        Ok(())
    }

    /**
    Write the byte-code generated for the given function definition.
    When `annotate_slots` is true, each instruction that accesses a named
    variable is followed by `var=name[slot]:type`.
    # Errors
    When the writer had problems.
    # Panics
    When unknown operators are encountered in the byte-code.
    */
    // bytecode disassembler with one match arm per opcode; structural complexity cannot be reduced
    #[allow(clippy::cognitive_complexity)]
    pub fn dump_code(
        &mut self,
        f: &mut dyn Write,
        d_nr: u32,
        data: &Data,
        annotate_slots: bool,
    ) -> Result<(), Error> {
        let stack_pos = Self::dump_fn_signature(f, d_nr, data)?;
        let start_pos = data.def(d_nr).code_position;
        self.code_pos = start_pos;
        writeln!(
            f,
            "{:4}[{stack_pos}]: return-address  @abs={start_pos}",
            self.code_pos - start_pos
        )?;
        while self.code_pos < start_pos + data.def(d_nr).code_length {
            let p = self.code_pos;
            let op = *self.code::<u8>();
            assert!(
                data.has_op(op),
                "Unknown operator {op} in byte_code of {}",
                data.def(d_nr).name
            );
            let def = data.operator(op);
            write!(f, "{:4}", p - start_pos)?;
            if self.stack.contains_key(&p) {
                write!(f, "[{}]", self.stack[&p])?;
            }
            if let Some(nr) = self.line_numbers.get(&p) {
                write!(f, ": [{nr}] ")?;
            } else {
                write!(f, ": ")?;
            }
            write!(f, "{}(", &def.name[2..])?;
            for (a_nr, a) in def.attributes.iter().enumerate() {
                if a_nr > 0 {
                    write!(f, ", ")?;
                }
                self.dump_op_arg(f, def, p, start_pos, d_nr, data, a_nr, a)?;
            }
            write!(f, ")")?;
            if def.returned != Type::Void {
                write!(
                    f,
                    " -> {}",
                    def.returned.show(data, &data.def(d_nr).variables)
                )?;
            }
            if let Some(t) = self.types.get(&p)
                && *t != u16::MAX
            {
                write!(f, " type={} {t:}", self.database.types[*t as usize].name)?;
            }
            if annotate_slots && let Some(v) = self.vars.get(&p) {
                let vars = &data.def(d_nr).variables;
                write!(
                    f,
                    " var={}[{}]:{}",
                    vars.name(*v),
                    vars.stack(*v),
                    vars.tp(*v).show(data, vars)
                )?;
            }
            if def.name == "OpConvRefFromNull" {
                write!(f, " ; [store-alloc]")?;
            } else if def.name == "OpFreeRef" {
                write!(f, " ; [store-free]")?;
            }
            writeln!(f)?;
        }
        writeln!(f)?;
        Ok(())
    }

    pub(super) fn fn_name(&mut self, f: &mut dyn Write, data: &Data) -> Result<(), Error> {
        let addr = *self.code::<i32>() as u32;
        let mut name = format!("Unknown[{addr}]");
        for d in &data.definitions {
            if d.code_position == addr {
                name.clone_from(&d.name);
            }
        }
        write!(f, "fn={name}")?;
        Ok(())
    }

    /**
    Output the given operator argument to a writer
    # Errors
    When the writer had problems.
    */
    pub(super) fn dump_attribute(&mut self, a: &Attribute) -> String {
        match a.typedef {
            Type::Integer(min, max, _) if i64::from(max) - i64::from(min) <= 256 && min == 0 => {
                format!("{}", i32::from(*self.code::<u8>()))
            }
            Type::Integer(min, max, _) if i64::from(max) - i64::from(min) <= 65536 && min == 0 => {
                format!("{}", i32::from(*self.code::<u16>()))
            }
            Type::Integer(min, max, _) if i64::from(max) - i64::from(min) <= 256 => {
                format!("{}", i32::from(*self.code::<i8>()))
            }
            Type::Integer(min, max, _) if i64::from(max) - i64::from(min) <= 65536 => {
                format!("{}", i32::from(*self.code::<i16>()))
            }
            Type::Integer(_, _, _) => format!("{}", *self.code::<i32>()),
            Type::Boolean => format!("{}", *self.code::<u8>() == 1),
            Type::Enum(_, false, _) => format!("{}", *self.code::<u8>()),
            Type::Long => format!("{}", *self.code::<i64>()),
            Type::Single => format!("{}", *self.code::<f32>()),
            Type::Float => format!("{}", *self.code::<f64>()),
            Type::Text(_) => {
                let s = self.code_str();
                if s == STRING_NULL {
                    "null".to_string()
                } else {
                    format!("\"{s}\"")
                }
            }
            Type::Character => format!("{}", *self.code::<char>()),
            Type::Keys => {
                let len = *self.code::<u8>();
                let mut keys = Vec::new();
                for _ in 0..len {
                    keys.push(Key {
                        type_nr: *self.code::<i8>(),
                        position: *self.code::<u16>(),
                    });
                }
                format!("{keys:?}")
            }
            _ => "unknown".to_string(),
        }
    }

    /// Inner execution loop used by [`execute_log_impl`].
    pub(super) fn execute_log_steps(
        &mut self,
        log: &mut dyn Write,
        d_nr: u32,
        config: &LogConfig,
        data: &Data,
    ) -> Result<(), Error> {
        self.fn_positions = data.definitions.iter().map(|d| d.code_position).collect();
        self.code_pos = data.def(d_nr).code_position;
        self.def_pos = self.code_pos;
        // Fix #88 (parity): push a synthetic CallFrame for the entry function so that
        // stack_trace() returns the same frame count as execute_argv.
        self.call_stack.push(super::CallFrame {
            d_nr,
            call_pos: 0,
            args_base: 4,
            args_size: 0,
            line: 0,
        });
        // Write the return address of the main function but do not override the record size.
        self.stack_pos = 4;
        self.put_stack(u32::MAX);

        // Compute the initial frame offset for the bridging invariant check.
        // At runtime we start at stack_pos=4 (the return address); the compile-time
        // tracking in self.stack may start at a different value (usually 0).
        let root_compile_start = self.stack.get(&self.code_pos).copied().map_or(0, i64::from);
        let mut frame_offset = 4i64 - root_compile_start;
        let mut prev_fn_start = self.code_pos;

        // TODO Allow command line parameters on main functions
        let mut step = 0;
        // alloc_free trace state: per-store-nr, the (pc, op_name) of the
        // op that allocated it.  Populated only when config.trace_alloc_free
        // is set; used to emit `[alloc #N at pc=…]` and matching `[free #N
        // (allocated at pc=…)]` lines around each opcode.
        let mut alloc_pcs: HashMap<u16, (u32, String)> = HashMap::new();
        let mut prev_free: Vec<bool> = if config.trace_alloc_free {
            self.database.allocations.iter().map(|s| s.free).collect()
        } else {
            Vec::new()
        };
        while self.code_pos < self.bytecode.len() as u32 {
            let code = self.code_pos;
            let op = *self.code::<u8>();
            let op_name = data.operator(op).name.clone();
            let op_base = &op_name[2..]; // strip "Op" prefix

            // Detect entry into a new function and re-calibrate frame_offset.
            let cur_d_nr = State::fn_d_nr_for_pos(code, data);
            if cur_d_nr != u32::MAX {
                let fn_start = data.def(cur_d_nr).code_position;
                if fn_start != prev_fn_start {
                    prev_fn_start = fn_start;
                    let compile_start = self.stack.get(&fn_start).copied().map_or(0, i64::from);
                    frame_offset = i64::from(self.stack_pos) - compile_start;
                }
            }

            let trace_this = config.trace_opcode(op_base);
            if trace_this {
                self.log_step(log, op, code, &(cur_d_nr, frame_offset), config, data)?;
            }
            OPERATORS[op as usize](self);
            if trace_this {
                self.log_result(log, op, code, data)?;
            }

            // alloc_free trace: emit `[alloc #N …]` / `[free #N …]` lines
            // by diffing the allocations' free state before/after the op.
            if config.trace_alloc_free {
                let allocs = &self.database.allocations;
                if allocs.len() > prev_free.len() {
                    prev_free.resize(allocs.len(), true);
                }
                for (s_nr, s) in allocs.iter().enumerate() {
                    let was_free = prev_free.get(s_nr).copied().unwrap_or(true);
                    if was_free && !s.free {
                        writeln!(log, "    [alloc #{s_nr} at pc={code} op={op_base}]")?;
                        alloc_pcs.insert(s_nr as u16, (code, op_name.clone()));
                    } else if !was_free && s.free {
                        let origin = alloc_pcs
                            .get(&(s_nr as u16))
                            .map_or_else(|| "<unknown>".to_string(), |(p, n)| format!("{n}@{p}"));
                        writeln!(
                            log,
                            "    [free  #{s_nr} at pc={code} op={op_base} (allocated at {origin})]"
                        )?;
                    }
                    if let Some(slot) = prev_free.get_mut(s_nr) {
                        *slot = s.free;
                    }
                }
            }

            // Variable introspection: dump live variables in the current frame
            // after each opcode when LOFT_DUMP_VARS is set.  Use this to track
            // when a variable's value changes unexpectedly (e.g. corruption
            // from a misaligned write).
            if trace_this && (config.dump_vars || std::env::var("LOFT_DUMP_VARS").is_ok()) {
                self.dump_frame_variables(log, data)?;
            }

            // Optional stack snapshot after the opcode.
            if config.snapshot_window > 0 && config.snapshot_opcode(op_base) {
                self.write_stack_snapshot(log, config.snapshot_window)?;
            }

            step += 1;
            assert!(step < 10_000_000, "Too many operations");
            if self.code_pos == u32::MAX {
                // TODO Validate that all databases & String values are also cleared.
                assert_eq!(self.stack_pos, 4, "Stack not correctly cleared");
                // Free the stack store. Mark constant stores (pre-built by
                // build_const_vectors) as free — they are program-lifetime.
                self.database.allocations[0].free = true;
                let const_store = crate::database::CONST_STORE as usize;
                if const_store < self.database.allocations.len() {
                    self.database.allocations[const_store].free = true;
                }
                // Mark all stores referenced by const_refs as free.
                for cr in &self.const_refs {
                    if cr.store_nr != u16::MAX
                        && (cr.store_nr as usize) < self.database.allocations.len()
                    {
                        self.database.allocations[cr.store_nr as usize].free = true;
                    }
                }
                for (s_nr, s) in self.database.allocations.iter().enumerate() {
                    // Locked stores are program-lifetime constants (e.g. the
                    // shared JsonValue::JNull sentinel allocated by
                    // `jv_null_sentinel`) — exempt from the leak check.
                    if s.is_locked() {
                        continue;
                    }
                    if !s.free {
                        if let Some((p, n)) = alloc_pcs.get(&(s_nr as u16)) {
                            panic!(
                                "Database {s_nr} not correctly freed (allocated by \
                                 {n} at pc={p}; rerun with LOFT_LOG=alloc_free for \
                                 the full trace)"
                            );
                        }
                        panic!("Database {s_nr} not correctly freed");
                    }
                }
                writeln!(log, "Finished")?;
                return Ok(());
            }
        }
        Ok(())
    }

    /// Find the definition number of the function whose bytecode contains `pos`.
    pub(super) fn fn_d_nr_for_pos(pos: u32, data: &Data) -> u32 {
        for d_nr in 0..data.definitions() {
            let def = data.def(d_nr);
            if !matches!(def.def_type, DefType::Function) || def.is_operator() {
                continue;
            }
            if def.code_position <= pos && pos < def.code_position + def.code_length {
                return d_nr;
            }
        }
        u32::MAX
    }

    /// Write a raw hex dump of the top `window` bytes of the stack to `log`.
    pub(super) fn write_stack_snapshot(
        &self,
        log: &mut dyn Write,
        window: usize,
    ) -> Result<(), Error> {
        let sp = self.stack_pos;
        let base = self.stack_cur.pos;
        let start = sp.saturating_sub(window as u32);
        write!(log, "  snapshot[{start}..{sp}]:")?;
        let store = self.database.store(&self.stack_cur);
        for offset in start..sp {
            let byte = *store.addr::<u8>(self.stack_cur.rec, base + offset);
            write!(log, " {byte:02x}")?;
        }
        writeln!(log)?;
        Ok(())
    }

    /// Log a single execution step.
    ///
    /// - `code` — bytecode position of the opcode byte (before it was consumed).
    /// - `fn_ctx` — `(d_nr, frame_offset)`: definition number of the function
    ///   currently executing (`u32::MAX` if unknown) and
    ///   `runtime_stack_pos − compile_stack_pos` at the current function entry.
    /// - `config` — logging configuration.
    #[allow(clippy::too_many_lines)]
    pub(super) fn log_step(
        &mut self,
        log: &mut dyn Write,
        op: u8,
        code: u32,
        fn_ctx: &(u32, i64),
        config: &LogConfig,
        data: &Data,
    ) -> Result<u8, Error> {
        let (d_nr, frame_offset) = *fn_ctx;
        let cur = self.code_pos;
        let stack = self.stack_pos;
        assert!(data.has_op(op), "Unknown operator {op}");
        let def = data.operator(op);
        let minus = if cur > self.def_pos { self.def_pos } else { 0 };
        write!(log, "{:5}:[{}]", cur - minus - 1, self.stack_pos)?;

        // Bridging invariant: check runtime vs compile-time stack position.
        if config.check_bridging
            && let Some(&compile_pos) = self.stack.get(&code)
        {
            let expected = i64::from(compile_pos) + frame_offset;
            if i64::from(self.stack_pos) != expected {
                write!(
                    log,
                    " [BRIDGING VIOLATION: runtime={} expected={expected}]",
                    self.stack_pos
                )?;
            }
        }

        if let Some(line) = self.line_numbers.get(&cur) {
            write!(log, " [{line}]")?;
        }
        write!(log, " {}(", &def.name[2..])?;
        // Inverse the order of reading the attributes correctly from the stack.
        let mut attr = BTreeMap::new();
        for (a_nr, a) in def.attributes.iter().enumerate() {
            if !a.mutable {
                if def.name == "OpStaticCall" {
                    let nr = *self.code::<i16>();
                    write!(log, "{})", FUNCTIONS[nr as usize].0)?;
                    self.code_pos = cur;
                    self.stack_pos = stack;
                    return Ok(op);
                } else if def.name == "OpReturn" && a_nr == 0 {
                    self.return_attr(&mut attr, a_nr);
                } else if def.name == "OpCall" && a_nr == 2 {
                    self.call_name(&mut attr, a_nr, data);
                } else if def.name.starts_with("OpGoto") && a_nr == 0 {
                    let to = i64::from(cur) + 2 + i64::from(*self.code::<i16>()) - i64::from(minus);
                    attr.insert(a_nr, format!("jump={to}"));
                } else if def.name == "OpIterate" {
                    self.iterate_args(log)?;
                    self.code_pos = cur;
                    self.stack_pos = stack;
                    return Ok(op);
                } else if a_nr == 0
                    && a.name == "pos"
                    && a.typedef == Type::Integer(0, 65535, false)
                {
                    let pos = *self.code::<u16>();
                    assert!(
                        u32::from(pos) <= self.stack_pos,
                        "Variable {pos} outside stack {}",
                        self.stack_pos
                    );
                    let abs_slot = self.stack_pos - u32::from(pos);
                    // Optionally annotate with variable name from codegen metadata.
                    let annotation =
                        if config.annotate_slots && d_nr != u32::MAX && code != u32::MAX {
                            if let Some(&v) = self.vars.get(&code) {
                                format!("={}", data.def(d_nr).variables.name(v))
                            } else {
                                String::new()
                            }
                        } else {
                            String::new()
                        };
                    attr.insert(a_nr, format!("var[{abs_slot}]{annotation}"));
                } else {
                    attr.insert(a_nr, format!("{}={}", a.name, self.dump_attribute(a)));
                }
            }
        }
        if def.name == "OpGetRecord" {
            self.get_record_keys(data, &mut attr);
        }
        for a_nr in (0..def.attributes.len()).rev() {
            let a = &def.attributes[a_nr];
            if a.mutable {
                // OpPutFnRef/OpVarFnRef _fnref attribute is typed as text but
                // the stack holds a 16-byte fn-ref (d_nr + closure DbRef).  Reading
                // it as Str dereferences a garbage pointer → SIGSEGV.
                if (def.name == "OpPutFnRef" || def.name == "OpVarFnRef")
                    && matches!(a.typedef, Type::Text(_))
                {
                    self.stack_pos -= 16;
                    attr.insert(a_nr, format!("{}: fn-ref[{}]", a.name, self.stack_pos));
                    continue;
                }
                let saved = self.stack_pos;
                let v = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    self.dump_stack(&a.typedef, u32::MAX, data)
                }))
                .unwrap_or_else(|_| {
                    self.stack_pos = saved;
                    "<display-error>".to_string()
                });
                attr.insert(a_nr, format!("{}={v}[{}]", a.name, self.stack_pos));
            }
        }
        // Reverse the argument order again for output.
        for (nr, (_, s)) in attr.iter().enumerate() {
            if nr > 0 {
                write!(log, ", ")?;
            }
            write!(log, "{s}")?;
        }
        write!(log, ")")?;
        self.code_pos = cur;
        self.stack_pos = stack;
        Ok(op)
    }

    pub(super) fn get_record_keys(&mut self, data: &Data, attr: &mut BTreeMap<usize, String>) {
        let db_tp = u16::from_str(&attr[&1][6..]).unwrap_or(0);
        let no_keys = u8::from_str(&attr[&2][8..]).unwrap_or(0) as usize;
        let keys = self.database.get_keys(db_tp);
        for (idx, key) in keys.iter().enumerate() {
            if idx >= no_keys {
                break;
            }
            let v = match key {
                0 => self.dump_stack(&I32, u32::MAX, data),
                1 => self.dump_stack(&Type::Long, u32::MAX, data),
                2 => self.dump_stack(&Type::Single, u32::MAX, data),
                3 => self.dump_stack(&Type::Float, u32::MAX, data),
                4 => self.dump_stack(&Type::Boolean, u32::MAX, data),
                5 => self.dump_stack(&Type::Text(Vec::new()), u32::MAX, data),
                6 => self.dump_stack(&Type::Character, u32::MAX, data),
                _ => self.dump_stack(
                    &Type::Enum(u32::MAX, false, Vec::new()),
                    u32::from(*key),
                    data,
                ),
            };
            attr.insert(idx + 3, format!("key{}={v}[{}]", idx + 1, self.stack_pos));
        }
    }

    pub(super) fn iterate_args(&mut self, log: &mut dyn Write) -> Result<(), Error> {
        let on = *self.code::<u8>();
        let arg = *self.code::<u16>();
        let keys_size = *self.code::<u8>();
        let mut keys = Vec::new();
        for _ in 0..keys_size {
            keys.push(Key {
                type_nr: *self.code::<i8>(),
                position: *self.code::<u16>(),
            });
        }
        let from_key = *self.code::<u8>();
        let till_key = *self.code::<u8>();
        let till = self.stack_key(till_key, &keys);
        let from = self.stack_key(from_key, &keys);
        let data = *self.get_stack::<DbRef>();
        write!(
            log,
            "data=ref({},{},{}), on={on}, arg={arg}, keys={keys:?}, from={from:?}, till={till:?})",
            data.store_nr, data.rec, data.pos
        )
    }

    pub(super) fn return_attr(&mut self, attr: &mut BTreeMap<usize, String>, a_nr: usize) {
        let cur_st = self.stack_pos;
        let ret = u32::from(*self.code::<u16>());
        let cur_code = self.code_pos;
        self.code::<u8>();
        let discard = *self.code::<u16>();
        self.stack_pos -= u32::from(discard);
        self.stack_pos += ret;
        let st = self.stack_pos;
        let addr = *self.get_var::<u32>(0);
        self.stack_pos = cur_st;
        self.code_pos = cur_code;
        attr.insert(a_nr, format!("ret={addr}[{st}]"));
    }

    pub(super) fn call_name(
        &mut self,
        attr: &mut BTreeMap<usize, String>,
        a_nr: usize,
        data: &Data,
    ) {
        let addr = *self.code::<i32>() as u32;
        let mut name = format!("Unknown[{addr}]");
        for d in &data.definitions {
            if d.code_position == addr {
                name.clone_from(&d.name);
            }
        }
        attr.insert(a_nr, format!("fn={name}"));
    }

    pub(super) fn log_result(
        &mut self,
        log: &mut dyn Write,
        op: u8,
        code: u32,
        data: &Data,
    ) -> Result<(), Error> {
        let stack = self.stack_pos;
        let def = data.operator(op);
        if def.name == "OpReturn" {
            writeln!(log, "{}", self.dump_result(code))?;
            return Ok(());
        }
        if def.returned == Type::Void {
            if def.name == "OpFreeRef" {
                writeln!(log, " ; store-free max={}", self.database.max)?;
            } else {
                writeln!(log)?;
            }
            return Ok(());
        }
        let saved = self.stack_pos;
        let v = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.dump_stack(&def.returned, code, data)
        }))
        .unwrap_or_else(|_| {
            self.stack_pos = saved;
            "<display-error>".to_string()
        });
        if def.name == "OpConvRefFromNull" {
            writeln!(log, " -> {v}[{}]", self.stack_pos)?;
            self.stack_pos = stack;
            let db = *self.get_stack::<DbRef>();
            writeln!(
                log,
                "  ; store-alloc nr={} max={}",
                db.store_nr, self.database.max
            )?;
            self.stack_pos = stack;
        } else {
            writeln!(log, " -> {v}[{}]", self.stack_pos)?;
            self.stack_pos = stack;
        }
        Ok(())
    }

    pub(super) fn dump_result(&mut self, code: u32) -> String {
        if let Some(k) = self.types.get(&code) {
            let stack = self.stack_pos;
            let known = *k;
            let res = match known {
                0 => format!("{}", *self.get_stack::<i32>()), // integer
                1 => format!("{}", *self.get_stack::<i64>()), // long
                2 => format!("{}", *self.get_stack::<f32>()), // single
                3 => format!("{}", *self.get_stack::<f64>()), // float
                4 => format!("{}", *self.get_stack::<u8>() == 1), // boolean
                5 => {
                    let s = self.string();
                    match s.try_str() {
                        None => {
                            return format!(" -> <raw:{:#x}>[{}]", s.ptr as usize, self.stack_pos);
                        }
                        Some(s) if s == STRING_NULL => "null".to_string(),
                        Some(s) => format!("\"{}\"", s.replace('"', "\\\"")),
                    }
                } // text
                6 => format!("{}", *self.get_stack::<char>()), // character
                _ if known != u16::MAX => match &self.database.types[known as usize].parts {
                    crate::database::Parts::Enum(_) => {
                        let val = *self.get_stack::<u8>();
                        format!("{}({val})", self.database.enum_val(known, val))
                    }
                    crate::database::Parts::Struct(_)
                    | crate::database::Parts::EnumValue(_, _)
                    | crate::database::Parts::Vector(_) => {
                        let val = *self.get_stack::<DbRef>();
                        let (depth, elems) = Self::dump_limits();
                        self.database.dump_compact(&val, known, depth, elems)
                    }
                    _ => String::new(),
                },
                _ => String::new(),
            };
            let after = self.stack_pos;
            self.stack_pos = stack;
            format!(" -> {res}[{after}]")
        } else {
            String::new()
        }
    }

    /// Read dump depth and element limits from environment variables.
    /// `LOFT_DUMP_DEPTH` (default 2) and `LOFT_DUMP_ELEMENTS` (default 8).
    fn dump_limits() -> (u16, u16) {
        let depth = std::env::var("LOFT_DUMP_DEPTH")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(2);
        let elems = std::env::var("LOFT_DUMP_ELEMENTS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(8);
        (depth, elems)
    }

    pub(super) fn dump_stack(&mut self, typedef: &Type, code: u32, data: &Data) -> String {
        match typedef {
            Type::Integer(_, _, _) => format!("{}", *self.get_stack::<i32>()),
            Type::Character => {
                let c = *self.get_stack::<char>();
                if c == char::from(0) {
                    "null".to_string()
                } else {
                    format!("'{c}'")
                }
            }
            Type::Enum(tp, false, _) => {
                if code == u32::MAX {
                    format!("{}", *self.get_stack::<u8>())
                } else {
                    let known = if self.types.contains_key(&code) {
                        self.types[&code]
                    } else if *tp == u32::MAX {
                        code as u16
                    } else {
                        data.def(*tp).known_type
                    };
                    let val = *self.get_stack::<u8>();
                    format!("{}({val})", self.database.enum_val(known, val))
                }
            }
            Type::Long => format!("{}", *self.get_stack::<i64>()),
            Type::Single => format!("{}", *self.get_stack::<f32>()),
            Type::Float => format!("{}", *self.get_stack::<f64>()),
            Type::Text(_) => {
                // Guard: check stack has room for a Str read.
                let needed = self.stack_cur.pos + self.stack_pos;
                let str_size = super::size_ptr();
                let cap = self.database.store(&self.stack_cur).byte_capacity();
                if u64::from(needed) < u64::from(str_size) || needed - str_size > cap as u32 {
                    self.stack_pos -= str_size;
                    return format!("<stack-oob:{needed}>");
                }
                let s = self.string();
                match s.try_str() {
                    None => format!("<raw:{:#x}>", s.ptr as usize),
                    Some(s) if s == STRING_NULL => "null".to_string(),
                    Some(s) => format!("\"{}\"", s.replace('"', "\\\"")),
                }
            }
            Type::Boolean => format!("{}", *self.get_stack::<u8>() == 1),
            Type::Reference(tp, _) | Type::Enum(tp, true, _) => {
                let known = if self.types.contains_key(&code) {
                    self.types[&code]
                } else {
                    data.def(*tp).known_type
                };
                let val = *self.get_stack::<DbRef>();
                if known == u16::MAX || val.store_nr as usize >= self.database.allocations.len() {
                    return format!("ref({},{},{})", val.store_nr, val.rec, val.pos);
                }
                let store = &self.database.allocations[val.store_nr as usize];
                if store.free {
                    return format!("ref({},{},{})=<freed>", val.store_nr, val.rec, val.pos);
                }
                // Guard: record must be within the store buffer.
                if u64::from(val.rec) * 8 + u64::from(val.pos) + 8 > store.byte_capacity() {
                    return format!("ref({},{},{})=<oob>", val.store_nr, val.rec, val.pos);
                }
                // Guard: the record must be live (positive fld-0 header) before we
                // dereference.
                if val.rec != 0 {
                    let hdr = *store.addr::<i32>(val.rec, 0);
                    if hdr <= 0 {
                        return format!("ref({},{},{})=<freed>", val.store_nr, val.rec, val.pos);
                    }
                }
                let (depth, elems) = Self::dump_limits();
                self.database.dump_compact(&val, known, depth, elems)
            }
            Type::Vector(_, _) => {
                let val = *self.get_stack::<DbRef>();
                let known = if self.types.contains_key(&code) {
                    self.types[&code]
                } else {
                    return format!("ref({},{},{})", val.store_nr, val.rec, val.pos);
                };
                // Guard: don't access freed stores (OpFreeRef may have already run).
                if (val.store_nr as usize) < self.database.allocations.len()
                    && self.database.allocations[val.store_nr as usize].free
                {
                    return format!("ref({},{},{})=<freed>", val.store_nr, val.rec, val.pos);
                }
                let (depth, elems) = Self::dump_limits();
                self.database.dump_compact(&val, known, depth, elems)
            }
            _ => "unknown".to_string(),
        }
    }
}

/// Public entry point for `State::execute_log` that avoids a circular method call
/// while still keeping the implementation in this module.
pub(super) fn execute_log_impl(
    state: &mut State,
    log: &mut dyn Write,
    name: &str,
    config: &LogConfig,
    data: &Data,
) -> Result<(), Error> {
    let d_nr = data.def_nr(&format!("n_{name}"));
    assert_ne!(d_nr, u32::MAX, "Unknown routine {name}");

    // Set up parallel context so n_parallel_for can access bytecode/library.
    let data_ptr = std::ptr::from_ref::<crate::data::Data>(data);
    state.data_ptr = data_ptr;
    let stk_lib_nr = state
        .library_names
        .get("n_stack_trace")
        .copied()
        .unwrap_or(u16::MAX);
    state.database.parallel_ctx = Some(Box::new(super::ParallelCtx {
        data: data_ptr,
        bytecode: &raw const state.bytecode,
        library: &raw const state.library,
        stack_trace_lib_nr: stk_lib_nr,
    }));
    // `LOFT_LOG=poison_free`: wire the runtime flag into the Stores so
    // every `free_named` overwrites the freed buffer with 0xDEADBEEF.
    state.database.poison_free = config.poison_free;

    // If logging is suppressed for this function, fall back to silent execution.
    if !config.phases.execution || !config.show_function(name) {
        state.execute(name, data);
        return Ok(());
    }

    if let Some(tail_n) = config.trace_tail {
        // Tail-buffer mode: keep only the last `tail_n` lines in memory.
        // Wrap in catch_unwind so the buffer is flushed even on panic.
        let mut tail = TailBuffer::new(tail_n);
        writeln!(tail, "Execute {name}:")?;
        // SAFETY: We hold all three mutable references exclusively and none
        // of them can be invalidated during catch_unwind on this thread.
        let self_raw = std::ptr::from_mut::<State>(state);
        let tail_raw = &raw mut tail;
        let data_raw = std::ptr::from_ref::<Data>(data);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let s = unsafe { &mut *self_raw };
            let t = unsafe { &mut *tail_raw };
            let d = unsafe { &*data_raw };
            s.execute_log_steps(t, d_nr, config, d)
        }));
        match result {
            Ok(r) => {
                tail.flush_to(log)?;
                r
            }
            Err(e) => {
                // On panic: flush tail to stderr so the trace is visible
                // even when `log` is a Vec<u8> that will be dropped.
                let _ = tail.flush_to(&mut std::io::stderr());
                std::panic::resume_unwind(e)
            }
        }
    } else {
        writeln!(log, "Execute {name}:")?;
        let r = state.execute_log_steps(log, d_nr, config, data);
        state.database.parallel_ctx = None;
        r
    }
}
