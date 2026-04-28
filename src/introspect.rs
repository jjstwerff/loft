// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Plan-08 phase 01: introspection CLI.
//!
//! Wraps the existing dump primitives (`State::dump_bytecode`,
//! `Output::output_native`, `variables::validate::dump_variables`) in
//! a single `loft --introspect <file>` flow that emits the program's
//! bytecode, generated Rust, and per-function variable slot tables to
//! stdout (or per-section files).
//!
//! See `doc/claude/plans/08-repl-and-introspection/01-introspection-cli.md`
//! for the surface design.

use crate::compile;
use crate::data::{Data, DefType};
use crate::diagnostics::Level;
use crate::generation;
use crate::log_config::LogConfig;
use crate::parser::Parser;
use crate::scopes;
use crate::state::State;
use crate::variables;
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufWriter, Write};

/// Section selector — mirrors the three things the introspection
/// tool can emit, in canonical order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Bytecode,
    Rust,
    Slots,
}

/// Options for `loft --introspect`.
#[allow(dead_code)] // lib_dirs / install_dir reserved for the standalone `run()` entry,
                    // unused by main.rs's emit_all path.
pub struct Options {
    /// Sections the user asked for.  Empty = all three.
    pub sections: Vec<Section>,
    /// Per-section output paths.  When unset, the section writes to
    /// the shared stdout sink with a `=== bytecode ===`-style header.
    pub bytecode_out: Option<String>,
    pub rust_out: Option<String>,
    pub slots_out: Option<String>,
    /// Restrict every section to functions whose name matches one of
    /// these strings (substring match, like `LOFT_LOG=fn:<name>`).
    /// Empty = include all user functions (filtered further by
    /// `all_fns`).
    pub fn_filter: Vec<String>,
    /// Include `default/` stdlib functions in every section.
    pub all_fns: bool,
    /// Library directories to pass to the parser (mirrors the main
    /// program's `--lib` / `--path` semantics).
    pub lib_dirs: Vec<String>,
    /// Path prefix to the loft installation (`--path` flag).  Empty
    /// means "binary directory" — same default as `loft <file>`.
    pub install_dir: String,
}

impl Options {
    /// Default introspect options — all three sections, no filter,
    /// stdout sink for everything.
    #[must_use]
    pub fn new() -> Self {
        Self {
            sections: Vec::new(),
            bytecode_out: None,
            rust_out: None,
            slots_out: None,
            fn_filter: Vec::new(),
            all_fns: false,
            lib_dirs: Vec::new(),
            install_dir: String::new(),
        }
    }

    fn includes(&self, s: Section) -> bool {
        self.sections.is_empty() || self.sections.contains(&s)
    }
}

impl Default for Options {
    fn default() -> Self {
        Self::new()
    }
}

/// Run the introspection tool against `filename`.
///
/// Standalone entry: parses the file from scratch, runs scopes +
/// codegen, then dispatches to `emit_all`.  `main.rs` skips this
/// and calls `emit_all` directly because it has already parsed
/// the file by the time the `--introspect` branch fires.  Kept
/// public + `#[allow(dead_code)]` because `tests/` may use it.
///
/// # Errors
/// Returns I/O errors from sink writes; propagates parser fatal
/// diagnostics by exiting with status 1 (mirrors `loft <file>`'s
/// behaviour).
#[allow(dead_code)]
pub fn run(filename: &str, opts: &Options) -> std::io::Result<()> {
    let abs_file = std::path::Path::new(filename)
        .canonicalize()
        .unwrap_or_else(|_| std::path::PathBuf::from(filename))
        .to_string_lossy()
        .into_owned();
    let mut p = Parser::new();
    p.lib_dirs = opts.lib_dirs.clone();
    let default_dir = if opts.install_dir.is_empty() {
        "default".to_string()
    } else {
        format!("{}default", opts.install_dir)
    };
    p.parse_dir(&default_dir, true, false)
        .expect("parse default/ stdlib");
    let start_def = p.data.definitions();
    p.parse(&abs_file, false);
    if !p.diagnostics.is_empty() {
        for entry in p.diagnostics.entries() {
            if entry.level == Level::Debug {
                continue;
            }
            eprintln!("{}", entry.to_string_compact());
        }
        if p.diagnostics.level() >= Level::Error {
            std::process::exit(1);
        }
    }
    scopes::check(&mut p.data);
    let mut state = State::new(p.database);
    compile::byte_code(&mut state, &mut p.data);
    let end_def = p.data.definitions();
    let _ = start_def;
    emit_all(&mut p.data, &mut state, end_def, opts)
}

