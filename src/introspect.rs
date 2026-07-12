// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @F50 — introspection (bytecode / native Rust / slot tables)

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
use crate::data::{Data, DefType, Value};
use crate::diagnostics::Level;
use crate::generation;
use crate::log_config::LogConfig;
use crate::parser::Parser;
use crate::scopes;
use crate::state::State;
use crate::variables;
use std::fs::File;
use std::io::{BufWriter, Write};

/// Section selector — mirrors the four things the introspection
/// tool can emit, in canonical order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Bytecode,
    Rust,
    Slots,
    /// Per-function variable types with dependency tracking — useful
    /// for diagnosing lifetime / dep-propagation bugs (e.g. P197
    /// where `s: text` should have read `s: text[a]`).
    Types,
    /// Round-trip check: dump each function's bytecode, re-assemble it
    /// from that text, and compare to the original byte stream.  Proves
    /// the labelled disassembly is a faithful, editable representation.
    Roundtrip,
    /// @PLN103 — per-binding store-ownership: each variable's resolved
    /// `Owned` / `Borrowed(base)` / `Join(base)` verdict (via
    /// `use_analysis::ownership_of`), with owned backing buffers rendered
    /// as `Owned (backing=…)`.  Surfaces the borrowed-vs-owned fact behind
    /// the store-lifetime bug corpus.  Opt-in (`--show-ownership`).
    Ownership,
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
    pub types_out: Option<String>,
    /// When set, capture all (non-redirected) section output to a
    /// buffer and `diff -u` it against this baseline file.  Useful
    /// for "did my parser tweak change anything?" — capture once
    /// before the change, then run with `--diff <baseline>` after.
    /// Exits 0 if identical, 1 if diff.  Requires `diff` on PATH.
    pub diff_against: Option<String>,
    /// `--show-types --trace` companion: per-expression type tape
    /// recorded by the parser.  Emitted after each variable table.
    /// Lines are tab-separated `<fn>\t<line>:<col>\t<type>`.
    pub trace_lines: Vec<String>,
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
            types_out: None,
            diff_against: None,
            trace_lines: Vec::new(),
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
///
/// # Panics
/// Panics if the default stdlib directory cannot be parsed (mirrors
/// `loft <file>`'s `unwrap()` on `parse_dir(default/)`).
#[allow(dead_code)]
pub fn run(filename: &str, opts: &Options) -> std::io::Result<()> {
    let abs_file = std::path::Path::new(filename)
        .canonicalize()
        .unwrap_or_else(|_| std::path::PathBuf::from(filename))
        .to_string_lossy()
        .into_owned();
    let mut p = Parser::new();
    p.lib_dirs.clone_from(&opts.lib_dirs);
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
/// When `opts.diff_against` is set, sections without an explicit
/// `*_out` file go into an in-memory buffer that is then `diff -u`'d
/// against the baseline.  Sections with an explicit `*_out` still
/// write to their files (the diff only covers stdout-bound output).
///
/// # Errors
/// Propagates I/O errors from any section's writer.  Also returns an
/// error if `--diff` is requested but `diff` is missing from PATH.
pub fn emit_all(
    data: &mut Data,
    state: &mut State,
    end_def: u32,
    opts: &Options,
) -> std::io::Result<()> {
    let stdout = std::io::stdout();
    // When diffing, accumulate all stdout-bound output into one
    // buffer so we can `diff -u baseline buffer` afterwards.
    // Per-section `*_out` paths are honoured independently.
    let diff_mode = opts.diff_against.is_some();
    let mut buffer: Vec<u8> = Vec::new();
    if opts.includes(Section::Bytecode) {
        if let Some(path) = opts.bytecode_out.as_deref() {
            let mut writer = BufWriter::new(File::create(path)?);
            emit_bytecode(&mut writer, state, data, opts)?;
        } else if diff_mode {
            writeln!(buffer, "=== bytecode ===")?;
            emit_bytecode(&mut buffer, state, data, opts)?;
        } else {
            let mut writer = stdout.lock();
            writeln!(writer, "=== bytecode ===")?;
            emit_bytecode(&mut writer, state, data, opts)?;
        }
    }
    if opts.includes(Section::Rust) {
        if let Some(path) = opts.rust_out.as_deref() {
            let mut writer = BufWriter::new(File::create(path)?);
            emit_rust(&mut writer, data, &state.database, end_def)?;
        } else if diff_mode {
            writeln!(buffer)?;
            writeln!(buffer, "=== rust ===")?;
            emit_rust(&mut buffer, data, &state.database, end_def)?;
        } else {
            let mut writer = stdout.lock();
            writeln!(writer)?;
            writeln!(writer, "=== rust ===")?;
            emit_rust(&mut writer, data, &state.database, end_def)?;
        }
    }
    if opts.includes(Section::Slots) {
        if let Some(path) = opts.slots_out.as_deref() {
            let mut writer = BufWriter::new(File::create(path)?);
            emit_slots(&mut writer, data, end_def, opts)?;
        } else if diff_mode {
            writeln!(buffer)?;
            writeln!(buffer, "=== slots ===")?;
            emit_slots(&mut buffer, data, end_def, opts)?;
        } else {
            let mut writer = stdout.lock();
            writeln!(writer)?;
            writeln!(writer, "=== slots ===")?;
            emit_slots(&mut writer, data, end_def, opts)?;
        }
    }
    if opts.includes(Section::Types) {
        if let Some(path) = opts.types_out.as_deref() {
            let mut writer = BufWriter::new(File::create(path)?);
            emit_types(&mut writer, data, end_def, opts)?;
        } else if diff_mode {
            writeln!(buffer)?;
            writeln!(buffer, "=== types ===")?;
            emit_types(&mut buffer, data, end_def, opts)?;
        } else {
            let mut writer = stdout.lock();
            writeln!(writer)?;
            writeln!(writer, "=== types ===")?;
            emit_types(&mut writer, data, end_def, opts)?;
        }
    }
    // @PLN103 — opt-in only (`--show-ownership`), NOT part of the no-flags
    // "all sections" default, so a plain `introspect <file>` is unchanged.
    if opts.sections.contains(&Section::Ownership) {
        if diff_mode {
            writeln!(buffer)?;
            writeln!(buffer, "=== ownership ===")?;
            emit_ownership(&mut buffer, data, end_def, opts)?;
        } else {
            let mut writer = stdout.lock();
            writeln!(writer)?;
            writeln!(writer, "=== ownership ===")?;
            emit_ownership(&mut writer, data, end_def, opts)?;
        }
    }
    // Opt-in only (a verification check, not a dump) — NOT part of the
    // no-flags "all sections" default, so it never pollutes a plain dump.
    if opts.sections.contains(&Section::Roundtrip) {
        let mut writer = stdout.lock();
        writeln!(writer)?;
        writeln!(writer, "=== roundtrip ===")?;
        emit_roundtrip(&mut writer, state, data, opts)?;
    }
    if let Some(baseline) = &opts.diff_against {
        run_diff_against_baseline(baseline, &buffer)?;
    }
    Ok(())
}

/// Dump each user function's bytecode, re-assemble it from that text via
/// [`compile::reassemble_function`], and compare to the original byte
/// stream.  Reports `ok` / `DIFFERS` / `error` per function plus a tally.
fn emit_roundtrip(
    w: &mut dyn Write,
    state: &mut State,
    data: &mut Data,
    opts: &Options,
) -> std::io::Result<()> {
    let (mut ok, mut bad) = (0u32, 0u32);
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        if !matches!(def.def_type, DefType::Function | DefType::Dynamic)
            || def.is_operator()
            || def.code_length == 0
        {
            continue;
        }
        let from_default =
            def.position.file.starts_with("default/") || def.position.file.starts_with("default\\");
        let pass_filter =
            opts.fn_filter.is_empty() || opts.fn_filter.iter().any(|f| def.name.contains(f));
        if (from_default && !opts.all_fns) || !pass_filter {
            continue;
        }
        let name = def.name.clone();
        let start = def.code_position as usize;
        let len = def.code_length as usize;
        let original = state.bytecode[start..start + len].to_vec();

        let mut buf: Vec<u8> = Vec::new();
        if let Err(e) = state.dump_code(&mut buf, d_nr, data, true) {
            writeln!(w, "  {name}: dump error: {e}")?;
            bad += 1;
            continue;
        }
        let dump = String::from_utf8_lossy(&buf);
        match compile::reassemble_function(&dump, data, &state.library_names) {
            Ok(rebuilt) if rebuilt == original => {
                writeln!(w, "  ok      {name}  ({len} bytes)")?;
                ok += 1;
            }
            Ok(rebuilt) => {
                let at = original
                    .iter()
                    .zip(&rebuilt)
                    .position(|(a, b)| a != b)
                    .unwrap_or(original.len().min(rebuilt.len()));
                writeln!(
                    w,
                    "  DIFFERS {name}  (orig {} B, rebuilt {} B, first diff at byte {at})",
                    original.len(),
                    rebuilt.len()
                )?;
                bad += 1;
            }
            Err(e) => {
                writeln!(w, "  error   {name}: {e}")?;
                bad += 1;
            }
        }
    }
    writeln!(w, "  ── {ok} identical, {bad} differing/error ──")?;
    Ok(())
}

