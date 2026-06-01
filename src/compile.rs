// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

#![allow(dead_code)]
//! Fast interpreter for binary code.
use crate::data::{Data, DefType, Type, Value};
use crate::keys::DbRef;
use crate::log_config::LogConfig;
use crate::native;
use crate::state::State;
use crate::variables::{Function, dump_variables};
use std::fmt::Write as _;
use std::io::{Error, Write};
// Bytecode generation

/// Create byte code from parsed Data.  Walks every user function,
/// emits its bytecode into `state`, then materialises constant
/// vectors into `CONST_STORE`.
pub fn byte_code(state: &mut State, data: &mut Data) {
    byte_code_from(state, data, 0);
}

/// Incremental variant of [`byte_code`] — only emit bytecode for
/// functions whose `d_nr >= start_d_nr`, and skip the one-time init
/// (`native::init`, `register_native_stubs`, `build_const_vectors`,
/// `CONST_STORE` lock) when `start_d_nr > 0`.
///
/// @P381 — fixes the "Claim on read-only store (locked by: compile.rs
/// CONST_STORE init)" panic when the test wrapper synthesis at
/// `src/main.rs:3135` calls `byte_code` a SECOND time over the same
/// `Data`/`State` to compile a freshly-parsed `fn main()` wrapper that
/// drives all `test_*()` functions.  The first call (from
/// `src/main.rs:2198`) emitted bytecode for every user fn, materialised
/// constant vectors into `CONST_STORE`, and locked the store.  A second
/// full call re-emitted bytecode for already-compiled fns, and
/// `Codegen::gen_text` (which writes long string literals into
/// `CONST_STORE` via `set_str`) tripped the assertion on the now-locked
/// store.  Compiling only the new wrapper avoids the re-emission entirely.
pub fn byte_code_from(state: &mut State, data: &mut Data, start_d_nr: u32) {
    if start_d_nr == 0 {
        native::init(state);
        register_native_stubs(state, data);
    }
    for d_nr in start_d_nr..data.definitions() {
        if !matches!(data.def(d_nr).def_type, DefType::Function) || data.def(d_nr).is_operator() {
            continue;
        }
        state.def_code(d_nr, data);
    }
    if start_d_nr == 0 {
        build_const_vectors(state, data);
        state.database.allocations[crate::database::CONST_STORE as usize]
            .lock_with_origin("compile.rs::compile (CONST_STORE init)");
    }
}

