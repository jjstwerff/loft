// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(dead_code)]
//! Fast interpreter for binary code.
use crate::data::{Data, DefType};
use crate::log_config::LogConfig;
use crate::native;
use crate::state::State;
use crate::variables::{Function, dump_variables};
use std::io::{Error, Write};
// Bytecode generation

/// Create byte code.
pub fn byte_code(state: &mut State, data: &mut Data) {
    native::init(state);
    for d_nr in 0..data.definitions() {
        if !matches!(data.def(d_nr).def_type, DefType::Function) || data.def(d_nr).is_operator() {
            continue;
        }
        state.def_code(d_nr, data);
    }
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
                        1 if matches!(a.typedef, Type::Integer(_, _)) => {
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