/// Write `buffer` (the captured stdout-bound introspection output)
/// to a temp file, then exec `diff -u <baseline> <tmp>` so the user
/// sees a familiar unified-diff output.  Exits the process with
/// `diff`'s status — 0 for "no difference", 1 for "differs", 2 for
/// "trouble".  Requires `diff` on PATH; falls back to a "use system
/// diff yourself" message if unavailable.
fn run_diff_against_baseline(baseline: &str, buffer: &[u8]) -> std::io::Result<()> {
    if !std::path::Path::new(baseline).exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("baseline file '{baseline}' not found"),
        ));
    }
    let tmp = std::env::temp_dir().join(format!("loft_introspect_diff_{}.txt", std::process::id()));
    std::fs::write(&tmp, buffer)?;
    let status = std::process::Command::new("diff")
        .arg("-u")
        .arg(baseline)
        .arg(&tmp)
        .status();
    let _ = std::fs::remove_file(&tmp);
    if let Ok(s) = status {
        // 0 = identical, 1 = differs.  Both are valid outcomes;
        // mirror diff's exit code.
        std::process::exit(s.code().unwrap_or(2));
    } else {
        eprintln!(
            "loft: --diff requires `diff` on PATH; \
             fall back to redirecting --introspect output and diffing manually."
        );
        std::process::exit(2);
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
    } else if !opts.all_fns {
        // Match the slots/types sections: restrict to user functions.
        // show_code already skips `default/`, but compiler-synthesized
        // runtime helpers (`i_parse_*`) carry an empty position and a
        // non-`n_` name, so they'd otherwise leak in.  List the user
        // `n_` functions explicitly (show_functions is a substring
        // include-list); empty = no user functions = empty section.
        let user_fns: Vec<String> = (0..data.definitions())
            .filter_map(|d| {
                let def = data.def(d);
                (def.def_type == DefType::Function
                    && def.name.starts_with("n_")
                    && !def.name.starts_with("n___lambda_")
                    && !crate::compile::is_default_file(&def.position.file))
                .then(|| def.name.clone())
            })
            .collect();
        config.show_functions = Some(user_fns);
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
    let mut out = generation::Output::new(data, stores);
    out.output_native(w, 0, end_def)
}