/// Extract literal values from vector constant Block IR and build
/// the vectors in CONST_STORE. Populates `state.const_refs` and
/// `data.definitions[d_nr].const_ref`.
fn build_const_vectors(state: &mut State, data: &mut Data) {
    // Ensure const_refs is large enough for all definitions.
    let null_ref = DbRef {
        store_nr: u16::MAX,
        rec: 0,
        pos: 0,
    };
    state
        .const_refs
        .resize(data.definitions() as usize, null_ref);
    // Mirror const_refs on Stores so native codegen (which has
    // `&mut Stores` but no `&mut State`) can resolve
    // `OpConstRef` via `stores.const_ref_at_runtime(d_nr)`.
    state
        .database
        .const_refs
        .resize(data.definitions() as usize, null_ref);

    for d_nr in 0..data.definitions() {
        if data.def(d_nr).def_type != DefType::Constant {
            continue;
        }
        let Type::Vector(ref elem_tp, _) = data.def(d_nr).returned else {
            continue;
        };
        let elem_tp = (**elem_tp).clone();
        let values = extract_literal_values(&data.def(d_nr).code, data);
        if values.is_empty() {
            continue;
        }
        // Build the vector in its own store using the normal Stores API.
        // This mirrors what OpDatabase + OpNewRecord + OpFinishRecord do at runtime.
        // Look up the main_vector<T> struct that wraps the vector field.
        let vec_struct_name = format!("main_vector<{}>", elem_tp.name(data));
        let vec_struct_dnr = data.def_nr(&vec_struct_name);
        if vec_struct_dnr == u32::MAX {
            continue;
        }
        let vec_tp = data.def(vec_struct_dnr).known_type;
        let size = u32::from(state.database.size(vec_tp));
        let db = state.database.database(size);
        state
            .database
            .store_mut(&db)
            .set_u32_raw(db.rec, 4, u32::from(vec_tp));
        state.database.set_default_value(vec_tp, &db);
        let vec_ref = DbRef {
            store_nr: db.store_nr,
            rec: 1,
            pos: 8,
        };
        for val in &values {
            let rec = state.database.record_new(&vec_ref, vec_tp, 0);
            match val {
                Value::Int(v) => {
                    state
                        .database
                        .store_mut(&rec)
                        .set_int(rec.rec, rec.pos, i64::from(*v));
                }
                Value::Float(v) => {
                    state
                        .database
                        .store_mut(&rec)
                        .set_float(rec.rec, rec.pos, *v);
                }
                Value::Single(v) => {
                    state
                        .database
                        .store_mut(&rec)
                        .set_single(rec.rec, rec.pos, *v);
                }
                Value::Long(v) => {
                    state
                        .database
                        .store_mut(&rec)
                        .set_long(rec.rec, rec.pos, *v);
                }
                Value::Text(v) => {
                    // Mirror the runtime OpSetText path (src/fill.rs::set_text):
                    // store the string in the same store as the vector record
                    // via set_str(), then write the returned record number
                    // into the text field as an int pointer.
                    let store = state.database.store_mut(&rec);
                    let s_pos = store.set_str(v);
                    store.set_u32_raw(rec.rec, rec.pos, s_pos);
                }
                _ => {}
            }
            state.database.record_finish(&vec_ref, &rec, vec_tp, 0);
        }
        state.database.allocations[db.store_nr as usize].lock();
        // High ref_count ensures free/dec_rc never actually frees this store.
        state.database.allocations[db.store_nr as usize].ref_count = u32::MAX / 2;
        data.definitions[d_nr as usize].const_ref = Some(vec_ref);
        state.const_refs[d_nr as usize] = vec_ref;
        state.database.const_refs[d_nr as usize] = vec_ref;
    }
}

/// Walk the Block IR for a vector constant and extract the literal values.
/// Returns an empty Vec if the IR contains non-literal expressions.
/// Public wrapper for reuse by native codegen's init-emission.
pub fn extract_literal_values_public(code: &Value, data: &Data) -> Vec<Value> {
    extract_literal_values(code, data)
}

fn extract_literal_values(code: &Value, data: &Data) -> Vec<Value> {
    let Value::Block(block) = code else {
        return vec![];
    };
    let mut values = Vec::new();
    // Look for patterns: Call(OpSetInt/Float/Single/Text, [_, Int(0), literal_value])
    let set_int_nr = data.def_nr("OpSetInt");
    let set_float_nr = data.def_nr("OpSetFloat");
    let set_single_nr = data.def_nr("OpSetSingle");
    let set_text_nr = data.def_nr("OpSetText");
    for op in &block.operators {
        let Value::Call(fn_nr, args) = op else {
            continue;
        };
        if args.len() < 3 {
            continue;
        }
        if *fn_nr == set_int_nr
            || *fn_nr == set_float_nr
            || *fn_nr == set_single_nr
            || *fn_nr == set_text_nr
        {
            match &args[2] {
                v @ (Value::Int(_)
                | Value::Float(_)
                | Value::Single(_)
                | Value::Long(_)
                | Value::Text(_)) => {
                    values.push(v.clone());
                }
                _ => return vec![], // non-literal value — can't pre-build
            }
        }
    }
    values
}

/// PKG.1: For each `#native "symbol"` declaration, register a stub function
/// that panics when called.  This lets codegen emit `OpStaticCall` with the
/// correct library index.  `extensions::load_all()` replaces the stubs with
/// real function pointers after bytecode generation.
fn register_native_stubs(state: &mut State, data: &Data) {
    use crate::database::Stores;
    use crate::keys::DbRef;

    let mut stub_syms = std::collections::HashSet::new();
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        if def.native.is_empty() {
            continue;
        }
        let sym = &def.native;
        // Skip if already registered (e.g. by native::init for built-in functions).
        if state.library_names.contains_key(sym) {
            continue;
        }
        stub_syms.insert(sym.clone());
        // Register a stub that panics with a descriptive message.
        let stub: fn(&mut Stores, &mut DbRef) = {
            // We can't capture sym_owned in a fn pointer, so use a single
            // generic stub.  The State tracks which library index maps to
            // which name, so the panic message comes from the dispatch side.
            |_stores: &mut Stores, _db: &mut DbRef| {
                panic!("native function not loaded — call extensions::load_all() first");
            }
        };
        state.static_fn(sym, stub);
    }
    // Store the set of stub symbols so wire_native_fns knows which to replace.
    crate::extensions::set_stub_symbols(stub_syms);
}