/// Emit the requested sections from an already-parsed program.
/// Used by the `loft --introspect` CLI dispatch in `main.rs` to
/// avoid re-parsing — the main flow has already run the parser +
/// scopes::check + compile::byte_code by the time the introspect
/// branch is reached.
///
/// # Errors
/// Propagates I/O errors from any section's writer.
pub fn emit_all(
    data: &mut Data,
    state: &mut State,
    end_def: u32,
    opts: &Options,
) -> std::io::Result<()> {
    let stdout = std::io::stdout();
    if opts.includes(Section::Bytecode) {
        let mut writer: Box<dyn Write> = make_writer(opts.bytecode_out.as_deref(), &stdout)?;
        if opts.bytecode_out.is_none() {
            writeln!(writer, "=== bytecode ===")?;
        }
        emit_bytecode(&mut writer, state, data, opts)?;
    }
    if opts.includes(Section::Rust) {
        let mut writer: Box<dyn Write> = make_writer(opts.rust_out.as_deref(), &stdout)?;
        if opts.rust_out.is_none() {
            writeln!(writer)?;
            writeln!(writer, "=== rust ===")?;
        }
        emit_rust(&mut writer, data, &state.database, end_def)?;
    }
    if opts.includes(Section::Slots) {
        let mut writer: Box<dyn Write> = make_writer(opts.slots_out.as_deref(), &stdout)?;
        if opts.slots_out.is_none() {
            writeln!(writer)?;
            writeln!(writer, "=== slots ===")?;
        }
        emit_slots(&mut writer, data, end_def, opts)?;
    }
    Ok(())
}

fn make_writer(
    path: Option<&str>,
    stdout: &std::io::Stdout,
) -> std::io::Result<Box<dyn Write>> {
    if let Some(p) = path {
        Ok(Box::new(BufWriter::new(File::create(p)?)))
    } else {
        Ok(Box::new(stdout.lock()))
    }
}

fn emit_bytecode(
    w: &mut dyn Write,
    state: &mut State,
    data: &mut Data,
    opts: &Options,
) -> std::io::Result<()> {
    let mut config = LogConfig::static_only();
    config.annotate_slots = true;
    config.show_all_functions = opts.all_fns;
    if !opts.fn_filter.is_empty() {
        config.show_functions = Some(opts.fn_filter.clone());
    }
    state
        .dump_bytecode(w, &config, data)
        .map_err(|e| std::io::Error::other(format!("dump_bytecode: {e}")))
}

fn emit_rust(
    w: &mut dyn Write,
    data: &Data,
    stores: &crate::database::Stores,
    end_def: u32,
) -> std::io::Result<()> {
    let mut out = generation::Output {
        data,
        stores,
        counter: 0,
        indent: 0,
        def_nr: 0,
        declared: HashSet::new(),
        reachable: HashSet::new(),
        loop_stack: Vec::new(),
        next_format_count: 0,
        yield_collect: false,
        fn_ref_context: false,
        i32_literal_context: false,
        tuple_text_to_string: false,
        call_stack_prefix: None,
        wasm_browser: false,
    };
    out.output_native(w, 0, end_def)
}

fn emit_slots(
    w: &mut dyn Write,
    data: &Data,
    end_def: u32,
    opts: &Options,
) -> std::io::Result<()> {
    for d_nr in 0..end_def {
        let def = data.def(d_nr);
        if def.def_type != DefType::Function {
            continue;
        }
        // User-callable functions are prefixed with `n_`; skip
        // operator definitions, generic templates, etc.  Mirrors
        // the bytecode path's user-fn filter.
        if !def.name.starts_with("n_") || def.name.starts_with("n___lambda_") {
            continue;
        }
        if !opts.all_fns && def.position.file.starts_with("default/") {
            continue;
        }
        if !opts.fn_filter.is_empty()
            && !opts
                .fn_filter
                .iter()
                .any(|f| def.name == *f || def.name == format!("n_{f}") || def.name.contains(f))
        {
            continue;
        }
        writeln!(w, "fn {}:", def.name)?;
        variables::dump_variables(w, &def.variables, data)
            .map_err(|e| std::io::Error::other(format!("dump_variables: {e}")))?;
        writeln!(w)?;
    }
    Ok(())
}