fn emit_slots(w: &mut dyn Write, data: &Data, end_def: u32, opts: &Options) -> std::io::Result<()> {
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
        if !opts.all_fns && crate::compile::is_default_file(&def.position.file) {
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
        if def.variables.count() == 0 {
            continue;
        }
        writeln!(w, "fn {}:", def.name)?;
        variables::dump_variables(w, &def.variables, data)
            .map_err(|e| std::io::Error::other(format!("dump_variables: {e}")))?;
        writeln!(w)?;
    }
    Ok(())
}

/// Per-function variable types with dependency tracking.
///
/// Each variable's full `Type` is rendered via `Type::show(data, vars)`,
/// which includes the `[dep_var, …]` suffix for types that carry
/// lifetime dependencies (`Text`, `Reference`, `Vector`, `Hash`,
/// etc.).  Designed to surface dep-propagation bugs at a glance —
/// e.g. P197 showed `s: text` (no deps) for a tuple-element text
/// read that should have inherited the host's `[a]` dependency.
/// @PLN103 P1 — per-binding store ownership.
///
/// For each user function, resolve every variable's ownership via
/// `use_analysis::ownership_of` over the COMMITTED `def.code` (P0.1: the borrowed
/// SOURCE binding survives synthesis, so no pre-synthesis snapshot is needed), and
/// render it with the P0.2 rule (`render_own`): an owned backing buffer reads
/// `Owned (backing=…)`, a genuine alias reads `Borrowed(base=…)`, a runtime split
/// reads `Join(base=…)`.  The function header shows `return_ownership`.
fn emit_ownership(
    w: &mut dyn Write,
    data: &Data,
    end_def: u32,
    opts: &Options,
) -> std::io::Result<()> {
    // @PLN103 P2.0 — ownership is BACKEND-SHARED: the interp bytecode and native Rust
    // lower this SAME verdict (both read `use_analysis::ownership_of`), so there is no
    // per-backend column. A runtime interp≠native value-identity split is a codegen bug,
    // caught by the differential value oracle / leak+ASan gates (and P3's per-backend timeline).
    writeln!(
        w,
        "# store ownership is backend-shared (interp + native lower the same verdict)"
    )?;
    for d_nr in 0..end_def {
        let def = data.def(d_nr);
        if def.def_type != DefType::Function {
            continue;
        }
        if !def.name.starts_with("n_") || def.name.starts_with("n___lambda_") {
            continue;
        }
        if !opts.all_fns && crate::compile::is_default_file(&def.position.file) {
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
        if def.variables.count() == 0 {
            continue;
        }
        let vars = &def.variables;
        let ret = crate::use_analysis::return_ownership(data, d_nr);
        writeln!(
            w,
            "fn {} -> {}:",
            def.name,
            crate::use_analysis::fmt_own(ret, vars)
        )?;
        writeln!(w, "  {:<4} {:<4} {:<22} ownership", "#", "arg", "name")?;
        writeln!(w, "  {}", "-".repeat(66))?;
        for v in 0..vars.count() {
            let arg = if vars.is_argument(v) { "arg" } else { "" };
            // Only heap-backed vars have store ownership; a scalar is by-value.
            let rendered = if vars.tp(v).heap_dep().is_some() {
                let own = crate::use_analysis::ownership_of(data, d_nr, &Value::Var(v));
                crate::use_analysis::render_own(own, vars, v)
            } else {
                "—  (scalar)".to_string()
            };
            writeln!(w, "  {v:<4} {arg:<4} {:<22} {rendered}", vars.name(v))?;
        }
        // @PLN103 P1.5 — delivery lens: the delivery OUTCOME for a heap (vector/reference)
        // return, read from `def.returned`'s deps on the COMMITTED IR — `["__retbuf"]` (a
        // synth buffer) = MATERIALISED into the return buffer, `[arg]` = a borrowed VIEW
        // returned, empty = OWNED fresh store. (Read from the return type, so it is robust;
        // a per-arm walk of the delivered IR was tried and dropped — post-synthesis the arm
        // structure is rewritten, so per-arm `ownership_of` is misleading. The per-arm
        // delivery target needs the parser to PERSIST its `Delivery` verdict — a later step.)
        if let Some(deps) = def.returned.heap_dep() {
            let note = if deps.is_empty() {
                "owned (fresh store)".to_string()
            } else if deps
                .iter()
                .any(|&d| d != u16::MAX && crate::use_analysis::is_synth_buffer(vars.name(d)))
            {
                "materialised → return buffer".to_string()
            } else {
                let names: Vec<&str> = deps
                    .iter()
                    .map(|&d| if d == u16::MAX { "?" } else { vars.name(d) })
                    .collect();
                format!("borrows {} (view returned)", names.join(", "))
            };
            writeln!(w, "  delivery: {}  — {note}", def.returned.show(data, vars))?;
        }
        writeln!(w)?;
    }
    Ok(())
}

fn emit_types(w: &mut dyn Write, data: &Data, end_def: u32, opts: &Options) -> std::io::Result<()> {
    for d_nr in 0..end_def {
        let def = data.def(d_nr);
        if def.def_type != DefType::Function {
            continue;
        }
        if !def.name.starts_with("n_") || def.name.starts_with("n___lambda_") {
            continue;
        }
        if !opts.all_fns && crate::compile::is_default_file(&def.position.file) {
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
        // Skip empty native-only fns (no user variables), which clutter
        // the output without adding signal.
        if def.variables.count() == 0 {
            continue;
        }
        let ret_str = def.returned.show(data, &def.variables);
        writeln!(w, "fn {} -> {ret_str}:", def.name)?;
        writeln!(w, "  {:<4} {:<4} {:<24} type [deps]", "#", "arg", "name")?;
        writeln!(w, "  {}", "-".repeat(70))?;
        for idx in 0..def.variables.count() {
            let arg_flag = if def.variables.is_argument(idx) {
                "arg"
            } else {
                ""
            };
            let var_name = def.variables.name(idx).to_string();
            let type_str = def.variables.tp(idx).show(data, &def.variables);
            writeln!(w, "  {idx:<4} {arg_flag:<4} {var_name:<24} {type_str}")?;
        }
        // Per-expression trace from the parser, if any was recorded.
        // Each line is `<fn_name>\t<line>:<col>\t<type>` where
        // fn_name is the raw user name (`first`), not the
        // `n_`-prefixed def name (`n_first`).
        let user_name = def.name.strip_prefix("n_").unwrap_or(&def.name);
        let fn_trace: Vec<&String> = opts
            .trace_lines
            .iter()
            .filter(|l| l.split('\t').next().is_some_and(|n| n == user_name))
            .collect();
        if !fn_trace.is_empty() {
            writeln!(w)?;
            writeln!(w, "  trace (per-expression types):")?;
            for line in fn_trace {
                let parts: Vec<&str> = line.splitn(3, '\t').collect();
                if let [_fn, pos, ty] = parts.as_slice() {
                    writeln!(w, "    {pos:<10} {ty}")?;
                }
            }
        }
        writeln!(w)?;
    }
    Ok(())
}