/// Plan-22 02d-vii follow-up — IR-only dump that doesn't
/// require `&mut State` / `&mut Data`.  Used by
/// `execute_log_impl` to print IR before execution starts when
/// `LOFT_LOG=ir:<fn>` is active, so `cargo run` (not just
/// `--dump`) shows the IR.
///
/// # Errors
/// Returns an error if the writer fails or `data.show_code`
/// fails to format an IR node.
pub fn show_ir_only(writer: &mut dyn Write, data: &Data, config: &LogConfig) -> Result<(), Error> {
    if !config.phases.ir {
        return Ok(());
    }
    for d_nr in 0..data.definitions() {
        if !matches!(
            data.def(d_nr).def_type,
            DefType::Function | DefType::Dynamic
        ) {
            continue;
        }
        let is_op = data.def(d_nr).is_operator();
        if is_op && !config.show_all_functions {
            continue;
        }
        let from_default = data.def(d_nr).position.file.starts_with("default/")
            || data.def(d_nr).position.file.starts_with("default\\");
        if from_default && !config.show_all_functions {
            continue;
        }
        if !config.show_function(&data.def(d_nr).name) {
            continue;
        }
        write!(writer, "{} ", data.def(d_nr).header(data, d_nr))?;
        let mut vars = Function::copy(&data.def(d_nr).variables);
        data.show_code(writer, &mut vars, &data.def(d_nr).code, 0, false)?;
        writeln!(writer, "\n")?;
    }
    Ok(())
}

/// Plan-22 02d-vii follow-up — capture-pipeline summary for fns
/// matching `LOFT_LOG=captures:<fn_name>`.  For each parent fn:
/// scalars_to_box.  For each lambda inside it: mutated_captures,
/// closure_record d_nr + name, per-attribute auto-Reference
/// status (`[12B share-by-DbRef]` vs `[N B inline]`).
///
/// Replaces 5+ separate `eprintln!` cycles that 02d-iii.e
/// needed to inspect closure-record attribute types across
/// passes.
///
/// # Errors
/// Returns an error if the writer fails.
pub fn show_captures_summary(writer: &mut dyn Write, data: &Data) -> Result<(), Error> {
    let Some(target) = crate::log_config::captures_trace_target() else {
        return Ok(());
    };
    // Two-pass dump:
    //   1. Parent fns matching the filter that have a non-empty
    //      scalars_to_box (the only diagnostic info parents carry).
    //   2. ALL lambdas with a closure_record (the name filter
    //      doesn't apply to `__lambda_N` synthetic names; instead
    //      we dump every lambda since the user filtered the parent
    //      already).
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        if !matches!(def.def_type, DefType::Function | DefType::Dynamic) {
            continue;
        }
        let is_lambda = def.closure_record != u32::MAX;
        let direct_match = def.name.contains(&target);
        if !direct_match && !is_lambda {
            continue;
        }
        if def.scalars_to_box.is_empty() && !is_lambda {
            continue;
        }
        writeln!(
            writer,
            "[captures] === {} (d_nr={d_nr}, {kind}) ===",
            def.name,
            kind = if is_lambda { "lambda" } else { "parent" }
        )?;
        if !def.scalars_to_box.is_empty() {
            writeln!(
                writer,
                "[captures]   scalars_to_box = {:?}",
                def.scalars_to_box
            )?;
        }
        if is_lambda {
            writeln!(
                writer,
                "[captures]   mutated_captures = {:?}",
                def.mutated_captures
            )?;
            let cr_d = def.closure_record;
            let cr = data.def(cr_d);
            writeln!(
                writer,
                "[captures]   closure_record = #{cr_d} {cr_name} ({n_attrs} attrs)",
                cr_name = cr.name,
                n_attrs = cr.attributes.len()
            )?;
            for (idx, attr) in cr.attributes.iter().enumerate() {
                let storage = match &attr.typedef {
                    crate::data::Type::Reference(_, deps) if deps.first() == Some(&u16::MAX) => {
                        "[12B share-by-DbRef (auto-Reference)]"
                    }
                    crate::data::Type::Reference(_, _) => "[12B owned Reference]",
                    crate::data::Type::Text(_) => "[16B inline Text]",
                    crate::data::Type::Integer(_) => "[8B inline Integer]",
                    crate::data::Type::Float => "[8B inline Float]",
                    crate::data::Type::Single => "[4B inline Single]",
                    crate::data::Type::Boolean => "[1B inline Boolean]",
                    crate::data::Type::Character => "[4B inline Character]",
                    crate::data::Type::Function(_, _, _) => {
                        "[20B inline Function (16B fn-ref + 4B pad)]"
                    }
                    _ => "[? inline / other]",
                };
                writeln!(
                    writer,
                    "[captures]     attr[{idx}] {name} : {tp:?}  {storage}",
                    name = attr.name,
                    tp = attr.typedef
                )?;
            }
        }
    }
    Ok(())
}

