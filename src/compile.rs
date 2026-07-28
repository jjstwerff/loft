// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode)

#![allow(dead_code)]
//! Fast interpreter for binary code.
use crate::data::{Data, DefType, Type, Value};
use crate::data_store::ValueType;
use crate::ir_node::IrNode;
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
    byte_code_from(state, data, 0, None);
}

/// @PLN11 G2/M6 — warm-cache entry: lower from a pre-loaded persistent program
/// store (the mmap'd cache bundle), so codegen reads bodies straight from it via
/// `def_body_node` — no per-run materialise and no `read_data` body rebuild.
pub fn byte_code_with_store(
    state: &mut State,
    data: &mut Data,
    program_store: Option<&(crate::database::Stores, crate::keys::DbRef)>,
) {
    byte_code_from(state, data, 0, program_store);
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
pub fn byte_code_from(
    state: &mut State,
    data: &mut Data,
    start_d_nr: u32,
    warm_store: Option<&(crate::database::Stores, crate::keys::DbRef)>,
) {
    // Step 0 (startup-cache plan): env-gated phase timing.  No-op unless
    // LOFT_TIMING is set.  Separates native::init from the codegen loop.
    // The clock reads are gated too, not just the print: `Instant::now()`
    // PANICS on wasm32-unknown-unknown ("time not implemented") — it took
    // the browser kernel down at compile time (@PLN18 08-S6 gate).
    let timing = std::env::var("LOFT_TIMING").is_ok();
    let t_init = timing.then(std::time::Instant::now);
    if start_d_nr == 0 {
        native::init(state);
        register_native_stubs(state, data);
    }
    let init_ms = t_init.map_or(0.0, |t| t.elapsed().as_secs_f64() * 1000.0);
    let t_codegen = timing.then(std::time::Instant::now);
    // @PLN11 G2/M2/M6 — codegen body source:
    //  * `warm_store` (M6): a pre-loaded mmap'd cache bundle — read bodies
    //    straight from it (no materialise, no `read_data` body rebuild).
    //  * else `LOFT_CODEGEN_STORE` (M2/M5 proof): materialise the whole `Data`
    //    into a fresh store once.
    //  * else None → native lowering (default, unchanged).
    let cold_materialized;
    let program_store: Option<&(crate::database::Stores, DbRef)> = if warm_store.is_some() {
        warm_store
    } else if std::env::var_os("LOFT_CODEGEN_STORE").is_some() {
        let mut stores = crate::database::Stores::new();
        let root = crate::ir_store::materialize_data(&mut stores, data);
        cold_materialized = (stores, root);
        Some(&cold_materialized)
    } else {
        None
    };
    for d_nr in start_d_nr..data.definitions() {
        if !matches!(data.def(d_nr).def_type(), DefType::Function) || data.def(d_nr).is_operator() {
            continue;
        }
        state.def_code(d_nr, data, program_store);
    }
    if start_d_nr == 0 {
        build_const_vectors(state, data, program_store);
        state.database.allocations[crate::database::CONST_STORE as usize]
            .lock_with_origin("compile.rs::compile (CONST_STORE init)");
    }
    if let Some(t_codegen) = t_codegen {
        eprintln!(
            "LOFT_TIMING byte_code_from start={start_d_nr} native_init={init_ms:.2}ms codegen={:.2}ms",
            t_codegen.elapsed().as_secs_f64() * 1000.0
        );
    }
}

/// Extract literal values from vector constant Block IR and build
/// the vectors in CONST_STORE. Populates `state.const_refs` and
/// `data.definitions[d_nr].const_ref`.
fn build_const_vectors(
    state: &mut State,
    data: &mut Data,
    program_store: Option<&(crate::database::Stores, DbRef)>,
) {
    // Ensure const_refs is large enough for all definitions.
    let null_ref = DbRef::NULL;
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
        if data.def(d_nr).def_type() != DefType::Constant {
            continue;
        }
        let Type::Vector(elem_tp, _) = data.def(d_nr).returned() else {
            continue;
        };
        let elem_tp = (**elem_tp).clone();
        // @PLN11 G2/M6 — read the const body from the persistent store when
        // present (store-backed), else the native graph.
        let body = match program_store {
            Some((stores, root)) => {
                IrNode::Store(stores, crate::ir_read::def_body_node(stores, *root, d_nr))
            }
            None => IrNode::Native(data.def(d_nr).code()),
        };
        let values = extract_literal_values(body, data);
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
        let vec_tp = data.def(vec_struct_dnr).known_type();
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
        // Plan-57 Phase C: pin the const store so `free_named` never frees it
        // (it lives for the whole program).  Replaces the `ref_count = u32::MAX/2`
        // sentinel as the ref-count is removed.
        state.database.allocations[db.store_nr as usize].pinned = true;
        data.definitions[d_nr as usize].const_ref = Some(vec_ref);
        state.const_refs[d_nr as usize] = vec_ref;
        state.database.const_refs[d_nr as usize] = vec_ref;
    }
}

