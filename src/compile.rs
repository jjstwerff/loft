// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(dead_code)]
//! Fast interpreter for binary code.
use crate::data::{Data, DefType, Type, Value};
use crate::keys::DbRef;
use crate::log_config::LogConfig;
use crate::native;
use crate::state::State;
use crate::variables::{Function, dump_variables};
use std::io::{Error, Write};
// Bytecode generation

/// Create byte code.
pub fn byte_code(state: &mut State, data: &mut Data) {
    byte_code_with_cache(state, data, None, &[]);
}

/// Create byte code, optionally using a `.loftc` cache file.
/// `cache_path` is the path to write/read the cache. When `None`, no caching.
/// `sources` is the list of (filename, content) pairs used for the cache key.
pub fn byte_code_with_cache(
    state: &mut State,
    data: &mut Data,
    cache_file: Option<&str>,
    sources: &[(&str, &str)],
) {
    native::init(state);
    register_native_stubs(state, data);

    // Try loading from cache.
    if let Some(path) = cache_file {
        let key = crate::cache::cache_key(sources);
        if let Some(cached) = crate::cache::read_cache(path, &key) {
            if load_from_cache(state, data, &cached) {
                return;
            }
        }
    }

    // Full compilation.
    for d_nr in 0..data.definitions() {
        if !matches!(data.def(d_nr).def_type, DefType::Function) || data.def(d_nr).is_operator() {
            continue;
        }
        state.def_code(d_nr, data);
    }
    build_const_vectors(state, data);
    state.database.allocations[crate::database::CONST_STORE as usize].lock();

    // Write cache for next run.
    if let Some(path) = cache_file {
        let key = crate::cache::cache_key(sources);
        let cache_data = crate::cache::collect_cache_data(state, data);
        let _ = crate::cache::write_cache(path, &key, &cache_data);
    }
}

/// Restore State + Data from cached compilation output. Returns true on success.
fn load_from_cache(state: &mut State, data: &mut Data, cached: &crate::cache::CacheData) -> bool {
    use std::sync::Arc;

    // Restore bytecode.
    state.bytecode = Arc::new(cached.bytecode.clone());

    // Restore CONST_STORE buffer.
    let cs_idx = crate::database::CONST_STORE as usize;
    let cs = &mut state.database.allocations[cs_idx];
    let words = cached.const_store_buf.len() / 8;
    if words > 0 {
        // Resize the store to fit the cached data, then copy.
        if cs.capacity_words() < words as u32 {
            cs.resize(0, words as u32);
        }
        unsafe {
            std::ptr::copy_nonoverlapping(
                cached.const_store_buf.as_ptr(),
                cs.ptr,
                cached.const_store_buf.len(),
            );
        }
    }
    cs.lock();
    cs.ref_count = u32::MAX / 2;

    // Restore vector constant stores.
    for vs in &cached.vector_stores {
        let idx = vs.store_nr as usize;
        // Ensure the allocations array is large enough.
        while state.database.allocations.len() <= idx {
            state.database.allocations.push(crate::store::Store::new(4));
        }
        let store = &mut state.database.allocations[idx];
        store.free = false;
        let words = vs.data.len() / 8;
        if store.capacity_words() < words as u32 {
            store.resize(0, words as u32);
        }
        unsafe {
            std::ptr::copy_nonoverlapping(vs.data.as_ptr(), store.ptr, vs.data.len());
        }
        store.lock();
        store.ref_count = u32::MAX / 2;
        // Update max store index.
        if vs.store_nr >= state.database.max {
            state.database.max = vs.store_nr + 1;
        }
    }

    // Restore const_refs.
    let null_ref = DbRef {
        store_nr: u16::MAX,
        rec: 0,
        pos: 0,
    };
    state
        .const_refs
        .resize(data.definitions() as usize, null_ref);
    for cr in &cached.const_refs {
        if (cr.d_nr as usize) < state.const_refs.len() {
            state.const_refs[cr.d_nr as usize] = DbRef {
                store_nr: cr.store_nr,
                rec: cr.rec,
                pos: cr.pos,
            };
            // Also set on the Definition for native codegen.
            data.definitions[cr.d_nr as usize].const_ref = Some(DbRef {
                store_nr: cr.store_nr,
                rec: cr.rec,
                pos: cr.pos,
            });
        }
    }

    // Restore function code positions.
    for func in &cached.functions {
        let idx = func.d_nr as usize;
        if idx < data.definitions.len() {
            data.definitions[idx].code_position = func.code_position;
            data.definitions[idx].code_length = func.code_length;
        }
    }

    true
}

/// P127: extract literal values from vector constant Block IR and build
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
            .set_int(db.rec, 4, i32::from(vec_tp));
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
                    state.database.store_mut(&rec).set_int(rec.rec, rec.pos, *v);
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
                _ => {}
            }
            state.database.record_finish(&vec_ref, &rec, vec_tp, 0);
        }
        state.database.allocations[db.store_nr as usize].lock();
        // High ref_count ensures free/dec_rc never actually frees this store.
        state.database.allocations[db.store_nr as usize].ref_count = u32::MAX / 2;
        data.definitions[d_nr as usize].const_ref = Some(vec_ref);
        state.const_refs[d_nr as usize] = vec_ref;
    }
}