/// Dump byte code result to the given writer, filtered by `config`.
///
/// - `config.phases.ir` — whether to show IR (intermediate representation).
/// - `config.phases.bytecode` — whether to show bytecode disassembly.
/// - `config.show_functions` — which functions to include (`None` = all
///   non-default functions).
/// - `config.annotate_slots` — whether to append `var=name[slot]:type`
///   annotations to bytecode instructions.
///
/// # Errors
/// When the writer didn't accept the data.
pub fn show_code(
    writer: &mut dyn Write,
    state: &mut State,
    data: &mut Data,
    config: &LogConfig,
) -> Result<(), Error> {
    for d_nr in 0..data.definitions() {
        if !matches!(
            data.def(d_nr).def_type,
            DefType::Function | DefType::Dynamic
        ) {
            continue;
        }
        let is_op = data.def(d_nr).is_operator();
        if is_op && !config.show_all_functions {
            continue;
        }
        let from_default = data.def(d_nr).position.file.starts_with("default/")
            || data.def(d_nr).position.file.starts_with("default\\");
        if from_default && !config.show_all_functions {
            continue;
        }
        if !config.show_function(&data.def(d_nr).name) {
            continue;
        }
        if config.phases.ir {
            write!(writer, "{} ", data.def(d_nr).header(data, d_nr))?;
            let mut vars = Function::copy(&data.def(d_nr).variables);
            data.show_code(writer, &mut vars, &data.def(d_nr).code, 0, false)?;
            writeln!(writer, "\n")?;
        }
        if config.phases.bytecode {
            write!(writer, "byte-code for {}:", data.def(d_nr).position.file)?;
            state.dump_code(writer, d_nr, data, config.annotate_slots)?;
        }
        if config.show_variables {
            write!(writer, "variables for {}:", data.def(d_nr).position.file)?;
            writeln!(writer, "{}", data.def(d_nr).header(data, d_nr))?;
            dump_variables(writer, &data.def(d_nr).variables, data)?;
        }
    }
    Ok(())
}

// ── Standalone bytecode disassembler ─────────────────────────────────────────

/// Build a table mapping opcode → instruction byte-length, indexed by the
/// FULL opcode number.  Extended opcodes encode as a `255` lead byte plus
/// an `ext` byte (`op = 255 + ext`, so `op ≥ 256`); the table therefore
/// runs past 256 and counts **2** lead bytes for them (1 for a base op
/// `0..=254`), plus the sum of the const-argument sizes.  `0` = unknown.
#[must_use]
pub fn build_opcode_len_table(data: &Data) -> Vec<u8> {
    use crate::data::Context;
    use crate::variables::size;
    let max_op = data.operators.keys().copied().max().unwrap_or(0);
    let mut table = vec![0u8; max_op as usize + 1]; // 0 = unknown opcode
    for (&op, &d_nr) in &data.operators {
        let def = &data.definitions[d_nr as usize];
        let mut len = if op >= 255 { 2u16 } else { 1u16 }; // opcode lead byte(s)
        for a in &def.attributes {
            if a.constant {
                len += size(&a.typedef, &Context::Constant);
            }
        }
        table[op as usize] = len as u8;
    }
    table
}