/// Walk the Block IR for a vector constant and extract the literal values.
/// Returns an empty Vec if the IR contains non-literal expressions.
/// Public wrapper for reuse by native codegen's init-emission.
pub fn extract_literal_values_public(code: &Value, data: &Data) -> Vec<Value> {
    extract_literal_values(IrNode::Native(code), data)
}

fn extract_literal_values(code: IrNode, data: &Data) -> Vec<Value> {
    if code.kind() != ValueType::Block {
        return vec![];
    }
    let block = code.as_block();
    let mut values = Vec::new();
    // Look for patterns: Call(OpSetInt/Float/Single/Text, [_, Int(0), literal_value])
    let set_int_nr = data.def_nr("OpSetInt");
    let set_float_nr = data.def_nr("OpSetFloat");
    let set_single_nr = data.def_nr("OpSetSingle");
    let set_text_nr = data.def_nr("OpSetText");
    for op in block.operators().iter() {
        if op.kind() != ValueType::Call {
            continue;
        }
        let fn_nr = op.call_to();
        let args = op.call_args();
        if args.len() < 3 {
            continue;
        }
        if fn_nr == set_int_nr
            || fn_nr == set_float_nr
            || fn_nr == set_single_nr
            || fn_nr == set_text_nr
        {
            let v = args.get(2);
            match v.kind() {
                ValueType::Int
                | ValueType::Float
                | ValueType::Single
                | ValueType::Long
                | ValueType::Text => {
                    values.push(v.to_owned_value());
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
        if def.native().is_empty() {
            continue;
        }
        let sym = def.native();
        // Skip if already registered (e.g. by native::init for built-in functions).
        if state.library_names.contains_key(sym) {
            continue;
        }
        stub_syms.insert(sym.to_string());
        if std::env::var("LOFT_STUB_DEBUG").is_ok() {
            eprintln!("STUBDBG panic-stub registered for native symbol '{sym}'");
        }
        // Register a stub that panics with an **actionable** message.  Reaching it
        // means a `#native` function's cdylib symbol could not be resolved at load
        // (`wire_native_fns` left the stub in place), so calling it aborts.  The cause
        // is almost always a missing or **stale** native cdylib — most often one built
        // against a *different* `libloft.rlib` (an auto-native library self-rebuilds when
        // the rlib changes; a hand-written `lib/<name>/native/` cdylib does **not**, so it
        // silently rots into this stub).  Name the fix, not the internal API: a generic
        // "call extensions::load_all() first" cost a multi-hour investigation once.
        // (A single generic stub — a `fn` pointer can't capture the symbol.)
        let stub: fn(&mut Stores, &mut DbRef) = {
            |_stores: &mut Stores, _db: &mut DbRef| {
                panic!(
                    "native function not loaded: its library's native cdylib is missing or \
                     stale (commonly: built against a different libloft.rlib). Rebuild the \
                     native libraries with `make rebuild-native-cdylibs` — or `cargo build \
                     --release` in the library's `native/` dir — then re-run."
                );
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
            data.def(d_nr).def_type(),
            DefType::Function | DefType::Dynamic
        ) {
            continue;
        }
        let is_op = data.def(d_nr).is_operator();
        if is_op && !config.show_all_functions {
            continue;
        }
        let from_default = is_default_file(&data.def(d_nr).position().file);
        if from_default && !config.show_all_functions {
            continue;
        }
        if !config.show_function(data.def(d_nr).name()) {
            continue;
        }
        write!(writer, "{} ", data.def(d_nr).header(data, d_nr))?;
        let mut vars = Function::copy(data.def(d_nr).variables());
        data.show_code(writer, &mut vars, data.def(d_nr).code(), 0, false)?;
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
        if !matches!(def.def_type(), DefType::Function | DefType::Dynamic) {
            continue;
        }
        let is_lambda = def.closure_record() != u32::MAX;
        let direct_match = def.name().contains(&target);
        if !direct_match && !is_lambda {
            continue;
        }
        if def.scalars_to_box().is_empty() && !is_lambda {
            continue;
        }
        writeln!(
            writer,
            "[captures] === {} (d_nr={d_nr}, {kind}) ===",
            def.name(),
            kind = if is_lambda { "lambda" } else { "parent" }
        )?;
        if !def.scalars_to_box().is_empty() {
            writeln!(
                writer,
                "[captures]   scalars_to_box = {:?}",
                def.scalars_to_box()
            )?;
        }
        if is_lambda {
            writeln!(
                writer,
                "[captures]   mutated_captures = {:?}",
                def.mutated_captures()
            )?;
            let cr_d = def.closure_record();
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
/// True if `file` is inside the `default/` standard library.  Handles
/// relative paths (test dumps parse `default/` relatively) and absolute
/// paths (the `--introspect` CLI resolves the stdlib to `<exe>/../default`),
/// so the default-skip filter holds in both.  Mirrors
/// `introspect::is_default_lib_path`.
pub(crate) fn is_default_file(file: &str) -> bool {
    file.starts_with("default/")
        || file.starts_with("default\\")
        || file.contains("/default/")
        || file.contains("\\default\\")
}

/// Write the static dump (IR and/or bytecode) for the functions selected by
/// `config` to `writer` — the engine behind `LOFT_LOG` dumps and the
/// `--introspect` bytecode section.
///
/// # Errors
/// Propagates any I/O error from writing to `writer`.
pub fn show_code(
    writer: &mut dyn Write,
    state: &mut State,
    data: &mut Data,
    config: &LogConfig,
) -> Result<(), Error> {
    for d_nr in 0..data.definitions() {
        if !matches!(
            data.def(d_nr).def_type(),
            DefType::Function | DefType::Dynamic
        ) {
            continue;
        }
        let is_op = data.def(d_nr).is_operator();
        if is_op && !config.show_all_functions {
            continue;
        }
        let from_default = is_default_file(&data.def(d_nr).position().file);
        if from_default && !config.show_all_functions {
            continue;
        }
        if !config.show_function(data.def(d_nr).name()) {
            continue;
        }
        if config.phases.ir {
            write!(writer, "{} ", data.def(d_nr).header(data, d_nr))?;
            let mut vars = Function::copy(data.def(d_nr).variables());
            data.show_code(writer, &mut vars, data.def(d_nr).code(), 0, false)?;
            writeln!(writer, "\n")?;
        }
        if config.phases.bytecode {
            write!(writer, "byte-code for {}:", data.def(d_nr).position().file)?;
            state.dump_code(writer, d_nr, data, config.annotate_slots)?;
        }
        if config.show_variables {
            write!(writer, "variables for {}:", data.def(d_nr).position().file)?;
            writeln!(writer, "{}", data.def(d_nr).header(data, d_nr))?;
            dump_variables(writer, data.def(d_nr).variables(), data)?;
        }
    }
    Ok(())
}

// ── Standalone bytecode disassembler ─────────────────────────────────────────

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
) -> Result<(), Error> {
    let def = data.def(d_nr);
    let start = def.code_position as usize;
    let end = start + def.code_length as usize;
    let vars = def.variables();

    let targets = collect_jump_targets(bytecode, start, end, data);
    writeln!(writer, "--- {} ---", def.name())?;

    let mut pc = start;
    while pc < end && pc < bytecode.len() {
        let rel = pc - start;
        let first = bytecode[pc];
        let (op, op_byte_len): (u16, usize) = if first == 255 && pc + 1 < bytecode.len() {
            (255u16 + u16::from(bytecode[pc + 1]), 2)
        } else {
            (u16::from(first), 1)
        };
        let ilen = instruction_len(bytecode, pc, data).unwrap_or(0);
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

/// Decode the byte length of the single instruction at `pc`, reading the
/// actual operands so variable-length constants advance correctly:
/// `Text` is `[len:u8][bytes]` and `Keys` is `[len:u8][(i8,u16) × len]`.
/// A fixed per-opcode table cannot express these (`size(Text)` reports the
/// in-memory pointer width, not the inline byte count).  `None` if the
/// opcode is unknown or its operands run past the buffer.
fn instruction_len(bytecode: &[u8], pc: usize, data: &Data) -> Option<usize> {
    use crate::data::Context;
    use crate::variables::size as type_size;
    let first = *bytecode.get(pc)?;
    let (op, lead) = if first == 255 {
        (255u16 + u16::from(*bytecode.get(pc + 1)?), 2usize)
    } else {
        (u16::from(first), 1usize)
    };
    if !data.has_op(op) {
        return None;
    }
    let mut cursor = pc + lead;
    for a in &data.operator(op).attributes {
        if !a.constant {
            continue;
        }
        let n = match &a.typedef {
            Type::Text(_) => 1 + *bytecode.get(cursor)? as usize,
            Type::Keys => 1 + (*bytecode.get(cursor)? as usize) * 3,
            t => type_size(t, &Context::Constant) as usize,
        };
        cursor += n;
    }
    Some(cursor - pc)
}

/// Pre-pass: scan the bytecode for goto-style instructions and collect
/// their target offsets.  Disassemblers emit a label at each target so
/// forward / backward jumps are readable — and so a re-assembler can
/// re-derive relative offsets after ops are inserted or removed (the jump
/// binds to a label identity, not a byte offset).
pub(crate) fn collect_jump_targets(
    bytecode: &[u8],
    start: usize,
    end: usize,
    data: &Data,
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
        let Some(ilen) = instruction_len(bytecode, pc, data) else {
            break;
        };
        if data.has_op(op) {
            let name = &data.operator(op).name;
            if (name == "OpGoto" || name == "OpGotoFalse") && ilen == 2 && pc + 1 < end {
                let off = bytecode[pc + 1] as i8;
                let target = (pc as i32 + 2 + i32::from(off)) as usize;
                targets.insert(target);
            } else if (name == "OpGotoWord" || name == "OpGotoFalseWord")
                && ilen == 5
                && pc + 4 < end
            {
                // 32-bit displacement (loft#654) — 1 opcode byte + 4 operand.
                let off = i32::from_le_bytes([
                    bytecode[pc + 1],
                    bytecode[pc + 2],
                    bytecode[pc + 3],
                    bytecode[pc + 4],
                ]);
                let target = (pc as i64 + 5 + i64::from(off)) as usize;
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

// ── Re-assembler: dump text → bytecode (inverse of `dump_code`) ───────────────

/// Escape a string constant for the bytecode dump so control characters
/// (newline, tab, …) and the delimiters `"` / `\` stay on one line and
/// round-trip through [`unescape_text`].
#[must_use]
pub(crate) fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

/// Inverse of [`escape_text`].
fn unescape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Find the matching `)` for a `(` already consumed, honouring quoted
/// strings (with `\"` escapes) and nested parens.  `s` starts just after
/// the opening `(`.
fn matching_paren(s: &str) -> Option<usize> {
    let mut depth = 1i32;
    let mut in_str = false;
    let mut esc = false;
    for (idx, c) in s.char_indices() {
        if esc {
            esc = false;
            continue;
        }
        match c {
            '\\' if in_str => esc = true,
            '"' => in_str = !in_str,
            '(' if !in_str => depth += 1,
            ')' if !in_str => {
                depth -= 1;
                if depth == 0 {
                    return Some(idx);
                }
            }
            _ => {}
        }
    }
    None
}

/// Extract the value rendered after `name=` in a dumped arg list.  Returns
/// the raw token (text values keep their surrounding quotes).
fn arg_value<'a>(args: &'a str, name: &str) -> Option<&'a str> {
    let bytes = args.as_bytes();
    let mut i = 0;
    while i + name.len() < args.len() {
        let boundary = i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
        if boundary && args[i..].starts_with(name) && bytes.get(i + name.len()) == Some(&b'=') {
            let rest = &args[i + name.len() + 1..];
            if let Some(stripped) = rest.strip_prefix('"') {
                // closing quote = first `"` not preceded by an odd run of `\`
                let sb = stripped.as_bytes();
                let mut j = 0;
                while j < sb.len() {
                    match sb[j] {
                        b'\\' => j += 2,
                        b'"' => return Some(&rest[..j + 2]),
                        _ => j += 1,
                    }
                }
                return Some(rest);
            }
            let end = rest.find(',').unwrap_or(rest.len());
            return Some(rest[..end].trim());
        }
        i += 1;
    }
    None
}

/// Split a dumped instruction line into `(stack_annotation, op_name, args)`.
/// Returns `None` for non-instruction lines (signature, blank, label).
fn parse_instr_line(line: &str) -> Option<(Option<i64>, &str, &str)> {
    let t = line.trim_start();
    let b = t.as_bytes();
    if b.is_empty() || !b[0].is_ascii_digit() {
        return None;
    }
    let mut i = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let mut stack = None;
    if i < b.len() && b[i] == b'[' {
        let close = t[i..].find(']')? + i;
        stack = t[i + 1..close].parse().ok();
        i = close + 1;
    }
    let colon = t[i..].find(':')? + i;
    i = colon + 1;
    while i < b.len() && b[i] == b' ' {
        i += 1;
    }
    if i < b.len() && b[i] == b'[' {
        let close = t[i..].find(']')? + i;
        i = close + 1;
        while i < b.len() && b[i] == b' ' {
            i += 1;
        }
    }
    let paren = t[i..].find('(')? + i;
    let op_name = t[i..paren].trim();
    let args_start = paren + 1;
    let args_end = matching_paren(&t[args_start..])? + args_start;
    Some((stack, op_name, &t[args_start..args_end]))
}

/// Encode one constant attribute's rendered value, mirroring the size
/// logic of `State::dump_attribute`.
fn encode_const(out: &mut Vec<u8>, a: &crate::data::Attribute, val: &str) -> Result<(), String> {
    let bad = |t: &str| format!("cannot parse {t} value {val:?} for `{}`", a.name);
    match &a.typedef {
        Type::Integer(s) if s.range() - 1 <= 256 && s.min == 0 => {
            out.push(val.parse::<i32>().map_err(|_| bad("u8"))? as u8);
        }
        Type::Integer(s) if s.range() - 1 <= 65536 && s.min == 0 => {
            out.extend_from_slice(
                &(val.parse::<i32>().map_err(|_| bad("u16"))? as u16).to_le_bytes(),
            );
        }
        Type::Integer(s) if s.range() - 1 <= 256 => {
            out.push(val.parse::<i32>().map_err(|_| bad("i8"))? as i8 as u8);
        }
        Type::Integer(s) if s.range() - 1 <= 65536 => {
            out.extend_from_slice(
                &(val.parse::<i32>().map_err(|_| bad("i16"))? as i16).to_le_bytes(),
            );
        }
        Type::Integer(_) => {
            out.extend_from_slice(&val.parse::<i64>().map_err(|_| bad("i64"))?.to_le_bytes());
        }
        Type::Boolean => out.push(u8::from(val == "true")),
        Type::Enum(_, false, _) => out.push(val.parse::<u8>().map_err(|_| bad("enum"))?),
        Type::Single => {
            out.extend_from_slice(&val.parse::<f32>().map_err(|_| bad("f32"))?.to_le_bytes());
        }
        Type::Float => {
            out.extend_from_slice(&val.parse::<f64>().map_err(|_| bad("f64"))?.to_le_bytes());
        }
        Type::Text(_) => {
            let inner = val
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .unwrap_or(val);
            let s = unescape_text(inner);
            let len = u8::try_from(s.len()).map_err(|_| bad("text-len"))?;
            out.push(len);
            out.extend_from_slice(s.as_bytes());
        }
        Type::Character => {
            let c = val.chars().next().ok_or_else(|| bad("char"))?;
            out.extend_from_slice(&(c as u32).to_le_bytes());
        }
        _ => return Err(format!("unsupported const type for `{}`", a.name)),
    }
    Ok(())
}

/// Re-assemble one function's bytecode from its `dump_code` text (the
/// `:POS`-labelled disassembly).  For an UNEDITED dump the result equals
/// the original function bytecode byte-for-byte — the round-trip property
/// that proves the dump is a faithful, editable representation.
///
/// `library_names` (name → index) inverts the `OpStaticCall` rendering,
/// which shows the resolved native-function name rather than its index.
///
/// # Errors
/// Returns a description of the first construct it cannot invert (unknown
/// opcode, unsupported attribute type, dangling label, …).
pub fn reassemble_function(
    dump: &str,
    data: &Data,
    library_names: &std::collections::HashMap<String, u16>,
) -> Result<Vec<u8>, String> {
    use std::collections::BTreeMap;
    let mut out: Vec<u8> = Vec::new();
    let mut labels: BTreeMap<String, usize> = BTreeMap::new();
    let mut fixups: Vec<(usize, String, usize)> = Vec::new(); // (byte_pos, label, width)

    for raw in dump.lines() {
        let line = raw.trim_end();
        let trimmed = line.trim_start();
        if trimmed.is_empty() || line.contains("return-address") || line.contains("byte-code for") {
            continue;
        }
        if let Some(name) = trimmed.strip_prefix(':') {
            labels.insert(name.trim().to_string(), out.len());
            continue;
        }
        let Some((stack, op_name, args)) = parse_instr_line(line) else {
            continue; // signature / annotation line
        };
        let op = opcode_by_name(data, &format!("Op{op_name}"));
        if op == u16::MAX {
            return Err(format!("unknown opcode `Op{op_name}` in: {line}"));
        }
        if op >= 255 {
            out.push(255);
            out.push((op - 255) as u8);
        } else {
            out.push(op as u8);
        }
        for (a_nr, a) in data.operator(op).attributes.iter().enumerate() {
            if !a.constant {
                continue; // mutable arg = stack operand, no bytes
            }
            if op_name.starts_with("Goto") {
                let v = arg_value(args, "jump")
                    .ok_or_else(|| format!("goto without `jump=`: {line}"))?;
                let width = match &a.typedef {
                    Type::Integer(s) if s.range() - 1 <= 256 => 1,
                    _ => 2,
                };
                fixups.push((out.len(), v.trim_start_matches(':').to_string(), width));
                out.extend(std::iter::repeat_n(0u8, width));
            } else if op_name == "Call" && a_nr == 2 {
                let name =
                    arg_value(args, "fn").ok_or_else(|| format!("call without `fn=`: {line}"))?;
                let pos = data
                    .definitions
                    .iter()
                    .find(|d| d.name == name)
                    .ok_or_else(|| format!("call target `{name}` not found"))?
                    .code_position;
                out.extend_from_slice(&i64::from(pos).to_le_bytes());
            } else if op_name == "StaticCall" {
                let name = args.trim();
                let idx = library_names
                    .get(name)
                    .ok_or_else(|| format!("unknown static call `{name}`"))?;
                out.extend_from_slice(&idx.to_le_bytes());
            } else if a.name == "pos"
                && a_nr == 0
                && let Some(vs) = args.find("var[").map(|x| x + 4)
            {
                let s = stack.ok_or_else(|| format!("var-slot needs stack annotation: {line}"))?;
                let ve = args[vs..]
                    .find(']')
                    .map(|x| x + vs)
                    .ok_or_else(|| format!("unterminated var[ in: {line}"))?;
                let slot: i64 = args[vs..ve]
                    .parse()
                    .map_err(|_| format!("bad slot in: {line}"))?;
                out.extend_from_slice(&((s - slot) as u16).to_le_bytes());
            } else {
                let v = arg_value(args, &a.name)
                    .ok_or_else(|| format!("missing arg `{}` in: {line}", a.name))?;
                encode_const(&mut out, a, v)?;
            }
        }
    }

    for (pos, name, width) in fixups {
        let target = *labels
            .get(&name)
            .ok_or_else(|| format!("jump to undefined label :{name}"))?;
        let delta = target as i64 - (pos + width) as i64;
        let bytes: [u8; 2] = if width == 1 {
            [
                i8::try_from(delta).map_err(|_| format!("jump :{name} out of i8 range"))? as u8,
                0,
            ]
        } else {
            i16::try_from(delta)
                .map_err(|_| format!("jump :{name} out of i16 range"))?
                .to_le_bytes()
        };
        for (k, b) in bytes.iter().take(width).enumerate() {
            if let Some(slot) = out.get_mut(pos + k) {
                *slot = *b;
            }
        }
    }
    Ok(out)
}