/// Walk the Block IR for a vector constant and extract the literal values.
/// Returns an empty Vec if the IR contains non-literal expressions.
fn extract_literal_values(code: &Value, data: &Data) -> Vec<Value> {
    let Value::Block(block) = code else {
        return vec![];
    };
    let mut values = Vec::new();
    // Look for patterns: Call(OpSetInt/Float/Single, [_, Int(0), literal_value])
    let set_int_nr = data.def_nr("OpSetInt");
    let set_float_nr = data.def_nr("OpSetFloat");
    let set_single_nr = data.def_nr("OpSetSingle");
    let set_long_nr = data.def_nr("OpSetLong");
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
            || *fn_nr == set_long_nr
        {
            match &args[2] {
                v @ (Value::Int(_) | Value::Float(_) | Value::Single(_) | Value::Long(_)) => {
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

/// Build a 256-entry table mapping opcode → instruction byte-length.
/// Length = 1 (opcode) + sum of const-argument sizes from the definition.
#[must_use]
pub fn build_opcode_len_table(data: &Data) -> [u8; 256] {
    use crate::data::Context;
    use crate::variables::size;
    let mut table = [0u8; 256]; // 0 = unknown opcode
    for (&op, &d_nr) in &data.operators {
        let def = &data.definitions[d_nr as usize];
        let mut len = 1u16; // opcode byte
        for a in &def.attributes {
            if a.constant {
                len += size(&a.typedef, &Context::Constant);
            }
        }
        table[op as usize] = len as u8;
    }
    table
}

/// Resolve opcode number by operator name.  Returns `u8::MAX` if not found.
#[must_use]
pub fn opcode_by_name(data: &Data, name: &str) -> u8 {
    for (&op, &d_nr) in &data.operators {
        if data.definitions[d_nr as usize].name == name {
            return op;
        }
    }
    u8::MAX
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
    op_len: &[u8; 256],
) -> Result<(), Error> {
    use crate::data::{Context, Type};
    use crate::variables::size;

    let def = data.def(d_nr);
    let start = def.code_position as usize;
    let end = start + def.code_length as usize;
    let vars = &def.variables;

    // Collect jump targets so we can label them.
    let mut targets = std::collections::BTreeSet::new();
    {
        let mut pc = start;
        while pc < end && pc < bytecode.len() {
            let op = bytecode[pc];
            let ilen = op_len[op as usize] as usize;
            if ilen == 0 {
                break;
            }
            // Detect goto/goto_false with i8 offset (2-byte instructions).
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
    }

    // Header.
    writeln!(writer, "--- {} ---", def.name)?;

    // Disassemble.
    let mut pc = start;
    while pc < end && pc < bytecode.len() {
        let rel = pc - start;
        let op = bytecode[pc];
        let ilen = op_len[op as usize] as usize;
        if ilen == 0 {
            writeln!(writer, "{rel:4}: ??? (opcode {op})")?;
            break;
        }

        // Label if this is a jump target.
        if targets.contains(&pc) {
            writeln!(writer, "  .L{rel}:")?;
        }

        // Opcode name.
        let op_name = if data.has_op(op) {
            let n = &data.operator(op).name;
            // Strip "Op" prefix for readability.
            if n.starts_with("Op") {
                &n[2..]
            } else {
                n.as_str()
            }
        } else {
            "???"
        };

        // Decode const arguments.
        let mut args = String::new();
        if data.has_op(op) {
            let op_def = data.operator(op);
            let mut cursor = pc + 1; // past opcode byte
            for a in &op_def.attributes {
                if a.constant {
                    let a_size = size(&a.typedef, &Context::Constant) as usize;
                    if !args.is_empty() {
                        args.push_str(", ");
                    }
                    // Decode based on size.
                    match a_size {
                        1 if matches!(a.typedef, Type::Integer(_, _, _)) => {
                            let v = bytecode[cursor] as i8;
                            // Check if this is a jump offset.
                            if op_name.contains("Goto") {
                                let target = (cursor as i32 + 1 + i32::from(v)) as usize - start;
                                args.push_str(&format!("{}=.L{target}", a.name));
                            } else {
                                args.push_str(&format!("{}={v}", a.name));
                            }
                        }
                        1 => {
                            args.push_str(&format!("{}={}", a.name, bytecode[cursor]));
                        }
                        2 if op_name.contains("Goto") => {
                            let v = i16::from_le_bytes([bytecode[cursor], bytecode[cursor + 1]]);
                            let target = (cursor as i32 + 2 + i32::from(v)) as usize - start;
                            args.push_str(&format!("{}=.L{target}", a.name));
                        }
                        2 => {
                            let v = u16::from_le_bytes([bytecode[cursor], bytecode[cursor + 1]]);
                            // Try to resolve as variable name.
                            let vname = find_var_at_slot(vars, v);
                            if let Some(name) = vname {
                                args.push_str(&format!("{}={name}@{v}", a.name));
                            } else {
                                args.push_str(&format!("{}={v}", a.name));
                            }
                        }
                        4 => {
                            let v = i32::from_le_bytes([
                                bytecode[cursor],
                                bytecode[cursor + 1],
                                bytecode[cursor + 2],
                                bytecode[cursor + 3],
                            ]);
                            // Check if this is a call target (function address).
                            if op_name == "Call" {
                                let fname = find_fn_at_addr(data, v as u32);
                                args.push_str(&format!(
                                    "{}={}",
                                    a.name,
                                    fname.unwrap_or_else(|| format!("@{v}"))
                                ));
                            } else {
                                args.push_str(&format!("{}={v}", a.name));
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
                            args.push_str(&format!("{}={v}", a.name));
                        }
                        _ => {
                            args.push_str(&format!("{}=?({a_size}B)", a.name));
                        }
                    }
                    cursor += a_size;
                } else {
                    // Stack argument — just show name:type.
                    if !args.is_empty() {
                        args.push_str(", ");
                    }
                    args.push_str(&format!("{}: {}", a.name, a.typedef.name(data)));
                }
            }
        }

        writeln!(writer, "{rel:4}: {op_name}({args})")?;
        pc += ilen;
    }
    writeln!(writer)?;
    Ok(())
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