/// Resolve opcode number by operator name.  Returns `u16::MAX` if not found.
#[must_use]
pub fn opcode_by_name(data: &Data, name: &str) -> u16 {
    for (&op, &d_nr) in &data.operators {
        if data.definitions[d_nr as usize].name == name {
            return op;
        }
    }
    u16::MAX
}

/// Disassemble bytecode for one function to `writer`.
///
/// Shows offset, opcode name, const operands (decoded), jump targets,
/// variable names where possible, and source line numbers.
///
/// # Errors
/// On write failures.
#[allow(
    clippy::too_many_lines,
    clippy::manual_strip,
    clippy::format_push_string
)]
pub fn disassemble(
    writer: &mut dyn Write,
    bytecode: &[u8],
    d_nr: u32,
    data: &Data,
    op_len: &[u8],
) -> Result<(), Error> {
    let def = data.def(d_nr);
    let start = def.code_position as usize;
    let end = start + def.code_length as usize;
    let vars = &def.variables;

    let targets = collect_jump_targets(bytecode, start, end, data, op_len);
    writeln!(writer, "--- {} ---", def.name)?;

    let mut pc = start;
    while pc < end && pc < bytecode.len() {
        let rel = pc - start;
        let first = bytecode[pc];
        let (op, op_byte_len): (u16, usize) = if first == 255 && pc + 1 < bytecode.len() {
            (255u16 + u16::from(bytecode[pc + 1]), 2)
        } else {
            (u16::from(first), 1)
        };
        let ilen = op_len.get(op as usize).copied().unwrap_or(0) as usize;
        if ilen == 0 {
            writeln!(writer, "{rel:4}: ??? (opcode {op})")?;
            break;
        }
        if targets.contains(&pc) {
            writeln!(writer, "  .L{rel}:")?;
        }
        let op_name = opcode_display_name(op, data);
        let args = format_op_args(
            op,
            bytecode,
            pc + op_byte_len - 1,
            data,
            vars,
            start,
            op_name,
        );
        writeln!(writer, "{rel:4}: {op_name}({args})")?;
        pc += ilen;
    }
    writeln!(writer)?;
    Ok(())
}

/// Pre-pass: scan the bytecode for goto-style instructions and collect
/// their target offsets.  Disassemblers emit a label at each target so
/// forward / backward jumps are readable — and so a re-assembler can
/// re-derive relative offsets after ops are inserted or removed (the jump
/// binds to a label identity, not a byte offset).  `op_len` is indexed by
/// the FULL opcode number (see [`build_opcode_len_table`]).
pub(crate) fn collect_jump_targets(
    bytecode: &[u8],
    start: usize,
    end: usize,
    data: &Data,
    op_len: &[u8],
) -> std::collections::BTreeSet<usize> {
    let mut targets = std::collections::BTreeSet::new();
    let mut pc = start;
    while pc < end && pc < bytecode.len() {
        let first = bytecode[pc];
        let op: u16 = if first == 255 && pc + 1 < bytecode.len() {
            255u16 + u16::from(bytecode[pc + 1])
        } else {
            u16::from(first)
        };
        let ilen = op_len.get(op as usize).copied().unwrap_or(0) as usize;
        if ilen == 0 {
            break;
        }
        if data.has_op(op) {
            let name = &data.operator(op).name;
            if (name == "OpGoto" || name == "OpGotoFalse") && ilen == 2 && pc + 1 < end {
                let off = bytecode[pc + 1] as i8;
                let target = (pc as i32 + 2 + i32::from(off)) as usize;
                targets.insert(target);
            } else if (name == "OpGotoWord" || name == "OpGotoFalseWord")
                && ilen == 3
                && pc + 2 < end
            {
                let off = i16::from_le_bytes([bytecode[pc + 1], bytecode[pc + 2]]);
                let target = (pc as i32 + 3 + i32::from(off)) as usize;
                targets.insert(target);
            }
        }
        pc += ilen;
    }
    targets
}

/// Return the printable opcode name, stripping the `Op` prefix so the
/// disassembly reads as `Call(...)` rather than `OpCall(...)`.
fn opcode_display_name(op: u16, data: &Data) -> &str {
    if data.has_op(op) {
        let n = &data.operator(op).name;
        n.strip_prefix("Op").unwrap_or(n.as_str())
    } else {
        "???"
    }
}

/// Decode and format the attribute list for a single opcode into
/// `"name1=val1, name2: type, ..."` form for the disassembler.
///
/// Resolves three special forms the reader cares about:
/// - goto offsets rendered as `.L{target}` labels
/// - word-sized slot indices resolved to their variable name
/// - 32-bit call targets resolved to the function name at that address
fn format_op_args(
    op: u16,
    bytecode: &[u8],
    pc: usize,
    data: &Data,
    vars: &Function,
    start: usize,
    op_name: &str,
) -> String {
    use crate::data::Context;
    use crate::variables::size;

    let mut args = String::new();
    if !data.has_op(op) {
        return args;
    }
    let op_def = data.operator(op);
    let mut cursor = pc + 1;
    for a in &op_def.attributes {
        if a.constant {
            let a_size = size(&a.typedef, &Context::Constant) as usize;
            if !args.is_empty() {
                args.push_str(", ");
            }
            match a_size {
                1 if matches!(a.typedef, Type::Integer(_)) => {
                    let v = bytecode[cursor] as i8;
                    if op_name.contains("Goto") {
                        let target = (cursor as i32 + 1 + i32::from(v)) as usize - start;
                        write!(&mut args, "{}=.L{target}", a.name).unwrap();
                    } else {
                        write!(&mut args, "{}={v}", a.name).unwrap();
                    }
                }
                1 => {
                    write!(&mut args, "{}={}", a.name, bytecode[cursor]).unwrap();
                }
                2 if op_name.contains("Goto") => {
                    let v = i16::from_le_bytes([bytecode[cursor], bytecode[cursor + 1]]);
                    let target = (cursor as i32 + 2 + i32::from(v)) as usize - start;
                    write!(&mut args, "{}=.L{target}", a.name).unwrap();
                }
                2 => {
                    let v = u16::from_le_bytes([bytecode[cursor], bytecode[cursor + 1]]);
                    if let Some(name) = find_var_at_slot(vars, v) {
                        write!(&mut args, "{}={name}@{v}", a.name).unwrap();
                    } else {
                        write!(&mut args, "{}={v}", a.name).unwrap();
                    }
                }
                4 => {
                    let v = i32::from_le_bytes([
                        bytecode[cursor],
                        bytecode[cursor + 1],
                        bytecode[cursor + 2],
                        bytecode[cursor + 3],
                    ]);
                    if op_name == "Call" {
                        let fname = find_fn_at_addr(data, v as u32);
                        write!(
                            &mut args,
                            "{}={}",
                            a.name,
                            fname.unwrap_or_else(|| format!("@{v}"))
                        )
                        .unwrap();
                    } else {
                        write!(&mut args, "{}={v}", a.name).unwrap();
                    }
                }
                8 => {
                    let v = i64::from_le_bytes([
                        bytecode[cursor],
                        bytecode[cursor + 1],
                        bytecode[cursor + 2],
                        bytecode[cursor + 3],
                        bytecode[cursor + 4],
                        bytecode[cursor + 5],
                        bytecode[cursor + 6],
                        bytecode[cursor + 7],
                    ]);
                    write!(&mut args, "{}={v}", a.name).unwrap();
                }
                _ => {
                    write!(&mut args, "{}=?({a_size}B)", a.name).unwrap();
                }
            }
            cursor += a_size;
        } else {
            if !args.is_empty() {
                args.push_str(", ");
            }
            write!(&mut args, "{}: {}", a.name, a.typedef.name(data)).unwrap();
        }
    }
    args
}

/// Find the variable whose stack position matches `slot`.
fn find_var_at_slot(vars: &Function, slot: u16) -> Option<String> {
    for i in 0..vars.count() {
        if vars.stack(i) == slot {
            let name = vars.name(i);
            if !name.starts_with("__") {
                return Some(name.to_string());
            }
        }
    }
    None
}

/// Find the function whose `code_position` matches `addr`.
fn find_fn_at_addr(data: &Data, addr: u32) -> Option<String> {
    for d in &data.definitions {
        if d.code_position == addr && !d.name.is_empty() {
            return Some(d.name.clone());
        }
    }
    None
}
