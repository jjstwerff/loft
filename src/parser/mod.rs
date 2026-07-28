// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I58 — Parser (two-pass recursive descent to IR)

//! Parse scripts and create internal code from it.
//! Including type checking.

use crate::data::{
    Argument, Context, Data, DefType, Deps, I32, IntegerSpec, Type, Value, to_default, v_block,
    v_if, v_loop, v_set,
};
use crate::database::{Parts, Stores};
use crate::diagnostics::{DiagEntry, Diagnostics, Level, diagnostic_format};
use crate::lexer::{LexItem, LexResult, Lexer, Link, Mode, Position};
use crate::platform::{other_sep, sep, sep_str};
use crate::variables::{Function, size as var_size};
use crate::{manifest, scopes, typedef};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::env;
use std::fs::{File, metadata, read_dir};
use std::io::Write;
use std::string::ToString;
use typedef::complete_definition;

/// The "you probably meant `pkg::name`" message for an unresolved bare call, or
/// `None` when no published package exports such a free function (@PLN13 phase 6,
/// diagnostics slice).
///
/// Bare calls into a library still do not RESOLVE — that is the rest of phase 6,
/// and it has to settle stdlib shadowing first (a bare `find(…)` already binds to
/// the stdlib `find`, silently). What this removes is the dead end: the name was
/// right, it just was not imported, and the message now says where it lives.
#[cfg(feature = "registry")]
fn registry_fn_hint(name: &str) -> Option<String> {
    let pkgs = crate::registry_index::packages_exporting_fn(name);
    let first = pkgs.first()?;
    let provider = if pkgs.len() == 1 {
        format!("the `{first}` package provides it")
    } else {
        let names: Vec<String> = pkgs.iter().map(|p| format!("`{p}`")).collect();
        format!("the {} packages provide it", names.join(" / "))
    };
    Some(format!(
        "Unknown function {name} — {provider}; call `{first}::{name}(…)`, or add `use {first};` and call it bare"
    ))
}

/// Registry-less build: no index to consult, so no hint (@PLN13 phase 6).
#[cfg(not(feature = "registry"))]
fn registry_fn_hint(_name: &str) -> Option<String> {
    None
}

/// @PLN102 case B (soften-nullflow-discharge.md) — the sign / lower-bound lattice used to
/// prove a domain-fault op's argument is in its safe domain (`sqrt` needs `≥ 0`, `ln` needs
/// `> 0`). `Pos ⊑ NonNeg ⊑ Unknown` (stronger → weaker); `Unknown` is the conservative default.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Sign {
    Pos,
    NonNeg,
    Unknown,
}

/**
The number of defined reserved text worker variables. A worker variable is needed when
two texts are added or a formatting text is used, and the result is used as a parameter to a call.
These are reused when possible. However, when calculating a text, a new text expression
is used a next worker variable is needed.
This number indicated the depth of these expressions, not the number of these expressions in a
function.
*/
// The parser holds several independent boolean mode flags (in_loop, default, first_pass,
// reverse_iterator) that each track a distinct parse phase or context.  Combining them into
// an enum or state machine would add complexity without benefit.
/// Whether a `use lib::...` statement imports all names or a specific subset.
#[derive(Clone)]
enum ImportSpec {
    Wildcard,
    /// Each entry is `(name_in_library, bind_name)`; `bind_name == name` for a
    /// plain `use lib::name`, or the alias for `use lib::name as bind` (@PLN22
    /// Phase 3).
    Names(Vec<(String, String)>),
}

/// A pending import queued when `use lib::spec` is parsed.
/// Applied after all definitions in `for_source` are fully parsed.
#[derive(Clone)]
struct PendingImport {
    for_source: u16,
    lib_source: u16,
    spec: ImportSpec,
}

/// Pure-resolution result from [`Parser::lib_path_manifest_resolve`].
/// Callers decide when to apply side effects (native-lib registration,
/// dependency queueing, etc.).  The legacy `lib_path_manifest` adapter
/// applies them immediately; Phase A of the package-mode driver
/// consults the manifest to build its package graph, then defers
/// side-effect application until after pass-1 parsing.
struct ResolvedPkg {
    pkg_dir: String,
    entry: String,
    /// `None` when the package directory exists but has no `loft.toml`
    /// (pure multi-file package without a manifest).
    manifest: Option<manifest::Manifest>,
}

#[allow(clippy::struct_excessive_bools)]
pub struct Parser {
    pub todo_files: Vec<(String, u16)>,
    /// @PLN11 arc E — set by the driver (`main.rs`) only when the whole-program
    /// startup cache is enabled; gates [`Parser::parsed_sources`] tracking so a
    /// normal (non-cache) run pays nothing.
    pub track_sources: bool,
    /// @PLN11 arc E — paths of every source file parsed (stdlib + lazily-loaded
    /// libs + user file), in load order, recorded only when `track_sources`.
    /// Only the parser sees the dynamically-loaded lib set; the whole-program
    /// cache hashes these files' contents to key the bundle + detect drift.
    /// Paths only (content is re-read once at save time — no per-file memory);
    /// may contain duplicates across the two parse passes, deduped at use.
    pub parsed_sources: Vec<String>,
    /// All definitions
    pub data: Data,
    pub database: Stores,
    /// The lexer on the current text file
    pub lexer: Lexer,
    /// Are we currently allowing break/continue statements?
    in_loop: bool,
    /// True while parsing an expression inside a format string `{…}`.
    /// Prevents the `v: type = expr` annotation from consuming `:`.
    pub(crate) in_format_expr: bool,
    /// @PLN86 — the host-supplied sandbox policy (profiles + designations).
    /// Empty by default; set by the embedder before parsing.  A script cannot
    /// designate itself — the designation is read from here, not the source.
    pub(crate) sandbox: crate::sandbox::SandboxConfig,
    /// @PLN86 step 1.2 — the sandbox profile NAME each designated function is
    /// parsed under (`def_nr -> profile`), recorded as the function is parsed.
    /// The compile-time admission walk reads this to know which defs are
    /// restricted.  Side-map (not on `Definition`) so it stays out of the IR
    /// serialization — the tag is policy, re-derivable from `sandbox`.
    pub(crate) def_sandbox: HashMap<u32, String>,
    /// @PLN86 step 3.1 — sandboxed defs that contain an unbounded `while` loop,
    /// mapped to one such loop's position.  Recorded at parse (`parse_while`
    /// uniquely knows it is a `while`, where the IR cannot reliably tell a `while`
    /// `Loop` from a bounded comprehension `Loop`); the totality admission reads
    /// it.  Keyed by def_nr so the two parse passes are idempotent.
    pub(crate) sandbox_unbounded_loops: HashMap<u32, crate::lexer::Position>,
    /// @PLN86 step 2.4 — sandboxed defs that perform a RAW WRITE to heap data
    /// (`x.field = v` / `v[i] = v`), mapped to one such write's position.
    /// Recorded at parse (`parse_assign` knows the LHS is a field/index target,
    /// not a bare local or a struct literal); the no-raw-write admission reads it.
    /// A sandboxed script may mutate host data only via allow-listed `*.write`
    /// ops, never a raw field/element assignment.  Keyed by def_nr (idempotent).
    pub(crate) sandbox_raw_writes: HashMap<u32, crate::lexer::Position>,
    /// @PLN110 3a — locals currently holding a `len(X)` result (`n = len(s)`), so
    /// `for i in 0..n` carries the same strict-index bound as the inline
    /// `for i in 0..len(s)`.  The two forms are the same units error, and the
    /// bound-to-a-local one is what the published `cbor` encoder shipped, so a lint
    /// that saw only the inline form would have missed the real bug.  Any other
    /// assignment to the local DROPS its entry, so an unclear binding yields a miss
    /// rather than a false warning.  Cleared per function.
    pub(crate) len_bound_locals: HashMap<u16, crate::parser::operators::VecKey>,
    /// @PLN86 L4 — the functions a sandboxed def references as a fn-ref VALUE
    /// (`apply(read_file)`, `let h = read_file`, `[read_file]`, a returned
    /// fn-ref), mapped `def_nr -> {referenced fn def_nrs}`.  Recorded at the
    /// fn-ref CREATION site (where a function name becomes a value), so it catches
    /// every flow — call-arg, assignment, struct field, collection element, return
    /// — completely, where a post-parse IR walk could not (a bare `Int(d_nr)`
    /// fn-ref is indistinguishable from an integer literal).  The admission unions
    /// these into the checked set so an indirect call can't escape (L4).
    pub(crate) sandbox_fn_refs: HashMap<u32, std::collections::HashSet<u32>>,
    /// @PLN86 P6.1 — the dotted names of every `capability` group declared in the
    /// program (stdlib + host code).  A `group#right` capability link resolves
    /// against this set; an undeclared/mistyped group is a load error.  The dotted
    /// name IS the namespace (matched hierarchically on the grant side, like the
    /// `fs.read` groups).  Parser-side (off the IR), idempotent across passes; IR
    /// persistence for a warm-cached stdlib is P6.8.
    pub(crate) declared_capabilities: HashSet<String>,
    /// @PLN86 P6.4 — capability links on struct fields, keyed by `(struct def_nr,
    /// field name)` → the `group#right` tokens written after the field's type
    /// (`health: int stats#read stats#update`).  The admission walk gates a
    /// sandboxed read/update/append of a host field on the matching grant.
    /// Parser-side (off the IR), recorded first-pass; IR persistence is P6.8.
    pub(crate) member_access: HashMap<(u32, String), Vec<String>>,
    /// @PLN86 P6.4 (F4) — sandboxed READS of a host field that carries a `#read`
    /// capability link, keyed by the reading sandboxed def → the `(read token,
    /// position)` of each such read.  Recorded second-pass at the field-access site
    /// (where the struct type + field name resolve); admission rejects a read whose
    /// token the profile does not grant.  Reads are default-allow, so only a field
    /// the host marked with a `#read` link is ever recorded here.
    pub(crate) sandbox_field_reads: HashMap<u32, Vec<(String, crate::lexer::Position)>>,
    /// @PLN86 P6.4 (F5) — sandboxed UPDATES (raw writes) of a host field that carries an
    /// `#update` capability link, keyed by the writing def → each `(struct def_nr, field,
    /// position)`.  A field write WITH an update link is diverted here (admission admits
    /// iff the token is granted); a write to a field with NO update link stays the coarse
    /// 2.4 `sandbox_raw_writes` reject (read-only by default).
    pub(crate) sandbox_field_updates: HashMap<u32, Vec<(u32, String, crate::lexer::Position)>>,
    /// @PLN86 P6.4 (F6) — sandboxed APPENDS (`e.f += x`, growing a collection field)
    /// of a host field that carries an `#append` link, keyed by the writing def → each
    /// `(struct def_nr, field, position)`.  A `+=` to an `#append`-linked field is
    /// diverted here (admission admits iff the token is granted); without an append
    /// link it falls back to the `#update` (F5) or coarse (2.4) path.
    pub(crate) sandbox_field_appends: HashMap<u32, Vec<(u32, String, crate::lexer::Position)>>,
    /// @PLN86 P6.4 (F5) — transient one-shot: the `(struct def_nr, field, #read-links
    /// recorded)` of the field access `field()` last built, so the assignment site can
    /// resolve which field a raw write targets (and un-record the spurious F4 read it
    /// just logged for a write LHS).  Set per field access in a sandboxed def; consumed
    /// at the write site.
    pub(crate) last_field_target: Option<(u32, String, usize)>,
    /// @PLN86 §7.2 (F7) — parameter `…#default` locks, keyed by `(fn def_nr, param
    /// index)` → the lock token written after the parameter's default (`count: int = 1
    /// spawn.count#default`).  At a sandboxed call site, an argument that OVERRIDES a
    /// locked parameter (differs from its default) is gated on the token.  Parser-side
    /// (off the IR), recorded first-pass; sandboxed programs always parse fresh.
    pub(crate) param_locks: HashMap<(u32, u32), String>,
    /// @PLN86 §7.2 (F7) — sandboxed OVERRIDES of a `…#default`-locked parameter, keyed by
    /// the calling def → each `(lock token, position)` of an argument that differed from
    /// the locked parameter's default.  Recorded second-pass at the call site (where the
    /// callee + its argument values resolve); admission rejects an override whose lock
    /// token the profile does not grant.
    pub(crate) sandbox_param_overrides: HashMap<u32, Vec<(String, crate::lexer::Position)>>,
    /// @PLN86 §7.2 (F7) — transient: the `(param index, lock token)` pairs parsed while
    /// reading a function's parameter list, ferried to `parse_function` to be recorded in
    /// `param_locks` once the function's def_nr is known (parameters parse BEFORE the def
    /// is created).  Cleared at the start of each parameter list; consumed per function.
    pub(crate) pending_param_locks: Vec<(usize, String)>,
    /// @PLN115 tail — a parameter's `(arg_index, name_pos, name_len)` captured while
    /// reading the signature (positions there, but the def_nr / var_nr are not yet
    /// established), ferried to `parse_function` to record each param's DECLARATION
    /// occurrence once `self.context` is known.  Populated only when recording;
    /// cleared per parameter list, like `pending_param_locks`.
    pub(crate) pending_param_positions: Vec<(u16, Position, u16)>,
    /// @PLN87 — transient one-shot: set while parsing a `&<lvalue>` binding (the prefix
    /// `&` in `b = &a`, or the `&` in a `b: &T = a` type annotation), consumed by
    /// `parse_assign_op` to lower a SCALAR reference to `OpCreateStack`. Cleared per
    /// binding so it never leaks into the next statement.
    pub(crate) amp_pending: bool,
    /// @PLN86 step 0.1 — true while parsing the BODY of a sandboxed def.  Gates
    /// the parser nesting guard so it never touches trusted code (zero cost
    /// there); set per-def in `parse_function`, cleared at its end.
    pub(crate) in_sandbox: bool,
    /// @PLN86 step 0.1 — current expression-nesting depth, counted ONLY while
    /// `in_sandbox`.  Recursive-descent over hostile deep nesting (`((((…))))`)
    /// overflows the native stack (rc=139); past `SANDBOX_MAX_PARSE_DEPTH` the
    /// parser rejects with a clean diagnostic instead — a LOAD-time rejection,
    /// never a runtime abort.  Reset to 0 at each sandboxed def's body.
    pub(crate) parse_depth: u32,
    /// @PLN86 step 0.1 — latched once the depth limit trips, so the diagnostic
    /// is emitted once per def rather than at every frame as the parser unwinds.
    pub(crate) depth_overflowed: bool,
    /// The current file number that is being parsed
    file: u32,
    pub diagnostics: Diagnostics,
    default: bool,
    /// The definition that is currently parsed (function or struct)
    context: u32,
    /// Extra library directories for 'use' resolution (from --lib / --project flags)
    pub lib_dirs: Vec<String>,
    /// Resolved paths of native shared libraries to load after `byte_code()`.
    /// Populated during `use` processing when a package manifest contains `native`.
    pub pending_native_libs: Vec<String>,
    /// The `(stem, pkg_dir)` registration inputs behind `pending_native_libs`.
    /// Persisted in the startup-cache manifest so a warm load can re-resolve
    /// the cdylibs with cold-equal freshness semantics (#310 — the resolved
    /// PATHS alone can go stale when loft itself is rebuilt).
    pub native_lib_regs: Vec<(String, String)>,
    /// @PLN11 Arc N / N3 — package dirs of libraries that opted into
    /// auto-compilation (`[library] compile = "native"`).  Recorded during `use`
    /// processing; the driver (`main.rs`) marks each library's public
    /// shared-store-dispatchable functions native (after `scopes::check`, before
    /// `byte_code`) and builds + loads the cdylib (after `byte_code`).  A library's
    /// functions are identified by `def.position().file.starts_with(pkg_dir)`.
    pub pending_native_compile: Vec<String>,
    /// PKG.3: package dependencies discovered during manifest reading.
    /// Each entry is (name, dir) — sibling packages are searched in `dir`.
    pending_pkg_deps: Vec<(String, String)>,
    /// Auto-`use` per-file scan cache: a file's `(lib:: refs, .method() calls)`
    /// are deterministic, so it is read + scanned at most once (keyed by path)
    /// and reused across the second pass and any `todo_files` re-parse.
    auto_use_scan_cache: std::collections::HashMap<String, (Vec<String>, Vec<String>)>,
    /// Per-directory cache for the dep-shadowing guard in `lib_path`: the
    /// nearest ancestor manifest's package root + its declared dependency
    /// names.  `None` = no manifest above that directory.
    pkg_dep_cache:
        std::collections::HashMap<String, Option<(String, std::collections::HashSet<String>)>>,
    /// Root project's `[dependencies]` version constraints (name → req), read
    /// once from the main script's nearest-ancestor `loft.toml` (via
    /// `source_dir`).  Pins source-level auto-installs across the WHOLE tree —
    /// direct AND transitive — so a consumer can pin any package it pulls (e.g.
    /// `glb = "=0.1.0"`), making exact pinning an available option instead of
    /// always resolving the newest release.  `None` until first looked up.
    #[cfg(feature = "registry")]
    root_dep_pins: Option<std::collections::HashMap<String, String>>,
    /// Tier-1 text-method trigger map: `method name -> providing package`,
    /// derived once per top-level parse from the current package's (and its
    /// trigger-enabled dependencies') declared triggers.  `None` until built.
    auto_use_trigger_map: Option<std::collections::HashMap<String, String>>,
    /// Tier-1 lazy *catalog* fallback: `method name -> providing package`,
    /// derived once from the cached registry `index.json` (`triggers` field).
    /// Consulted only for methods the local `auto_use_trigger_map` did not
    /// resolve — i.e. a package the user has NOT declared as a dependency but
    /// that the registry says provides the method.  `None` until first miss;
    /// cached empty when the catalog is absent or the registry feature is off.
    // Only read by the `#[cfg(feature = "registry")]` `catalog_trigger_map`; with
    // the registry feature off the field is initialised but never consulted.
    #[cfg_attr(not(feature = "registry"), allow(dead_code))]
    auto_use_catalog_map: Option<std::collections::HashMap<String, String>>,
    /// Is this the first pass on parsing:
    /// - Do not assume that all struct / enum types are already parsed.
    /// - Define variables, try to determine their type (can become clear from later code).
    /// - Claim working text variables for expressions that gather text data outside variables.
    /// - Links between memory allocations (text, stores) their type knows the variable numbers.
    /// - Move variables to a lower scope if an expression still links to their content.
    /// - Determine mutations to stores and administer these in arguments.
    ///
    /// The second pass:
    /// - Creates code, assumes that all types are known.
    // The two-pass contract (H5, @PLAN59): pass 1 registers definitions
    // and FINAL signatures — since @PLAN59 every body-carrying plain fn's
    // hidden `__retbuf` exists from its pass-1 signature parse, so attr
    // counts CANNOT change in pass 2 (`ref_return` debug-asserts it; only
    // lambda-class defs may still grow, and they have no earlier callers).
    // Pass 2 re-parses bodies against those fixed signatures.  Variable
    // tables persist across passes BY NAME (pass 2 re-finds rather than
    // re-creates) — the parser is therefore NOT re-entrant beyond two
    // passes: a third pass leaves tables half-migrated (verified by the
    // #339 third-pass experiment, which segfaulted).  The "attr counts cannot
    // change in pass 2" claim above is now ENFORCED (not just asserted at the
    // `ref_return` growth site) by `assert_pass2_def_attr_stable` — a per-def
    // count snapshot compared end-of-pass-1 vs end-of-pass-2 in debug/armed
    // builds, so any future cross-pass divergence fails loud (H5).
    first_pass: bool,
    /// @PLN104 P3 — def_nrs the post-pass-2 oracle (`report_tret_promotions`) marks
    /// for `__tret` retbuf promotion (a frame-local text return with no buffer). Read
    /// by `do_tret_bind`'s gate on the THIRD pass so the promotion is forward-ref-safe:
    /// the attr is decided before the pass, so every caller re-lowers with the buffer.
    force_tret: std::collections::HashSet<u32>,
    /// Set by `parse_in_range` when `rev(collection)` (without a `..` range) is parsed.
    /// Consumed by `fill_iter` to add the reverse bit (64) into the `on` byte of OpIterate/OpStep.
    reverse_iterator: bool,
    /// D-key-1: true only while parsing the *iterable* of a `for`/comprehension (set with
    /// save/restore around the iterable expression in `parse_in_range`).  A keyed range /
    /// partial-key subscript produces a `for`-only iterator (`Value::Iter`); `parse_key`
    /// reads this flag to reject that subscript in a value position (`x = coll[lo..hi]`)
    /// with a clean diagnostic instead of a parse/codegen panic.
    iterable_context: bool,
    /// O8.5: range bounds captured by `parse_in_range_body` for const-unroll detection.
    pub(crate) last_range_from: Option<Value>,
    pub(crate) last_range_till: Option<Value>,
    /// @PLN35 PC1 — set while matching over a CURSOR (a struct with a `vector<T>` source + an
    /// integer `pos`): `(cursor_var, cursor_def, pos_field_idx, pos_var)`.  `pos_var` holds the
    /// current position (reads are offset by it); the match PREFIX-consumes (gate `pos + fixed <=
    /// len`, not `len == fixed`) and advances `cursor.pos` by the consumed count on a match.  `None`
    /// = a plain vector/stream match (whole-consume) — the default, so those paths are unchanged.
    pub(crate) match_cursor: Option<(u16, u32, usize, u16)>,
    /// @PLN35 PC5 — the attribute index of an OPTIONAL `farthest: integer` field on the cursor
    /// struct, if present.  The match maintains it as a monotonic high-water mark (`max(farthest,
    /// new_pos)`) at every advance, so after a failed parse `cursor.farthest` is the PEG
    /// farthest-reached position — the token to point an error at.  `None` = the cursor has no such
    /// field (tracking is opt-in, so a plain `{ src, pos }` cursor is unaffected).
    pub(crate) match_cursor_farthest: Option<usize>,
    /// @PLN35 PC3 — sub-rule invocation edges `(enclosing_rule, invoked_rule, site)` recorded on
    /// pass 2, so a post-parse well-formedness pass can reject a left-recursive grammar (a cycle in
    /// this graph) at compile time rather than hang at runtime.  Every PC2 invocation is at cursor
    /// position 0 (whole-pattern), so any cycle is non-consuming — it cannot terminate.
    pub(crate) subrule_edges: Vec<(u32, u32, Position)>,
    vars: Function,
    /// Last seen line inside the source code, an increase inserts it in the internal code.
    line: u32,
    /// Wildcard and selective imports waiting to be applied once the target source is fully parsed.
    pending_imports: Vec<PendingImport>,
    /// every (for_source, lib_source, ImportSpec) pair that
    /// `apply_pending_imports` applied during this parse pass.  Retained so
    /// that `resolve_deferred_unknowns` can re-apply them with overwrite
    /// semantics after cyclic `use` declarations have left Unknown stubs.
    applied_imports: Vec<PendingImport>,
    /// `DefType::Unknown` stubs collected by `actual_types_deferred`
    /// during each `parse_file` run.  Resolved (or finally reported) by
    /// `resolve_deferred_unknowns` after all files in the recursion have
    /// had their pass-1 / pass-2 definitions registered.
    deferred_unknown: Vec<(u16, u32, Position)>,
    /// @PLN115 — record each resolved identifier occurrence during parse.  DEFAULT
    /// OFF (only the LSP parse sets it, S3); zero-cost when off.  See
    /// `doc/claude/plans/115-resolution-index/`.
    record_resolutions: bool,
    /// @PLN115 — the recorded occurrences (empty unless `record_resolutions`),
    /// cleared per parse alongside `deferred_unknown`.
    resolutions: Vec<crate::resolution::Occurrence>,
    /// Whether the most recently parsed expression is from a `not null` field access.
    /// Set by `get_field`; consumed by `handle_operator` to warn on redundant null checks.
    expr_not_null: bool,
    /// The field name for the most recently parsed `not null` field access (for diagnostics).
    expr_not_null_name: String,
    /// Counter incremented each time a lambda expression is parsed.
    /// Lambda names are `__lambda_N`; the same N is produced on both passes because the counter
    /// advances identically in both passes (same token order → same parse order).
    pub lambda_counter: u32,
    /// The single **expected type** — the `⇐` checking mode (formal `(T-Chk)`) pushed into
    /// the value currently being parsed, where the operand's own `var_tp` does not already
    /// carry it (a call argument, a function return-body tail, an `f#read` RHS).  Set before
    /// the value is parsed and reset to `Type::Unknown(0)` after.  There is ONE channel, not
    /// four: readers dispatch on its SHAPE via the helpers below —
    /// - a `Type::Function` → short-form lambda (`|x| {…}`) parameter inference ([`Self::lambda_hint`]);
    /// - an enum type → a bare value-position variant (`f(Red)`) resolves against it ([`Self::enum_hint`]);
    /// - a `Type::Vector` of concrete narrow elements → a bare literal (`[10,255,20]`) builds at the
    ///   element width (#432, [`Self::vector_hint`]);
    /// - any type → an `f#read` infers its byte width from it ([`Self::read_target_type`]).
    ///
    /// (`var_tp` already carries the type for typed-local decls / `==` / struct-field init,
    /// so those need no push.)  Consolidating the former four `*_hint` fields is
    /// [formal/types.md D1](../../doc/claude/formal/types.md) — one judgment, not four side-channels.
    pub(crate) expected: Type,
    /// Set by `iter_op` when `#fields` is encountered. Holds the struct `def_nr`.
    /// Checked by `parse_for` to take the unrolling path. Reset after use.
    pub(crate) fields_of: u32,
    /// Outer-scope variable names and types, populated when parsing a lambda body.
    /// When a variable is not found in the lambda's scope but exists here, it is a capture.
    /// Empty when not inside a lambda.
    pub(crate) capture_context: Vec<(String, Type)>,
    /// Accumulates captured variable names and types during lambda body parsing.
    /// Reset at the start of each lambda; read after parsing to synthesize the closure record.
    pub(crate) captured_names: Vec<(String, Type)>,
    /// Variable number of the __closure parameter inside a lambda body (second pass).
    /// `u16::MAX` when not inside a capturing lambda.
    pub(crate) closure_param: u16,
    /// @PLN25 E2 — def_nr of the generic type-variable stub (`T`) currently in
    /// scope while parsing a `fn f<T>(…)` signature/body; `u32::MAX` outside a
    /// generic function.  Consulted ONLY by `e2_nullable_elem` so a generic
    /// `vector<T>` is NOT rewritten to `vector<__nullable<T>>` (T is opaque at
    /// definition time — nullability is decided at instantiation by whatever
    /// concrete element type the caller's vector carries).  Reset per function.
    pub(crate) cur_type_var: u32,
    // maps fn-ref variable numbers to their closure record work variable numbers.
    pub(crate) closure_vars: std::collections::HashMap<u16, u16>,
    // last closure work variable created by emit_lambda_code (transient).
    pub(crate) last_closure_work_var: u16,
    // closure allocation expression to inject at the call site.
    pub(crate) last_closure_alloc: Option<Box<Value>>,
    // outer variable numbers captured by the most recently parsed lambda.
    // Consumed by try_fn_ref_call to mark them as read at call-injection time.
    pub(crate) last_closure_captured_vars: Vec<u16>,
    /// #314: capturing lambdas synthesized during each function body in
    /// pass 1, keyed by the enclosing context's def_nr.  Consumed by
    /// `reject_shared_mutable_scalar_captures` at the parent's body end
    /// — the first moment the parent's `scalars_to_box` accumulation is
    /// complete — to diagnose a mutated scalar captured by more than
    /// one closure (GOALS.md § "Stability trumps features").
    pub(crate) fn_lambdas: std::collections::HashMap<u32, Vec<u32>>,
    /// #91: when > 0, record $.<field> accesses for circular-init detection.
    /// Decremented after each init(expr) is parsed.
    pub(crate) init_field_tracking: bool,
    /// #91: field names accessed via $ during the current init(expr) parse.
    pub(crate) init_field_deps: Vec<String>,
    /// M11-a: true while parsing the body of a `for … par(…) { … }` loop.
    /// `yield` inside a `par()` body is illegal — the worker runs in a separate
    /// thread with its own store; there is no safe coroutine resumption path.
    pub(crate) in_par_body: bool,
    /// Field-capture aliases created by `if expr is Variant { field } { body }`.
    /// Drained by `parse_if` after the body to restore previous name mappings.
    pub(crate) is_capture_aliases: Vec<(String, Option<u16>)>,
    /// Post-2c: captures the most recently parsed `as <alias>` cast target's
    /// def_nr when the alias has a `size(N)` annotation.  Consumed by
    /// `append_to_file` so that `f += x as i32` narrows the serialised
    /// payload to the alias's byte width.  Reset to `u32::MAX` at the start
    /// of each top-level statement; irrelevant outside file-I/O `+=`.
    pub(crate) last_cast_alias: u32,
    /// The narrow target `N` of the just-parsed checked narrowing cast
    /// (`e as N?`, or `e as N` immediately left of `??`).  A discharging `??`
    /// reads this so its result types as `N` instead of the full `integer` the
    /// nullable cast has to carry for its null sentinel (see
    /// `build_null_coalesce_default`).  Set in the `as` handler, consumed by the
    /// next `??`, and cleared by any other intervening operator.
    pub(crate) dn4_checked_narrow: Option<Type>,
    /// @PLN116 `x?` — a pre-built default RHS for the postfix default-fallback
    /// operator.  `x?` desugars to `x ?? construct_default(T)`; rather than parse a
    /// `??` right operand from source, the postfix-`?` site builds the type's default
    /// value here and `build_null_coalesce_default` consumes it INSTEAD of parsing —
    /// so `x?` reuses the whole `??` emission path and is bytecode-identical to a
    /// hand-written `x ?? <default>`.  Set at the `?` site, taken exactly once.
    pub(crate) pending_default_rhs: Option<(Value, Type)>,
    /// @PLN116 `x?` — a synthetic default-value SOURCE for the postfix operator, used
    /// when the default must be parsed IN the `??` right-operand context rather than
    /// pre-built (an empty collection `[]`, whose ownership view-model depends on that
    /// context — parsing it standalone leaks).  `build_null_coalesce_default` swaps the
    /// lexer to this source at its own parse site, so `x?` matches `x ?? []` exactly.
    pub(crate) pending_default_src: Option<String>,
    /// @PLN99 Arc C — set by `convert` when it dispatches a struct/reference-returning
    /// USER conversion (`x as T` via `fn OpConvTFromS`).  Such a conversion ALLOCATES a
    /// fresh owned store, so its result must NOT inherit the source's deps (the reinterpret-
    /// cast graft would mark it a view and leak the new store).  The `as` handler reads this
    /// right after `convert` to use the conversion fn's real (Owned) return type as the
    /// result, then clears it.
    pub(crate) conv_owned_result: Option<Type>,
    /// Field-binding Set nodes created by `if expr is Variant { field }`.
    /// Drained by `parse_if` and prepended to the if-body so they only
    /// execute when the discriminant matches.
    pub(crate) is_capture_bindings: Vec<Value>,
    /// `--show-types --trace`: when `true`, `parse_part` appends one
    /// trace line per resolved sub-expression (after each `.field`,
    /// `.tuple_idx`, `[idx]` step).  Surfaces dep-tracking flow that
    /// the per-variable view misses — e.g. P197 was a missing dep on
    /// the `.0` step of a tuple field read.
    pub trace_types: bool,
    /// Accumulated trace entries; drained by the introspection CLI.
    /// Each entry is a tab-separated record:
    /// `<fn_name>\t<line>:<col>\t<step_kind>\t<type_with_deps>`.
    pub trace_types_lines: Vec<String>,
    /// Plan-07 phase 4h — per-(struct_d_nr, attr_idx) read counter.
    /// Incremented by `Parser::field()` on the second pass each time a
    /// field is READ (not assigned) on a user struct.  Surfaced at
    /// end of parse: each non-`not_null` field whose read count >=
    /// `HINT_NOT_NULL_THRESHOLD` AND whose `defended_field_reads` set
    /// does NOT contain the (d_nr, attr_idx) gets a `Level::Warning`
    /// at its declaration suggesting `not null`.  Stdlib (`self.default
    /// == true`) is exempt.  Silenceable via `LOFT_NO_HINT_NOT_NULL=1`.
    pub(crate) field_read_counts: std::collections::HashMap<(u32, u32), u32>,
    /// Plan-07 phase 4h — set of (struct_d_nr, attr_idx) that have at
    /// least one defensive read site (`obj.field ?? default` or `if
    /// obj.field != null` flow analysis).  Membership in this set
    /// suppresses the `not null` hint regardless of read count — the
    /// developer has explicitly acknowledged null is possible.
    pub(crate) defended_field_reads: std::collections::HashSet<(u32, u32)>,
    /// @PLN25 DN3 flow-narrowing — the stack of local-var slots PROVEN non-null by an
    /// enclosing guard (`if v != null { … }` / `if v { … }`, the `== null` else-arm).
    /// A read of `Var(v)` while `v ∈ this` types as the peeled (non-null) base, not `τ?`.
    /// Pushed on entry to the proven branch, truncated to the saved length on exit; a
    /// reassignment of `v` inside the branch removes it (the proof no longer holds).
    pub(crate) narrowed_non_null: Vec<u16>,
    /// @PLN25 DN3 fault-op narrowing — local-var slots PROVEN non-zero by an enclosing
    /// `if v != 0 { … }` guard. A division/mod whose divisor is in this set (or a constant
    /// non-zero literal) is provably fit and types NON-null; otherwise it types `τ?`. Same
    /// push/truncate/invalidate discipline as `narrowed_non_null`.
    pub(crate) divisor_nonzero: Vec<u16>,
    /// @PLN25 DN3 fault-op (index) — set by `parse_vector_index` for each SCALAR `v[i]` read
    /// (true = the index is provably in-bounds: a non-negative constant, a for-loop iter var, or
    /// a var proven `< len(v)` by an enclosing guard), read immediately after by `parse_index` to
    /// decide whether the element type wraps `Optional`. Write-then-read per read; nested reads
    /// (`v[w[j]]`) set inner-then-outer so the outer read sees the outer index's fit.
    pub(crate) last_index_fit: bool,
    /// @PLN25 DN3 fault-op (index) — the `(idx_var, vec)` pairs proven in-bounds by an enclosing
    /// `if idx < len(vec) { … }` guard (skip-pattern 5). A `vec[idx]` whose pair is here types
    /// NON-null. Pushed for the THEN branch in `parse_if`, truncated on exit — same discipline as
    /// `divisor_nonzero`, but keyed on the (index, vector) pair via `VecKey`.
    pub(crate) index_bounded: Vec<(u16, crate::parser::operators::VecKey)>,
    /// Plan-07 phase 4h — site of the most recently parsed field
    /// read.  Set by `Parser::field()` after each read, taken by
    /// `handle_null_coalesce` to mark the read as defended when
    /// `expr ?? default` follows.  `None` between distinct
    /// expressions / statements.  Conservative: covers the common
    /// `p.field ?? default` shape; complex expressions like
    /// `(p.field + 1) ?? 0` and `if p.field != null` are
    /// under-detected today (slice 2).
    pub(crate) last_field_read_site: Option<(u32, u32)>,
    /// @PLAN12 Phase 6.7 — dedupe set for the per-invocation advisory
    /// check.  Once a `(name, version)` tuple has been classified
    /// against the advisory feed during this parser's lifetime, we
    /// don't re-fire the warning.  Critical-severity matches abort
    /// via `process::exit(3)` before the second probe could fire
    /// anyway; this dedupe is for the warning/note tiers.
    #[cfg(feature = "registry")]
    advisory_checked: std::collections::HashSet<(String, String)>,
}

// Operators ordered on their precedence
static OPERATORS: &[&[&str]] = &[
    &["??"],
    &["||", "or"],
    &["&&", "and"],
    &["==", "!=", "<", "<=", ">", ">="],
    &["|"],
    &["^"],
    &["&"],
    &["<<", ">>"],
    &["-", "+"],
    &["*", "/", "%"],
    &["**"],
    &["as"],
];

static SKIP_TOKEN: [&str; 8] = ["}", ".", "<", ">", "^", "+", "-", "#"];
static SKIP_WIDTH: [&str; 10] = ["}", ".", "x", "X", "o", "b", "e", "j", "d", "f"];

pub(crate) struct OutputState<'a> {
    pub(crate) radix: i32,
    pub(crate) width: Value,
    pub(crate) token: &'a str,
    pub(crate) plus: bool,
    pub(crate) note: bool,
    pub(crate) dir: i32,
    pub(crate) float: bool,
    /// @PLN99 Arc B — the raw `{x:spec}` spec string for a custom-type value whose
    /// own `to_text(self, spec)` renders it (`""` for a bare `{x}` or a built-in).
    pub(crate) spec: &'a str,
}

impl OutputState<'_> {
    pub(crate) fn db_format(&self) -> i32 {
        i32::from(self.note) + if self.radix < 0 { 2 } else { 0 }
    }
}

pub(crate) const OUTPUT_DEFAULT: OutputState = OutputState {
    radix: 10,
    width: Value::Int(0),
    token: " ",
    plus: false,
    note: false,
    dir: 2, // 2 = unset; text defaults to left (-1), numbers to right (1)
    float: false,
    spec: "",
};

// Sub-modules
pub(super) mod builtins;
pub(super) mod collections;
pub(super) mod control;
pub(super) mod definitions;
pub(super) mod expressions;
pub(super) mod fields;
pub(super) mod objects;
pub(super) mod operators;
pub(super) mod vectors;

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

fn is_op(name: &str) -> bool {
    name.len() >= 3 && name.starts_with("Op") && name.chars().nth(2).unwrap().is_uppercase()
}

/// Validate function, attribute, value, and field names
fn is_lower(name: &str) -> bool {
    for c in name.chars() {
        if c.is_uppercase() {
            return false;
        }
    }
    true
}

#[allow(dead_code)]
/// Used to validate constant names
fn is_upper(name: &str) -> bool {
    for c in name.chars() {
        if c.is_lowercase() {
            return false;
        }
    }
    true
}

/// Validate type, enum, enum values and struct names
fn is_camel(name: &str) -> bool {
    let c = name.chars().next().unwrap();
    if c.is_lowercase() {
        return false;
    }
    for c in name.chars() {
        if c == '_' {
            return false;
        }
    }
    true
}

/// Outcome of [`Parser::parse_statement`] — the REPL read-eval step's parse half.
#[derive(Debug)]
pub enum ParseResult {
    /// Fully parsed.  Any new top-level definition is registered in `data` and
    /// its body IR is built, ready for codegen + execute.  `entry_def_nr` is the
    /// newest definition's number, or `u32::MAX` when the statement added no
    /// runnable definition.
    Ready { entry_def_nr: u32 },
    /// Input ends mid-construct (open bracket, unterminated string, trailing
    /// operator).  The REPL should read another line and re-call with the
    /// concatenated input.
    NeedMore,
    /// Parse failed.  `data` has been rolled back to its pre-call state; the
    /// entries are the diagnostics this statement produced.
    Error(Vec<DiagEntry>),
}

impl Parser {
    #[must_use]
    pub fn new() -> Self {
        let mut data = Data::new();
        // Register internal-only functions (i_ prefix) that are never visible to user code.
        // These are resolved by the compiler via data.def_nr("i_...") and mapped to native
        // Rust implementations in native.rs.
        let pos = Position {
            file: String::new(),
            line: 0,
            pos: 0,
        };
        let d = data.add_def("i_parse_errors", &pos, DefType::Function);
        data.definitions[d as usize].returned = Type::Text(Deps::none());
        let d = data.add_def("i_parse_error_push", &pos, DefType::Function);
        data.definitions[d as usize].returned = Type::Void;
        {
            let mut lexer = Lexer::default();
            data.add_attribute(&mut lexer, d, "msg", Type::Text(Deps::none()));
        }
        Parser {
            todo_files: Vec::new(),
            track_sources: false,
            parsed_sources: Vec::new(),
            data,
            database: Stores::new(),
            lexer: Lexer::default(),
            in_loop: false,
            in_format_expr: false,
            sandbox: crate::sandbox::SandboxConfig::default(),
            def_sandbox: HashMap::new(),
            sandbox_unbounded_loops: HashMap::new(),
            sandbox_raw_writes: HashMap::new(),
            sandbox_fn_refs: HashMap::new(),
            declared_capabilities: HashSet::new(),
            member_access: HashMap::new(),
            sandbox_field_reads: HashMap::new(),
            sandbox_field_updates: HashMap::new(),
            sandbox_field_appends: HashMap::new(),
            last_field_target: None,
            param_locks: HashMap::new(),
            sandbox_param_overrides: HashMap::new(),
            len_bound_locals: HashMap::new(),
            pending_param_locks: Vec::new(),
            pending_param_positions: Vec::new(),
            amp_pending: false,
            in_sandbox: false,
            parse_depth: 0,
            depth_overflowed: false,
            file: 1,
            diagnostics: Diagnostics::new(),
            default: false,
            context: u32::MAX,
            first_pass: true,
            force_tret: std::collections::HashSet::new(),
            reverse_iterator: false,
            iterable_context: false,
            last_range_from: None,
            last_range_till: None,
            match_cursor: None,
            match_cursor_farthest: None,
            subrule_edges: Vec::new(),
            vars: Function::new("", "none"),
            line: 0,
            lib_dirs: Vec::new(),
            pending_native_libs: Vec::new(),
            native_lib_regs: Vec::new(),
            pending_native_compile: Vec::new(),
            pending_pkg_deps: Vec::new(),
            auto_use_scan_cache: std::collections::HashMap::new(),
            pkg_dep_cache: std::collections::HashMap::new(),
            #[cfg(feature = "registry")]
            root_dep_pins: None,
            auto_use_trigger_map: None,
            auto_use_catalog_map: None,
            pending_imports: Vec::new(),
            applied_imports: Vec::new(),
            deferred_unknown: Vec::new(),
            record_resolutions: false,
            resolutions: Vec::new(),
            expr_not_null: false,
            expr_not_null_name: String::new(),
            lambda_counter: 0,
            expected: Type::Unknown(0),
            fields_of: u32::MAX,
            capture_context: Vec::new(),
            captured_names: Vec::new(),
            fn_lambdas: std::collections::HashMap::new(),
            closure_param: u16::MAX,
            cur_type_var: u32::MAX,
            closure_vars: std::collections::HashMap::new(),
            last_closure_work_var: u16::MAX,
            last_closure_alloc: None,
            last_closure_captured_vars: vec![],
            init_field_tracking: false,
            init_field_deps: Vec::new(),
            in_par_body: false,
            is_capture_aliases: Vec::new(),
            is_capture_bindings: Vec::new(),
            last_cast_alias: u32::MAX,
            dn4_checked_narrow: None,
            pending_default_rhs: None,
            pending_default_src: None,
            conv_owned_result: None,
            trace_types: false,
            trace_types_lines: Vec::new(),
            field_read_counts: std::collections::HashMap::new(),
            defended_field_reads: std::collections::HashSet::new(),
            narrowed_non_null: Vec::new(),
            divisor_nonzero: Vec::new(),
            last_index_fit: false,
            index_bounded: Vec::new(),
            last_field_read_site: None,
            #[cfg(feature = "registry")]
            advisory_checked: std::collections::HashSet::new(),
        }
    }

    /// @PLN86 step 1.2 — install the host's sandbox policy before parsing.  The
    /// designation is host-controlled; a script can never mark itself sandboxed.
    pub fn set_sandbox_config(&mut self, config: crate::sandbox::SandboxConfig) {
        self.sandbox = config;
    }

    /// @PLN86 — does the loaded `[sandbox]` policy designate any sandboxed source?
    /// The CLI uses this to disable the program warm-cache for a sandboxed program:
    /// a warm load restores the IR without re-parsing, so `def_sandbox` would never
    /// form and admission (+ the force-interpret guard) would be silently bypassed.
    #[must_use]
    pub fn sandbox_is_active(&self) -> bool {
        self.sandbox.is_active()
    }

    /// @PLN86 step 1.2 — the sandbox profile a function was parsed under, or
    /// `None` for unrestricted (trusted) code.  The admission walk reads this to
    /// apply the totality / capability rules only to sandboxed defs.
    #[must_use]
    pub fn def_sandbox_profile(&self, def_nr: u32) -> Option<&str> {
        self.def_sandbox.get(&def_nr).map(String::as_str)
    }

    /// @PLN86 step 1.3 — the sandbox-reachable set: every def reachable from the
    /// sandboxed entries via calls + fn-ref literals, descending only into
    /// sandboxed defs (trusted symbols are leaves).  Step 2.3 capability-checks
    /// the non-sandboxed members.  Call after parsing, when bodies are resolved.
    #[must_use]
    pub fn sandbox_reachable_set(&self) -> std::collections::HashSet<u32> {
        crate::sandbox::reachable_set(&self.data, &self.def_sandbox, &self.sandbox_fn_refs)
    }

    /// @PLN86 step 1.4 — external `[native]` cdylib bridges reachable from the
    /// sandboxed entries.  Empty unless a sandboxed def reaches an external FFI
    /// symbol; admission rejects a non-empty result when the profile forbids
    /// native FFI (RCE by construction).  Backend force-interpret + the
    /// no-default-features cdylib removal are the remaining 1.4 work.
    #[must_use]
    pub fn sandbox_ffi_bridges(&self) -> Vec<u32> {
        crate::sandbox::reachable_ffi_bridges(&self.data, &self.sandbox_reachable_set())
    }

    /// @PLN86 — the `group#right` call-gate link a def declares (in its signature),
    /// or `None` if unlinked.  Admission gates each trusted symbol in the reachable
    /// set against the profile via `SandboxConfig::allows`.
    #[must_use]
    pub fn def_cap_group(&self, def_nr: u32) -> Option<&str> {
        let cap = self.data.def(def_nr).cap();
        (!cap.is_empty()).then_some(cap)
    }

    /// @PLN86 step 2.3 — the capability-admission walk: every trusted symbol a
    /// sandboxed def reaches must carry a `group#right` call-gate link its profile
    /// grants (or be `native_ffi`-allowed).  An empty result means the sandboxed code is
    /// admitted; otherwise each `CapViolation` names the offending reference for
    /// a diagnostic.  Run after parsing.
    #[must_use]
    pub fn sandbox_admit(&self) -> Vec<crate::sandbox::CapViolation> {
        crate::sandbox::admit_capabilities(
            &self.data,
            &self.sandbox,
            &self.def_sandbox,
            &self.sandbox_fn_refs,
        )
    }

    /// @PLN86 step 2.2 — the capability-coverage lint: public functions lacking a
    /// `group#right` call-gate link.  The host runs this over the stdlib + libraries to find the
    /// surface still to tag; an empty result is full coverage (L3-cap).
    #[must_use]
    pub fn untagged_public_symbols(&self) -> Vec<u32> {
        crate::sandbox::untagged_public_symbols(&self.data)
    }

    /// @PLN86 — the library a def belongs to (derived from its source), the
    /// wholesale-admission key for a profile's `allow_libs`.  `None` for a
    /// synthetic / sourceless def.
    #[must_use]
    pub fn def_library(&self, def_nr: u32) -> Option<String> {
        crate::sandbox::def_library(&self.data, def_nr)
    }

    /// @PLN86 step P3 — totality violations: the sandboxed script is rejected if
    /// it cannot be proven to terminate (an unbounded `while`, or a recursion
    /// cycle).  Empty for a provably-total script.
    #[must_use]
    pub fn sandbox_totality(&self) -> Vec<crate::sandbox::TotalityViolation> {
        crate::sandbox::admit_totality(
            &self.data,
            &self.def_sandbox,
            &self.sandbox_unbounded_loops,
            &self.sandbox_fn_refs,
        )
    }

    /// @PLN86 step 2.4 — no-raw-write violations: sandboxed defs that directly
    /// mutate heap data (`x.field = v` / `v[i] = v`).  Empty when every mutation
    /// goes through an allow-listed `*.write` op.
    #[must_use]
    pub fn sandbox_raw_writes(&self) -> Vec<crate::sandbox::RawWriteViolation> {
        crate::sandbox::raw_write_violations(&self.def_sandbox, &self.sandbox_raw_writes)
    }

    /// @PLN86 P6.1 — true iff `group` (the part before `#` in a `group#right`
    /// capability link) is covered by a declared `capability`: either declared
    /// exactly, or a sub-namespace of one (a `capability game` declaration covers a
    /// `game.entity#read` link, matching the grant-side namespacing).  A link to an
    /// uncovered group is a load error; admission resolves links against the declared
    /// set once every declaration is registered (parsing complete), so forward +
    /// cross-file references are fine.
    #[must_use]
    pub fn cap_is_declared(&self, group: &str) -> bool {
        self.declared_capabilities
            .iter()
            .any(|d| crate::sandbox::cap_prefix_match(d, group))
    }

    /// @PLN86 P6.8 (F8) — every capability link in the **main program** whose group is
    /// NOT covered by a declared `capability` (a typo'd / forgotten declaration).  Scans
    /// the three link homes: a function call gate (`Definition.cap`), a struct-field link
    /// (`member_access`), and a parameter `#default` lock (`param_locks`).  An uncovered
    /// group can never match a grant, so today it denies SILENTLY — this turns it into a
    /// clean, named load error.  Reported once per distinct group.
    ///
    /// Scoped to `MAIN_SOURCE` — the program the author is iterating on (the §6 modder
    /// feedback loop), which is always parsed fresh so its declarations are present in
    /// the registry.  The stdlib + installed libraries (other sources) are TRUSTED: their
    /// links were validated when authored as a main program, and they may be loaded from
    /// the IR cache (where the parser-side `capability` registry is not restored), so
    /// re-checking them here would falsely reject a clean program.  Registry IR
    /// persistence — needed to widen this to cached library links — is a later step.
    #[must_use]
    pub fn sandbox_undeclared_links(&self) -> Vec<String> {
        let mut undeclared: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        let mut check = |token: &str| {
            if let Some((group, _)) = token.split_once('#')
                && !group.is_empty()
                && !self.cap_is_declared(group)
            {
                undeclared.insert(group.to_string());
            }
        };
        let is_main = |d_nr: u32| {
            (d_nr as usize) < self.data.definitions.len()
                && self.data.definitions[d_nr as usize].source == crate::data::MAIN_SOURCE
        };
        for d in &self.data.definitions {
            if !d.cap.is_empty() && d.source == crate::data::MAIN_SOURCE {
                check(&d.cap);
            }
        }
        for ((struct_nr, _), tokens) in &self.member_access {
            if is_main(*struct_nr) {
                for t in tokens {
                    check(t);
                }
            }
        }
        for ((fn_nr, _), t) in &self.param_locks {
            if is_main(*fn_nr) {
                check(t);
            }
        }
        undeclared
            .into_iter()
            .map(|g| {
                format!(
                    "undeclared capability `{g}` — a `group#right` link references it but no \
                     matching `capability` is declared.\n  fix: add `capability {g}` at top \
                     level, or correct the link."
                )
            })
            .collect()
    }

    /// @PLN86 P6.4 — record a `group#right` capability link on a struct field
    /// (`struct def_nr`, field name).  Called as each field's links are parsed.
    pub(crate) fn record_member_link(&mut self, struct_nr: u32, field: &str, token: String) {
        self.member_access
            .entry((struct_nr, field.to_string()))
            .or_default()
            .push(token);
    }

    /// @PLN86 P6.4 — the `group#right` capability links on a struct field, or empty
    /// if the field carries none (read-allow / no-update / no-append by default).
    #[must_use]
    pub fn member_links(&self, struct_nr: u32, field: &str) -> &[String] {
        self.member_access
            .get(&(struct_nr, field.to_string()))
            .map_or(&[], Vec::as_slice)
    }

    /// @PLN86 step 2.5 — ALL sandbox admission errors (capability + totality +
    /// no-raw-write), each rendered as a correct, specific, actionable message
    /// (position + the rule + the fix).  Empty when the script is admitted.  This
    /// is the host's primary surface — the contract a modder iterates against.
    #[must_use]
    pub fn sandbox_admission_errors(&self) -> Vec<String> {
        let caps = self.sandbox_admit().into_iter().map(|v| {
            crate::sandbox::describe_violation(&self.data, &self.sandbox, &self.def_sandbox, &v)
        });
        let totality = self
            .sandbox_totality()
            .into_iter()
            .map(|v| crate::sandbox::describe_totality_violation(&self.data, &v));
        let raw_writes = self
            .sandbox_raw_writes()
            .into_iter()
            .map(|v| crate::sandbox::describe_raw_write_violation(&self.data, &v));
        let field_reads = crate::sandbox::field_read_violations(
            &self.sandbox,
            &self.def_sandbox,
            &self.sandbox_field_reads,
        )
        .into_iter()
        .map(|v| crate::sandbox::describe_field_read_violation(&self.data, &v));
        let field_updates = crate::sandbox::field_update_violations(
            &self.sandbox,
            &self.def_sandbox,
            &self.member_access,
            &self.sandbox_field_updates,
        )
        .into_iter()
        .map(|v| crate::sandbox::describe_field_update_violation(&self.data, &v));
        let field_appends = crate::sandbox::field_append_violations(
            &self.sandbox,
            &self.def_sandbox,
            &self.member_access,
            &self.sandbox_field_appends,
        )
        .into_iter()
        .map(|v| crate::sandbox::describe_field_append_violation(&self.data, &v));
        let param_locks = crate::sandbox::param_lock_violations(
            &self.sandbox,
            &self.def_sandbox,
            &self.sandbox_param_overrides,
        )
        .into_iter()
        .map(|v| crate::sandbox::describe_param_lock_violation(&self.data, &v));
        // @PLN86 P6.8 (F8) — an authoring error in the host's OWN links: a `group#right`
        // referencing a capability that was never declared.  Surfaced alongside the
        // grant violations so the host fixes its contract before a modder iterates.
        let undeclared = self.sandbox_undeclared_links();
        // @PLN86 §8 (F11) — the data envelope: the proven peak-heap footprint must fit
        // the profile's declared data_budget (or be provable at all).
        let data_env =
            crate::sandbox::data_envelope_violations(&self.data, &self.sandbox, &self.def_sandbox)
                .into_iter()
                .map(|v| crate::sandbox::describe_data_violation(&v));
        // #631 — a profile that allow-lists the library its own sandboxed code lives
        // in.  Listed FIRST: it disables the checks that produce the other findings,
        // so any verdict computed under it is unsound and fixing it comes before
        // anything else the walk reports.
        let self_allow = crate::sandbox::self_allow_list_violations(
            &self.data,
            &self.sandbox,
            &self.def_sandbox,
        )
        .into_iter()
        .map(|v| crate::sandbox::describe_self_allow_list_violation(&self.data, &v));
        self_allow
            .chain(caps)
            .chain(totality)
            .chain(raw_writes)
            .chain(field_reads)
            .chain(field_updates)
            .chain(field_appends)
            .chain(param_locks)
            .chain(undeclared)
            .chain(data_env)
            .collect()
    }

    /// @PLN86 — does this program designate ANY sandboxed def?  True gates the
    /// load-time admission walk (`sandbox_admission_errors`), which is **backend-
    /// agnostic**: an admitted script is total and fault-free on the interpreter AND
    /// on `--native` (bounded loops + an acyclic call graph + partial ops that yield
    /// null on both backends — div/mod-zero, OOB, overflow), so the host keeps its
    /// choice of backend.  (The earlier forced interpret-only was dropped: it rested
    /// on a false "native traps where the interpreter is total" premise — verified
    /// untrue.  A deployment that wants to forbid host-side `rustc` on mod-derived
    /// input can reintroduce it as a per-profile opt-in; the cdylib-FFI surface stays
    /// gated by `native_ffi`.)
    #[must_use]
    pub fn has_sandboxed_defs(&self) -> bool {
        !self.def_sandbox.is_empty()
    }

    /// @PLN86 step 3.4 — the worst-case complexity DEGREE of the sandboxed code:
    /// its step count is `O(n^degree)` in the largest input size.  An admitted
    /// script is total, so this is finite; the host reads it to bound the inputs
    /// so no single frame stalls (L5).
    #[must_use]
    pub fn sandbox_complexity_degree(&self) -> u32 {
        crate::sandbox::sandbox_complexity_degree(&self.data, &self.def_sandbox)
    }

    /// @PLN86 space budget — the worst-case SPACE (peak-heap) degree of the
    /// sandboxed code: `O(n^degree)` memory in the largest input.  The host bounds
    /// inputs against it so a bounded loop building a structure cannot OOM.
    #[must_use]
    pub fn sandbox_space_degree(&self) -> u32 {
        crate::sandbox::sandbox_space_degree(&self.data, &self.def_sandbox)
    }

    /// @PLN86 P7.1 (F9) — the worst-case peak-heap footprint `(degree, coeff)`: peak
    /// heap is bounded by `coeff · n^degree` bytes in the largest input `n`.  The
    /// coefficient is `Σ record_size` over the accumulating allocation sites; F11
    /// compares `coeff · max_input_n^degree` against the profile's `data_budget`.
    #[must_use]
    pub fn sandbox_space_footprint(&self) -> (u32, u32) {
        crate::sandbox::sandbox_space_footprint(&self.data, &self.def_sandbox)
    }

    /// @PLN86 prevention #3 — host capabilities that are NOT total: every
    /// `#cap`-tagged function whose call tree can reach an abort op.  A host-side
    /// lint (the mirror of the script-side 3.3 exclusion); an empty result means the
    /// loft-bodied capability surface cannot fault the host on a script value.
    #[must_use]
    pub fn sandbox_capability_totality_violations(
        &self,
    ) -> Vec<crate::sandbox::CapTotalityViolation> {
        crate::sandbox::capability_totality_violations(&self.data, &self.sandbox_fn_refs)
    }

    /// @PLN86 — the human-readable worst-case complexity report (time + space).
    #[must_use]
    pub fn sandbox_complexity_report(&self) -> String {
        crate::sandbox::complexity_report(
            self.sandbox_complexity_degree(),
            self.sandbox_space_degree(),
        )
    }

    /// @PLN110 3a — warn on `for i in 0..len(s) { … s.byte_at(i) … }`.
    ///
    /// `len(text)` counts CHARACTERS while `byte_at` indexes BYTES, so this loop
    /// stops one byte short per multi-byte character. It fails in the worst way
    /// available: no diagnostic, no fault, a shorter buffer with plausible contents
    /// that ASCII input never exposes. The published `cbor` library shipped exactly
    /// this in its RFC 8949 text encoder — encoding `"José"` produced a short buffer,
    /// `decode` reported success with an empty string, and a signature over the
    /// round-trip still verified, because both sides ran the same truncating encoder.
    ///
    /// This is the `byte_at` sibling of the `text[i]` lint in `fields.rs`: the same
    /// units error, the same bound, a different read. Advisory (the types are
    /// correct) — the fix is `0..size(s)` for a byte walk, or `for c in s`.
    fn warn_text_len_byte_index(&mut self, d_nr: u32, code: &Value) {
        if self.first_pass || self.default || !crate::keys::text_index_units_lint_enabled() {
            return;
        }
        let Value::Call(_, args) = code else { return };
        if self.data.def(d_nr).original_name() != "byte_at" || args.len() != 2 {
            return;
        }
        let Value::Var(iv) = args[1].unspan() else {
            return;
        };
        let Some(bound) = self.vars.loop_len_bound(*iv) else {
            return;
        };
        if crate::parser::operators::vec_key(&args[0], &self.data) != Some(bound) {
            return;
        }
        let iname = self.vars.name(*iv).to_string();
        diagnostic!(
            self.lexer,
            Level::Warning,
            "index `{iname}` walks `0..len(text)` (a character count) but `byte_at({iname})` \
             reads bytes — this truncates by one byte per multi-byte character and is silent \
             on ASCII; use `0..size(text)` for a byte walk, or iterate with `for c in text` \
             (@PLN110 strict-index)"
        );
    }

    /// @PLN86 L4 — record that the current def references `fn_d_nr` as a fn-ref
    /// VALUE (a function name used as a value).  No-op outside sandboxed code.
    /// Called at the fn-ref creation site so every flow is caught.
    pub(crate) fn record_sandbox_fn_ref(&mut self, fn_d_nr: u32) {
        if self.in_sandbox {
            self.sandbox_fn_refs
                .entry(self.context)
                .or_default()
                .insert(fn_d_nr);
        }
    }

    /// # Panics
    /// With filesystem problems.
    pub fn parse(&mut self, filename: &str, default: bool) -> bool {
        // under the `wasm` feature, check VIRT_FS before trying the real filesystem.
        #[cfg(feature = "wasm")]
        if let Some(content) = crate::wasm::virt_fs_get(filename) {
            return self.parse_virtual(&content, filename, default);
        }
        // @PLN11 arc E — record the input file for the whole-program cache key.
        if self.track_sources {
            self.parsed_sources.push(filename.to_string());
        }
        // @PLAN49 T1 — set the breadcrumb phase + initial file/line so
        // a watchdog-fired hard-kill localises any parse-time hang.
        crate::timeout::checkpoint_parse(filename, 0);
        self.default = default;
        // #255 / @PLN9: establish `source_dir` (the running program's own
        // directory) once, at the first non-default parse.  This is the single
        // home every execution path inherits — CLI run, `loft --test`, the wrap
        // integration runner, and the wasm/native front-ends all reach this
        // `parse()`.  `data.reset()` below clears `Data` but not `Stores`, so the
        // value survives the two-pass re-parse; the `is_empty()` guard keeps the
        // *first* (main) file winning over later directory/import re-parses.
        // (main.rs additionally sets it for the startup-cache path, which loads a
        // pre-parsed snapshot and never calls `parse()`.)
        if !default && self.database.source_dir.is_empty() {
            self.database.source_dir = std::path::Path::new(filename)
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
        }
        self.vars.logging = false;
        self.lexer.switch(filename);
        self.first_pass = true;
        self.pending_imports.clear();
        self.applied_imports.clear();
        self.deferred_unknown.clear();
        self.resolutions.clear();
        // @PLN86 1.2 — the def→profile side-map is keyed by def_nr, which
        // `data.reset()` reassigns; clear it so a re-parse re-derives the
        // designation rather than reading a stale entry.
        self.def_sandbox.clear();
        self.sandbox_unbounded_loops.clear();
        self.sandbox_raw_writes.clear();
        self.sandbox_fn_refs.clear();
        self.declared_capabilities.clear();
        self.member_access.clear();
        self.sandbox_field_reads.clear();
        self.sandbox_field_updates.clear();
        self.sandbox_field_appends.clear();
        self.last_field_target = None;
        self.param_locks.clear();
        self.sandbox_param_overrides.clear();
        self.pending_param_locks.clear();
        self.amp_pending = false;
        self.data.reset();
        // @PLN22 Phase 2 — the main program parses under its own source
        // (MAIN_SOURCE), distinct from the stdlib prelude (source 0), so a user
        // definition shadows a prelude name instead of colliding on `(name, 0)`.
        // `reset()` set source to 0; the default stdlib parse stays at 0.
        if !default {
            self.data.source = crate::data::MAIN_SOURCE;
        }
        self.lambda_counter = 0;
        self.fn_lambdas.clear();
        self.parse_file();
        self.resolve_deferred_unknowns();
        // H5 — two-pass name-stability contract: snapshot every def's attribute
        // count at the end of pass 1.  `data.reset()` between passes preserves
        // `definitions`, so def numbers are stable and pass 2 must reproduce these
        // counts exactly.  Post-arity-cascade (signatures freeze at declaration)
        // this is an INVARIANT, not an aspiration — a divergence is a real
        // cross-pass bug, asserted below after pass 2.
        #[cfg(debug_assertions)]
        let pass1_attr_counts: Vec<usize> = (0..self.data.definitions.len())
            .map(|d| self.data.attributes(d as u32))
            .collect();
        let lvl = self.lexer.diagnostics().level();
        if lvl != Level::Error && lvl != Level::Fatal {
            self.first_pass = false;
            self.reverse_iterator = false;
            self.iterable_context = false;
            self.applied_imports.clear();
            self.deferred_unknown.clear();
            self.resolutions.clear();
            self.data.reset();
            if !default {
                self.data.source = crate::data::MAIN_SOURCE;
            }
            self.lambda_counter = 0;
            self.fn_lambdas.clear();
            self.lexer.switch(filename);
            self.parse_file();
            self.resolve_deferred_unknowns();
            // @PLN35 PC3 — reject a left-recursive sub-rule grammar (a cycle in the invocation
            // graph) at compile time, before it can hang at runtime.  Post-parse over pass-2 edges.
            self.check_subrule_wellformedness();
            #[cfg(debug_assertions)]
            self.assert_pass2_def_attr_stable(&pass1_attr_counts);
            // @PLN104 P2 — oracle pass: flag frame-local text returns the interpreter would
            // orphan (#568) into `force_tret` (default-on; opt out with LOFT_NO_TRET_FIX).
            self.report_tret_promotions();
            // @PLN104 P3 — the targeted promotion: promote each flagged frame-local text
            // return (`force_tret`) to a `__tret` retbuf IN PLACE on the pass-2 IR and patch
            // only its direct callers to push the buffer.  The promotion set is decided BEFORE
            // this (in `report_tret_promotions`), so every caller — forward- OR backward-ref —
            // gets the retbuf without an ABI-growth crash; post-H5, so the extra attrs never
            // trip the pass1==pass2 contract.  This replaced a whole-file re-parse ("third
            // pass") whose non-idempotent re-lowering corrupted unrelated defs (var__vec /
            // diagnostics / s5-s7); touching only the promoted defs + their callers removes
            // that collateral class by construction.  See
            // doc/claude/plans/104-tret-promotion/targeted-promotion-design.md.
            if !self.force_tret.is_empty() {
                self.targeted_tret_promotion();
            }
        }
        self.backfill_native_symbol_crates();
        // Plan-07 phase 4h — emit `not null` field-reminder hints
        // after the second pass completes (so all reads have been
        // counted).  Silenceable via `LOFT_NO_HINT_NOT_NULL=1`.
        if !self.first_pass && !default {
            self.emit_not_null_hints();
        }
        self.diagnostics.fill(self.lexer.diagnostics());
        self.diagnostics.is_empty()
    }

    /// H5 (two-pass name-stability contract): assert pass 2 reproduced pass 1's
    /// per-def attribute counts exactly.  The parser is not re-entrant beyond two
    /// passes (the #339 third-pass experiment segfaulted on half-migrated variable
    /// tables) and pass 2 re-finds synthesized names by position, so a def whose
    /// attribute count changed between passes is a silent contract break.  Post the
    /// @PLAN59 arity cascade (signatures freeze at declaration) this is an
    /// INVARIANT — a firing assert is a real cross-pass bug, not noise.
    ///
    /// This attribute-count check is the COMPLETE H5 validation: the H5 spec's other
    /// named residual — work-ref (`__ref_N`) counter equality per fn — was dissolved
    /// by H1 (@PLAN59), exactly as the spec's item 3 ("re-evaluate after H1") foresaw.
    /// `work_refs()` now fires zero times across the whole debug corpus, and the
    /// counter's value in a stored table is unconditionally reset to 0 by
    /// `Function::append` at store time, so a work-ref-counter assert here would be
    /// permanently vacuous.  The one failure mode it could ever have caught — a
    /// cross-pass `__ref_N` name shift making `ref_return` add a spurious attr — IS
    /// caught here, because that spurious attr is itself an attribute-count divergence.
    #[cfg(debug_assertions)]
    fn assert_pass2_def_attr_stable(&self, pass1_attr_counts: &[usize]) {
        // Pass-2 def GROWTH has exactly one legal form (the fuzzer's F1 catch):
        // the reduce/map/filter builtin family desugars on pass 2 only — pass 1
        // early-returns the result type because unresolved lambda/forward types
        // make the full desugar impossible there — and the desugar machinery
        // lazily mints synthetic vector wrapper defs (`vector<T>` +
        // `main_vector<T>`, `Data::vector_def`) that pass 1 never reached.  For
        // `map` the OUTPUT element wrapper is unknowable in pass 1 (the lambda's
        // return type), so symmetric pass-1 minting is structurally impossible.
        // These mints are name-keyed, idempotent APPENDS: every pass-1 def
        // number is untouched, so the numbering contract H5 protects holds.
        // Anything else appearing only in pass 2 is a real cross-pass bug.
        // The second legal append: a GENERIC INSTANTIATION.  Instantiation is
        // pass-2-only BY DESIGN (`parse_call`: pass 1 only predicts the return
        // type — instantiating there would capture the template's still-being-
        // built body IR), so the monomorphised `t_<LEN><Type>_<fn>` def first
        // exists in pass 2.  Recognized precisely: the mangled tail must name
        // an existing `n_<fn>` template of `DefType::Generic`.
        for d in pass1_attr_counts.len()..self.data.definitions.len() {
            let name = self.data.def(d as u32).name();
            let dt = self.data.def_type(d as u32);
            let lazy_wrapper = (matches!(dt, DefType::Vector) && name.starts_with("vector<"))
                || (matches!(dt, DefType::Struct) && name.starts_with("main_vector<"));
            let lazy_instantiation =
                matches!(dt, DefType::Function) && self.h5_names_a_generic_template(name);
            debug_assert!(
                lazy_wrapper || lazy_instantiation,
                "H5: pass-2-only definition `{name}` (#{d}, {dt:?}) is not a lazy vector \
                 wrapper or generic instantiation — a real cross-pass divergence \
                 (pass1={}, pass2={})",
                pass1_attr_counts.len(),
                self.data.definitions.len(),
            );
        }
        for (d, &c1) in pass1_attr_counts.iter().enumerate() {
            let c2 = self.data.attributes(d as u32);
            // The attr-level lazy appends (both PROVEN identical on origin/main,
            // i.e. long-latent, when the DA calibration first checked them):
            // `__closure` — the capture hidden-arg is positioned in pass 2 from
            // pass 1's closure record (`parse_lambda*`: captures are only known
            // after the body parses); `__work_N` — a text-return work-buffer
            // promotion the pass-1 classify could not yet see.  Both are
            // name-keyed TRAILING appends: every pass-1 attr keeps its index.
            // Anything else — notably `__ref_N` / `__retbuf` growth, the
            // ref_return drift class this assert was built for — stays fatal.
            if c2 > c1 {
                for a in c1..c2 {
                    let n = self.data.attr_name(d as u32, a);
                    debug_assert!(
                        n == "__closure" || n.starts_with("__work_"),
                        "H5 two-pass contract: def `{}` (#{d}) grew a pass-2-only \
                         attribute `{n}` (pass1={c1}, pass2={c2}) that is not a \
                         documented lazy append — a real cross-pass divergence",
                        self.data.def(d as u32).name(),
                    );
                }
                continue;
            }
            debug_assert_eq!(
                c1,
                c2,
                "H5 two-pass contract: def `{}` (#{d}) attribute count diverged across \
                 passes (pass1={c1}, pass2={c2})",
                self.data.def(d as u32).name(),
            );
        }
    }

    /// H5 helper: does `name` carry the instantiation mangling
    /// `t_<LEN><SafeType>_<fn>` (see `try_generic_instantiation`) AND does the
    /// `n_<fn>` template exist as a `DefType::Generic`?  Only such defs are
    /// legal pass-2-only appends of the Function kind — a source-declared
    /// method parses in pass 1 and can never appear as a trailing pass-2 def.
    #[cfg(debug_assertions)]
    fn h5_names_a_generic_template(&self, name: &str) -> bool {
        let Some(rest) = name.strip_prefix("t_") else {
            return false;
        };
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        let Ok(type_len) = rest[..digits].parse::<usize>() else {
            return false;
        };
        let after = &rest[digits..];
        if after.len() <= type_len || !after.is_char_boundary(type_len) {
            return false;
        }
        let Some(fn_name) = after[type_len..].strip_prefix('_') else {
            return false;
        };
        let g_nr = self.data.def_nr(&format!("n_{fn_name}"));
        g_nr != u32::MAX && matches!(self.data.def_type(g_nr), DefType::Generic)
    }

    /// Plan-07 phase 4h — walk user struct definitions and emit
    /// `Level::Warning` for each non-`not_null` field whose read
    /// count exceeds `HINT_NOT_NULL_THRESHOLD` AND whose
    /// `defended_field_reads` set is empty.  The hint suggests
    /// adding `not null` at the field declaration so the
    /// constructor enforces non-null at write-time, eliminating
    /// the entire class of fault sites for that field.
    ///
    /// Threshold is conservative (10 reads) to keep the hint
    /// from firing on small / illustrative struct definitions.
    /// Lower thresholds + smarter defended-detection are slice 2.
    ///
    /// Stdlib (`self.default == true`) is exempt — its struct
    /// definitions are language-internal and the suggestion target
    /// is user code, not the host library.
    fn emit_not_null_hints(&mut self) {
        const HINT_NOT_NULL_THRESHOLD: u32 = 10;
        // @PLN25 DN1/F2: the `not null` hint is RETIRED. Under the dense model a plain scalar is
        // NON-null by default (nullability rides `?`/`Optional`), so `not null` is redundant (and
        // being retired), and the only nullable form left is `τ?` — for which suggesting
        // `not null` is contradictory. Superseded like the div/index fault warnings. (Also, under
        // F2 a heavily-read plain field is non-null → never accrues, so the hint would only ever
        // fire on a `?` field.)
        if crate::keys::pln25_dn1_enabled()
            || std::env::var("LOFT_NO_HINT_NOT_NULL").is_ok_and(|v| v == "1" || v == "true")
        {
            self.field_read_counts.clear();
            self.defended_field_reads.clear();
            return;
        }
        // Snapshot the keys so we can borrow `self.lexer` mutably
        // while iterating.
        let counts: Vec<((u32, u32), u32)> = self
            .field_read_counts
            .iter()
            .map(|(k, v)| (*k, *v))
            .collect();
        for ((d_nr, attr_idx), count) in counts {
            if count < HINT_NOT_NULL_THRESHOLD {
                continue;
            }
            if self.defended_field_reads.contains(&(d_nr, attr_idx)) {
                continue;
            }
            // Re-check `nullable` at emission time — definition
            // mutations between count + emit are unlikely but not
            // forbidden.
            let attrs = self.data.def(d_nr).attributes();
            if attr_idx as usize >= attrs.len() {
                continue;
            }
            let attr = &attrs[attr_idx as usize];
            if !attr.nullable {
                continue;
            }
            let struct_name = self.data.def(d_nr).name().to_string();
            let field_name = attr.name.clone();
            let pos = self.data.def(d_nr).position().clone();
            self.lexer.pos_diagnostic(
                Level::Warning,
                &pos,
                &format!(
                    "field `{struct_name}.{field_name}` is read {count} times and never \
                     defended with `??` or `if x.{field_name} != null` — consider \
                     marking it `not null` so the constructor enforces non-null at \
                     write-time"
                ),
            );
        }
        self.field_read_counts.clear();
        self.defended_field_reads.clear();
    }

    /// after `parse_file` has run (and all `todo_files` have drained,
    /// so every file in the recursion has had its definitions registered),
    /// reconcile any `DefType::Unknown` stubs that `actual_types_deferred`
    /// collected during parsing.
    ///
    /// The cyclic `use` case: file B references a type `Player` defined in
    /// file A, but B's `use A;` fires while A is suspended mid-parse — so
    /// B's body parsed with `Player` as a stub.  After the full recursion
    /// returns, A's `Player` is registered; Phase C re-applies imports with
    /// overwrite semantics (replacing B's stub binding with A's real def),
    /// then rewrites every `Type::Unknown(stub_nr)` occurrence to the real
    /// resolved type.
    ///
    /// Stubs that remain unresolved after this reconciliation surface as
    /// the original "Undefined type" error at the stored `Position`.
    fn resolve_deferred_unknowns(&mut self) {
        // Step 1: re-apply all previously-applied imports with overwrite
        // semantics.  This replaces any target-source `Unknown` stub with
        // the now-registered real def in the library source.
        let applied = std::mem::take(&mut self.applied_imports);
        for pi in &applied {
            match &pi.spec {
                ImportSpec::Wildcard => {
                    self.data.import_all_overwrite(pi.lib_source, pi.for_source);
                }
                ImportSpec::Names(names) => {
                    for (name, bind) in names {
                        self.data
                            .import_name_overwrite(pi.lib_source, pi.for_source, name, bind);
                    }
                }
            }
        }
        // Keep them on the list for any later pass (pass 2 re-populates).
        self.applied_imports = applied;

        // Step 2: for each deferred stub, resolve via the post-import
        // def binding.  Three outcomes per stub:
        //
        //  (a) The stub def got UPGRADED in-place to a real type (most
        //      common — `parse_struct` does this when it finds an
        //      existing stub by name).  `def(stub_nr).def_type` is no
        //      longer `Unknown`; call `rewrite_unknown_refs(stub, stub)`
        //      so that `Type::Unknown(stub)` references resolve to
        //      `def(stub).returned`.
        //
        //  (b) The stub's source has a DIFFERENT real def (e.g. when
        //      `import_all_overwrite` just routed the source-level
        //      binding to a real def from another source).  Rewrite
        //      Unknown references to point at that real def.
        //
        //  (c) Still unresolved — emit the "Undefined type" error at
        //      the stored `Position`.
        let deferred = std::mem::take(&mut self.deferred_unknown);
        for (source, stub_nr, pos) in deferred {
            let stub_name = self.data.def(stub_nr).name().to_string();
            // Case (a): stub upgraded in place
            if !matches!(self.data.def(stub_nr).def_type(), DefType::Unknown) {
                self.data.rewrite_unknown_refs(stub_nr, stub_nr);
                continue;
            }
            // Case (b): lookup via post-import source binding
            let resolved_nr = self.data.source_nr(source, &stub_name);
            if resolved_nr != u32::MAX
                && resolved_nr != stub_nr
                && !matches!(self.data.def(resolved_nr).def_type(), DefType::Unknown)
            {
                self.data.rewrite_unknown_refs(stub_nr, resolved_nr);
                continue;
            }
            // Case (c): emit the deferred error.  `string` used to be special-cased
            // here (and in `typedef.rs`); it is now one row of the cross-language
            // alias table `suggest_type_name` consults, so both deferred and direct
            // sites word it identically from one home.
            let msg = if let Some(s) = self.data.suggest_type_name(&stub_name) {
                format!("Undefined type {stub_name} — did you mean '{s}'?")
            } else {
                format!("Undefined type {stub_name}")
            };
            self.lexer.pos_diagnostic(Level::Error, &pos, &msg);
        }
    }

    /// After both parse passes, every `#native "<sym>"` annotation should map
    /// to its owning native package crate in `native_symbol_crates`.  If the
    /// manifest was registered before the .loft source that declared the
    /// native symbol was parsed, the original mapping pass in
    /// `lib_path_manifest` / `register_native_manifest` saw no definitions
    /// and left the symbol unmapped — which later surfaces as a `todo!()`
    /// stub in the `--native` output and a runtime panic.
    ///
    /// Walk every definition once more and bind each `#native` symbol still
    /// missing from the map to the native package that OWNS its definition:
    /// the registered package whose directory is a prefix of the def's source
    /// file (longest prefix wins, for nested packages).  Ownership-by-path is
    /// the same invariant the per-manifest pass enforces, run once at the end
    /// when every def exists, so it covers ANY number of native packages.
    /// (The earlier `len() == 1` shortcut silently skipped programs using two
    /// native packages — e.g. `graphics` + `random` — leaving BOTH unmapped:
    /// the interpreter still dispatched them via `def.native` + dlopen, but
    /// `--native` rejected the first reachable call with a P269 compile error.)
    pub(crate) fn backfill_native_symbol_crates(&mut self) {
        if self.data.native_packages.is_empty() {
            return;
        }
        let mut binds: Vec<(String, String)> = Vec::new();
        for d_nr in 0..self.data.definitions() {
            let def = self.data.def(d_nr);
            let sym = def.native();
            if sym.is_empty() || self.data.native_symbol_crates.contains_key(sym) {
                continue;
            }
            let sym = sym.to_string();
            let file = def.position().file.clone();
            // Owner = the registered native package whose dir is the longest
            // prefix of this def's source file.
            if let Some((crate_name, _)) = self
                .data
                .native_packages
                .iter()
                .filter(|(_, pkg_dir)| file.starts_with(pkg_dir.as_str()))
                .max_by_key(|(_, pkg_dir)| pkg_dir.len())
            {
                binds.push((sym, crate_name.replace('-', "_")));
            }
        }
        for (sym, rust_crate) in binds {
            self.data.native_symbol_crates.insert(sym, rust_crate);
        }
    }

    /// Parse in-memory `content` as if it were the file at `filename` — the
    /// filesystem-free twin of [`parse`](Self::parse), available on every target
    /// (unlike the wasm-only `parse_virtual`).  `@PLN98` P3.1 uses it to bootstrap
    /// the live interpreter from EMBEDDED stdlib + program blobs (no `default/`
    /// dir, no `LOFT_LIVE_SRC` file) — the delivery a browser build needs.  Mirrors
    /// `parse`'s two-pass + `MAIN_SOURCE` assignment (a non-default file shadows a
    /// prelude name rather than colliding on `(name, 0)`), so an embedded bootstrap
    /// parks the SAME world the fs path does.  Returns `true` when the parse is
    /// diagnostic-clean.
    pub fn parse_source(&mut self, content: &str, filename: &str, default: bool) -> bool {
        // @PLN13 — establish `source_dir` from `filename` (like `parse`) so a `--script`
        // run resolves `use` imports + relative I/O against the script's own directory.
        if !default && self.database.source_dir.is_empty() {
            self.database.source_dir = std::path::Path::new(filename)
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
        }
        self.default = default;
        self.vars.logging = false;
        self.first_pass = true;
        self.pending_imports.clear();
        self.applied_imports.clear();
        self.deferred_unknown.clear();
        self.resolutions.clear();
        self.data.reset();
        // @PLN22 — the main program parses under MAIN_SOURCE (not the prelude's
        // source 0), matching `parse`; `reset()` left source at STD_SOURCE.
        if !default {
            self.data.source = crate::data::MAIN_SOURCE;
        }
        self.lambda_counter = 0;
        self.fn_lambdas.clear();
        self.lexer.parse_string(content, filename);
        self.parse_file();
        self.resolve_deferred_unknowns();
        let lvl = self.lexer.diagnostics().level();
        if lvl != Level::Error && lvl != Level::Fatal {
            self.first_pass = false;
            self.applied_imports.clear();
            self.deferred_unknown.clear();
            self.resolutions.clear();
            self.data.reset();
            if !default {
                self.data.source = crate::data::MAIN_SOURCE;
            }
            self.lambda_counter = 0;
            self.fn_lambdas.clear();
            self.lexer.parse_string(content, filename);
            self.parse_file();
            self.resolve_deferred_unknowns();
        }
        self.diagnostics.fill(self.lexer.diagnostics());
        self.diagnostics.is_empty()
    }

    /// Parse `content` as if it were the file at `filename`.
    /// Used by the WASM virtual-FS path to bypass real filesystem access.
    #[cfg(feature = "wasm")]
    fn parse_virtual(&mut self, content: &str, filename: &str, default: bool) -> bool {
        // @PLN11 arc E — record the input file for the whole-program cache key.
        if self.track_sources {
            self.parsed_sources.push(filename.to_string());
        }
        self.default = default;
        self.vars.logging = false;
        self.first_pass = true;
        self.pending_imports.clear();
        self.applied_imports.clear();
        self.deferred_unknown.clear();
        self.resolutions.clear();
        self.data.reset();
        self.lambda_counter = 0;
        self.fn_lambdas.clear();
        self.lexer.parse_string(content, filename);
        self.parse_file();
        self.resolve_deferred_unknowns();
        let lvl = self.lexer.diagnostics().level();
        if lvl != Level::Error && lvl != Level::Fatal {
            self.first_pass = false;
            self.applied_imports.clear();
            self.deferred_unknown.clear();
            self.resolutions.clear();
            self.data.reset();
            self.lambda_counter = 0;
            self.fn_lambdas.clear();
            self.lexer.parse_string(content, filename);
            self.parse_file();
            self.resolve_deferred_unknowns();
        }
        self.diagnostics.fill(self.lexer.diagnostics());
        self.diagnostics.is_empty()
    }

    /// Parse all .loft files found in a directory tree in alphabetical ordering.
    /// # Errors
    /// With filesystem problems.
    pub fn parse_dir(&mut self, dir: &str, default: bool, debug: bool) -> std::io::Result<()> {
        let paths = read_dir(dir)?;
        let mut files: BTreeSet<String> = BTreeSet::new();
        for path in paths {
            let p = path?;
            let own_file = p
                .path()
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("loft"));
            let file_name = p.path().to_string_lossy().to_string();
            let data = metadata(&file_name)?;
            if own_file || data.is_dir() {
                files.insert(file_name);
            }
        }
        for f in files {
            let types = self.database.types.len();
            let from = self.data.definitions();
            let data = metadata(&f)?;
            if data.is_dir() {
                self.parse_dir(&f, default, debug)?;
            } else if !self.parse(&f, default) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("{}", self.diagnostics),
                ));
            }
            scopes::check(&mut self.data);
            if debug {
                self.output(&f, types, from)?;
            }
        }
        Ok(())
    }

    fn output(&mut self, f: &str, types: usize, from: u32) -> std::io::Result<()> {
        let f_norm = f.replace(other_sep(), sep_str());
        let file = f_norm.rsplit(sep()).next().unwrap_or(f);
        let to = format!("tests/dumps/{file}.txt");
        let _ = std::fs::create_dir_all("tests/dumps");
        if let Ok(mut w) = File::create(to.clone()) {
            let to = self.database.types.len();
            for tp in types..to {
                writeln!(w, "Type {tp}:{}", self.database.show_type(tp as u16, true))?;
            }
            for d_nr in from..self.data.definitions() {
                if *self.data.def(d_nr).code() == Value::Null {
                    continue;
                }
                write!(w, "{} ", self.data.def(d_nr).header(&self.data, d_nr))?;
                let mut vars = Function::copy(self.data.def(d_nr).variables());
                self.data
                    .show_code(&mut w, &mut vars, self.data.def(d_nr).code(), 0, false)?;
                writeln!(w, "\n")?;
            }
        } else {
            diagnostic!(self.lexer, Level::Error, "Could not write: {to}");
        }
        Ok(())
    }

    /// Only parse a specific string, only useful for parser tests.
    #[allow(dead_code)]
    pub fn parse_str(&mut self, text: &str, filename: &str, logging: bool) {
        // Start each standalone parse on a fresh lexer.  `restart` resets the
        // cursor but not the diagnostics or format-mode flags, and
        // `Diagnostics::level` is monotonic — so reusing the lexer would leak a
        // prior parse's errors into this one (poisoning REPL re-entrancy: a typo
        // would make every later input mis-parse).  A new lexer is fully clean.
        self.lexer = Lexer::default();
        self.first_pass = true;
        self.default = false;
        self.vars.logging = logging;
        self.lexer.parse_string(text, filename);
        self.applied_imports.clear();
        self.deferred_unknown.clear();
        self.resolutions.clear();
        self.data.reset();
        self.lambda_counter = 0;
        self.fn_lambdas.clear();
        self.declared_capabilities.clear();
        self.member_access.clear();
        self.sandbox_field_reads.clear();
        self.sandbox_field_updates.clear();
        self.sandbox_field_appends.clear();
        self.last_field_target = None;
        self.param_locks.clear();
        self.sandbox_param_overrides.clear();
        self.pending_param_locks.clear();
        self.parse_file();
        self.resolve_deferred_unknowns();
        let lvl = self.lexer.diagnostics().level();
        if lvl == Level::Error || lvl == Level::Fatal {
            self.diagnostics.fill(self.lexer.diagnostics());
            return;
        }
        self.applied_imports.clear();
        self.deferred_unknown.clear();
        self.resolutions.clear();
        self.data.reset();
        self.lambda_counter = 0;
        self.fn_lambdas.clear();
        self.lexer.parse_string(text, filename);
        self.first_pass = false;
        self.parse_file();
        self.resolve_deferred_unknowns();
        // @PLN35 PC3 — reject a left-recursive sub-rule grammar (see `check_subrule_termination`).
        self.check_subrule_wellformedness();
        self.diagnostics.fill(self.lexer.diagnostics());
    }

    /// Session-scope snippet parse (#350, live-reload): like
    /// [`parse_str`](Self::parse_str) but KEEPS the session's import scoping —
    /// `use_names` and the applied imports survive, and both passes run under
    /// `source` (the def-source of the fn being replaced) — so the snippet
    /// resolves the same library names its original file did.  `parse_str`'s
    /// `data.reset()` exists for whole-program loads, where each pass re-runs
    /// the `use` statements and re-registers scoping in order; a snippet has
    /// no `use` lines, so the reset left it with no library scope at all
    /// ("Unknown library" on every lib-qualified name).  No library loading
    /// happens here: every `use` the program needs already ran at install.
    pub fn parse_snippet(&mut self, text: &str, filename: &str, source: u16) {
        self.lexer = Lexer::default();
        self.first_pass = true;
        self.default = false;
        self.vars.logging = false;
        self.lexer.parse_string(text, filename);
        self.deferred_unknown.clear();
        self.resolutions.clear();
        self.lambda_counter = 0;
        self.fn_lambdas.clear();
        self.data.source = source;
        self.parse_file();
        self.resolve_deferred_unknowns();
        let lvl = self.lexer.diagnostics().level();
        if lvl == Level::Error || lvl == Level::Fatal {
            self.diagnostics.fill(self.lexer.diagnostics());
            return;
        }
        self.deferred_unknown.clear();
        self.resolutions.clear();
        self.lambda_counter = 0;
        self.fn_lambdas.clear();
        self.lexer.parse_string(text, filename);
        self.first_pass = false;
        self.data.source = source;
        self.parse_file();
        self.resolve_deferred_unknowns();
        self.diagnostics.fill(self.lexer.diagnostics());
    }

    /// @PLN12 phase 02 — does `input` end mid-construct, so a REPL should read
    /// more lines before trying to parse it?
    ///
    /// Returns `true` when a bracket is still open (`(`, `[`, `{`), a `"…"`
    /// string literal is unterminated, or the last meaningful token is a
    /// binary / continuation operator (`1 +`, `x.`).  `//` line comments and
    /// escaped quotes are skipped.  It is deliberately conservative: a missed
    /// "incomplete" just produces the ordinary parse error the caller already
    /// handles, never a crash.  Pure over the input string — no parser state.
    #[must_use]
    pub fn statement_incomplete(input: &str) -> bool {
        let chars: Vec<char> = input.chars().collect();
        let mut depth: i32 = 0;
        let (mut in_str, mut esc) = (false, false);
        let mut last: Option<char> = None;
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if in_str {
                if esc {
                    esc = false;
                } else if c == '\\' {
                    esc = true;
                } else if c == '"' {
                    in_str = false;
                    last = Some('"');
                }
                i += 1;
                continue;
            }
            // `//` to end of line is a comment — skip it.
            if c == '/' && chars.get(i + 1) == Some(&'/') {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            match c {
                '"' => in_str = true,
                '(' | '[' | '{' => {
                    depth += 1;
                    last = Some(c);
                }
                ')' | ']' | '}' => {
                    depth -= 1;
                    last = Some(c);
                }
                w if w.is_whitespace() => {}
                other => last = Some(other),
            }
            i += 1;
        }
        if in_str || depth > 0 {
            return true;
        }
        // A trailing binary / continuation operator means more is coming.
        matches!(
            last,
            Some('+' | '-' | '*' | '/' | '%' | '&' | '|' | '^' | '=' | '<' | '>' | ',' | '.')
        )
    }

    /// @PLN12 phase 02 — parse one REPL input against the live session.
    ///
    /// Reuses the parser's accumulated `data` + `database`, so a definition
    /// from an earlier call is visible to this one.  This works because
    /// `data.reset()` (which `parse_str` calls between its two passes) clears
    /// only import scoping, never `definitions` — so `parse_str` *appends* the
    /// input's new definitions to the existing stdlib + session.
    ///
    /// Returns [`ParseResult::NeedMore`] for input that ends mid-construct,
    /// [`ParseResult::Error`] (with `data` rolled back) on a parse error, or
    /// [`ParseResult::Ready`] with the new definition's number on success.
    ///
    /// A top-level definition (`struct`/`enum`/`fn`/`type`/…) is parsed as-is.
    /// Any other input (an expression, a call, an assignment) is wrapped in a
    /// synthetic runnable `fn repl_<n>()` so it parses and can be executed;
    /// `entry_def_nr` then points at that wrapper.  Locals declared in such a
    /// statement do NOT yet persist across inputs — the `__repl_session`
    /// local-persistence path is the remaining increment, paired with phase 03's
    /// runtime that keeps the session instance alive.
    pub fn parse_statement(&mut self, input: &str) -> ParseResult {
        if Self::statement_incomplete(input) {
            return ParseResult::NeedMore;
        }
        let pre_defs = self.data.definitions();
        let pre_diag = self.diagnostics.entries().len();
        let is_def = Self::starts_top_level_def(input);
        // The wrapper's name is keyed on the pre-call def count, which is
        // monotonic across successful statements (each commit adds ≥1 def), so
        // it never collides; a rolled-back attempt truncates `definitions` back
        // to `pre_defs`, freeing the number for the next statement.
        let wrapper = format!("n_repl_{pre_defs}");
        if is_def {
            self.parse_str(input, "<repl>", false);
        } else {
            let src = format!("fn repl_{pre_defs}() {{\n{input}\n}}");
            self.parse_str(&src, "<repl>", false);
        }
        // Only diagnostics this statement produced — `Diagnostics::level` is
        // monotonic, so a prior error would otherwise mask a now-clean parse.
        let produced: Vec<DiagEntry> = self.diagnostics.entries()[pre_diag..].to_vec();
        if produced.iter().any(|e| e.level >= Level::Error) {
            self.data.rollback_to(pre_defs);
            return ParseResult::Error(produced);
        }
        // A definition isn't executed (nothing to run); a wrapped expression
        // returns its synthetic fn, resolved by name so a lambda appended inside
        // the body can't be mistaken for the entry point.
        let entry_def_nr = if is_def {
            u32::MAX
        } else {
            self.data.def_nr(&wrapper)
        };
        ParseResult::Ready { entry_def_nr }
    }

    /// True if `input` opens with a top-level definition keyword, so it can be
    /// parsed directly rather than wrapped in a synthetic REPL fn.
    pub(crate) fn starts_top_level_def(input: &str) -> bool {
        let word: String = input
            .trim_start()
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        matches!(
            word.as_str(),
            "struct"
                | "enum"
                | "fn"
                | "type"
                | "pub"
                | "use"
                | "interface"
                | "typedef"
                | "const"
                | "capability"
        )
    }

    // ********************
    // * Helper functions *
    // ********************

    /// canonical entry point for building a vector
    /// database type from a content `Type`.  Resolves the element id
    /// through `Data::vector_element_type` (the single derivation of
    /// that fact — narrow leaf, nested vector, or plain `known_type`;
    /// shared with `typedef.rs::fill_database` for struct fields).
    /// Falls back to the default `integer` slot (0) when the content
    /// has no registered type yet.  Every `database.vector(...)` call
    /// in `src/parser/` should route through this helper so locals,
    /// parameters, returns, and literals get the same narrow storage
    /// that struct fields get via fill_database.
    pub(crate) fn vector_of(&mut self, content: &Type) -> u16 {
        if let Some(elem) = self.data.vector_element_type(content, &mut self.database) {
            return self.database.vector(elem);
        }
        let c_nr = self.data.type_elm(content);
        if c_nr == u32::MAX {
            return self.database.vector(u16::MAX);
        }
        let c_tp = self.data.def(c_nr).known_type();
        // Known_type may be unset for forward references; fall back to
        // the default integer slot (0) so the vector type still
        // registers correctly.  The content's own fill pass will
        // update once it runs.
        //
        // EXCEPT inside a generic template: there the unresolved content is
        // the type VARIABLE, which never fills.  Registering the placeholder
        // `vector<integer>` row from the template's parse SHIFTS the runtime
        // type table for everything registered after it, silently breaking
        // the layout-coincident derivations downstream (#483 — a stdlib
        // generic made an unrelated nested-vector literal read garbage).
        // Bake the MAX sentinel without registering anything; the
        // instantiation fixups re-derive the concrete ids.
        if c_tp == u16::MAX
            && self.context != u32::MAX
            && self.data.def_type(self.context) == DefType::Generic
        {
            return u16::MAX;
        }
        let resolved = if c_tp == u16::MAX { 0 } else { c_tp };
        self.database.vector(resolved)
    }

    /// @P314 — the element STORAGE type id to pass as the `OpAppendVector` /
    /// `vector_add` `tp` arg for a vector whose element type is `content`.
    /// Routes through `vector_of` (which consults `narrow_vector_content`) so a
    /// NARROW element (`u8`/`i16`/`i32`) resolves to its narrow storage type
    /// rather than the wide `integer`.  Using `def(type_def_nr(content))
    /// .known_type` instead loses the narrowness for integer aliases, so
    /// `vector_add` strides by 8 over a 1-/2-/4-byte-packed vector and corrupts
    /// (zeroes) the appended elements.  `single` (a distinct base type) is
    /// unaffected either way; this only matters for narrow integer aliases.
    pub(crate) fn append_elem_tp(&mut self, content: &Type) -> i32 {
        // One derivation for every element shape — `Data::vector_element_type`.
        // A NESTED element (`content` is itself a vector) resolves to the inner
        // vector's OWN database type, the 4-byte handle row `record_new` writes;
        // it used to be spelled `vector_of(content)`, which was the same id only
        // while a nested element registered level-COLLAPSED.  Now that
        // `vector<vector<T>>` registers honestly, `vector_of` is the CONTAINER
        // and passing it strode `vector_add` one level too deep.
        i32::from(
            self.data
                .vector_element_type(content, &mut self.database)
                .unwrap_or_else(|| {
                    let vec_tp = self.vector_of(content);
                    self.database.content(vec_tp)
                }),
        )
    }

    /// Get an iterator.
    /// The iterable expression is in *code.
    /// Creating the iterator will be in *code afterward.
    /// Return the next expression; with `Value::None` the iterator creation was impossible.
    /// Expected function type for a short-form lambda (`|x| {…}`) — the `⇐` push
    /// ([`Self::expected`]) filtered to `Type::Function` (D1: one channel, not four).
    pub(crate) fn lambda_hint(&self) -> Type {
        if matches!(self.expected, Type::Function(_, _, _)) {
            self.expected.clone()
        } else {
            Type::Unknown(0)
        }
    }

    /// Expected enum type for a bare value-position variant (`f(Red)`) — `expected`
    /// filtered to enum context.
    pub(crate) fn enum_hint(&self) -> Type {
        if self.enum_context(&self.expected) {
            self.expected.clone()
        } else {
            Type::Unknown(0)
        }
    }

    /// Expected `vector<…>` element-width hint for a bare literal — `expected` filtered to a
    /// concrete narrow-element vector (#432; [`Self::seeds_vector_hint`]).
    pub(crate) fn vector_hint(&self) -> Type {
        if Self::seeds_vector_hint(&self.expected) {
            self.expected.clone()
        } else {
            Type::Unknown(0)
        }
    }

    /// Expected destination type for an `f#read` (no `(n)`, no `as T`) — the raw `⇐` push,
    /// any shape; the read infers its byte width from it.
    pub(crate) fn read_target_type(&self) -> Type {
        self.expected.clone()
    }

    /// @PLAN48 P2: true when converting `src` → `dst` narrows a loft integer to a
    /// smaller explicit width (e.g. `integer` → `i32`, or `i32` → `u8`), which
    /// loses data.  Widening (`i32` → `integer`) and same-width are not narrowing.
    /// A plain `integer`/`wide`/`u32` has no `forced_size` and is treated as 8 bytes.
    fn is_narrowing_int(src: &Type, dst: &Type) -> bool {
        let (Type::Integer(s), Type::Integer(d)) = (src, dst) else {
            return false;
        };
        // The integer model (formal/types.md § the integer model): width lives in the
        // value RANGE.  A `dst` with no `forced_size` is the FULL integer — `IntegerSpec`'s
        // i32/u32 bounds cannot represent the i64 range and the "full integer" has several
        // bound encodings (`signed32` max = i32::MAX, `wide` max = u32::MAX), so
        // `forced_size = None` is the canonical "full range" marker; nothing narrows to it.
        // For a genuinely narrow (forced) storage, `src` narrows iff its range is not
        // contained in `dst`'s — `[s.min,s.max] ⊆ [d.min,d.max]`.  This is the same
        // range+sign test codegen's `narrow_int_cast` uses, so the two width derivations
        // now agree (D2/D3/D5).  Containment also makes signedness visible: `i8` (down to
        // -128) is not contained in `u8`.
        if d.forced_size.is_none() {
            return false;
        }
        s.min < d.min || s.max > d.max
    }

    /// @PLAN48 P2: render an integer type with its explicit narrow alias
    /// (`i32`/`u8`/`u16`/`i8`/`i16`) so a narrowing diagnostic doesn't print
    /// the bare `integer` for both sides (they share bounds).
    fn int_type_name(&self, t: &Type) -> String {
        if let Type::Integer(s) = t {
            match s.forced_size.map(std::num::NonZeroU8::get) {
                Some(4) => return "i32".to_string(),
                Some(2) => return if s.min < 0 { "i16" } else { "u16" }.to_string(),
                Some(1) => return if s.min < 0 { "i8" } else { "u8" }.to_string(),
                _ => {}
            }
        }
        t.name(&self.data)
    }

    /// @PLAN48 P2: literal exemption — true when `code` is a constant integer that
    /// provably fits `dst`'s full declared range, so `x: i32 = 5` / `f(5)` /
    /// `f(65535)` to a `u16` param stay legal without `as`.  This is the TYPE-fit
    /// question (a full-width register value); the narrower nullable-narrow-FIELD
    /// sentinel reservation is a separate, store-only check
    /// ([`Self::nullable_sentinel_hint`]) applied at the field-store sites.
    fn int_value_fits(&self, code: &Value, dst: &Type) -> bool {
        let Type::Integer(spec) = dst else {
            return false;
        };
        let n = match code.unspan() {
            Value::Int(n) => i64::from(*n),
            Value::Long(n) => *n,
            other => match crate::const_eval::const_eval(other, &self.data) {
                Some(Value::Int(n)) => i64::from(n),
                Some(Value::Long(n)) => n,
                _ => return false,
            },
        };
        n >= i64::from(spec.min) && n <= i64::from(spec.max)
    }

    /// When a literal stored into a NULLABLE narrow field fits the type's full
    /// range but lands on the reserved null sentinel (out of the usable range),
    /// return a hint explaining WHY — e.g. `255` in a nullable `u8`.  This tells
    /// the developer the value is the null encoding, not just "too big", and
    /// points at `not null` for the full range.  `None` for the ordinary
    /// out-of-range case (a generic narrowing message fits that).
    fn nullable_sentinel_hint(&self, code: &Value, dst: &Type, dst_name: &str) -> Option<String> {
        let Type::Integer(spec) = dst else {
            return None;
        };
        // @PLN25 F2 (range reconciliation): a plain (non-`Optional`) narrow integer is NON-null
        // under DN1, so it uses the FULL width — no reserved sentinel, nothing to reject. `dst`
        // here is a `Type::Integer` (an `Optional` target hit the let-else above), i.e. exactly
        // the non-null narrow that F2 makes full-range. (Reserving the sentinel for an `Optional`
        // narrow — rejecting the literal `255` into a `u8?` — is a separate Part-2 slice.)
        if crate::keys::pln25_f2_enabled() {
            return None;
        }
        if spec.not_null {
            return None;
        }
        let n = match code.unspan() {
            Value::Int(n) => i64::from(*n),
            Value::Long(n) => *n,
            other => match crate::const_eval::const_eval(other, &self.data) {
                Some(Value::Int(n)) => i64::from(n),
                Some(Value::Long(n)) => n,
                _ => return None,
            },
        };
        let fits_full = n >= i64::from(spec.usable_min(false)) && n <= spec.usable_max(false);
        let fits_usable = n >= i64::from(spec.usable_min(true)) && n <= spec.usable_max(true);
        if fits_full && !fits_usable {
            Some(format!(
                "{n} is reserved as the null sentinel of a nullable {dst_name} \
                 (usable {}..={}); declare the field `not null` for the full range, \
                 or cast with `as {dst_name}`",
                spec.usable_min(true),
                spec.usable_max(true),
            ))
        } else {
            None
        }
    }

    /// @PLN25 slice (c) — the `(N-Store)` teeth at a STORE site (typed assignment, field
    /// construction, an index, a return). An un-discharged nullable `τ?` cannot be committed
    /// to a non-nullable target — discharge it first with `?? <default>` or `match` (both
    /// yield the non-null base). UNLIKE `convert`, this runs ONLY at store sites, so null-CHECK
    /// comparisons (`x == null`) stay legal. Gated on `LOFT_PLN25_DN3`; returns `true` (and
    /// emits the diagnostic) on a violation. A no-op (returns `false`) off / first pass.
    /// @PLN102 (N-Store) — emit an N-Store diagnostic at the STORED VALUE's own span
    /// (`at`) when the caller supplies one, else at the lexer cursor (the historical
    /// position).  `None` is byte-identical to the old `diagnostic!(self.lexer, …)` path
    /// (`lexer.diagnostic` == `pos_diagnostic` at the current position); a `Some` anchor
    /// is used by the block-finalization callers whose cursor has advanced to the block's
    /// `}`.  See doc/claude/plans/102-stability-contract/nstore-position-fix.md.
    fn nstore_diag(&mut self, at: Option<&Position>, level: Level, message: &str) {
        match at {
            Some(p) => self.lexer.pos_diagnostic(level, p, message),
            None => self.lexer.diagnostic(level, message),
        }
    }

    fn n_store_violation(
        &mut self,
        value_tp: &Type,
        target_tp: &Type,
        what: &str,
        at: Option<&Position>,
    ) -> bool {
        if self.first_pass {
            return false;
        }
        // Tuples store ELEMENT-WISE, so `(N-Store)` applies per position: an Optional (DN3)
        // or bare-null (DN1) element cannot slip into a non-null element slot just because
        // the enclosing tuple type isn't itself `Optional`. A tuple type appears either as
        // a literal `Type::Tuple` (a field / typed assign) or as the synthetic-struct
        // rewrite `Reference(__tuple<…>)` (a tuple RETURN type, rewritten by
        // `parse_function`; elements = the def's attribute typedefs, the same normalization
        // as the destructure reader in expressions.rs). Normalize both sides and recurse
        // (nested tuples included); each element rides the same DN3/DN1 checks below.
        // Arity mismatches are left to the regular type checker.
        fn tuple_elems(data: &crate::data::Data, tp: &Type) -> Option<Vec<Type>> {
            match tp {
                Type::Tuple(elems) => Some(elems.clone()),
                Type::Reference(d, _) if data.def(*d).name().starts_with("__tuple<") => Some(
                    data.def(*d)
                        .attributes
                        .iter()
                        .map(|a| a.typedef.clone())
                        .collect(),
                ),
                _ => None,
            }
        }
        if let (Some(v_elems), Some(t_elems)) = (
            tuple_elems(&self.data, value_tp),
            tuple_elems(&self.data, target_tp),
        ) && v_elems.len() == t_elems.len()
        {
            let mut hit = false;
            for (i, (ve, te)) in v_elems.iter().zip(t_elems.iter()).enumerate() {
                hit |= self.n_store_violation(ve, te, &format!("element {i} of {what}"), at);
            }
            return hit;
        }
        // DN3: an un-discharged nullable `τ?` (Optional value) into a non-null target.
        if crate::keys::pln25_dn3_enabled()
            && let Type::Optional(inner) = value_tp
            && !matches!(
                target_tp,
                Type::Optional(_) | Type::Void | Type::Never | Type::Null
            )
        {
            let nm = inner.name(&self.data);
            // @PLN102 (N-Store) Phase 1 — the warn/error split (types.md § Null-flow, (N-Store)).
            // WARN (a nudge; the store PROCEEDS — `convert` peels the Optional and the slot holds
            // the null sentinel) where τ reserves its null DISTINCTLY even in the non-null form
            // (full `integer`, `float`, `single`, `boolean`, `character`, `text`, refs, aggregates).
            // Keep the hard ERROR only for a NARROW width (`byte_width < 8`), whose non-null form
            // spends the whole width on real values, so a null there would silently corrupt.
            // Gate OFF → the current uniform hard error (this branch stays byte-identical).
            let narrow = matches!(target_tp, Type::Integer(s) if s.byte_width(false) < 8);
            if crate::keys::nullflow_enabled() && !narrow {
                let msg = diagnostic_format(
                    Level::Warning,
                    format_args!(
                        "a nullable `{nm}?` is stored into {what} of the non-null type `{}` — it becomes null there; discharge with `?` (the type's default), `?? <default>`, or `match` if that is not intended",
                        target_tp.name(&self.data)
                    ),
                );
                self.nstore_diag(at, Level::Warning, &msg);
                return false; // store proceeds — `convert` peels the Optional and stores the sentinel
            }
            let msg = diagnostic_format(
                Level::Error,
                format_args!(
                    "a nullable `{nm}?` cannot be stored into {what} of the non-null type `{}` — discharge it first with `?` (the type's default), `?? <default>`, or `match`",
                    target_tp.name(&self.data)
                ),
            );
            self.nstore_diag(at, Level::Error, &msg);
            return true;
        }
        // DN1 (the default flip): under DN1 a plain scalar is NON-null, so a bare `null` cannot be
        // stored into a non-Optional scalar target — declare the target `τ?` to allow null.
        // (Heap types — reference/vector/enum — stay nullable; only the SCALAR default flips.)
        // The stdlib is held to the SAME rule (no STD_SOURCE exemption): F1b(b)'s `min`/`max`/`clamp`
        // non-null bodies are now clean (nullable args — including DN3-typed division results —
        // route to the `τ?` overload), so no trusted-source `return null` remains to exempt.
        if crate::keys::pln25_dn1_enabled()
            && matches!(value_tp, Type::Null)
            && Self::is_non_null_scalar(target_tp)
        {
            let nm = target_tp.name(&self.data);
            // @PLN102 (N-Store) Phase 1 — same warn/error split as the DN3 branch: a bare `null`
            // into a NON-narrow scalar target warns (the slot reserves its null distinctly, so it
            // holds null and reads back null); a NARROW width keeps the hard error (no room).
            let narrow = matches!(target_tp, Type::Integer(s) if s.byte_width(false) < 8);
            if crate::keys::nullflow_enabled() && !narrow {
                let msg = diagnostic_format(
                    Level::Warning,
                    format_args!(
                        "`null` is stored into {what} of the non-null scalar type `{nm}` — the slot holds null; declare it `{nm}?` to make that explicit"
                    ),
                );
                self.nstore_diag(at, Level::Warning, &msg);
                return false;
            }
            let msg = diagnostic_format(
                Level::Error,
                format_args!(
                    "`null` cannot be stored into {what} of the non-null scalar type `{nm}` — declare it `{nm}?` to allow null"
                ),
            );
            self.nstore_diag(at, Level::Error, &msg);
            return true;
        }
        false
    }

    /// @PLN25 DN1 — the scalar types whose default flips to NON-null (a bare `null` needs `τ?`).
    /// Heap-nullable types (reference / vector / enum / keyed) are NOT here — they stay nullable.
    fn is_non_null_scalar(tp: &Type) -> bool {
        matches!(
            tp,
            Type::Integer(_)
                | Type::Text(_)
                | Type::Boolean
                | Type::Float
                | Type::Single
                | Type::Character
        )
    }

    fn convert(&mut self, code: &mut Value, is_type: &Type, should: &Type) -> bool {
        // @PLAN48 P2: implicitly narrowing a loft `integer` to a smaller explicit
        // width (e.g. `integer` → `i32`) loses data and must be an explicit `as`.
        // A constant that provably fits is exempt.  Emit here, then fall through to
        // the `is_equal` accept — the Error fails compilation, and returning via
        // `is_equal` avoids a second (generic) diagnostic from the caller's
        // `validate_convert` (which still sees integer/i32 as can_convert-compatible).
        if !self.first_pass
            && Self::is_narrowing_int(is_type, should)
            && !self.int_value_fits(code, should)
        {
            let src = self.int_type_name(is_type);
            let dst = self.int_type_name(should);
            diagnostic!(
                self.lexer,
                Level::Error,
                "cannot implicitly narrow {src} to {dst} (may lose data) — cast explicitly with `as {dst}`"
            );
        }
        if is_type.is_equal(should) {
            return true;
        }
        // Never (return/break/continue) is compatible with any type.
        if matches!(is_type, Type::Never) {
            return true;
        }
        let _ = code;
        // Struct-literal inline constructors are typed as Rewritten(Reference(...)); strip
        // the wrapper so method calls chained on the constructor are accepted correctly.
        if let Type::Rewritten(inner) = is_type {
            return self.convert(code, inner, should);
        }
        if let Type::Rewritten(inner) = should {
            return self.convert(code, is_type, inner);
        }
        // @PLN25 slice (b): `Optional(τ)` is the nullable former. Behaviour-preserving until
        // DN1/DN3 give it teeth — peel and recurse on the base. A nullable TARGET also accepts
        // a bare `null`; a nullable SOURCE still implicitly unwraps to its base (DN2 removes
        // that unwrap later). Both arms converge: `Optional==Optional` is caught by `is_equal`
        // above, and a differing pair peels each side once.
        if let Type::Optional(inner) = should {
            if matches!(is_type, Type::Null) {
                // null into a nullable target: run the base's null→typed-null coercion so
                // `code` becomes the base sentinel op (e.g. `OpConvIntFromNull`, not a bare
                // `null` that natively renders `()`), but a nullable target ALWAYS accepts
                // null regardless of the base's own nullability.
                self.convert(code, is_type, inner);
                return true;
            }
            // Implicit CHECKED narrowing into a nullable narrow target: an integer or
            // `integer?` coerced into `Optional(narrow int)` (e.g. `u8?`) yields the value
            // when it fits, else null. Allowed WITHOUT an explicit `as` because the target is
            // nullable — an out-of-range value becomes a VISIBLE null, never a silent
            // truncation. A non-null narrow target (`u8`) is unchanged (still needs `as`).
            if !self.first_pass && Self::is_narrowing_int(is_type.base(), inner.base()) {
                let dst_base = inner.base().clone();
                let src_base = is_type.base().clone();
                self.dn4_checked_cast(code, &dst_base, &src_base);
                return true;
            }
            return self.convert(code, is_type, inner);
        }
        if let Type::Optional(inner) = is_type {
            // @PLN25 slice (b): the behaviour-preserving implicit unwrap. The `(N-Store)` teeth
            // (DN3) do NOT belong here — `convert` also services COMPARISONS (`x == null`), so
            // rejecting an Optional source here wrongly flags the very null-CHECKS that are how
            // you test nullability. (N-Store) must live at the STORE / decl / index sites (the
            // design's per-site checks), exempting null-compare. See RESUME.md § Step 3 slice c.
            return self.convert(code, inner, should);
        }
        // Plan-06 phase 4d: tuple-to-tuple convert is element-wise.
        // Without this, a value with a `Rewritten(Reference)` element
        // (e.g. `(Inner { … }, 11)` from inline struct construction)
        // fails to match a field declared as `(Inner, integer)` even
        // though the underlying types are compatible.  We don't
        // mutate `code` here — element conversions are by-shape only.
        if let (Type::Tuple(src_elems), Type::Tuple(dst_elems)) = (is_type, should)
            && src_elems.len() == dst_elems.len()
        {
            let mut all_compatible = true;
            for (s, d) in src_elems.iter().zip(dst_elems.iter()) {
                let mut placeholder = Value::Null;
                if !self.convert(&mut placeholder, s, d) {
                    all_compatible = false;
                    break;
                }
            }
            if all_compatible {
                return true;
            }
        }
        if let (Type::Reference(ref_tp, _), Type::Enum(enum_tp, true, _)) = (is_type, should) {
            for a in self.data.def(*enum_tp).attributes() {
                if a.name == self.data.def(*ref_tp).name() {
                    return true;
                }
            }
            // @PLN25 — a nullable struct SOURCE (`Reference(S)`, possibly the null
            // sentinel) flows into a synthetic `__nullable<S>` field.  Accept it here;
            // `handle_field` emits the wrap (null → discriminant 0, present → `Some`).
            if self.data.def(*enum_tp).name
                == format!("__nullable<{}>", self.data.def(*ref_tp).name())
            {
                return true;
            }
        }
        // @PLN25 single-payload — the REVERSE coercion: a `__nullable<S>` value flows
        // into a dense `S` slot (`f(v[i])` where `fn f(r: S)`, a `??` result, a
        // dense-local assign, a return).  The dense `S` IS the `Some` variant's inline
        // `payload` field, so unwrap by SUB-REFERENCING the payload — the sub-ref is a
        // valid dense `S` reference (it shares S's offset table), with NO copy.
        // (NOTE: the by-REF `&S` arg form is NOT handled here — a sub-ref reaches the
        // arg path as a by-VALUE `Reference` and gets copied, so a `&mut` write would not
        // propagate.  That seam is handled at the call-arg site, not in `convert`.)
        if let (Type::Enum(enum_d, true, _), Type::Reference(struct_d, _)) = (is_type, should)
            && self.data.def(*enum_d).name
                == format!("__nullable<{}>", self.data.def(*struct_d).name())
        {
            if !self.first_pass {
                let enum_d = *enum_d;
                let struct_d = *struct_d;
                let some_d = self.data.variant_of(enum_d, "Some");
                if some_d != u32::MAX {
                    // `get_val` for an INLINE struct field produces a sub-ref (pos+offset),
                    // not a pointer deref — the same form the field-access unwrap uses
                    // (fields.rs).  A raw `OpGetField` with the struct type would deref the
                    // payload bytes as a DbRef → garbage.  The payload IS dense `S`.
                    let payload_pos = u32::from(
                        self.database
                            .position(self.data.def(some_d).known_type(), "payload"),
                    );
                    *code = self.get_val(
                        &Type::Reference(struct_d, crate::data::Deps::none()),
                        false,
                        payload_pos,
                        code.clone(),
                        u32::MAX,
                    );
                }
            }
            return true;
        }
        // @PLN25 E2 — a synth `__nullable<S>` value used as a BOOLEAN (a condition
        // or bool arg, e.g. `if hash[k]` / `assert(hash[k])`) coerces to its
        // truthiness = "is present" = discriminant != 0.  Read the disc directly
        // (`OpGetEnum` @ offset 0, which has the rec==0 null-record guard, so an
        // ABSENT lookup result — rec 0 — reads disc 0 = not present), mirroring the
        // inline branch of `null_check_builder`.  Without this the raw nullable
        // DbRef reaches the bool context and native emits `(DbRef) as u8` (E0605);
        // a dense `Reference` lookup gets `OpConvBoolFromRef` on the same path.
        if let (Type::Enum(enum_d, true, _), Type::Boolean) = (is_type, should)
            && self.data.def(*enum_d).name.starts_with("__nullable<")
        {
            if !self.first_pass {
                let get_enum = self.cl("OpGetEnum", &[code.clone(), Value::Int(0)]);
                let disc = self.cl("OpConvIntFromEnum", &[get_enum]);
                let is_null = self.cl("OpEqInt", &[disc, Value::Int(0)]);
                *code = self.cl("OpNot", &[is_null]);
            }
            return true;
        }
        if let Type::RefVar(ref_tp) = is_type
            && self.convert(code, ref_tp, should)
        {
            return true;
        }
        // @PLN25 single-payload — a `__nullable<S>` value flows into a `&S` (`RefVar(Reference
        // (S))`) parameter (`fn f(r: &S)` that may MUTATE through `r`).  Unwrap to the payload
        // sub-ref, then pass it BY-REFERENCE via a work-ref + `OpCreateStack` (with `skip_free`,
        // since the work-ref holds only a borrowed DbRef into the element, not its own store) —
        // exactly the complex-expression by-ref path below.  A `&mut` write through the sub-ref
        // then propagates straight back to the source element's payload.  This must run BEFORE
        // the `ref_tp.is_equal(is_type)` arm (which fails: `Reference(S)` != `Enum(__nullable<S>)`).
        if let Type::RefVar(ref_tp) = should
            && let Type::Reference(struct_d, _) = &**ref_tp
            && let Type::Enum(enum_d, true, _) = is_type
            && self.data.def(*enum_d).name
                == format!("__nullable<{}>", self.data.def(*struct_d).name())
        {
            if !self.first_pass {
                let struct_d = *struct_d;
                let enum_d = *enum_d;
                let some_d = self.data.variant_of(enum_d, "Some");
                if some_d != u32::MAX {
                    let payload_pos = u32::from(
                        self.database
                            .position(self.data.def(some_d).known_type(), "payload"),
                    );
                    let dense = Type::Reference(struct_d, Deps::none());
                    let sub = self.get_val(&dense, false, payload_pos, code.clone(), u32::MAX);
                    let wv = self.vars.work_refs(&dense, &mut self.lexer);
                    self.vars.set_skip_free(wv);
                    *code = Value::Insert(vec![
                        v_set(wv, sub),
                        self.cl("OpCreateStack", &[Value::Var(wv)]),
                    ]);
                }
            }
            return true;
        }
        if let Type::RefVar(ref_tp) = should
            && ref_tp.is_equal(is_type)
        {
            // #266: a receiver/argument that is ITSELF an already-borrowed
            // reference (its declared var type is `RefVar(_)`, e.g. a `&self`
            // parameter) must be forwarded to a `&`-parameter by VALUE, not
            // re-wrapped in `OpCreateStack`.  A borrowed-reference var's slot
            // already holds a DbRef that points one level up (toward the
            // owning struct); `OpCreateStack` would build a DbRef pointing at
            // *that slot*, so the callee's single `OpGetStackRef` deref lands
            // on the borrowing frame instead of the struct — the inner
            // method's writes hit the stack store and never persist (and
            // clobber the caller's reference slot).  Passing the var through
            // unchanged gives the callee exactly the same live borrow the
            // current frame holds.  Native passes the reference by the Rust
            // ABI and is immune, so this is interpreter-correctness parity.
            // Only non-text references need this — `&text` borrows have their
            // own work-text copy semantics handled in the Text arm below.
            if !matches!(**ref_tp, Type::Text(_))
                && let Value::Var(v) = code
                && matches!(self.vars.tp(*v), Type::RefVar(vinner) if vinner.is_equal(ref_tp))
            {
                return true;
            }
            if matches!(**ref_tp, Type::Text(_)) {
                // Text → &text: use OpCreateStack for plain variables (write-back),
                // allocate a work-text copy for complex expressions (read-only).
                let orig = std::mem::replace(code, Value::Null);
                if let Value::Var(_) = &orig {
                    *code = self.cl("OpCreateStack", &[orig]);
                } else {
                    let wv = self.vars.work_text(&mut self.lexer);
                    let mut ls = Vec::new();
                    if orig != Value::Text(String::new()) {
                        ls.push(self.cl("OpAppendText", &[Value::Var(wv), orig]));
                    }
                    ls.push(self.cl("OpCreateStack", &[Value::Var(wv)]));
                    *code = v_block(
                        ls,
                        Type::Reference(self.data.def_nr("reference"), Deps::frame1(wv)),
                        "text_ref",
                    );
                }
            } else {
                let orig = std::mem::replace(code, Value::Null);
                if matches!(orig, Value::Var(_)) {
                    *code = self.cl("OpCreateStack", &[orig]);
                } else {
                    // produce a `Value::Insert` so that scope
                    // analysis (`scopes::scan_args`) hoists the
                    // pre-call Set into the enclosing statement list.
                    // Insert does not form a scope, so the work-ref
                    // lives at function scope and its slot survives
                    // the call.  Using `v_block` instead would create
                    // a block scope whose exit FreeStack clobbers the
                    // ref-target bytes and corrupts preceding args.
                    //
                    // The work-ref holds only a COPY of an existing
                    // DbRef — it does not own a store — so tell
                    // scopes to suppress the scope-exit `OpFreeRef`.
                    // Without `skip_free`, the shared store would
                    // be decremented once per call and eventually
                    // reach ref_count 0, dangling the caller's
                    // owning reference across loop iterations.
                    //
                    // Pass-2 ONLY: `skip_free` is a GLOBAL per-var bit on a
                    // NAME-pooled work-ref (`__ref_N`, counter-numbered per
                    // pass), and it persists in the stored var table across
                    // the pass boundary.  When the two passes' `work_refs`
                    // call sequences differ, pass 1's carrier NAME can be
                    // pass 2's OWNED literal temp — the pass-1 stamp then
                    // disarms that temp's scope-exit free and its store
                    // leaks (the p179 `&`-field-arg cell; the counter-
                    // coupling hazard, COMPILER.md).  Pass-1 IR is discarded,
                    // so the stamp's only lasting effect IS the poison.
                    let wv = self.vars.work_refs(is_type, &mut self.lexer);
                    if !self.first_pass {
                        self.vars.set_skip_free(wv);
                    }
                    *code = Value::Insert(vec![
                        v_set(wv, orig),
                        self.cl("OpCreateStack", &[Value::Var(wv)]),
                    ]);
                }
            }
            return true;
        }
        let mut check_type = is_type;
        let r = Type::Reference(self.data.def_nr("reference"), Deps::none());
        let e = Type::Enum(0, false, Deps::none());
        if let Type::Vector(_nr, _) = is_type {
            if let Type::Vector(v, _) = should
                && v.is_unknown()
            {
                return true;
            }
        } else if let Type::Reference(_, _) = is_type {
            if matches!(*should, Type::Reference(0, _)) {
                return true;
            }
            check_type = &r;
        } else if matches!(
            is_type,
            Type::Hash(_, _, _)
                | Type::Sorted(_, _, _)
                | Type::Index(_, _, _)
                | Type::Radix(_, _, _)
        ) {
            // A keyed-collection handle IS a `DbRef`, so it satisfies a bare
            // `reference` parameter unchanged — no conversion op.  Used by
            // `store_persist_bind`, whose `bind_path` snapshots the whole
            // dedicated Store regardless of the collection kind.
            if let Type::Reference(rd, _) = should
                && *rd == self.data.def_nr("reference")
            {
                return true;
            }
        } else if let Type::Enum(_, false, _) = is_type {
            if *should == e {
                return true;
            }
            check_type = &e;
        }
        // @PLN25: a null literal returned/assigned where a vector is expected becomes
        // the null sentinel (store_nr=u16::MAX), reusing the reference sentinel producer
        // — distinct from an empty `[]` (a valid store with length 0).
        if *is_type == Type::Null && matches!(should, Type::Vector(_, _)) {
            let sentinel_nr = self.data.def_nr("OpNullRefSentinel");
            *code = Value::Call(sentinel_nr, vec![]);
            return true;
        }
        // @PLN102 — `null` assigned to a value-enum target (`n: Color? = null`, a
        // return, an arg, a field) must become the enum's typed null
        // (`OpConvEnumFromNull` → the 255 sentinel), NOT stay a bare `Value::Null`
        // (byte 0).  The `FromNull` loop below matches by RETURN type, but
        // `OpConvEnumFromNull` returns the generic `enumerate`, never `is_equal` to a
        // specific `Enum(Color)` — so the conversion was skipped and the slot held 0
        // while every consumer (`== null`, `??`, `!`, `{n}`) tests the 255 sentinel,
        // leaving the value un-null-checkable.  Mirrors `Parser::null`'s Enum arm.
        // Inline `__nullable<S>` fields keep their own disc-0 representation (excluded);
        // a value-enum VECTOR element has no wired per-element null and is rejected in
        // `parse_vector` (which no longer relies on this convert failing).
        if *is_type == Type::Null
            && let Type::Enum(tp, _, _) = should.base()
            && !self.data.def(*tp).name.starts_with("__nullable<")
        {
            *code = self.cl(
                "OpConvEnumFromNull",
                &[Value::Int(i32::from(self.data.def(*tp).known_type()))],
            );
            return true;
        }
        // @PLN99 Arc C — a struct/reference-returning user conversion carries a hidden
        // destination parameter (attributes() > 1), so it must go through `call_nr` (whose
        // `add_defaults` appends the dest).  But `call_nr` needs `&mut self`, and this scan
        // borrows `self.data` immutably — so record the winner and dispatch it AFTER the loop.
        let mut struct_conv: Option<u32> = None;
        for &dnr in self.data.get_possible("OpConv", &self.lexer) {
            if self.data.def(dnr).name().ends_with("FromNull") {
                if *is_type == Type::Null {
                    if matches!(self.data.def(dnr).returned(), Type::Reference(_, _))
                        && let Type::Reference(_, _) = *should
                    {
                        // Use the non-allocating sentinel instead of OpConvRefFromNull so that
                        // null comparisons (`s == null`, `s != null`) do not leak a store.
                        let sentinel_nr = self.data.def_nr("OpNullRefSentinel");
                        *code = Value::Call(sentinel_nr, vec![]);
                        return true;
                    } else if self.data.def(dnr).returned().is_equal(should) {
                        *code = Value::Call(dnr, vec![]);
                        return true;
                    }
                }
            } else if self.data.attributes(dnr) > 0
                && (self.data.attr_type(dnr, 0).is_equal(check_type)
                    || self.data.attr_type(dnr, 0).is_equal(is_type))
                && self.data.def(dnr).returned().is_equal(should)
            {
                // @PLN99 Arc C — for a `Reference` source, `check_type` was flattened to the
                // generic `reference` (line ~2408) so stdlib `OpConv…FromRef` match any handle.
                // That hides a specific struct→struct user conversion (`fn OpConvBFromA(a: A)`),
                // so we ALSO try the concrete `is_type`.  The `returned().is_equal(should)` guard
                // keeps this exact: a candidate only wins when BOTH its source and target align.
                if self.data.attributes(dnr) > 1 {
                    struct_conv = Some(dnr); // struct-returning: dispatch after the loop
                    break;
                }
                // Stdlib primitive conversions (attributes() == 1) keep the direct Call.
                *code = Value::Call(dnr, vec![code.clone()]);
                return true;
            }
        }
        if let Some(dnr) = struct_conv {
            // Pass the CONCRETE source type (`is_type`), not the flattened `check_type` —
            // `process_call_args` needs the real type to wire the argument.  A Null result
            // means the arg couldn't be wired; leave `code` untouched and report no match.
            let src = code.clone();
            let rtp = self.call_nr(
                code,
                dnr,
                &[src],
                std::slice::from_ref(is_type),
                false,
                &[],
                None,
            );
            if rtp == Type::Null {
                return false;
            }
            // The conversion allocated a fresh owned store; hand its real return type
            // (Owned deps) to the `as` handler so it does NOT graft the source's deps.
            self.conv_owned_result = Some(rtp);
            return true;
        }
        false
    }

    /// Cast a type to another type when possible
    /// Returns false when impossible.
    fn cast(&mut self, code: &mut Value, is_type: &Type, should: &Type) -> bool {
        if self.first_pass {
            return true;
        }
        let mut should_nr = self.data.type_def_nr(should);
        if let Type::Vector(c_tp, _) = should {
            let c_nr = self.data.type_def_nr(c_tp);
            // route through `vector_of` so narrow aliases
            // (vector<i32>, vector<u8>) get the correct narrow element
            // type — matching what fill_database registers for struct
            // fields.
            let tp = self.vector_of(c_tp);
            should_nr = self.data.check_vector(c_nr, tp, self.lexer.pos());
        }
        let should_kt = if should_nr == u32::MAX {
            u16::MAX
        } else {
            self.data.def(should_nr).known_type()
        };
        let is_nr = self.data.type_def_nr(is_type);
        let is_kt = if is_nr == u32::MAX {
            u16::MAX
        } else {
            self.data.def(is_nr).known_type()
        };
        if let Type::Reference(tp, _) = should
            && self.data.def(*tp).returned().is_equal(is_type)
            && matches!(is_type, Type::Enum(_, true, _))
        {
            let get_e = self.cl("OpGetEnum", &[code.clone(), Value::Int(0)]);
            let get = self.cl("OpConvIntFromEnum", &[get_e]);
            if let Value::Enum(nr, _) = self.data.def(*tp).attributes()[0].value {
                *code = v_if(
                    self.cl("OpEqInt", &[get, Value::Int(i32::from(nr))]),
                    code.clone(),
                    self.cl("OpConvRefFromNull", &[]),
                );
            }
            return true;
        }
        if matches!(is_type, Type::Text(_))
            && matches!(should, Type::Enum(_, true, _) | Type::Reference(_, _))
        {
            *code = self.cl(
                "OpCastVectorFromText",
                &[code.clone(), Value::Int(i32::from(should_kt))],
            );
            return true;
        }
        for &dnr in self.data.get_possible("OpCast", &self.lexer) {
            if self.data.attributes(dnr) == 1
                && self.data.attr_type(dnr, 0).is_same(is_type)
                && self.data.def(dnr).returned().is_same(should)
            {
                if let Type::Enum(tp, false, _) = should {
                    *code = Value::Call(
                        dnr,
                        vec![
                            code.clone(),
                            Value::Int(i32::from(self.data.def(*tp).known_type())),
                        ],
                    );
                } else {
                    *code = Value::Call(dnr, vec![code.clone()]);
                }
                return true;
            } else if self.data.attributes(dnr) == 2
                && self.data.attr_type(dnr, 0).is_same(is_type)
                && self.data.def(dnr).returned().is_same(should)
                && should_kt != u16::MAX
            {
                *code = Value::Call(dnr, vec![code.clone(), Value::Int(i32::from(should_kt))]);
                return true;
            } else if self.data.attributes(dnr) == 2
                && self.data.attr_type(dnr, 0).is_same(is_type)
                && self.data.def(dnr).returned().is_same(should)
                && is_kt != u16::MAX
            {
                *code = Value::Call(dnr, vec![code.clone(), Value::Int(i32::from(is_kt))]);
                return true;
            }
        }
        false
    }

    /// Validate that two types are equal
    fn can_convert(&mut self, test_type: &Type, should: &Type) -> bool {
        if *test_type != *should && !test_type.is_unknown() {
            if let Type::RefVar(tp) = should
                && tp.is_equal(test_type)
            {
                return true;
            }
            if let (Type::Enum(_e, _, _), Type::Enum(o, _, _)) = (test_type, should)
                && self.data.def(*o).name() == "enumerate"
            {
                return true;
            }
            if let (Type::Reference(r_nr, _), Type::Enum(e_nr, true, _)) = (test_type, should)
                && e_nr == r_nr
            {
                return true;
            }
            // @PLN25 E2 — a `__nullable<S>` value is accepted into a dense `S`
            // slot; `convert` emits the payload sub-ref (gap 2).
            if let (Type::Enum(enum_d, true, _), Type::Reference(struct_d, _)) = (test_type, should)
                && self.data.def(*enum_d).name
                    == format!("__nullable<{}>", self.data.def(*struct_d).name())
            {
                return true;
            }
            if let (Type::Enum(t, false, _), Type::Enum(s, false, _)) = (test_type, should)
                && *t == *s
            {
                return true;
            }
            if let (Type::Enum(_, false, _), Type::Integer(_)) = (test_type, should) {
                return true;
            }
            if let Type::Reference(r, _) = should
                && *r == self.data.def_nr("reference")
                && let Type::Reference(_, _) = test_type
                && self.generic_type_name(test_type).is_none()
            {
                return true;
            }
            // A keyed-collection argument (hash / sorted / index / spatial)
            // also satisfies a bare `reference` parameter — its handle is a
            // `DbRef`.  Mirrors the `convert` branch; used by
            // `store_persist_bind`.
            if let Type::Reference(r, _) = should
                && *r == self.data.def_nr("reference")
                && matches!(
                    test_type,
                    Type::Hash(_, _, _)
                        | Type::Sorted(_, _, _)
                        | Type::Index(_, _, _)
                        | Type::Radix(_, _, _)
                )
            {
                return true;
            }
            // Bare collection parameter (sorted / hash / index / spatial)
            // accepts the corresponding parameterised collection
            // argument.  Mirrors how `Type::Vector(_, _)` matches via
            // is_same: the parameter type carries no element-type
            // constraint, so any concrete instantiation is structurally
            // compatible.  Used for stdlib helpers like `len(both: sorted)`.
            if let Type::Reference(r, _) = should {
                let r = *r;
                let bare = (r == self.data.def_nr("sorted")
                    && matches!(test_type, Type::Sorted(_, _, _)))
                    || (r == self.data.def_nr("hash") && matches!(test_type, Type::Hash(_, _, _)))
                    || (r == self.data.def_nr("index")
                        && matches!(test_type, Type::Index(_, _, _)))
                    || (r == self.data.def_nr("spatial")
                        && matches!(test_type, Type::Radix(_, _, _)));
                if bare {
                    return true;
                }
            }
            // Text types with different dep lists are structurally compatible.
            if matches!((test_type, should), (Type::Text(_), Type::Text(_))) {
                return true;
            }
            // Function types with compatible params and return type.
            if let (Type::Function(tp, tr, _), Type::Function(sp, sr, _)) = (test_type, should)
                && tp.len() == sp.len()
                && tp.iter().zip(sp.iter()).all(|(a, b)| a.is_equal(b))
                && tr.is_equal(sr)
            {
                return true;
            }
            false
        } else {
            true
        }
    }

    fn validate_convert(&mut self, context: &str, test_type: &Type, should: &Type, pos: &Position) {
        if !self.first_pass && !self.can_convert(test_type, should) {
            // Plan-07 phase 6 (partial) — "expected E, got G on context"
            // reads the same direction as English ("we expected this,
            // we got that"); the old shape "G should be E on context"
            // forced a mental flip and confused users new to the
            // language.  `pos` is the offending value's start, captured at
            // parse time — the lexer cursor has drifted to the `;` by now.
            diagnostic_at!(
                self.lexer,
                pos,
                Level::Error,
                "expected {}, got {} on {context}",
                should.name(&self.data),
                test_type.name(&self.data)
            );
        }
    }

    /// Check if a type is a generic type variable (a dummy struct used as T).
    /// Returns the type variable name if it is, None otherwise.
    pub(crate) fn generic_type_name(&self, tp: &Type) -> Option<&str> {
        if let Type::Reference(d, _) = tp {
            let d = *d as usize;
            if d < self.data.definitions.len()
                && self.data.definitions[d].def_type == DefType::Struct
                && self.data.definitions[d].attributes.is_empty()
                && self.context != u32::MAX
                && self.data.definitions[self.context as usize].def_type == DefType::Generic
            {
                return Some(&self.data.definitions[d].name);
            }
        }
        None
    }

    /// Check whether the current generic function's bounds include an interface that
    /// declares the given method.  Returns false if not inside a generic or if no bound
    /// declares the method.
    pub(crate) fn has_bound_for_method(&self, method: &str) -> bool {
        if self.context == u32::MAX {
            return false;
        }
        let bounds = &self.data.definitions[self.context as usize].bounds;
        for &iface_nr in bounds {
            for child_nr in self.data.children_of(iface_nr) {
                let name = self.data.def(child_nr).name();
                // Interface stubs use "__iface_{d_nr}_{method}" naming
                if let Some(rest) = name.strip_prefix("__iface_")
                    && let Some((_, m)) = rest.split_once('_')
                    && m == method
                {
                    return true;
                }
            }
        }
        false
    }

    /// Search for definitions with the given name and call that with the given parameters.
    #[allow(clippy::too_many_arguments)]
    fn call(
        &mut self,
        code: &mut Value,
        source: u16,
        name: &str,
        list: &[Value],
        types: &[Type],
        named_args: &[(String, Value, Type)],
        arg_pos: &[Position],
        name_pos: &Position,
    ) -> Type {
        // Create a new list of parameters based on the current ones
        // We still need to know the types.
        let mut d_nr = if self.default && is_op(name) {
            self.data.def_nr(name)
        } else {
            // @PLN25 F1b(b): a `both`/`self`-dispatched function takes uniform-nullability
            // params, so dispatch on whether ANY argument is nullable — not just arg0. This
            // routes `max(5, a?)` to the `τ?` overload the same as `max(a?, 5)`, so null
            // propagates regardless of position. (arg0-only dispatch missed the arg1 case.)
            let unknown = Type::Unknown(0);
            let nullable_holder;
            let dispatch_tp: &Type = if types.is_empty() || types[0] == Type::Null {
                &unknown
            } else if types.iter().any(|t| matches!(t, Type::Optional(_))) {
                nullable_holder = Type::optional(types[0].base().clone());
                &nullable_holder
            } else {
                &types[0]
            };
            self.data.find_fn(source, name, dispatch_tp)
        };
        // Trace point: post-find_fn dispatch state.  Captures the most
        // common debugging vantage — what name resolved to which
        // d_nr, whether it was a Generic that got skipped, and which
        // pass we're on.  Enable with `LOFT_TRACE=call`.
        crate::loft_trace!(
            call,
            "name={} types={:?} d_nr={} def_type={:?} first_pass={}",
            name,
            types,
            d_nr,
            if d_nr == u32::MAX {
                None
            } else {
                Some(self.data.def(d_nr).def_type().clone())
            },
            self.first_pass,
        );
        // skip generic templates — they are not callable directly.
        if d_nr != u32::MAX && self.data.def(d_nr).def_type() == DefType::Generic {
            d_nr = u32::MAX;
        }
        // @PLN115 S5 — record a free-function CALL as a Global reference at its
        // name.  Gated on the recording flag (so it is a single predictable bool
        // check on every normal compile) and pass 2 (the name has resolved).  Only
        // user free functions (`n_<name>`) are Globals here; operators (`Op…`) are
        // not user-navigable names and methods (`t_…`) resolve in fields.rs (S6).
        if self.record_resolutions
            && !self.first_pass
            && d_nr != u32::MAX
            && self.data.def(d_nr).name().starts_with("n_")
        {
            self.record(
                name_pos,
                name.chars().count() as u16,
                crate::resolution::Resolution::Global(d_nr),
            );
        }
        // Plan-17 phase 01 (A) — propagate the substituted return type
        // on first pass so receiving variables (`t = min_max(7, 3)`)
        // get a correctly-typed `Type::Tuple([…])` slot, enabling
        // `t.0` / `t.1` on the SAME pass.  Without this, first-pass
        // returned `Type::Unknown(0)`; the receiving variable stayed
        // Unknown (change_var_type is a no-op for Unknown); downstream
        // `t.0` rejected with "Expect token ;" because tuple element
        // access requires a typed Tuple receiver.  That error aborted
        // second pass entirely (lexer.token() emits errors regardless
        // of pass), so the second-pass full instantiation never ran.
        //
        // The fix splits the work: on first pass, we predict the
        // return type only (no def creation — first-pass body IR is
        // still being built and would produce a stale instantiation);
        // on second pass, the full `try_generic_instantiation`
        // creates the monomorphised def.  First-pass IR for the call
        // is `Value::Null` placeholder; second-pass re-parse builds
        // the real `Value::Call`.
        // @PLN102 arc C — generic instantiation is CALLER-SOURCE-AGNOSTIC: a
        // stdlib fn calling a generic (`sum_of`→`sum`) instantiates it exactly like
        // a user program does.  The old `&& !self.default` guard skipped the stdlib
        // parse, so a stdlib-internal generic call resolved to "Unknown function"
        // (surfaced by the step-6 dogfood).  Inert for the current stdlib — no
        // stdlib call was a generic — and inert for a non-generic unresolved name
        // (`predict_generic_return_type` / `try_generic_instantiation` return
        // Unknown/MAX with no side effect, so it falls through to "Unknown function"
        // exactly as before).
        if d_nr == u32::MAX {
            if self.first_pass {
                let predicted = self.predict_generic_return_type(name, types);
                if !predicted.is_unknown() {
                    *code = Value::Null;
                    return predicted;
                }
            } else {
                d_nr = self.try_generic_instantiation(name, types);
            }
        }
        if d_nr != u32::MAX {
            let ret = self.call_with_named(
                code,
                d_nr,
                list,
                types,
                named_args,
                true,
                arg_pos,
                Some(name_pos),
            );
            // @PLN102 Phase 3.5 — constant-in-domain elision ("PI blocks it"): a domain-partial
            // math fn with a provably in-domain CONSTANT argument cannot be null (`sqrt(4.0)`,
            // `pow(2.0, 3.0)`, `ln(2.0)`), so peel the `τ?` its decl carries. Only under
            // LOFT_NULLFLOW, and only the constant subset (variable-arg range-tracking is deferred).
            if crate::keys::nullflow_enabled()
                && matches!(ret, Type::Optional(_))
                && self.math_arg_in_domain(name, list)
            {
                // Phase 3.5 elision: a provably-in-domain constant arg peels the `τ?`.
                ret.base().clone()
            } else if (matches!(name, "min" | "max" | "clamp") || crate::keys::nullflow_enabled())
                && Self::is_null_transparent(name)
                && !matches!(ret, Type::Optional(_))
                && Self::is_non_null_scalar(&ret)
                && types.iter().any(|t| matches!(t, Type::Optional(_)))
            {
                // Phase 5.3 propagation: a null-transparent fn with a nullable arg → `τ?` + a
                // runtime guard (if any arg is null the result is null). `min`/`max`/`clamp` used
                // to carry this in hand-written `τ?` overloads, which were DEFAULT-ON (DN3), so the
                // guard runs for them in ALL modes (it replaces those overloads); the rest
                // (`abs`, `floor`, …) are the new LOFT_NULLFLOW behaviour.
                if self.first_pass {
                    Type::optional(ret)
                } else {
                    self.wrap_null_transparent(code, types, &ret)
                }
            } else {
                ret
            }
        } else if self.first_pass && !self.default {
            Type::Unknown(0)
        } else if name == "len"
            && types.len() == 1
            && named_args.is_empty()
            && matches!(types[0], Type::Index(_, _, _))
        {
            // P192: `len(ix)` for `ix: index<T[key]>`.  Dispatched
            // here (not via stdlib overload) because the runtime
            // helper `tree::count` needs the per-record bookkeeping
            // byte offset, which is `database.fields(tp)` — only
            // computable at parse time once the type is registered.
            //
            // Strict arity / named-arg gates: `len(ix, x)` or
            // `len(ix, key: 1)` should NOT route here — they would
            // fall through standard dispatch to the existing
            // `Unknown function len` error message which lists the
            // method-style alternative.
            let known = self.get_type(&types[0]);
            let op_d_nr = self.data.def_nr("OpLengthIndex");
            if known != u16::MAX && op_d_nr != u32::MAX {
                let fields = self.database.fields(known);
                let mut args = list.to_vec();
                args.push(Value::Int(i32::from(fields)));
                *code = Value::Call(op_d_nr, args);
                return crate::data::I64.clone();
            }
            // Type or op not registered — drop to the standard
            // error path so the user sees the same diagnostic shape
            // they get for any other unresolved `len()` call.  P07.5
            // adds a "did you mean" suffix when a similarly-named
            // user function exists.
            if let Some(s) = self.suggest_function_name(name) {
                diagnostic_at!(
                    self.lexer,
                    name_pos,
                    Level::Error,
                    "Unknown function {name} — did you mean '{s}'?"
                );
                self.lexer.suggest_last(&s);
            } else {
                diagnostic_at!(
                    self.lexer,
                    name_pos,
                    Level::Error,
                    "Unknown function {name}"
                );
            }
            Type::Unknown(0)
        } else if name == "size"
            && types.len() == 1
            && named_args.is_empty()
            && matches!(types[0], Type::Vector(_, _))
        {
            // @PLN110 1a: `size(v)` for `v: vector<T>` = element count × the
            // element's in-buffer stride (a heap element — text, nested
            // collection, linked struct — counts as its 4-byte record pointer,
            // never the target's content).  Dispatched here (not a stdlib
            // overload) because the stride is type-derived — the SAME
            // `vector_elem_iter_stride` iteration walks the buffer with — and
            // only known at parse time.  Strict arity / named-arg gates mirror
            // `len(ix)` above; `size(v, x)` falls through to the standard error.
            let op_d_nr = self.data.def_nr("OpSizeVector");
            if let Type::Vector(inner, _) = &types[0]
                && op_d_nr != u32::MAX
            {
                let elem = (**inner).clone();
                let stride = self.vector_elem_iter_stride(&elem);
                let mut args = list.to_vec();
                args.push(Value::Int(i32::from(stride)));
                *code = Value::Call(op_d_nr, args);
                return crate::data::I64.clone();
            }
            Type::Unknown(0)
        } else if name == "size"
            && types.len() == 1
            && named_args.is_empty()
            && matches!(types[0], Type::Reference(_, _))
        {
            // @PLN110 1b: `size(s)` for a struct `s` = its packed record size — a
            // compile-time constant (all instances of a struct type share one
            // layout).  Inline fields and inline sub-records count fully; `text` /
            // collection / reference fields count as their 4-byte stored width
            // (allocation-local).  The argument is still evaluated for its side
            // effects (the op consumes it); only its type feeds the const size.
            // A bare enum-variant value (`Circle { r: 2.0 }`) is also a
            // `Type::Reference`, to an `EnumValue` — a struct-like record — so it is
            // handled here too, reporting the variant's own packed record size.  A
            // `Type::Reference` that is NEITHER a struct NOR a variant record is not
            // a valid `size` target: emit the standard error (never a silent
            // `Unknown`, which would leave the call unresolved).
            let known = self.get_type(&types[0]);
            let op_d_nr = self.data.def_nr("OpSizeStruct");
            if known != u16::MAX
                && op_d_nr != u32::MAX
                && (self.database.is_struct(known) || self.database.is_enum_value(known))
            {
                let sz = self.database.size(known);
                let mut args = list.to_vec();
                args.push(Value::Int(i32::from(sz)));
                *code = Value::Call(op_d_nr, args);
                return crate::data::I64.clone();
            }
            diagnostic_at!(
                self.lexer,
                name_pos,
                Level::Error,
                "Unknown function {name}"
            );
            Type::Unknown(0)
        } else if name == "size"
            && types.len() == 1
            && named_args.is_empty()
            && matches!(
                types[0],
                Type::Integer(_) | Type::Float | Type::Boolean | Type::Character | Type::Single
            )
        {
            // @PLN110 1e: `size(x)` for a scalar `x` = its storage width — a
            // compile-time constant (L-Scalar / L-Narrow).  The arg is evaluated
            // for its side effects (every scalar is one 8-byte eval-stack slot,
            // which the op consumes) but only its type feeds the const width.
            // Integers go through the FINISHED type (`get_type` → `database.size`)
            // so a narrow / `forced_size` width (u8 1, u16 2, i32 4) is honoured —
            // `Type::size` reads the value range only and would over-report i32 as
            // 8.  The other scalars have a fixed width via `element_size`
            // (boolean 1, character / single 4, float 8).
            let known = self.get_type(&types[0]);
            let sz: u16 = if matches!(types[0], Type::Integer(_)) && known != u16::MAX {
                self.database.size(known)
            } else {
                crate::data::element_stack_size(&types[0]) as u16
            };
            let op_d_nr = self.data.def_nr("OpSizeScalar");
            if op_d_nr != u32::MAX {
                let mut args = list.to_vec();
                args.push(Value::Int(i32::from(sz)));
                *code = Value::Call(op_d_nr, args);
                return crate::data::I64.clone();
            }
            Type::Unknown(0)
        } else if name == "size"
            && types.len() == 1
            && named_args.is_empty()
            && matches!(types[0], Type::Sorted(_, _, _))
        {
            // @PLN110 1d: a `sorted` shares vector's length-prefixed buffer, so its
            // size is that buffer = element count × the element's in-buffer stride
            // (identical to `size(vector)` — reuse `OpSizeVector`).
            let elem = types[0].content();
            let stride = self.vector_elem_iter_stride(&elem);
            let op_d_nr = self.data.def_nr("OpSizeVector");
            if op_d_nr != u32::MAX {
                let mut args = list.to_vec();
                args.push(Value::Int(i32::from(stride)));
                *code = Value::Call(op_d_nr, args);
                return crate::data::I64.clone();
            }
            Type::Unknown(0)
        } else if name == "size"
            && types.len() == 1
            && named_args.is_empty()
            && matches!(types[0], Type::Index(_, _, _) | Type::Radix(_, _, _))
        {
            // @PLN110 1d: an `index` (red-black tree) / `spatial` (radix/Morton tree)
            // keeps its ordering as bookkeeping embedded IN each element record —
            // there is NO separate structure allocation to sum, so `size` reports a
            // SINGLE node record's size (a compile-time constant), reusing the
            // struct-record path.  The arg is evaluated; only its element type feeds
            // the const record size.
            let elem_kt = match &types[0] {
                Type::Index(tp, _, _) | Type::Radix(tp, _, _) => self.data.def(*tp).known_type(),
                _ => u16::MAX,
            };
            let op_d_nr = self.data.def_nr("OpSizeStruct");
            if elem_kt != u16::MAX && op_d_nr != u32::MAX {
                let sz = self.database.size(elem_kt);
                let mut args = list.to_vec();
                args.push(Value::Int(i32::from(sz)));
                *code = Value::Call(op_d_nr, args);
                return crate::data::I64.clone();
            }
            Type::Unknown(0)
        } else if name == "size"
            && types.len() == 1
            && named_args.is_empty()
            && matches!(types[0], Type::Enum(_, _, _))
        {
            // @PLN110 enums: a SIMPLE enum (no data-carrying variant) is a 1-byte
            // inline discriminant — scalar-like, reuse `OpSizeScalar` (an 8-byte eval
            // slot it consumes); a DATA enum is a `DbRef` to a record (1-byte tag +
            // the max variant's packed fields) — struct-like, reuse `OpSizeStruct`.
            // `database.size` gives the right width either way (1 vs the record size);
            // only the op differs, because the two are delivered differently (inline
            // value vs reference). The arg is evaluated; only its type feeds the const.
            let known = self.get_type(&types[0]);
            if known != u16::MAX {
                let sz = self.database.size(known);
                let op_name = if matches!(types[0], Type::Enum(_, true, _)) {
                    "OpSizeStruct"
                } else {
                    "OpSizeScalar"
                };
                let op_d_nr = self.data.def_nr(op_name);
                if op_d_nr != u32::MAX {
                    let mut args = list.to_vec();
                    args.push(Value::Int(i32::from(sz)));
                    *code = Value::Call(op_d_nr, args);
                    return crate::data::I64.clone();
                }
            }
            Type::Unknown(0)
        } else {
            // generic-specific error for method calls on T.
            if let Some(tv_name) = types.first().and_then(|t| self.generic_type_name(t)) {
                diagnostic_at!(
                    self.lexer,
                    name_pos,
                    Level::Error,
                    "generic type {tv_name}: method call requires a concrete type",
                );
            } else {
                // QUALITY 6c (follow-on): when a free call fails but a method
                // `t_<LEN><Type>_<name>` exists on some other type, tell the
                // user to call it as a method.  Mirror image of the
                // field-access hint that covers the method→free direction.
                // P07.5: when no method receiver is found EITHER, fall back to
                // a similar-name suggestion across all user functions.
                let method_types = self.find_method_receivers(name);
                if method_types.is_empty() {
                    // @PLN13 phase 6 (diagnostics slice): the name may simply be
                    // unimported rather than wrong.  An EXACT hit in a published
                    // package outranks the fuzzy same-name guess below — `rand`
                    // is not a misspelling of a local function, it lives in
                    // `random` — so this is offered first.  Bare calls still do
                    // not resolve; this only replaces a dead end with the two
                    // ways to say what was meant.
                    if let Some(hint) = registry_fn_hint(name) {
                        diagnostic_at!(self.lexer, name_pos, Level::Error, "{hint}");
                    } else if let Some(s) = self.suggest_function_name(name) {
                        diagnostic_at!(
                            self.lexer,
                            name_pos,
                            Level::Error,
                            "Unknown function {name} — did you mean '{s}'?"
                        );
                        self.lexer.suggest_last(&s);
                    } else {
                        diagnostic_at!(
                            self.lexer,
                            name_pos,
                            Level::Error,
                            "Unknown function {name}"
                        );
                    }
                } else {
                    let receivers = method_types.join(" / ");
                    diagnostic_at!(
                        self.lexer,
                        name_pos,
                        Level::Error,
                        "Unknown function {name} — did you mean the method `x.{name}(…)` on {receivers}? (stdlib declared `{name}` as a method; see LOFT.md § Methods and function calls)"
                    );
                }
            }
            Type::Unknown(0)
        }
    }

    /// The constant `f64` value of `v` if it is one — a literal, or (case-C residual) any
    /// expression that `const_eval` reduces to a number, e.g. the call-valued consts `PI` /
    /// `E` (`OpMathPiFloat()` → π). So a math arg / divisor written in terms of `PI` folds
    /// like a literal would.
    #[allow(clippy::cast_precision_loss)]
    fn const_f64(&self, v: Option<&Value>) -> Option<f64> {
        match v?.unspan() {
            Value::Float(f) => Some(*f),
            Value::Single(f) => Some(f64::from(*f)),
            Value::Int(n) => Some(f64::from(*n)),
            Value::Long(n) => Some(*n as f64),
            other => match crate::const_eval::const_eval(other, &self.data) {
                Some(Value::Float(f)) => Some(f),
                Some(Value::Single(f)) => Some(f64::from(f)),
                Some(Value::Int(n)) => Some(f64::from(n)),
                Some(Value::Long(n)) => Some(n as f64),
                _ => None,
            },
        }
    }

    /// @PLN102 Phase 3.5 — is a call to a domain-partial math fn provably IN its real domain,
    /// from CONSTANT arguments alone? Then its result is non-null and the `τ?` elides. The
    /// constant subset of the "provably-fits" elision (variable-arg range-tracking is deferred).
    // These are exact domain-boundary checks, not approximate arithmetic: a `Long` too large to
    // represent exactly in `f64` is trivially outside any bounded domain, and `base != 1.0` is the
    // genuine boundary (a `log` base of exactly 1 is undefined), so the two lints do not apply.
    #[allow(clippy::cast_precision_loss, clippy::float_cmp)]
    fn math_arg_in_domain(&self, name: &str, args: &[Value]) -> bool {
        let a0 = self.const_f64(args.first());
        let a1 = self.const_f64(args.get(1));
        let const_ok = match name {
            "sqrt" => matches!(a0, Some(x) if x >= 0.0),
            "asin" | "acos" => matches!(a0, Some(x) if (-1.0..=1.0).contains(&x)),
            "ln" | "log2" | "log10" => matches!(a0, Some(x) if x > 0.0),
            // `log(x, base)`: x > 0 and a valid base (> 0, ≠ 1).
            "log" => {
                matches!(a0, Some(x) if x > 0.0) && matches!(a1, Some(b) if b > 0.0 && b != 1.0)
            }
            // `pow(base, exp)`: a non-negative base is always defined; a negative base only when
            // the exponent is a whole number (`pow(-2, 3)` = -8, but `pow(-2, 0.5)` = null).
            "pow" => matches!(a0, Some(b) if b >= 0.0) || matches!(a1, Some(e) if e.fract() == 0.0),
            _ => false,
        };
        if const_ok {
            return true;
        }
        // @PLN102 case B (soften-nullflow-discharge.md) — beyond the constant subset, prove the
        // argument is in-domain from an EXPRESSION via the sign lattice (`sqrt(a*a + b*b)`,
        // `sqrt(max(x, 0.01))`). Opt-in until the default-on flip (B5); default-off keeps the
        // surface byte-identical (a narrowed `τ?` would else re-flag `… ?? d` as redundant).
        if !crate::keys::math_domain_enabled() {
            return false;
        }
        let base_sign = args.first().map_or(Sign::Unknown, |v| self.domain_sign(v));
        match name {
            // sqrt / pow-base need arg ≥ 0; ln / log need arg > 0 (strict).
            "sqrt" => matches!(base_sign, Sign::NonNeg | Sign::Pos),
            "ln" | "log2" | "log10" => base_sign == Sign::Pos,
            "pow" => matches!(base_sign, Sign::NonNeg | Sign::Pos),
            // `log(x, base)`: x > 0 (lattice) and a valid CONSTANT base (upper-bound-free lattice
            // can't prove base ≠ 1, so keep base constant).
            "log" => base_sign == Sign::Pos && matches!(a1, Some(b) if b > 0.0 && b != 1.0),
            // asin / acos need a TWO-sided bound [-1, 1] — the interval pass proves it for the
            // real cases (`sin`/`cos` outputs, `clamp(_, -1, 1)`, the `min(max(e,-1),1)` clamp).
            "asin" | "acos" => {
                let (lo, hi) = args
                    .first()
                    .map_or((f64::NEG_INFINITY, f64::INFINITY), |v| self.pm_bounds(v));
                lo >= -1.0 && hi <= 1.0
            }
            _ => false,
        }
    }

    /// @PLN102 case B — a small interval-bounds pass for the TWO-sided `asin`/`acos` domain
    /// `[-1, 1]` (the sign lattice gives only a lower bound). Returns a provable `[lo, hi]`;
    /// `±∞` means unbounded. Only the constructs that actually keep a value in range: constants,
    /// `sin`/`cos` outputs, `clamp(e, lo, hi)` with constant bounds, and `min`/`max` (so the
    /// manual `min(max(e, -1.0), 1.0)` clamp is proved). Everything else → unbounded (matched by
    /// exact stdlib def name; `OpMinFloat` is subtraction, not `min`, and is deliberately unmatched).
    #[allow(clippy::cast_precision_loss, clippy::float_cmp)]
    fn pm_bounds(&self, v: &Value) -> (f64, f64) {
        let open = (f64::NEG_INFINITY, f64::INFINITY);
        match v.unspan() {
            Value::Float(f) => (*f, *f),
            Value::Single(f) => (f64::from(*f), f64::from(*f)),
            Value::Int(n) => (f64::from(*n), f64::from(*n)),
            Value::Long(n) => (*n as f64, *n as f64),
            Value::Call(d, args) => {
                let nm = self.data.def(*d).name.as_str();
                // Unary negation (`-1.0` parses to `OpMinSingleFloat(1.0)`): flip + swap bounds,
                // so a negated literal reaches `clamp`/`min`/`max` as a real constant bound.
                if args.len() == 1 && matches!(nm, "OpMinSingleFloat" | "OpMinSingleSingle") {
                    let (lo, hi) = self.pm_bounds(&args[0]);
                    return (-hi, -lo);
                }
                if matches!(
                    nm,
                    "t_5float_sin" | "t_6single_sin" | "t_5float_cos" | "t_6single_cos"
                ) {
                    return (-1.0, 1.0);
                }
                if args.len() == 3 && matches!(nm, "t_5float_clamp" | "t_6single_clamp") {
                    // clamp(e, lo, hi) ∈ [lo, hi] when lo/hi are constants and lo ≤ hi.
                    let (llo, lhi) = self.pm_bounds(&args[1]);
                    let (hlo, hhi) = self.pm_bounds(&args[2]);
                    if llo == lhi && hlo == hhi && llo <= hhi {
                        return (llo, hhi);
                    }
                    return open;
                }
                if args.len() == 2 && matches!(nm, "t_5float_min" | "t_6single_min") {
                    let (al, ah) = self.pm_bounds(&args[0]);
                    let (bl, bh) = self.pm_bounds(&args[1]);
                    return (al.min(bl), ah.min(bh));
                }
                if args.len() == 2 && matches!(nm, "t_5float_max" | "t_6single_max") {
                    let (al, ah) = self.pm_bounds(&args[0]);
                    let (bl, bh) = self.pm_bounds(&args[1]);
                    return (al.max(bl), ah.max(bh));
                }
                open
            }
            _ => open,
        }
    }

    /// @PLN102 case B — the sign / lower-bound lattice over a PURE float/single expression:
    /// is its value provably `> 0` (`Pos`), `≥ 0` (`NonNeg`), or unknown? Conservative — the
    /// default is `Unknown`, and only exact, sound transfer functions promote (a square is ≥ 0,
    /// a sum of non-negatives is ≥ 0, `abs`/`sqrt` are ≥ 0, `max` takes the stronger bound).
    /// Node kinds are matched by their EXACT stdlib def name (`OpMulFloat`, `t_5float_max`, …),
    /// never a suffix, so a user method can't be mistaken for one. Anything unrecognised → Unknown.
    fn domain_sign(&self, v: &Value) -> Sign {
        fn of_const(x: f64) -> Sign {
            if x > 0.0 {
                Sign::Pos
            } else if x == 0.0 {
                Sign::NonNeg
            } else {
                Sign::Unknown
            }
        }
        match v.unspan() {
            Value::Float(f) => of_const(*f),
            #[allow(clippy::cast_precision_loss)]
            Value::Single(f) => of_const(f64::from(*f)),
            Value::Int(n) => of_const(f64::from(*n)),
            #[allow(clippy::cast_precision_loss)]
            Value::Long(n) => of_const(*n as f64),
            Value::Call(d_nr, args) => {
                let nm = self.data.def(*d_nr).name.as_str();
                if args.len() == 2 && matches!(nm, "OpMulFloat" | "OpMulSingle") {
                    // A square (`a * a`, structurally identical operands) is ≥ 0 regardless of
                    // sign; otherwise combine signs (a non-null float is never the null sentinel,
                    // so no null leaks through a product).
                    if args[0].unspan() == args[1].unspan() {
                        return Sign::NonNeg;
                    }
                    return match (self.domain_sign(&args[0]), self.domain_sign(&args[1])) {
                        (Sign::Pos, Sign::Pos) => Sign::Pos,
                        (Sign::Pos | Sign::NonNeg, Sign::Pos | Sign::NonNeg) => Sign::NonNeg,
                        _ => Sign::Unknown,
                    };
                }
                if args.len() == 2 && matches!(nm, "OpAddFloat" | "OpAddSingle") {
                    return match (self.domain_sign(&args[0]), self.domain_sign(&args[1])) {
                        (Sign::Pos, Sign::Pos | Sign::NonNeg) | (Sign::NonNeg, Sign::Pos) => {
                            Sign::Pos
                        }
                        (Sign::NonNeg, Sign::NonNeg) => Sign::NonNeg,
                        _ => Sign::Unknown,
                    };
                }
                // `abs(x)` and `sqrt(x)` are ≥ 0 by definition (exact stdlib names only).
                if matches!(
                    nm,
                    "t_5float_abs" | "t_6single_abs" | "t_5float_sqrt" | "t_6single_sqrt"
                ) {
                    return Sign::NonNeg;
                }
                // `max(a, b)` ≥ each operand, so its lower bound is the STRONGER of the two.
                if args.len() == 2 && matches!(nm, "t_5float_max" | "t_6single_max") {
                    let (a, b) = (self.domain_sign(&args[0]), self.domain_sign(&args[1]));
                    return if a == Sign::Pos || b == Sign::Pos {
                        Sign::Pos
                    } else if a == Sign::NonNeg || b == Sign::NonNeg {
                        Sign::NonNeg
                    } else {
                        Sign::Unknown
                    };
                }
                // `min(a, b)` ≤ each, so its lower bound is the WEAKER of the two.
                if args.len() == 2 && matches!(nm, "t_5float_min" | "t_6single_min") {
                    let (a, b) = (self.domain_sign(&args[0]), self.domain_sign(&args[1]));
                    return if a == Sign::Unknown || b == Sign::Unknown {
                        Sign::Unknown
                    } else if a == Sign::NonNeg || b == Sign::NonNeg {
                        Sign::NonNeg
                    } else {
                        Sign::Pos
                    };
                }
                Sign::Unknown
            }
            _ => Sign::Unknown,
        }
    }

    /// @PLN102 Phase 5.3 — a NULL-TRANSPARENT stdlib scalar fn: `f(…, null, …) = null`. A nullable
    /// argument propagates to a nullable result. One general list drives the [`Self::wrap_null_transparent`]
    /// guard, replacing the per-function `τ?` overloads. (`sqrt`/`ln`/… already return `τ?`, so they
    /// are not here.)
    fn is_null_transparent(name: &str) -> bool {
        matches!(
            name,
            "abs"
                | "min"
                | "max"
                | "clamp"
                | "floor"
                | "ceil"
                | "round"
                | "sin"
                | "cos"
                | "tan"
                | "atan"
                | "exp"
        )
    }

    /// @PLN102 Phase 5.3 — the GENERAL null-propagation algorithm. A null-transparent call `f(a, b)`
    /// with one or more nullable args becomes `{ t = a; …; if (t not null && …) { f(t, …) } else
    /// { null } }` typed `τ?`. Each nullable arg is evaluated ONCE into a temp (used in both the
    /// null-check and the call). This is the general form of what the `min`/`max` `τ?` overloads did
    /// by hand — floats propagate NaN on their own, but integer `max`/`abs` need this guard, so it
    /// runs uniformly. Second-pass only (the caller returns the `τ?` type in the first pass).
    fn wrap_null_transparent(
        &mut self,
        code: &mut Value,
        arg_types: &[Type],
        ret_base: &Type,
    ) -> Type {
        let opt = Type::optional(ret_base.clone());
        let mut sets: Vec<Value> = Vec::new();
        let mut checks: Vec<(u16, Type)> = Vec::new();
        if let Value::Call(_d, args) = code.unspan_mut() {
            for (i, at) in arg_types.iter().enumerate() {
                if matches!(at, Type::Optional(_)) && i < args.len() {
                    let tmp = self.create_unique("_ntp", at);
                    let orig = std::mem::replace(&mut args[i], Value::Var(tmp));
                    sets.push(crate::data::v_set(tmp, orig));
                    checks.push((tmp, at.base().clone()));
                }
            }
        } else {
            return opt;
        }
        if checks.is_empty() {
            return opt;
        }
        let mut inner = code.clone();
        for (tmp, base) in checks.iter().rev() {
            let mut is_not_null = Value::Var(*tmp);
            self.convert(&mut is_not_null, base, &Type::Boolean);
            let null_arm = self.null(ret_base);
            inner = crate::data::v_if(is_not_null, inner, null_arm);
        }
        sets.push(inner);
        *code = crate::data::v_block(sets, opt.clone(), "null_transparent");
        opt
    }

    /// Scan all definitions for methods named `name` (encoded as
    /// `t_<LEN><TypeName>_<name>`) and return the list of receiver type
    /// names in definition order, de-duplicated.  Powers the 6c
    /// free→method hint in `call`.
    fn find_method_receivers(&self, name: &str) -> Vec<String> {
        let suffix = format!("_{name}");
        let mut receivers: Vec<String> = Vec::new();
        for d_nr in 0..self.data.definitions() {
            let def_name = self.data.def(d_nr).name();
            let Some(rest) = def_name.strip_prefix("t_") else {
                continue;
            };
            if !rest.ends_with(&suffix) {
                continue;
            }
            let digit_end = rest.bytes().take_while(u8::is_ascii_digit).count();
            if digit_end == 0 {
                continue;
            }
            let Ok(type_len) = rest[..digit_end].parse::<usize>() else {
                continue;
            };
            let type_start = digit_end;
            let Some(type_end) = type_start.checked_add(type_len) else {
                continue;
            };
            if rest.len() != type_end + suffix.len() || !rest.is_char_boundary(type_end) {
                continue;
            }
            let type_name = &rest[type_start..type_end];
            if !type_name.is_empty() && !receivers.iter().any(|t| t == type_name) {
                receivers.push(type_name.to_string());
            }
        }
        receivers
    }

    /// Plan-17 phase 01 (A) — predict the substituted return type of a
    /// generic call WITHOUT instantiating the def.  Used on first pass
    /// so the receiving variable's type is set correctly for downstream
    /// inference (e.g. `t = min_max(7, 3)` followed by `t.0`).  Returns
    /// `Type::Unknown(0)` if no prediction is possible (forward decl,
    /// unresolvable type variable, etc.) — caller falls back to the
    /// existing first-pass-Unknown path.
    ///
    /// #395 — route a monomorph's concrete `Type::Tuple` return through the
    /// synthetic `__tuple<…>` struct, exactly as `parser/definitions.rs`'s
    /// `needs_tuple_rewrite` (~line 897) does for a normally-parsed tuple return.
    /// A generic TEMPLATE deliberately defers that rewrite ("`T` resolves later")
    /// and nothing re-applied it once `T` was concrete — so the copied body returns
    /// a DbRef (the template compiled `T` as a `Reference` dummy) while the declared
    /// return stayed `Tuple`, and the caller read the DbRef inline as garbage
    /// (interp) / mismatched the Rust tuple ABI (native E0308).  Both the first-pass
    /// prediction and the second-pass instantiation route through HERE so the
    /// receiving variable's type agrees across passes.  Wide (>8 B) or
    /// lifetime-bearing tuples are rewritten; an 8-byte pure-value tuple and
    /// fn-element tuples keep their existing ABI (mirrors the deferral predicate).
    /// Does `t` mention the type variable `tv_nr` anywhere?  A type variable appears
    /// as a `Reference`/`Enum` to its def; recurses `Vector`/`Optional`/`Tuple`.
    fn type_mentions_tv(t: &Type, tv_nr: u32) -> bool {
        match t {
            Type::Reference(d, _) | Type::Enum(d, _, _) => *d == tv_nr,
            Type::Vector(inner, _) | Type::Optional(inner) => Self::type_mentions_tv(inner, tv_nr),
            Type::Tuple(elems) => elems.iter().any(|e| Self::type_mentions_tv(e, tv_nr)),
            _ => false,
        }
    }

    /// Does the current def's return SHAPE depend on its generic type variable
    /// (`-> T`, `-> (T, T)`, `-> vector<T>`)?  False for a non-generic context and
    /// for a generic template whose return is already CONCRETE (`-> (text, text)`).
    ///
    /// @PLN85 generic-tuple-return-fix.md — the pass-STABLE predicate that lets the
    /// return-promotion chokepoint (the `__tuple` sig rewrite + hidden `__retbuf`
    /// param in `definitions.rs`, the body rewrite + `ref_return` in `block_result`)
    /// run for a concrete-return generic template so the monomorph inherits it.
    /// Keying the guards on this (not on `is_generic_template`) collapses the 4
    /// re-assertion sites onto the one existing non-generic flow.  The type variable
    /// is the template's own (`extract_type_var` of its first attribute) — NOT any
    /// `DefType::Generic` def (that is the FUNCTION, not the type param).
    fn return_shape_depends_on_type_var(&self, t: &Type) -> bool {
        if self.context == u32::MAX || self.data.def_type(self.context) != DefType::Generic {
            return false;
        }
        let attrs = self.data.def(self.context).attributes();
        if attrs.is_empty() {
            return false;
        }
        let tv_nr = Self::extract_type_var(&attrs[0].typedef);
        tv_nr != u32::MAX && Self::type_mentions_tv(t, tv_nr)
    }

    fn tuple_return_rewrite(&mut self, returned: Type, from_type_var: bool) -> Type {
        // Only the `-> T` shape needs this.  When the template return type IS the
        // bare type variable, the body delivers T as a DbRef (the template compiled
        // T as a `Reference` dummy), so a tuple substitution must wrap it in the
        // synthetic struct.  A return type that is a LITERAL tuple in the signature
        // (e.g. `-> (integer, integer)`) is constructed BY VALUE in the body and
        // correctly uses the bare-tuple ABI — rewriting it would break the
        // value-tuple generic returns (p329/p330/p240/plan17).
        if !from_type_var {
            return returned;
        }
        let Type::Tuple(elems) = &returned else {
            return returned;
        };
        let wide = u32::from(crate::variables::size(
            &returned,
            &crate::data::Context::Argument,
        )) > 8;
        let has_fn = elems.iter().any(|e| matches!(e, Type::Function(_, _, _)));
        if elems.iter().any(crate::data::has_lifetime_concern) || (wide && !has_fn) {
            let elems_clone = elems.clone();
            let synth = self.data.tuple_def(&mut self.lexer, &elems_clone);
            Type::Reference(synth, crate::data::Deps::none())
        } else {
            returned
        }
    }

    /// Reads the generic template's already-populated `returned` field, applies
    /// the type substitution, then routes a concrete tuple return through the
    /// synthetic `__tuple` struct (see [`tuple_return_rewrite`]) so the predicted
    /// type matches what `try_generic_instantiation` later produces (else the
    /// receiving variable would "change type" between passes — #395).  Registers
    /// the synthetic struct on first encounter (idempotent via `tuple_def`);
    /// otherwise side-effect-free and safe to call repeatedly.
    fn predict_generic_return_type(&mut self, name: &str, types: &[Type]) -> Type {
        let generic_name = format!("n_{name}");
        let g_nr = self.data.def_nr(&generic_name);
        if g_nr == u32::MAX || self.data.def(g_nr).def_type() != DefType::Generic {
            return Type::Unknown(0);
        }
        if types.is_empty() || types[0].is_unknown() {
            return Type::Unknown(0);
        }
        let tv_nr = Self::extract_type_var(&self.data.def(g_nr).attributes()[0].typedef);
        if tv_nr == u32::MAX {
            return Type::Unknown(0);
        }
        let concrete = Self::resolve_type_var(
            &self.data.def(g_nr).attributes()[0].typedef,
            tv_nr,
            &types[0],
        );
        if concrete.is_unknown() {
            return Type::Unknown(0);
        }
        let tmpl_returned = self.data.definitions[g_nr as usize].returned.clone();
        let from_tv = matches!(&tmpl_returned, Type::Reference(d, _) if *d == tv_nr);
        let predicted = self.tuple_return_rewrite(
            Self::substitute_type(tmpl_returned, tv_nr, &concrete),
            from_tv,
        );
        // Trace point: predicted return type for first-pass type
        // inference of generic call sites.  Used during plan-17 (A)
        // debugging.  Enable with `LOFT_TRACE=generic`.
        crate::loft_trace!(
            generic,
            "predict name={} types={:?} concrete={:?} → {:?}",
            name,
            types,
            concrete,
            predicted,
        );
        predicted
    }

    /// Try to instantiate a generic function template for the given call-site types.
    /// Returns the `def_nr` of the instantiated function, or `u32::MAX` if no generic matches.
    fn try_generic_instantiation(&mut self, name: &str, types: &[Type]) -> u32 {
        let generic_name = format!("n_{name}");
        let g_nr = self.data.def_nr(&generic_name);
        if g_nr == u32::MAX || self.data.def(g_nr).def_type() != DefType::Generic {
            return u32::MAX;
        }
        if types.is_empty() || types[0].is_unknown() {
            // First-pass argument types may be incomplete; defer the diagnostic
            // to second pass when types are stable.  Returning MAX here is the
            // same effect; it just doesn't emit a noisy first-pass error.
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Cannot infer type for generic parameter — provide an explicit type annotation"
                );
            }
            return u32::MAX;
        }
        // Find the type variable def_nr and resolve the concrete type T maps to.
        let tv_nr = Self::extract_type_var(&self.data.def(g_nr).attributes()[0].typedef);
        if tv_nr == u32::MAX {
            return u32::MAX;
        }
        let concrete = Self::resolve_type_var(
            &self.data.def(g_nr).attributes()[0].typedef,
            tv_nr,
            &types[0],
        );
        if concrete.is_unknown() {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Cannot resolve generic type parameter from argument type"
                );
            }
            return u32::MAX;
        }
        // Build the mangled name for the instantiated function.
        let type_nr = self.data.type_def_nr(&concrete);
        let mangled = if type_nr == u32::MAX {
            format!("n_{name}")
        } else {
            // @PLN25 E2 — this mangled name becomes a Rust function identifier in
            // native codegen.  A concrete element type whose NAME carries angle
            // brackets / commas (synthetic wrappers — `__nullable<Row>`,
            // `__tuple<…>`) would emit `fn t_15__nullable<Row>_count(…)`, which
            // rustc parses as a chained comparison.  Flatten those to
            // identifier-safe chars.  The replacement is 1:1 (each bracket/comma
            // → one `_`), so the LEN prefix that `original_name` /
            // `find_method_receivers` parse back stays correct.  Plain names
            // (user structs, `vector`) contain none of these and are unchanged.
            let safe = self
                .data
                .def(type_nr)
                .name()
                .replace(['<', '>', ',', ' '], "_");
            format!("t_{}{}_{name}", safe.len(), safe)
        };
        // Return existing instantiation if already created.
        let existing = self.data.def_nr(&mangled);
        if existing != u32::MAX {
            return existing;
        }
        // Clone the template data before mutating self.data.
        let tmpl_code = self.data.definitions[g_nr as usize].code.clone();
        let tmpl_returned = self.data.definitions[g_nr as usize].returned.clone();
        let tmpl_attrs: Vec<_> = self.data.definitions[g_nr as usize]
            .attributes
            .iter()
            .map(|a| Argument {
                name: a.name.clone(),
                typedef: Self::substitute_type(a.typedef.clone(), tv_nr, &concrete),
                default: a.value.clone(),
                constant: false,
            })
            .collect();
        let tmpl_vars = self.data.definitions[g_nr as usize].variables.clone();
        let tmpl_pos = self.data.definitions[g_nr as usize].position.clone();
        // The per-element iteration stride for vector<T=concrete> — from the
        // ONE home (`vector_elem_iter_stride`), threaded into the fixup so the
        // generic path can never drift from the direct-emission stride again.
        let iter_stride = i32::from(self.vector_elem_iter_stride(&concrete));
        let new_code =
            Self::substitute_type_in_value(tmpl_code, tv_nr, &concrete, iter_stride, &self.data);
        // `from_tv` computed on the PRE-substitution template return, identically to
        // `predict_generic_return_type`, so the second-pass instantiated return type
        // matches the first-pass prediction (the cross-pass H5 contract).
        let from_tv = matches!(&tmpl_returned, Type::Reference(d, _) if *d == tv_nr);
        let new_returned = self.tuple_return_rewrite(
            Self::substitute_type(tmpl_returned, tv_nr, &concrete),
            from_tv,
        );
        // Register the new definition.
        let d_nr = self.data.add_def(&mangled, &tmpl_pos, DefType::Function);
        for a in &tmpl_attrs {
            let a_nr = self
                .data
                .add_attribute(&mut self.lexer, d_nr, &a.name, a.typedef.clone());
            self.data.set_attr_value(d_nr, a_nr, a.default.clone());
        }
        self.data.set_returned(d_nr, new_returned.clone());
        // Trace point: full instantiation result.  Used during plan-17
        // (A) debugging when verifying that the second-pass def
        // creation produced the right monomorphised signature.
        // Enable with `LOFT_TRACE=generic`.
        crate::loft_trace!(
            generic,
            "instantiate name={} mangled={} d_nr={} concrete={:?} returned={:?}",
            name,
            mangled,
            d_nr,
            concrete,
            new_returned,
        );
        // Copy the variable table with substituted types.
        let mut vars = Function::copy(&tmpl_vars);
        vars.substitute_type(tv_nr, &concrete);
        // P241 fix (2026-05-11): post-substitution rewrite of the
        // parametric vector-element-write triplet to the primitive
        // shape, plus elm-var type patch.  Runs after both code
        // substitution AND vars substitution because the patch needs
        // to override `vars`' substituted-to-primitive elm var type
        // back to `Reference(...)` (it holds a DbRef, not the
        // primitive value).  See `rewrite_generic_vector_writes`.
        let new_code = Self::rewrite_generic_vector_writes(
            new_code,
            &concrete,
            &mut vars,
            &self.data,
            &mut self.database,
        );
        self.data.definitions[d_nr as usize].code = new_code;
        self.data.definitions[d_nr as usize].variables = vars;
        // @PLN85 category A — engage the text-return promotion the parse-time
        // path skips for IR-substituted monomorphs, so a `-> text` monomorph
        // delivers through a hidden `&text` buffer (no orphaned owned String).
        // Runs BEFORE returning `d_nr` so the call site sees the promoted ABI.
        self.promote_monomorph_text_return(d_nr);
        // I6: verify the concrete type satisfies every declared bound.
        // Emit a diagnostic and return u32::MAX if any required method is missing.
        if !self.check_satisfaction(g_nr, type_nr) {
            // Return d_nr (not u32::MAX) so `call` doesn't emit a redundant
            // "Unknown function" error — the satisfaction error is sufficient.
            // The function won't execute because parsing will halt on errors.
        }
        d_nr
    }

    /// I6: Check that the concrete type (identified by `concrete_nr`) implements every
    /// interface in `g_nr`'s bounds.  Returns `true` if satisfied (or no bounds),
    /// `false` and emits a diagnostic for the first missing method otherwise.
    fn check_satisfaction(&mut self, g_nr: u32, concrete_nr: u32) -> bool {
        let bounds = self.data.definitions[g_nr as usize].bounds.clone();
        if bounds.is_empty() {
            return true;
        }
        if concrete_nr == u32::MAX {
            return true; // can't check without a concrete type def_nr
        }
        let concrete_name = self.data.def(concrete_nr).name().to_string();
        let mut satisfied = true;
        for iface_nr in bounds {
            let iface_name = self.data.def(iface_nr).name().to_string();
            let children: Vec<u32> = self.data.children_of(iface_nr).collect();
            for child_nr in children {
                let child_name = self.data.def(child_nr).name().to_string();
                // Extract method name from "__iface_{d_nr}_{method}" or legacy "t_4Self_{method}"
                let self_prefix = format!("t_{}Self_", "Self".len());
                let method_suffix = if let Some(rest) = child_name.strip_prefix("__iface_") {
                    rest.split_once('_')
                        .map_or(rest.to_string(), |(_, m)| m.to_string())
                } else if child_name.starts_with(&self_prefix) {
                    child_name[self_prefix.len()..].to_string()
                } else {
                    child_name.clone()
                };
                // I9-prim: use find_fn which checks both the method-style convention
                // (t_7integer_OpLt) and the add_op convention (OpLtInt via possible map).
                let concrete_type = self.data.def(concrete_nr).returned().clone();
                let mut found = self.data.find_fn(u16::MAX, &method_suffix, &concrete_type);
                // @PLN25 E2 — a synth `__nullable<S>` delegates method/interface
                // resolution to its underlying `S` (a method call on a nullable
                // element unwraps through `Some` to call `S`'s method), so satisfy
                // the bound against `S`'s methods when the wrapper itself lacks them.
                // Gate-off-inert (no `__nullable<` type exists).
                if found == u32::MAX
                    && let Some(inner) = concrete_name
                        .strip_prefix("__nullable<")
                        .and_then(|r| r.strip_suffix('>'))
                {
                    let s_nr = self.data.def_nr(inner);
                    if s_nr != u32::MAX {
                        let s_type = self.data.def(s_nr).returned().clone();
                        found = self.data.find_fn(u16::MAX, &method_suffix, &s_type);
                    }
                }
                if found == u32::MAX {
                    let msg = crate::diagnostics::diagnostic_format(
                        Level::Error,
                        format_args!(
                            "'{concrete_name}' does not satisfy interface '{iface_name}': missing {method_suffix}",
                        ),
                    );
                    let peek_pos = self.lexer.peek().position.clone();
                    self.lexer.pos_diagnostic(Level::Error, &peek_pos, &msg);
                    satisfied = false;
                }
            }
        }
        satisfied
    }

    /// Extract the type variable `def_nr` from a type tree.
    /// Returns the `def_nr` of the first `Reference` that refers to the type variable,
    /// or `u32::MAX` if not found.
    fn extract_type_var(tp: &Type) -> u32 {
        match tp {
            Type::Reference(d, _) => *d,
            Type::Vector(inner, _) => Self::extract_type_var(inner),
            _ => u32::MAX,
        }
    }

    /// Unify a template parameter type with a concrete argument type to extract
    /// what the type variable `tv_nr` resolves to.
    /// E.g. template `vector<T>` + concrete `vector<integer>` → `integer`.
    fn resolve_type_var(template_tp: &Type, tv_nr: u32, concrete_tp: &Type) -> Type {
        // `Rewritten(T)` is a value-construction marker (e.g. `P { v: 99 }`
        // becoming an Insert sequence) that should not propagate into the
        // bound T — the type variable describes the data shape, not how
        // a particular argument got assembled.  Strip it before unifying.
        if let Type::Rewritten(inner) = concrete_tp {
            return Self::resolve_type_var(template_tp, tv_nr, inner);
        }
        match template_tp {
            // Same principle as the `Rewritten` strip above: the call-site
            // argument's dep list records what THAT expression borrows in the
            // CALLER's frame — not part of the data shape.  Bound verbatim, it
            // is baked into the instantiated def's attr/return types where the
            // indices are misread as callee attr deps: the instantiation's
            // return then claims to borrow its argument, so the caller never
            // frees the fresh store the method's `__retbuf` delivered (one
            // record leaked per call).
            Type::Reference(d, _) if *d == tv_nr => match concrete_tp {
                Type::Reference(cd, _) => Type::Reference(*cd, crate::data::Deps::none()),
                Type::Vector(inner, _) => Type::Vector(inner.clone(), crate::data::Deps::none()),
                Type::Enum(cd, mixed, _) => Type::Enum(*cd, *mixed, crate::data::Deps::none()),
                Type::Text(_) => Type::Text(crate::data::Deps::none()),
                other => other.clone(),
            },
            Type::Vector(inner, _) => {
                if let Type::Vector(c_inner, _) = concrete_tp {
                    Self::resolve_type_var(inner, tv_nr, c_inner)
                } else {
                    Type::Unknown(0)
                }
            }
            _ => Type::Unknown(0),
        }
    }

    /// Re-resolve a Call target: if the called function's first parameter references
    /// the type variable, look up the correct overload for the concrete type.
    fn re_resolve_call(d_nr: u32, tv_nr: u32, concrete: &Type, data: &Data) -> u32 {
        if d_nr == u32::MAX || d_nr as usize >= data.definitions.len() {
            return d_nr;
        }
        let def = &data.definitions[d_nr as usize];
        if def.attributes().is_empty() {
            return d_nr;
        }
        // Check if any attribute's type references the type variable.
        let has_tv = def.attributes.iter().any(|a| a.typedef.contains_def(tv_nr));
        if !has_tv {
            // Also check for Integer(0, tv_nr) patterns — operators sometimes encode
            // type info in the Integer bounds.
            return d_nr;
        }
        // Resolve the concrete first-arg type by substituting tv_nr in the attribute type.
        let concrete_arg =
            Self::substitute_type(def.attributes()[0].typedef.clone(), tv_nr, concrete);
        // Extract the user-facing function name from the mangled definition name.
        // Mangled names: "t_<LEN><Type>_<name>" or "n_<name>" or operator names.
        let name = def.name();
        let fn_name = if let Some(rest) = name.strip_prefix("t_") {
            // Skip the LEN digits and type name, extract name after the underscore.
            if let Some(idx) = rest.find('_') {
                &rest[idx + 1..]
            } else {
                name
            }
        } else if let Some(rest) = name.strip_prefix("n_") {
            rest
        } else {
            // Operator name — use as-is for find_fn.
            name
        };
        let mut resolved = data.find_fn(u16::MAX, fn_name, &concrete_arg);
        // @PLN25 E2 — a bounded-generic method call whose receiver monomorphises to a synth
        // `__nullable<S>` (a nullable vector element, e.g. `for x in v: vector<T>` where
        // `T = IfItem` → `__nullable<IfItem>`, then `x.is_valid()`) must resolve to S's CONCRETE
        // method via the `Some` payload — the nullable enum itself has no methods, so `find_fn`
        // returns MAX and the parametric bound stub `t_1T_<m>` (emitted `todo!()` in native)
        // would leak to runtime (86).  Mirror the interface-satisfaction unwrap above: retry
        // against S.  Gate-off-inert (no `__nullable<` type exists).
        if resolved == u32::MAX
            && let Type::Enum(nd, true, _) = &concrete_arg
            && data.def(*nd).name().starts_with("__nullable<")
        {
            let some = data.variant_of(*nd, "Some");
            let pa = if some == u32::MAX {
                usize::MAX
            } else {
                data.attr(some, "payload")
            };
            if pa != usize::MAX
                && let Type::Reference(s, _) = data.attr_type(some, pa)
            {
                let s_type = data.def(s).returned().clone();
                resolved = data.find_fn(u16::MAX, fn_name, &s_type);
            }
        }
        if resolved != u32::MAX && resolved != d_nr {
            resolved
        } else {
            d_nr
        }
    }

    /// Substitute all occurrences of `Type::Reference(tv_nr, _)` with `concrete` in a type.
    fn substitute_type(tp: Type, tv_nr: u32, concrete: &Type) -> Type {
        match tp {
            Type::Reference(d, _) if d == tv_nr => concrete.clone(),
            Type::Vector(inner, deps) => Type::Vector(
                Box::new(Self::substitute_type(*inner, tv_nr, concrete)),
                deps,
            ),
            // Plan-17 phase 01 — substitute through tuple element types so a
            // generic `<T: Bound>` returning `(T, T)` (or any tuple shape
            // containing T) monomorphises correctly.  Without this, the
            // signature stayed `(DbRef, DbRef)` (the parametric T form)
            // even when params became `i64`, and native codegen rejected
            // the body's tuple literal with E0308.
            Type::Tuple(elems) => Type::Tuple(
                elems
                    .into_iter()
                    .map(|e| Self::substitute_type(e, tv_nr, concrete))
                    .collect(),
            ),
            // #493 — substitute through an `Optional` wrapper so a generic
            // `<T>` returning `T?` (or with a `T?` param) monomorphises: without
            // this arm `last_element<T>(…) -> T?` kept the parametric
            // `Optional(Reference(tv))` return, typing the return slot as a 12 B
            // DbRef while the body yields the 8 B scalar — a stale/garbage DbRef
            // read on the interpreter (DA `get_stack<DbRef>` OOB) and an E0308 on
            // native.  Mirrors the Vector/Tuple arms above; `Type::optional` is
            // idempotent so it never double-wraps.
            Type::Optional(inner) => Type::optional(Self::substitute_type(*inner, tv_nr, concrete)),
            other => other,
        }
    }

    /// Recursively substitute types in a Value IR tree and re-resolve Call targets
    /// whose first parameter references the type variable.
    /// Walks a generic-template's IR and substitutes the type variable
    /// `tv_nr` with the concrete `concrete` type, both in variable types
    /// and in IR-shape decisions that depend on T's resolved shape.
    fn substitute_type_in_value(
        val: Value,
        tv_nr: u32,
        concrete: &Type,
        iter_stride: i32,
        data: &Data,
    ) -> Value {
        match val {
            Value::Call(d, args) => {
                let new_args: Vec<_> = args
                    .into_iter()
                    .map(|a| Self::substitute_type_in_value(a, tv_nr, concrete, iter_stride, data))
                    .collect();
                // Re-resolve call target if it references the type variable.
                let new_d = Self::re_resolve_call(d, tv_nr, concrete, data);
                // I9-vec: fix vector element access with baked-in elm_size=0.
                // The template bakes elm_size=0 for type-variable elements and omits the
                // value-extraction wrapper (OpGetInt/OpGetFloat/etc.).  Fix both here.
                //
                // P252 fix (2026-05-11): also recognise `OpGetVectorNullable`
                // — plan-07 phase 4 step 4.6 swapped the for-loop iter step
                // from `OpGetVector` to its Nullable peer (so OOB at end-of-
                // iteration returns null instead of raising).  Without this
                // arm, bounded-generic for-loops over a struct vector left
                // the iter step at `OpGetVectorNullable(v, 0, idx)` with
                // size=0 — every iteration read element 0, producing the
                // FIRST item's value for every iteration (P252).  The
                // Nullable peer's arg shape is identical to OpGetVector
                // (r, size, idx) so the elm_size fixup logic is unchanged.
                if new_d != u32::MAX
                    && (new_d as usize) < data.definitions.len()
                    && (data.def(new_d).name() == "OpGetVector"
                        || data.def(new_d).name() == "OpGetVectorNullable")
                    && new_args.len() == 3
                {
                    let cur_size = if let Value::Int(n) = &new_args[1] {
                        *n
                    } else {
                        0
                    };
                    // The stride comes from `vector_elem_iter_stride` (the one
                    // home, computed by the caller) — NOT a re-derived byte-sum;
                    // see that helper for why the two drifted.
                    let elm_size = iter_stride;
                    if elm_size != cur_size {
                        let mut fixed = new_args;
                        fixed[1] = Value::Int(elm_size);
                        let call = Value::Call(new_d, fixed);
                        return Self::wrap_vector_get_val(call, concrete, data);
                    }
                    return Self::wrap_vector_get_val(Value::Call(new_d, new_args), concrete, data);
                }
                // P239 fix (2026-05-11): the for-loop iter-termination
                // check generated by `parser/collections.rs::iter_for`
                // emits `OpConvBoolFromRef(Var(loop_var))` for any
                // loop variable typed `Reference` — including
                // `Reference(T_d_nr, …)` for generic-T element
                // iteration.  When T monomorphises to a primitive
                // (integer / text / float / single / character /
                // enum), the substituted Var is now that primitive
                // type but the IR still has `OpConvBoolFromRef`,
                // which crashes interp (treats `i64` as a `DbRef` —
                // SIGSEGV) and breaks native (rustc E0610 `i64.rec`).
                // Swap the conversion op to the matching primitive
                // peer when the substituted concrete type tells us
                // what shape the value actually is.
                if new_d != u32::MAX
                    && (new_d as usize) < data.definitions.len()
                    && data.def(new_d).name() == "OpConvBoolFromRef"
                    && new_args.len() == 1
                {
                    let conv_name = match concrete.base() {
                        Type::Integer(_) => Some("OpConvBoolFromInt"),
                        Type::Text(_) => Some("OpConvBoolFromText"),
                        Type::Float => Some("OpConvBoolFromFloat"),
                        Type::Single => Some("OpConvBoolFromSingle"),
                        Type::Enum(_, false, _) => Some("OpConvBoolFromEnum"),
                        // Reference / Vector / struct-enum / tuple stay
                        // on OpConvBoolFromRef (the existing behaviour
                        // works for any DbRef-shaped loop variable).
                        _ => None,
                    };
                    if let Some(name) = conv_name {
                        let conv_d_nr = data.def_nr(name);
                        if conv_d_nr != u32::MAX {
                            return Value::Call(conv_d_nr, new_args);
                        }
                    }
                }
                // I9-text fixup: when a T-stub had an extra __work_1 parameter
                // (for text-returning interface methods) but the concrete method
                // doesn't, drop the trailing argument to match the concrete signature.
                if new_d != d && new_d != u32::MAX && (new_d as usize) < data.definitions.len() {
                    let concrete_params = data.def(new_d).attributes().len();
                    if new_args.len() > concrete_params {
                        let mut trimmed = new_args;
                        trimmed.truncate(concrete_params);
                        return Value::Call(new_d, trimmed);
                    }
                }
                Value::Call(new_d, new_args)
            }
            Value::Block(bl) => {
                let recursed: Vec<Value> = bl
                    .operators
                    .into_iter()
                    .map(|v| Self::substitute_type_in_value(v, tv_nr, concrete, iter_stride, data))
                    .collect();
                Value::Block(Box::new(crate::data::Block {
                    operators: recursed,
                    result: Self::substitute_type(bl.result, tv_nr, concrete),
                    name: bl.name,
                    scope: bl.scope,
                    var_size: bl.var_size,
                }))
            }
            Value::Set(v, expr) => Value::Set(
                v,
                Box::new(Self::substitute_type_in_value(
                    *expr,
                    tv_nr,
                    concrete,
                    iter_stride,
                    data,
                )),
            ),
            Value::Return(expr) => Value::Return(Box::new(Self::substitute_type_in_value(
                *expr,
                tv_nr,
                concrete,
                iter_stride,
                data,
            ))),
            Value::If(cond, t, f) => Value::If(
                Box::new(Self::substitute_type_in_value(
                    *cond,
                    tv_nr,
                    concrete,
                    iter_stride,
                    data,
                )),
                Box::new(Self::substitute_type_in_value(
                    *t,
                    tv_nr,
                    concrete,
                    iter_stride,
                    data,
                )),
                Box::new(Self::substitute_type_in_value(
                    *f,
                    tv_nr,
                    concrete,
                    iter_stride,
                    data,
                )),
            ),
            Value::Loop(bl) => Value::Loop(Box::new(crate::data::Block {
                operators: bl
                    .operators
                    .into_iter()
                    .map(|v| Self::substitute_type_in_value(v, tv_nr, concrete, iter_stride, data))
                    .collect(),
                result: Self::substitute_type(bl.result, tv_nr, concrete),
                name: bl.name,
                scope: bl.scope,
                var_size: bl.var_size,
            })),
            Value::Drop(expr) => Value::Drop(Box::new(Self::substitute_type_in_value(
                *expr,
                tv_nr,
                concrete,
                iter_stride,
                data,
            ))),
            Value::Insert(ops) => Value::Insert(
                ops.into_iter()
                    .map(|v| Self::substitute_type_in_value(v, tv_nr, concrete, iter_stride, data))
                    .collect(),
            ),
            Value::Iter(name, create, next, extra) => Value::Iter(
                name,
                Box::new(Self::substitute_type_in_value(
                    *create,
                    tv_nr,
                    concrete,
                    iter_stride,
                    data,
                )),
                Box::new(Self::substitute_type_in_value(
                    *next,
                    tv_nr,
                    concrete,
                    iter_stride,
                    data,
                )),
                Box::new(Self::substitute_type_in_value(
                    *extra,
                    tv_nr,
                    concrete,
                    iter_stride,
                    data,
                )),
            ),
            Value::Span(b) => {
                let (pos, inner) = *b;
                let new_inner =
                    Self::substitute_type_in_value(inner, tv_nr, concrete, iter_stride, data);
                Value::with_span(pos, new_inner)
            }
            // P237: tuple-constructor elements may contain calls to
            // bound-supplied operator stubs (`t_1T_OpAdd(x, x)` etc.).
            // Without this recursion the call stayed pointing at the
            // generic stub instead of the concrete `t_<len>integer_OpAdd`,
            // producing rustc E0308 (`expected DbRef, found i64`) at
            // codegen time and silent garbage / SIGSEGV under interp.
            Value::Tuple(elems) => Value::Tuple(
                elems
                    .into_iter()
                    .map(|e| Self::substitute_type_in_value(e, tv_nr, concrete, iter_stride, data))
                    .collect(),
            ),
            Value::TuplePut(v, idx, val) => Value::TuplePut(
                v,
                idx,
                Box::new(Self::substitute_type_in_value(
                    *val,
                    tv_nr,
                    concrete,
                    iter_stride,
                    data,
                )),
            ),
            Value::BreakWith(n, val) => Value::BreakWith(
                n,
                Box::new(Self::substitute_type_in_value(
                    *val,
                    tv_nr,
                    concrete,
                    iter_stride,
                    data,
                )),
            ),
            Value::Yield(val) => Value::Yield(Box::new(Self::substitute_type_in_value(
                *val,
                tv_nr,
                concrete,
                iter_stride,
                data,
            ))),
            Value::CallRef(v_nr, args) => Value::CallRef(
                v_nr,
                args.into_iter()
                    .map(|a| Self::substitute_type_in_value(a, tv_nr, concrete, iter_stride, data))
                    .collect(),
            ),
            other => other,
        }
    }

    /// P241 fix (2026-05-11) — slice 2: integer-only.  POST-PASS that
    /// walks the substituted IR + patches the variable table to
    /// rewrite the parametric vector-element-write triplet to its
    /// primitive shape.  Runs AFTER `substitute_type_in_value` AND
    /// AFTER `vars.substitute_type` so it sees the substituted-types
    /// IR and can patch the elm variable's type back to `Reference`
    /// (it was originally `Reference(T_d_nr, deps)` and
    /// `vars.substitute_type` turned it into the wrong primitive
    /// type because it holds a DbRef, not a primitive value).
    ///
    /// Detects the post-substitution triplet:
    ///
    /// ```ignore
    /// // Triplet at adjacent positions i, i+1, i+2:
    /// Set(elm_var, Call(OpNewRecord, [Var(out_var), Int(t_T), Int(MAX)]))
    /// Call(OpCopyRecord, [src_value, Var(elm_var), Int(t_T)])
    /// Call(OpFinishRecord, [Var(out_var), Var(elm_var), Int(t_T), Int(MAX)])
    /// ```
    ///
    /// When `concrete` is a primitive (slice 2: only `Type::Integer`),
    /// rewrites the triplet to the primitive shape:
    ///
    /// ```ignore
    /// // 4-op sequence:
    /// Call(OpPreAllocVector, [Var(out_var), Int(1), Int(elem_size)])
    /// Set(elm_var, Call(OpNewRecord, [Var(out_var), Int(t_concrete_vec), Int(MAX)]))
    /// Call(OpSetInt, [Var(elm_var), Int(0), src_value])
    /// Call(OpFinishRecord, [Var(out_var), Var(elm_var), Int(t_concrete_vec), Int(MAX)])
    /// ```
    ///
    /// AND patches `vars.set_type(elm_var, Type::Reference(...))`.
    ///
    /// Recurses into nested Blocks / Loops / Ifs / etc.  Slice 2 ships
    /// only `Type::Integer`; struct-T is a no-op (the existing
    /// OpCopyRecord path is correct because the source IS a DbRef).
    /// P241 fix slice 3 — true when `concrete` is a primitive type
    /// whose vector-element write uses a primitive setter
    /// (`OpSetInt` / `OpSetText` / `OpSetFloat` / etc.) at the
    /// parse-time concrete-T path instead of `OpCopyRecord`.
    /// `Type::Reference` / `Type::Vector` / etc. are NOT primitive
    /// targets — their vector-element shape uses `OpCopyRecord`,
    /// which the parametric IR already encodes correctly (the source
    /// IS a DbRef regardless of substitution).
    pub(crate) fn is_primitive_vector_element_target(tp: &Type) -> bool {
        matches!(
            tp,
            Type::Integer(_)
                | Type::Float
                | Type::Single
                | Type::Boolean
                | Type::Character
                | Type::Text(_)
                | Type::Function(_, _, _)
                | Type::Enum(_, false, _) // plain enum (struct-enums use OpCopyRecord)
        )
    }

    /// Does the rewrite apply to this concrete type?  Both primitive and
    /// struct (Reference) targets need the type-id substitution; the only
    /// difference is whether OpCopyRecord is replaced (primitive) or kept
    /// with patched type-id args (struct).
    pub(crate) fn is_rewritable_vector_element_target(tp: &Type) -> bool {
        Self::is_primitive_vector_element_target(tp) || matches!(tp, Type::Reference(_, _))
    }

    /// P241 fix slice 3 — build the per-type primitive setter Call
    /// for the rewritten triplet's middle op.  Mirrors the parse-time
    /// concrete-T dispatch in `parser/vectors.rs:1560-1599`.
    /// Returns `None` only when the type is not a primitive target
    /// (the caller should pre-check via `is_primitive_vector_element_target`).
    fn primitive_setter_call(
        concrete: &Type,
        elm_var: u16,
        src_value: Value,
        data: &Data,
    ) -> Option<Value> {
        let elm = Value::Var(elm_var);
        let pos = Value::Int(0);
        // Resolve op def_nrs.  Each branch resolves only the ones it needs.
        let op = match concrete {
            Type::Integer(spec) => {
                // narrow-int dispatch mirrors `vectors.rs:1576-1586`.
                // size(N) on an integer alias selects a narrower setter.
                let alias_nr = data.type_elm(concrete);
                let forced = data.forced_size(alias_nr);
                match forced {
                    Some(1) => {
                        let m = Value::Int(spec.min);
                        let d = data.def_nr("OpSetByte");
                        Value::Call(d, vec![elm, pos, m, src_value])
                    }
                    Some(2) => {
                        let m = Value::Int(spec.min);
                        let d = data.def_nr("OpSetShortRaw");
                        Value::Call(d, vec![elm, pos, m, src_value])
                    }
                    Some(4) => {
                        let d = data.def_nr("OpSetInt4");
                        Value::Call(d, vec![elm, pos, src_value])
                    }
                    _ => {
                        let d = data.def_nr("OpSetInt");
                        Value::Call(d, vec![elm, pos, src_value])
                    }
                }
            }
            Type::Float => {
                let d = data.def_nr("OpSetFloat");
                Value::Call(d, vec![elm, pos, src_value])
            }
            Type::Single => {
                let d = data.def_nr("OpSetSingle");
                Value::Call(d, vec![elm, pos, src_value])
            }
            Type::Boolean => {
                // Booleans store as a 0/1 byte; same shape as `set_field`'s
                // Boolean arm at `parser/mod.rs:2799`.  Must wrap the
                // raw boolean value in `if val { 1 } else { 0 }` so the
                // OpSetByte writes the correct integer encoding —
                // without the wrap, OpSetByte sees a DbRef-shaped
                // boolean that codegen can't decode.
                let d = data.def_nr("OpSetByte");
                let wrapped = crate::data::v_if(src_value, Value::Int(1), Value::Int(0));
                Value::Call(d, vec![elm, pos, Value::Int(0), wrapped])
            }
            Type::Character => {
                let d = data.def_nr("OpSetCharacter");
                Value::Call(d, vec![elm, pos, src_value])
            }
            Type::Text(_) => {
                let d = data.def_nr("OpSetText");
                Value::Call(d, vec![elm, pos, src_value])
            }
            Type::Function(_, _, _) => {
                // Plan-06 phase 4d.A.2 — fn-ref vector elements store the
                // 4-byte i32 d_nr.  Same shape as `vectors.rs:1597`.
                let d = data.def_nr("OpSetInt4");
                Value::Call(d, vec![elm, pos, src_value])
            }
            Type::Enum(_, false, _) => {
                // Plain enum — variants encode as a small integer index.
                let d = data.def_nr("OpSetEnum");
                Value::Call(d, vec![elm, pos, src_value])
            }
            _ => return None,
        };
        // Defensive: if any def_nr lookup returned u32::MAX, return None
        // so the caller falls through (rather than emitting a malformed Call).
        if let Value::Call(d, _) = &op
            && *d == u32::MAX
        {
            return None;
        }
        Some(op)
    }

    pub(crate) fn rewrite_generic_vector_writes(
        val: Value,
        concrete: &Type,
        vars: &mut crate::variables::Function,
        data: &Data,
        database: &mut Stores,
    ) -> Value {
        // Slice 2: Integer.  Slice 3: extended to all primitive
        // types whose vector-element shape uses a primitive setter
        // instead of OpCopyRecord.  P255 (2026-05-12): also handle
        // struct T (Type::Reference) — the OpCopyRecord shape is
        // kept but its tp arg AND the surrounding OpNewRecord /
        // OpFinishRecord parent_tp args must be patched from the
        // parametric T type-id to the concrete struct's type-ids,
        // otherwise the runtime reads the wrong record size.
        if !Self::is_rewritable_vector_element_target(concrete) {
            return val;
        }
        match val {
            Value::Block(bl) => {
                let recursed: Vec<Value> = bl
                    .operators
                    .into_iter()
                    .map(|v| Self::rewrite_generic_vector_writes(v, concrete, vars, data, database))
                    .collect();
                let rewritten =
                    Self::rewrite_vector_write_triplets(recursed, concrete, vars, data, database);
                Value::Block(Box::new(crate::data::Block {
                    operators: rewritten,
                    result: bl.result,
                    name: bl.name,
                    scope: bl.scope,
                    var_size: bl.var_size,
                }))
            }
            Value::Loop(lp) => {
                let recursed: Vec<Value> = lp
                    .operators
                    .into_iter()
                    .map(|v| Self::rewrite_generic_vector_writes(v, concrete, vars, data, database))
                    .collect();
                let rewritten =
                    Self::rewrite_vector_write_triplets(recursed, concrete, vars, data, database);
                Value::Loop(Box::new(crate::data::Block {
                    operators: rewritten,
                    result: lp.result,
                    name: lp.name,
                    scope: lp.scope,
                    var_size: lp.var_size,
                }))
            }
            Value::If(c, t, f) => Value::If(
                Box::new(Self::rewrite_generic_vector_writes(
                    *c, concrete, vars, data, database,
                )),
                Box::new(Self::rewrite_generic_vector_writes(
                    *t, concrete, vars, data, database,
                )),
                Box::new(Self::rewrite_generic_vector_writes(
                    *f, concrete, vars, data, database,
                )),
            ),
            Value::Set(v, expr) => Value::Set(
                v,
                Box::new(Self::rewrite_generic_vector_writes(
                    *expr, concrete, vars, data, database,
                )),
            ),
            Value::Return(expr) => Value::Return(Box::new(Self::rewrite_generic_vector_writes(
                *expr, concrete, vars, data, database,
            ))),
            Value::Drop(expr) => Value::Drop(Box::new(Self::rewrite_generic_vector_writes(
                *expr, concrete, vars, data, database,
            ))),
            Value::Span(b) => {
                let (pos, inner) = *b;
                Value::Span(Box::new((
                    pos,
                    Self::rewrite_generic_vector_writes(inner, concrete, vars, data, database),
                )))
            }
            Value::Call(d, args) => Value::Call(
                d,
                args.into_iter()
                    .map(|a| Self::rewrite_generic_vector_writes(a, concrete, vars, data, database))
                    .collect(),
            ),
            Value::Insert(ops) => Value::Insert(
                ops.into_iter()
                    .map(|v| Self::rewrite_generic_vector_writes(v, concrete, vars, data, database))
                    .collect(),
            ),
            other => other,
        }
    }

    /// P241 fix (2026-05-11) — slice 2: integer-only.  Walks a Block's
    /// post-substitution operator list looking for the parametric
    /// vector-element-write triplet emitted by
    /// `parser/vectors.rs::new_record` (for parametric T):
    ///
    /// ```ignore
    /// // Triplet at adjacent positions i, i+1, i+2:
    /// Set(elm_var, Call(OpNewRecord, [Var(out_var), Int(t_T), Int(MAX)]))
    /// Call(OpCopyRecord, [src_value, Var(elm_var), Int(t_T_known)])
    /// Call(OpFinishRecord, [Var(out_var), Var(elm_var), Int(t_T), Int(MAX)])
    /// ```
    ///
    /// When `concrete` is a primitive (slice 2: only `Type::Integer`),
    /// rewrites the triplet to the primitive shape:
    ///
    /// ```ignore
    /// // 4-op sequence:
    /// Call(OpPreAllocVector, [Var(out_var), Int(1), Int(elem_size)])
    /// Set(elm_var, Call(OpNewRecord, [Var(out_var), Int(t_concrete_vec), Int(MAX)]))
    /// Call(OpSetInt, [Var(elm_var), Int(0), src_value])
    /// Call(OpFinishRecord, [Var(out_var), Var(elm_var), Int(t_concrete_vec), Int(MAX)])
    /// ```
    ///
    /// Where:
    /// - `elem_size = type_element_size(concrete, data)`
    /// - `t_concrete_vec = database.vector(database.db_type(concrete, data))`
    ///   (mirrors the parse-time concrete path at `vectors.rs:1532-1535`).
    ///
    /// Slice 2 ships only the `Type::Integer` arm.  Slice 3 extends to
    /// Text / Float / Single / Boolean / Character / Enum / Function +
    /// narrow-int variants.  For struct T (the `Type::Reference` shape)
    /// the rewrite is a no-op — the existing OpCopyRecord path is
    /// correct because the source IS a DbRef.
    ///
    /// Tolerates `Value::Span` and `Value::Line` markers between or
    /// inside the triplet operators by using `unspan` for shape
    /// matching.  Multi-element pushes (`out += [a, b, c]`) produce
    /// N adjacent triplets; this function processes them
    /// independently — each triplet rewrites once.
    fn rewrite_vector_write_triplets(
        ops: Vec<Value>,
        concrete: &Type,
        vars: &mut crate::variables::Function,
        data: &Data,
        database: &mut Stores,
    ) -> Vec<Value> {
        // Slice 3 covers all primitive vector-element targets
        // (Integer/Text/Float/Single/Boolean/Character/Enum/Function +
        // narrow-int variants).  P255 extends to struct T (Reference)
        // by keeping OpCopyRecord and patching its tp arg.
        if !Self::is_rewritable_vector_element_target(concrete) {
            return ops;
        }
        let is_struct_target = matches!(concrete, Type::Reference(_, _));
        // Resolve op def_nrs once — re-resolution per-triplet would
        // cost N lookups for a long Block.
        let new_record_d = data.def_nr("OpNewRecord");
        let copy_record_d = data.def_nr("OpCopyRecord");
        let finish_record_d = data.def_nr("OpFinishRecord");
        let pre_alloc_d = data.def_nr("OpPreAllocVector");
        if new_record_d == u32::MAX
            || copy_record_d == u32::MAX
            || finish_record_d == u32::MAX
            || pre_alloc_d == u32::MAX
        {
            return ops; // missing op definitions — bail safely
        }
        // Look up the concrete vector-element record type-id.
        // Mirrors `vectors.rs:1532-1535` — `database.vector(content_db_type)`
        // returns the synthetic vector<concrete> type id (registers
        // it on first use; idempotent on subsequent calls).
        let content_db_type = database.db_type(concrete, data);
        let concrete_vec_tp = i32::from(database.vector(content_db_type));
        let elem_size = Self::type_element_size(concrete, data);
        // Walk operators looking for the triplet.  Build a new vec
        // with rewrites applied; copy unchanged ops verbatim.
        // Drain `ops` into a deque-like cursor so we can take owned
        // values without index juggling — rewriting consumes 3 ops
        // and produces 4, so we can't use simple in-place mutation.
        let mut iter = ops.into_iter();
        let mut out: Vec<Value> = Vec::new();
        let mut buf: Vec<Value> = Vec::new();
        loop {
            // Refill buffer to at least 3 entries (the triplet length).
            while buf.len() < 3 {
                match iter.next() {
                    Some(v) => buf.push(v),
                    None => break,
                }
            }
            if buf.len() < 3 {
                // Tail — nothing left that could be a full triplet.
                out.extend(buf);
                break;
            }
            let matched = Self::match_vector_write_triplet(
                &buf[0],
                &buf[1],
                &buf[2],
                new_record_d,
                copy_record_d,
                finish_record_d,
            );
            if let Some((elm_var, out_var, src_value)) = matched {
                // Consume the matched triplet.
                buf.drain(0..3);
                // Patch the elm var's type back to `Reference(content_def_nr, [out_var])`.
                // After `vars.substitute_type`, `Reference(T_d_nr, deps)` became
                // `Type::<concrete>` (deps lost — primitive types don't carry deps).
                // But the elm var is the destination of `OpNewRecord`, so it
                // holds a DbRef regardless of T's concrete type.  Without this
                // patch, codegen emits the wrong opcodes for elm reads/writes
                // (e.g. `VarInt` instead of `VarRef`) and the runtime reads
                // garbage.  The dep on `out_var` mirrors `unique_elm_var`'s
                // `self.vars.depend(elm, vec)` so elm doesn't outlive the
                // backing store.
                let content_def_nr = data.type_def_nr(concrete);
                vars.set_type(
                    elm_var,
                    Type::Reference(content_def_nr, Deps::frame1(out_var)),
                );
                // 1. OpPreAllocVector(Var(out_var), Int(1), Int(elem_size))
                //    Mirrors `vectors.rs:1161-1178` for perf parity with
                //    concrete-T vector pushes.
                out.push(Value::Call(
                    pre_alloc_d,
                    vec![Value::Var(out_var), Value::Int(1), Value::Int(elem_size)],
                ));
                // 2. Set(elm_var, OpNewRecord(Var(out_var), Int(concrete_vec_tp), Int(MAX)))
                out.push(Value::Set(
                    elm_var,
                    Box::new(Value::Call(
                        new_record_d,
                        vec![
                            Value::Var(out_var),
                            Value::Int(concrete_vec_tp),
                            Value::Int(i32::from(u16::MAX)),
                        ],
                    )),
                ));
                // 3. Middle op:
                //    - Primitive T: per-type setter (OpSetInt / OpSetText / …)
                //      replaces OpCopyRecord (no DbRef to copy).
                //    - Struct T: keep OpCopyRecord but patch its tp arg
                //      from the parametric T's known_type to the concrete
                //      struct's known_type so `state::copy_record` reads
                //      the correct record size.
                if is_struct_target {
                    let known_tp = if (content_def_nr as usize) < data.definitions.len() {
                        i32::from(data.def(content_def_nr).known_type())
                    } else {
                        i32::from(u16::MAX)
                    };
                    out.push(Value::Call(
                        copy_record_d,
                        vec![src_value, Value::Var(elm_var), Value::Int(known_tp)],
                    ));
                } else {
                    let setter = Self::primitive_setter_call(concrete, elm_var, src_value, data);
                    if let Some(call) = setter {
                        out.push(call);
                    } else {
                        // Concrete type matched `is_primitive_vector_element_target`
                        // but no setter mapping exists — should not happen.
                        // Bail by emitting a no-op (the OpFinishRecord still
                        // fires; the value just isn't written).  Defensive
                        // fall-through to keep the IR well-formed.
                    }
                }
                // 4. OpFinishRecord(Var(out_var), Var(elm_var), Int(concrete_vec_tp), Int(MAX))
                out.push(Value::Call(
                    finish_record_d,
                    vec![
                        Value::Var(out_var),
                        Value::Var(elm_var),
                        Value::Int(concrete_vec_tp),
                        Value::Int(i32::from(u16::MAX)),
                    ],
                ));
            } else {
                // No triplet at this position — emit one op and slide.
                out.push(buf.remove(0));
            }
        }
        out
    }

    /// P241 fix slice 2 — pattern-matches the parametric vector-element
    /// write triplet.  Returns `Some((elm_var, out_var, src_value))`
    /// when all three statements match the expected shape AND the
    /// per-statement vars cross-reference correctly:
    /// - The Set's target var equals the OpCopyRecord's 2nd arg's
    ///   var equals the OpFinishRecord's 2nd arg's var (`elm_var`).
    /// - The OpNewRecord's 1st arg's var equals the OpFinishRecord's
    ///   1st arg's var (`out_var`).
    /// - All Int args match expected sentinel shape (Int(MAX) for
    ///   `fld`; matching parametric type-id between OpNewRecord and
    ///   OpFinishRecord).
    ///
    /// `src_value` is the OpCopyRecord's 1st arg (the value being
    /// copied into the new vector slot).  Returned by-value (cloned
    /// from the matched IR) so the caller can construct the new
    /// `OpSetInt` Call without borrowing back into `ops`.
    fn match_vector_write_triplet(
        op0: &Value,
        op1: &Value,
        op2: &Value,
        new_record_d: u32,
        copy_record_d: u32,
        finish_record_d: u32,
    ) -> Option<(u16, u16, Value)> {
        // op0: Set(elm_var, Call(OpNewRecord, [Var(out_var), Int(_), Int(MAX)]))
        let (elm_var_set, out_var, _new_record_tp) = match op0.unspan() {
            Value::Set(elm, set_val) => {
                let inner = set_val.unspan();
                if let Value::Call(d, args) = inner
                    && *d == new_record_d
                    && args.len() == 3
                    && let Value::Var(out) = args[0].unspan()
                    && let Value::Int(tp) = args[1].unspan()
                    && let Value::Int(fld) = args[2].unspan()
                    && *fld == i32::from(u16::MAX)
                {
                    (*elm, *out, *tp)
                } else {
                    return None;
                }
            }
            _ => return None,
        };
        // op1: Call(OpCopyRecord, [src_value, Var(elm_var), Int(_)])
        let src_value = match op1.unspan() {
            Value::Call(d, args) => {
                if *d == copy_record_d
                    && args.len() == 3
                    && let Value::Var(elm_in_copy) = args[1].unspan()
                    && *elm_in_copy == elm_var_set
                {
                    args[0].clone()
                } else {
                    return None;
                }
            }
            _ => return None,
        };
        // op2: Call(OpFinishRecord, [Var(out_var), Var(elm_var), Int(_), Int(MAX)])
        match op2.unspan() {
            Value::Call(d, args) => {
                if *d == finish_record_d
                    && args.len() == 4
                    && let Value::Var(out_in_finish) = args[0].unspan()
                    && let Value::Var(elm_in_finish) = args[1].unspan()
                    && let Value::Int(_finish_tp) = args[2].unspan()
                    && let Value::Int(fld) = args[3].unspan()
                    && *out_in_finish == out_var
                    && *elm_in_finish == elm_var_set
                    && *fld == i32::from(u16::MAX)
                {
                    Some((elm_var_set, out_var, src_value))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// I9-vec: compute element store size from the Type alone (no database needed).
    fn type_element_size(tp: &Type, data: &Data) -> i32 {
        // @PLN25: `Optional(τ)` stores at its base's width (sentinel storage); peel so a
        // nullable narrow-int / scalar element gets its real stride, not the `_ => 12` DbRef.
        let tp = tp.base();
        // Post-2c: honor size(N) on integer aliases.
        if matches!(tp, Type::Integer(_)) {
            let alias_nr = data.type_elm(tp);
            if let Some(n) = data.forced_size(alias_nr) {
                return i32::from(n);
            }
        }
        match tp {
            Type::Single
            | Type::Boolean
            | Type::Character
            | Type::Text(_)
            | Type::Enum(_, false, _) => 4,
            Type::Integer(_) | Type::Float => 8,
            // for Reference(struct_nr), compute the struct's inline field
            // size from its attributes rather than assuming 12 (DbRef size).
            // Vector elements of struct type are stored inline, not as pointers.
            Type::Reference(d_nr, _) => {
                if (*d_nr as usize) < data.definitions.len()
                    && data.def(*d_nr).def_type() == DefType::Struct
                {
                    let mut total = 0i32;
                    for attr in data.def(*d_nr).attributes() {
                        if attr.constant {
                            continue;
                        }
                        total += Self::type_element_size(&attr.typedef, data);
                    }
                    if total > 0 {
                        return total;
                    }
                }
                12 // non-struct reference: DbRef = 12 bytes
            }
            _ => 12,
        }
    }

    /// I9-vec: wrap an `OpGetVector` result with the appropriate value-extraction op
    /// for concrete value types (`OpGetInt`, `OpGetFloat`, etc.).  Reference types need
    /// no wrapper — the `DbRef` IS the value.
    fn wrap_vector_get_val(code: Value, tp: &Type, data: &Data) -> Value {
        let p = Value::Int(0);
        // @PLN25: peel `Optional(τ)` — a nullable scalar element needs the SAME value-
        // extraction op as its base; without this it fell to `_ => return code` (no OpGet)
        // and the raw slot was read as a DbRef.
        let op_name = match tp.base() {
            Type::Integer(_) => "OpGetInt",
            Type::Float => "OpGetFloat",
            Type::Single => "OpGetSingle",
            Type::Text(_) => "OpGetText",
            // @PLN17: byte-stored boolean read, preserving 0/1/255 (like enum).
            Type::Boolean => "OpGetBoolean",
            _ => return code, // reference/struct types: no wrapper needed
        };
        let d = data.def_nr(op_name);
        if d == u32::MAX {
            return code;
        }
        Value::Call(d, vec![code, p])
    }

    // @F17 — resolve named args into positional slots + fill omitted defaults
    /// Resolve named arguments into positional slots, then delegate to `call_nr`.
    #[allow(clippy::too_many_arguments)]
    fn call_with_named(
        &mut self,
        code: &mut Value,
        d_nr: u32,
        positional: &[Value],
        pos_types: &[Type],
        named: &[(String, Value, Type)],
        is_method: bool,
        arg_pos: &[Position],
        // The call NAME's position, forwarded to `call_nr` for the arc-C steer caret.
        name_pos: Option<&Position>,
    ) -> Type {
        if named.is_empty() {
            return self.call_nr(
                code, d_nr, positional, pos_types, is_method, arg_pos, name_pos,
            );
        }
        // Build full argument vector with named args placed at the correct indices.
        let n_params = self.data.attributes(d_nr);
        let mut args = vec![Value::Null; n_params];
        let mut arg_types = vec![Type::Unknown(0); n_params];
        // Place positional args first.
        for (i, (val, tp)) in positional.iter().zip(pos_types.iter()).enumerate() {
            if i < n_params {
                args[i] = val.clone();
                arg_types[i] = tp.clone();
            }
        }
        let pos_count = positional.len();
        // Place named args by looking up parameter names.
        for (name, val, tp) in named {
            let idx = self.data.attr(d_nr, name);
            if idx == usize::MAX {
                if !self.first_pass {
                    diagnostic!(self.lexer, Level::Error, "Unknown parameter '{name}'");
                }
                continue;
            }
            if idx < pos_count {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Parameter '{name}' already provided as positional argument {idx}"
                    );
                }
                continue;
            }
            if args[idx] != Value::Null {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Duplicate named argument '{name}'"
                    );
                }
                continue;
            }
            args[idx] = val.clone();
            arg_types[idx] = tp.clone();
        }
        // Trim trailing Null args — add_defaults will fill them.
        let mut last_provided = args.len();
        while last_provided > 0 && args[last_provided - 1] == Value::Null {
            last_provided -= 1;
        }
        args.truncate(last_provided);
        arg_types.truncate(last_provided);
        // Named args are reordered into parameter slots, so the parse-order
        // `arg_pos` no longer aligns; fall back to the cursor for the rare
        // named-mismatch case.
        self.call_nr(code, d_nr, &args, &arg_types, is_method, &[], name_pos)
    }

    fn single_op(&mut self, op: &str, f: Value, t: Type) -> Value {
        let mut code = Value::Null;
        self.call_op(&mut code, op, &[f], &[t]);
        code
    }

    fn conv_op(&mut self, op: &str, f: Value, n: Value, f_tp: Type, n_tp: Type) -> Value {
        let mut code = Value::Null;
        self.call_op(&mut code, op, &[f, n], &[f_tp, n_tp]);
        code
    }

    fn op(&mut self, op: &str, f: Value, n: Value, t: Type) -> Value {
        let mut code = Value::Null;
        self.call_op(&mut code, op, &[f, n], &[t.clone(), t]);
        code
    }

    fn get_field(&mut self, d_nr: u32, f_nr: usize, code: Value) -> Value {
        // #91: track $.<field> accesses during init(expr) parsing.
        if self.init_field_tracking && code == Value::Var(0) && f_nr != usize::MAX {
            let name = self.data.attr_name(d_nr, f_nr);
            if !self.init_field_deps.contains(&name) {
                self.init_field_deps.push(name);
            }
        }
        let tp = self.data.attr_type(d_nr, f_nr);
        let nullable = self.data.attr_nullable(d_nr, f_nr);
        self.expr_not_null = !nullable;
        if !nullable && f_nr != usize::MAX {
            self.expr_not_null_name = self.data.attr_name(d_nr, f_nr);
        } else {
            self.expr_not_null_name.clear();
        }
        let pos = if f_nr == usize::MAX {
            0
        } else {
            let nm = self.data.attr_name(d_nr, f_nr);
            self.database
                .position(self.data.def(d_nr).known_type(), &nm)
        };
        // Post-2c: pass the field's alias def_nr so `get_val` can honor
        // size(N) for integer subtypes (e.g. i32 → OpGetInt4).
        let alias = if f_nr == usize::MAX {
            u32::MAX
        } else {
            self.data.def(d_nr).attributes()[f_nr].alias_d_nr
        };
        // P215: for fn-ref fields with the legacy 4B int layout, the
        // database has no `<attr>__closure_rec` half — reading at
        // pos+4 would corrupt the next attribute.  Synthesise the
        // null DbRef sentinel for the closure half instead.  The 8B
        // split layout (assigned by a capturing lambda) keeps the
        // existing dual-read path.  The split/legacy answer comes
        // from `fn_ref_field_is_split` (the database layout), NOT
        // from `assigned_lambda_d_nr` directly — the flag is only
        // set when the assigning body parses, so a body parsed
        // earlier would wrongly see the legacy layout (#313).
        if let Type::Function(_, _, _) = &tp
            && f_nr != usize::MAX
        {
            // @PLN114 — the layout decides the reader, and BOTH answers are now
            // explicit: a split field reads its closure_rec child, a legacy one
            // synthesises a NULL closure.  `get_val`'s Function arm is the legacy
            // read (tuple / vector elements), so a split field must not fall
            // through to it.
            return if self.fn_ref_field_is_split(d_nr, f_nr) {
                self.read_fn_ref_split(&tp, u32::from(pos), code)
            } else {
                let read_dnr = self.cl("OpGetInt4", &[code, Value::Int(i32::from(pos))]);
                let read_clos = self.cl("OpNullRefSentinel", &[]);
                crate::data::v_block(vec![read_dnr, read_clos], tp.clone(), "fn_ref_field_read")
            };
        }
        self.get_val(&tp, nullable, u32::from(pos), code, alias)
    }

    /// #322: switch the lexer to a `use`-resolved dependency file,
    /// recording it for the program startup-cache manifest first.
    /// Library files are parsed via an inline lexer switch (never a
    /// `parse()` entry), so without this record the manifest misses
    /// them and an edited library keeps executing from the stale
    /// cached program.
    fn switch_to_dep(&mut self, f: &str) {
        if self.track_sources && !self.parsed_sources.iter().any(|s| s == f) {
            self.parsed_sources.push(f.to_string());
        }
        self.lexer.switch(f);
    }

    /// Is the fn-ref struct field `d_nr.f_nr` stored in the split 8B
    /// layout (`<attr>` d_nr + `<attr>__closure_rec`) rather than the
    /// legacy 4B int layout?
    ///
    /// In the second pass the database layout is the answer's one
    /// stable home: it was built from the COMPLETE first pass, while
    /// `assigned_lambda_d_nr` is derived during body parsing — a body
    /// parsed before the assigning body cannot trust the flag and
    /// would bake in the wrong field shape (#313).  Deriving from the
    /// layout keeps the read/write shape in lockstep with the bytes
    /// actually laid out.  In the first pass the layout does not
    /// exist yet, so fall back to the flag; the first pass's IR is
    /// discarded and only the flag's end state (consumed by
    /// `typedef::fill_database`) matters.
    fn fn_ref_field_is_split(&self, d_nr: u32, f_nr: usize) -> bool {
        if self.first_pass {
            self.data.def(d_nr).attributes()[f_nr].assigned_lambda_d_nr != u32::MAX
        } else {
            let nm = self.data.attr_name(d_nr, f_nr);
            let kt = self.data.def(d_nr).known_type();
            self.database.position(kt, &format!("{nm}__closure_rec")) != u16::MAX
        }
    }

    /// #318: does `tp` (transitively) store a capturing closure — a
    /// struct with a capturing-lambda fn field (`assigned_lambda_d_nr`
    /// set), reached through struct fields, vector/keyed-collection
    /// content, or tuple elements?
    ///
    /// Values of such types are frame-bound: the closure record holds
    /// raw DbRefs into the stores of the frame that owns the captures,
    /// and copying the value into storage that outlives that frame (a
    /// return value, another struct's field, a collection element)
    /// leaves dangling DbRefs — silent cross-object corruption once
    /// the store slot is reused.  The three escape sinks reject on
    /// this predicate; locals and downward argument passing stay free.
    pub(crate) fn type_carries_closure(&self, tp: &Type) -> bool {
        // Like `fn_ref_field_is_split`, derive from the registered
        // database layout (built from the COMPLETE first pass) rather
        // than `assigned_lambda_d_nr`, so the answer is independent of
        // pass-2 body-parse order.
        fn walk_def(
            data: &Data,
            db: &crate::database::Stores,
            d: u32,
            seen: &mut std::collections::HashSet<u32>,
        ) -> bool {
            if d == u32::MAX || (d as usize) >= data.definitions.len() || !seen.insert(d) {
                return false;
            }
            let kt = data.def(d).known_type();
            data.def(d).attributes().iter().any(|a| {
                db.position(kt, &format!("{}__closure_rec", a.name)) != u16::MAX
                    || walk(data, db, &a.typedef, seen)
            })
        }
        fn walk(
            data: &Data,
            db: &crate::database::Stores,
            tp: &Type,
            seen: &mut std::collections::HashSet<u32>,
        ) -> bool {
            match tp {
                // #328: a `reference<T>` POINTER field (the u16::MAX share
                // marker) copies only a 12-byte DbRef, never the record —
                // it cannot smuggle a closure record's bytes, so the
                // carrying-walk stops at the pointer boundary.  (Escaping
                // a pointer to frame-local state is the generic, documented
                // reference<T> borrow hazard — not the #318 copy class.)
                Type::Reference(_, deps) if deps.contains(&u16::MAX) => false,
                Type::Reference(d, _) => walk_def(data, db, *d, seen),
                Type::Vector(c, _) => walk(data, db, c, seen),
                Type::Hash(d, _, _)
                | Type::Sorted(d, _, _)
                | Type::Index(d, _, _)
                | Type::Radix(d, _, _) => walk_def(data, db, *d, seen),
                Type::Tuple(elems) => elems.iter().any(|e| walk(data, db, e, seen)),
                _ => false,
            }
        }
        walk(
            &self.data,
            &self.database,
            tp,
            &mut std::collections::HashSet::new(),
        )
    }

    /// @PLN114 — read a fn-ref struct field stored in the SPLIT layout: 4B `d_nr`
    /// at `pos` plus the `<attr>__closure_rec` child-record at `pos + 4`, which
    /// `typedef.rs` registers only when a capturing lambda was assigned to the
    /// attribute.  Tuple and vector elements never have that second field — they
    /// read through [`Self::get_val`]'s legacy arm instead.
    fn read_fn_ref_split(&mut self, tp: &Type, pos: u32, code: Value) -> Value {
        let p = Value::Int(pos as i32);
        let read_dnr = self.cl("OpGetInt4", &[code.clone(), p]);
        let crec_field = self.cl(
            "OpGetField",
            &[code, Value::Int(pos as i32 + 4), Value::Int(0)],
        );
        let read_clos = self.cl("OpRefFromChildRec", &[crec_field]);
        crate::data::v_block(vec![read_dnr, read_clos], tp.clone(), "fn_ref_field_read")
    }

    fn get_val(&mut self, tp: &Type, nullable: bool, pos: u32, code: Value, alias: u32) -> Value {
        let p = Value::Int(pos as i32);
        match tp {
            // @PLN25 slice (b): an `Optional(τ)` field shares its base's sentinel storage —
            // read it exactly as `τ` (the marker is compile-time only).
            Type::Optional(inner) => self.get_val(inner, nullable, pos, code, alias),
            Type::Integer(spec) => {
                // Narrow-integer width selection:
                // * `alias` is set → this is a struct-field read whose
                //   captured alias may carry `size(N)`.  Use that, else
                //   the bounds-heuristic `byte_width` (which works for
                //   plain `integer` and `integer limit(...)` fields).
                // * `alias == u32::MAX` AND spec has a forced_size → we're
                //   likely inside a vector element read.  Use
                //   `vector_narrow_width` to mirror Phase 2's actual
                //   storage decision (1 and 4 bytes narrow; 2 stays wide
                //   until the short-encoding Phase 4 round lands); fall
                //   through to 8 when Phase 2 stored wide so reads align.
                // * `alias == u32::MAX` AND no forced_size → bounds
                //   heuristic (struct-field path for plain or limited
                //   `integer`).
                // Narrow-vec path: alias is absent AND spec carries a
                // forced_size AND the gate is open.  This branch maps
                // to `Parts::ShortRaw` for 2-byte (Phase 4b) and to
                // `Parts::Byte` / `Parts::Int` for 1/4-byte (Phase 4a).
                // When the gate is CLOSED for a given forced_size
                // (fallback), storage stays wide (8-byte) and the
                // read must match — use `unwrap_or(8)` so closed-gate
                // forced_size reads dispatch to `OpGetInt`.
                let narrow_vec = alias == u32::MAX
                    && spec.forced_size.is_some()
                    && spec.vector_narrow_width().is_some();
                let s = if alias != u32::MAX {
                    self.data
                        .forced_size(alias)
                        .unwrap_or_else(|| spec.byte_width(nullable))
                } else if spec.forced_size.is_some() {
                    spec.vector_narrow_width().unwrap_or(8)
                } else {
                    spec.byte_width(nullable)
                };
                debug_assert!(
                    matches!(s, 1 | 2 | 4 | 8),
                    "get_val: unexpected integer field width s={s} \
                     (alias_d_nr={alias}) — only 1/2/4/8 are supported \
                     by the OpGet* family"
                );
                // H4-medium: the op KIND comes from the ONE width→op home
                // (`NarrowIntKind::of`), so this READ op and the matching WRITE
                // op in `set_field_check` cannot drift.  A nullable byte STRUCT
                // FIELD decodes the reserved 256th code to null (`ByteNullable`);
                // a narrow-vector element keeps the raw direct encoding (its
                // stride/value contract is the narrow-vector one, not the
                // field-sentinel one) — `narrow_vec` selects that.
                let kind =
                    crate::data::NarrowIntKind::of(s, nullable, narrow_vec, spec.unsigned_wide());
                if kind.takes_min() {
                    // H6: a sentinel-reserving kind (`ByteNullable`/`Short` — a
                    // nullable narrow FIELD *or* vector element) shrinks its usable
                    // range by one edge, so the read decodes against `usable_min`;
                    // raw kinds keep the full `min`.  Deriving from the KIND (not a
                    // re-computed `nullable && !narrow_vec`) keeps this in lockstep
                    // with the write op's `min`.
                    let mn = spec.usable_min(kind.reserves_sentinel());
                    self.cl(kind.get_op(), &[code, p, Value::Int(mn)])
                } else {
                    self.cl(kind.get_op(), &[code, p])
                }
            }
            Type::Enum(_, false, _) => self.cl("OpGetEnum", &[code, p]),
            // @PLN17: byte-stored boolean read, preserving 0/1/255 (like enum).
            Type::Boolean => self.cl("OpGetBoolean", &[code, p]),
            Type::Float => self.cl("OpGetFloat", &[code, p]),
            Type::Single => self.cl("OpGetSingle", &[code, p]),
            // A `vector<character>` element read had no `get_val` arm (only the
            // write side — OpSetCharacter — existed), so `v[0]` / `for c in v`
            // fell through to "Field access not supported on type character".
            // Mirror the OpSetCharacter write with the OpGetCharacter read.
            Type::Character => self.cl("OpGetCharacter", &[code, p]),
            Type::Text(_) => self.cl("OpGetText", &[code, p]),
            Type::Hash(_, _, _)
            | Type::Sorted(_, _, _)
            | Type::Radix(_, _, _)
            | Type::Index(_, _, _)
            | Type::Enum(_, true, _)
            | Type::Vector(_, _) => {
                let info = self.type_info(tp);
                self.cl("OpGetField", &[code, p, info])
            }
            Type::Reference(_, deps) => {
                if deps.is_empty() {
                    // Inline struct field: OpGetField adds the field offset to the base ref.
                    // Linked/base type dereference is handled at the call site (fields.rs)
                    // using OpVectorRef, which combines the 4-byte pointer read + deref.
                    let info = self.type_info(tp);
                    self.cl("OpGetField", &[code, p, info])
                } else {
                    // Plan-22 phase 02b (2026-05-12): auto-Reference field
                    // — the field stores a 12-byte DbRef pointing at the
                    // source record (shared storage).  Read the full
                    // DbRef back via OpGetDbRef.  Phase 02c is the
                    // producer that gives a closure-record attribute
                    // non-empty deps; user struct fields always have
                    // empty deps (legacy inline-bytes path above).
                    self.cl("OpGetDbRef", &[code, p])
                }
            }
            Type::Function(_, _, _) => {
                // P213: storage is two database fields per loft attribute
                //   `<attr>`              — 4B i32 holding the lambda's d_nr
                //   `<attr>__closure_rec` — 4B vector header at pos+4
                //                            (empty = non-capturing /
                //                            default-init; populated =
                //                            capturing closure record
                //                            co-located in host's Store).
                // Read both halves: `OpGetInt4` pushes the 8B d_nr,
                // `OpVectorFirstOrNull` pushes the 12B closure DbRef
                // (or the null sentinel when the vector is empty).
                // Together they form the 20B stack-side fn-ref slot
                // shape `fn_call_ref` already consumes unchanged.
                // @PLN114 — this is the LEGACY single-field read: 4B d_nr at `pos`
                // and a synthesised NULL closure.  `get_val` is reached for TUPLE and
                // VECTOR elements, which `typedef.rs`'s `Type::Function` arm keeps on
                // the legacy layout by design ("closure_rec field would be wasted
                // space and breaks layouts of containers (tuples) that pre-computed
                // positions assuming 4B per fn-ref slot").
                //
                // Reading a closure_rec at `pos + 4` here read a field that does not
                // exist for those elements — harmless only while alignment padding
                // sat there, and a collision with the NEXT element once tuples pack
                // tight like records.  The SPLIT layout (a capturing lambda assigned
                // to a struct field) has its own reader, `read_fn_ref_split`, called
                // from the site that knows the field is split.
                let read_dnr = self.cl("OpGetInt4", &[code, p.clone()]);
                let read_clos = self.cl("OpNullRefSentinel", &[]);
                crate::data::v_block(vec![read_dnr, read_clos], tp.clone(), "fn_ref_field_read")
            }
            Type::Tuple(elems) => {
                // Plan-06 phase 4d: tuple struct field read.  Each
                // element is read from `pos + element_offsets[i]`
                // using the same OpGet* opcodes that ordinary struct
                // fields use; the assembled stack tuple matches the
                // shape `Type::Tuple(...)` consumers expect.
                let elems_vec = elems.clone();
                let tuple_d_nr = self.data.tuple_def(&mut self.lexer, &elems_vec);
                let offsets: Vec<u16> = crate::data::stored_tuple_offsets_for_def(
                    &self.data,
                    &self.database,
                    tuple_d_nr,
                    elems_vec.len(),
                )
                .unwrap_or_else(|| {
                    crate::data::element_stack_offsets(&elems_vec)
                        .into_iter()
                        .map(|x| x as u16)
                        .collect()
                });
                let mut tuple_elems = Vec::with_capacity(elems_vec.len());
                for (i, et) in elems_vec.iter().enumerate() {
                    let elem_pos = pos + u32::from(offsets[i]);
                    let elem_val = self.get_val(et, false, elem_pos, code.clone(), u32::MAX);
                    tuple_elems.push(elem_val);
                }
                Value::Tuple(tuple_elems)
            }
            // Pass-1 deferral: reading a field whose declared type is still
            // `Unknown` (a forward-referenced or cross-package field type, e.g.
            // `struct Box { inner: Cell }` parsed above `struct Cell`) must not
            // emit — `actual_types_deferred` resolves the field type after this
            // pass, and pass-2 re-reads it with the concrete type.  Emitting
            // here would (via the pass-2 gate) suppress that resolution.  In
            // pass-2 a still-`Unknown` type is genuinely undefined and its
            // "Undefined type" error already fires from type resolution, so
            // fall through to the diagnostic only there.
            Type::Unknown(_) if self.first_pass => Value::Null,
            _ => {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Field access not supported on type {}",
                    tp.name(&self.data)
                );
                Value::Null
            }
        }
    }

    fn set_field(
        &mut self,
        d_nr: u32,
        f_nr: usize,
        d_pos: u16,
        ref_code: Value,
        val_code: Value,
    ) -> Value {
        self.set_field_check(d_nr, f_nr, d_pos, ref_code, val_code, true)
    }

    /// Plan-06 phase 4d: emit the per-element OpSet* sequence for a
    /// tuple struct field's value.  Recurses for nested Tuple element
    /// types so `((1, 2), (3, 4))` written into a `((int, int), (int,
    /// int))` field flattens to four `OpSetInt` calls at offsets
    /// `[0, 8, 16, 24]`.
    ///
    /// `base_pos` is the byte offset within the host record where the
    /// tuple's first element begins.  `elems` lists the tuple's
    /// element types in declaration order.  `val_code` is the IR for
    /// the tuple value.
    ///
    /// Two flavours of `val_code`:
    /// - `Value::Tuple([…])` literal — walked element-wise without a
    ///   temp variable, allowing direct recursion for nested tuples.
    /// - Anything else (variable, function call, etc.) — stashed in a
    ///   work-ref tuple temp first, then read element-by-element via
    ///   `Value::TupleGet`.  Nested tuple elements in this branch
    ///   require codegen support for `TupleGet` on `Type::Tuple`
    ///   (see `state/codegen.rs::Value::TupleGet` stack-tuple arm).
    pub(crate) fn emit_tuple_set_ops(
        &mut self,
        ref_code: &Value,
        base_pos: u16,
        elems: &[Type],
        val_code: Value,
    ) -> Vec<Value> {
        let elems_vec = elems.to_vec();
        let tuple_d_nr = self.data.tuple_def(&mut self.lexer, &elems_vec);
        let offsets: Vec<u16> = crate::data::stored_tuple_offsets_for_def(
            &self.data,
            &self.database,
            tuple_d_nr,
            elems_vec.len(),
        )
        .unwrap_or_else(|| {
            crate::data::element_stack_offsets(&elems_vec)
                .into_iter()
                .map(|x| x as u16)
                .collect()
        });
        if let Value::Tuple(values) = val_code {
            // Literal tuple value — recurse element-wise so nested
            // tuple elements flatten into per-leaf OpSet* calls.
            let mut ops = Vec::new();
            for (i, (elem_tp, value_i)) in elems_vec.iter().zip(values).enumerate() {
                // First-pass `base_pos` can be the `database.position`
                // u16::MAX sentinel for not-yet-resolved fields.  The
                // raw `+` panics under dev-profile overflow checks
                // (caught by `make iter`); release silently wraps.
                // The IR built during pass 1 is regenerated in pass 2,
                // so a saturating placeholder is safe and keeps the
                // arithmetic well-defined across both profiles.
                let elem_pos = base_pos.saturating_add(offsets[i]);
                ops.extend(self.emit_set_one_element(ref_code, elem_pos, elem_tp, value_i));
            }
            return ops;
        }
        // PLAN51 V-b — when the source is a Call returning the
        // heap-promoted form of THIS tuple shape (i.e.
        // `Type::Reference(__tuple<…>, _)` whose def_nr matches
        // tuple_d_nr we just resolved), the native codegen would
        // otherwise stash a `DbRef` return into a `Type::Tuple` work-ref
        // and emit an illegal Rust `as (DbRef, DbRef)` cast at
        // `src/generation/dispatch.rs:459-468` (rustc E0605).  Skip
        // the per-element TupleGet stash and emit a SINGLE
        // OpCopyRecord that deep-copies the entire inner-tuple struct
        // into the host field at `base_pos`.  Mirrors the
        // `Type::Reference(_, deps.is_empty())` arm in
        // `set_field_check` (line 3202-3216).
        let promoted_src_def: Option<u32> = if self.first_pass {
            None
        } else {
            match val_code.unspan() {
                Value::Call(d_nr, _) => {
                    if let Type::Reference(d, _) = self.data.def(*d_nr).returned()
                        && *d == tuple_d_nr
                    {
                        Some(tuple_d_nr)
                    } else {
                        None
                    }
                }
                _ => None,
            }
        };
        if let Some(inner_d) = promoted_src_def {
            let inner_kt = i32::from(self.data.def(inner_d).known_type());
            let field_ref = self.cl(
                "OpGetField",
                &[
                    ref_code.clone(),
                    Value::Int(i32::from(base_pos)),
                    Value::Int(inner_kt),
                ],
            );
            return vec![self.cl("OpCopyRecord", &[val_code, field_ref, Value::Int(inner_kt)])];
        }
        // Non-literal source: stash to a work-ref Tuple local, then
        // read each element via `Value::TupleGet`.
        let tup_tp = Type::Tuple(elems_vec.clone());
        let tmp = self.vars.work_refs(&tup_tp, &mut self.lexer);
        if !self.first_pass {
            self.change_var_type(tmp, &tup_tp);
        }
        let mut ops = vec![v_set(tmp, val_code)];
        for (i, elem_tp) in elems_vec.iter().enumerate() {
            let elem_pos = base_pos.saturating_add(offsets[i]);
            let elem_val = Value::TupleGet(tmp, i as u16);
            ops.extend(self.emit_set_one_element(ref_code, elem_pos, elem_tp, elem_val));
        }
        ops
    }

    /// Emit the OpSet* (or recursive flatten) for a single tuple
    /// element at a fixed byte offset within the host record.
    /// Returns a vec because nested-tuple elements expand to multiple
    /// per-leaf set ops.
    fn emit_set_one_element(
        &mut self,
        ref_code: &Value,
        pos: u16,
        elem_tp: &Type,
        value: Value,
    ) -> Vec<Value> {
        let pos_v = Value::Int(i32::from(pos));
        // @PLN25: peel `Optional(τ)` — a nullable tuple element stores via its base's
        // OpSet* (sentinel storage); without this it fell to `_` and was REJECTED with
        // "Tuple struct field cannot contain element of type integer?".
        let single = match elem_tp.base() {
            Type::Integer(_) => self.cl("OpSetInt", &[ref_code.clone(), pos_v, value]),
            Type::Function(_, _, _) => {
                // P196: storage holds the 4-byte i32 d_nr only.  Reduce
                // `Value::FnRef` to its bare `Value::Int(d_nr)` so the
                // OpSetInt4 template body sees an i64 the interpreter
                // pops in 8 bytes; for fn-ref-shaped sources (Var,
                // TupleGet, function call), interpreter `TupleGet`
                // already pushes only the d_nr's 8 bytes via OpVarInt,
                // and `output_call` projects `.0` natively (see the
                // val_is_fn_ref handling in src/generation/calls.rs).
                //
                // P251 fix (2026-05-11): for `Value::Var(v)` where `v`
                // has type `Function`, wrap in `Value::FnRefDnr(v)` so
                // native emit projects `(var_v.0 as i64)` (the d_nr
                // half of the runtime `(u32, DbRef)` tuple).  Without
                // this projection, native codegen emits
                // `let _v_val = (var_v); ... _v_val as i32` which
                // rustc rejects (E0308: expected `(u32, DbRef)`,
                // found `i64`; E0605: non-primitive cast).  The
                // direct fn-ref-struct-field-write path
                // (`emit_fn_ref_field_write`, parser/mod.rs:4886)
                // already does this projection — extending it to the
                // tuple-element-of-struct-field path closes the
                // remaining gap.
                let d_nr_only = match value {
                    Value::FnRef(d_nr, _, _) => Value::Int(d_nr),
                    Value::Var(v)
                        if matches!(self.vars.tp(v), Type::Function(_, _, _))
                            && !self.closure_vars.contains_key(&v) =>
                    {
                        Value::FnRefDnr(v)
                    }
                    other => other,
                };
                self.cl("OpSetInt4", &[ref_code.clone(), pos_v, d_nr_only])
            }
            Type::Float => self.cl("OpSetFloat", &[ref_code.clone(), pos_v, value]),
            Type::Single => self.cl("OpSetSingle", &[ref_code.clone(), pos_v, value]),
            Type::Character => self.cl("OpSetCharacter", &[ref_code.clone(), pos_v, value]),
            Type::Boolean => {
                let v = v_if(value, Value::Int(1), Value::Int(0));
                self.cl("OpSetByte", &[ref_code.clone(), pos_v, Value::Int(0), v])
            }
            Type::Text(_) => self.cl("OpSetText", &[ref_code.clone(), pos_v, value]),
            Type::Reference(inner_d_nr, _) | Type::Enum(inner_d_nr, true, _) => {
                let type_nr = if self.first_pass {
                    Value::Int(i32::from(u16::MAX))
                } else {
                    Value::Int(i32::from(self.data.def(*inner_d_nr).known_type()))
                };
                let field_ref = self.cl(
                    "OpGetField",
                    &[ref_code.clone(), pos_v.clone(), type_nr.clone()],
                );
                self.cl("OpCopyRecord", &[value, field_ref, type_nr])
            }
            Type::Vector(content, _) => {
                let vec_tp = self.vector_of(content);
                let elem_db_tp = self.database.content(vec_tp);
                let field_ref = self.cl(
                    "OpGetField",
                    &[
                        ref_code.clone(),
                        pos_v.clone(),
                        Value::Int(i32::from(vec_tp)),
                    ],
                );
                self.cl(
                    "OpAppendVector",
                    &[field_ref, value, Value::Int(i32::from(elem_db_tp))],
                )
            }
            Type::Hash(_, _, _)
            | Type::Index(_, _, _)
            | Type::Radix(_, _, _)
            | Type::Sorted(_, _, _) => self.cl("OpSetInt4", &[ref_code.clone(), pos_v, value]),
            // Plan-06 phase 4d: nested tuple element — recurse into
            // `emit_tuple_set_ops` with the inner tuple's offsets so
            // each leaf primitive lands at `outer_pos +
            // outer_offsets[i] + inner_offsets[j]`.  The flattened
            // ops are returned as a single Vec.
            Type::Tuple(inner_elems) => {
                return self.emit_tuple_set_ops(ref_code, pos, inner_elems, value);
            }
            _ => {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Tuple struct field cannot contain element of type {}",
                        elem_tp.name(&self.data)
                    );
                }
                Value::Null
            }
        };
        vec![single]
    }

    fn set_field_no_check(
        &mut self,
        d_nr: u32,
        f_nr: usize,
        d_pos: u16,
        ref_code: Value,
        val_code: Value,
    ) -> Value {
        self.set_field_check(d_nr, f_nr, d_pos, ref_code, val_code, false)
    }

    /// @PLN25 single-payload — emit the steps that turn a `Some` record (`some_ref`, type
    /// `some_d`) into the PRESENT state from a dense-`S` source `src`: set the discriminant
    /// present (offset 0), then copy the whole dense `S` into the inline `payload` field.
    /// Single-payload makes `Some = { enum, payload: S }`, so this is ONE record copy (the
    /// `Reference` arm of `set_field_check` emits the `OpGetField`+`OpCopyRecord`), replacing
    /// the per-field copy loops the individual-field layout forced.  The caller owns the
    /// null branch (set the discriminant 0) and any source stashing.
    pub(crate) fn build_some_present(
        &mut self,
        some_d: u32,
        some_ref: Value,
        src: Value,
    ) -> Vec<Value> {
        let payload_attr = self.data.attr(some_d, "payload");
        vec![
            self.cl(
                "OpSetEnum",
                &[some_ref.clone(), Value::Int(0), Value::Enum(2, u16::MAX)],
            ),
            self.set_field_no_check(some_d, payload_attr, 0, some_ref, src),
        ]
    }

    fn set_field_check(
        &mut self,
        d_nr: u32,
        f_nr: usize,
        d_pos: u16,
        ref_code: Value,
        val_code: Value,
        emit_check: bool,
    ) -> Value {
        let tp = self.data.attr_type(d_nr, f_nr);
        // @PLN25 slice (b): an `Optional(τ)` field writes exactly like its base — same
        // sentinel storage, same set-op. Peel the marker here so the whole emit path is
        // transparent to it (nullability is read separately via `attr_nullable`).
        let tp = tp.base().clone();
        let nm = self.data.attr_name(d_nr, f_nr);
        // #318 sink R2: a closure-carrying struct value cannot be
        // copied into another struct's field — the copy's closure
        // record keeps raw DbRefs into the constructing frame, which
        // the field's host may outlive (silent corruption on slot
        // reuse).  The direct fn-field write (Type::Function arm) IS
        // the supported feature and stays; `emit_check == false` is
        // the closure-record population path (captures share by
        // DbRef, no copy) and is exempt.
        if emit_check
            && !self.first_pass
            && !matches!(tp, Type::Function(_, _, _))
            && self.type_carries_closure(&tp)
        {
            diagnostic!(
                self.lexer,
                Level::Error,
                "field `{nm}` would store a value of a type that holds a capturing \
                 closure; such values are bound to the function frame that owns the \
                 captures and cannot be copied into another struct — keep the closure \
                 holder in a local variable and pass it down as an argument (#318)"
            );
            return Value::Null;
        }
        let pos = self
            .database
            .position(self.data.def(d_nr).known_type(), &nm);
        let pos_val = Value::Int(if f_nr == usize::MAX {
            i32::from(d_pos)
        } else {
            i32::from(pos + d_pos)
        });
        let has_check = emit_check
            && f_nr != usize::MAX
            && !self.first_pass
            && self
                .data
                .def(d_nr)
                .attributes
                .get(f_nr)
                .is_some_and(|a| a.check != Value::Null);
        let ref_for_check = if has_check {
            Some(ref_code.clone())
        } else {
            None
        };
        let set_op = match tp {
            Type::Integer(ref spec) => {
                // Post-2c: honor size(N) on the alias recorded during field
                // parsing; fall back to the limit()-based heuristic.
                let alias_nr = if f_nr == usize::MAX {
                    u32::MAX
                } else {
                    self.data.def(d_nr).attributes()[f_nr].alias_d_nr
                };
                // narrow-vec path mirrors `get_val`.
                // Reached by `insert(vector<u16>, ...)` where
                // `set_field` is invoked with `f_nr == usize::MAX` and
                // `tp` is the narrow element Type.  Struct-field
                // writes (alias_nr != u32::MAX) stay on the legacy
                // `OpSetShort` / `Parts::Short` `+1` encoding path.
                let narrow_vec = alias_nr == u32::MAX
                    && spec.forced_size.is_some()
                    && spec.vector_narrow_width().is_some();
                let s = self.data.forced_size(alias_nr).unwrap_or_else(|| {
                    if narrow_vec {
                        spec.vector_narrow_width().unwrap()
                    } else {
                        tp.size(self.data.attr_nullable(d_nr, f_nr))
                    }
                });
                // Size-consistency gate: the size resolved from
                // `forced_size` / limit must be one of the four
                // supported widths.  Any other value indicates a
                // post-2c regression in `size()` or a novel alias that
                // needs a matching Op emission branch here.
                debug_assert!(
                    matches!(s, 1 | 2 | 4 | 8),
                    "set_field_check: unexpected integer field width \
                     s={s} for {}.{} (alias_d_nr={alias_nr}) — only \
                     1/2/4/8 are supported by the OpSet* family",
                    self.data.def(d_nr).name(),
                    if f_nr == usize::MAX {
                        "<unknown>".to_string()
                    } else {
                        self.data.def(d_nr).attributes()[f_nr].name.clone()
                    },
                );
                // H4-medium: same width→op home (`NarrowIntKind::of`) as the
                // READ in `get_val`, so the write op matches the read op for the
                // field.  A NULLABLE byte STRUCT FIELD reserves the 256th code as
                // the null sentinel (the Nullable op pair translates null ↔ the
                // sentinel); `not null` fields and narrow-vector elements keep the
                // raw op.
                let nullable = f_nr != usize::MAX && self.data.attr_nullable(d_nr, f_nr);
                let kind =
                    crate::data::NarrowIntKind::of(s, nullable, narrow_vec, spec.unsigned_wide());
                // H6: the WRITE op encodes against the same `usable_min` the READ
                // op (`get_val`) decodes against — derived from the KIND so a
                // sentinel-reserving kind (`ByteNullable`/`Short`) shrinks the
                // range identically on both sides and raw kinds keep the full `min`.
                let m = Value::Int(spec.usable_min(kind.reserves_sentinel()));
                if kind.takes_min() {
                    self.cl(kind.set_op(), &[ref_code, pos_val, m, val_code])
                } else {
                    self.cl(kind.set_op(), &[ref_code, pos_val, val_code])
                }
            }
            Type::Vector(ref content, _)
                if f_nr != usize::MAX && !matches!(val_code.unspan(), Value::Int(_)) =>
            {
                // A real struct/tuple vector FIELD write — e.g. the tuple-return
                // heap-promotion (`control.rs::rewrite_tail_tuple_with_work_ref`)
                // hands `val_code` the WHOLE vector.  Deep-copy it into the
                // field's own freshly-allocated store, exactly as
                // `emit_set_one_element` (above) and the struct constructor
                // (`objects.rs`) already do.  A bare `OpSetInt4` here writes the
                // 8-byte vector DbRef as a 4-byte int → stack skew → garbage
                // write into the locked CONST_STORE (`store.rs:1374` tuple-return
                // crash; native rejects the `DbRef as i32` cast).  The
                // `f_nr == usize::MAX` narrow-vec `insert` path (raw 4-byte
                // header) keeps `OpSetInt4` in the arm below.
                let vec_tp = self.vector_of(content);
                let elem_db_tp = self.database.content(vec_tp);
                let field_ref = self.cl(
                    "OpGetField",
                    &[ref_code, pos_val, Value::Int(i32::from(vec_tp))],
                );
                self.cl(
                    "OpAppendVector",
                    &[field_ref, val_code, Value::Int(i32::from(elem_db_tp))],
                )
            }
            Type::Vector(_, _)
            | Type::Hash(_, _, _)
            | Type::Index(_, _, _)
            | Type::Radix(_, _, _)
            | Type::Sorted(_, _, _) => {
                // Collection header is a 4-byte u32 record pointer.  Post-2c
                // `OpSetInt` writes 8 bytes (i64), which overflows into the
                // next field.  Use `OpSetInt4` to write only 4 bytes.  Reached
                // by the narrow-vec `insert` raw-header path (`f_nr ==
                // usize::MAX`); keyed-collection struct fields deep-copy earlier
                // in `objects.rs`.
                self.cl("OpSetInt4", &[ref_code, pos_val, val_code])
            }
            Type::Function(_, _, _) => {
                // P213: storage is now TWO database fields per loft
                // attribute — `<attr>` (4B int holding the lambda's
                // d_nr; database name matches the loft attribute name
                // so `database.position(name)` lookups still resolve)
                // and `<attr>__closure_rec` (4B vector header at
                // `pos + 4`, pointing at the co-located closure record
                // in host's Store; empty for non-capturing).  At
                // read time `OpGetInt4` recovers the d_nr;
                // `OpVectorFirstOrNull` recovers the closure DbRef.
                //
                // Field-write paths:
                //   - Non-capturing lambda (`FnRef(d, MAX, _)`):
                //     write d_nr only; the closure_rec vector stays
                //     at its zero default (no records).
                //   - Capturing lambda (a `fn_ref_with_closure` Block
                //     ending in `FnRef(d, w, _)`): run the lambda's
                //     existing alloc_steps to build the closure
                //     record in parent's Store under `var(w)`, then
                //     write d_nr + `OpAppendVector` deep-copies the
                //     parent-Store closure record into element [0]
                //     of host's `__closure_rec` vector.  Parent's
                //     record gets freed by the existing
                //     parent-scope `OpFreeRef(w)`; host owns its own
                //     deep-copied copy.
                //   - Non-inline source (Var / Call returning a
                //     fn-ref): diagnose ("only inline lambda
                //     literals can be stored in fn-ref struct
                //     fields in this release") — out of scope here;
                //     follow-on plan extends to the non-inline case.
                //
                // Record the lambda's d_nr on the host attribute so
                // `typedef::fill_database`'s `Type::Function` arm
                // can register the `__closure_rec` field with the
                // correct closure-record schema.  Heterogeneous
                // captures across multiple constructors of the same
                // host struct are diagnosed at the second site.
                if let Some((lambda_d, _w)) = find_capturing_fn_ref(&self.data, &val_code)
                    && f_nr != usize::MAX
                {
                    let lambda_d_u = lambda_d as u32;
                    let prev = self.data.def(d_nr).attributes()[f_nr].assigned_lambda_d_nr;
                    if prev == u32::MAX {
                        self.data.definitions[d_nr as usize].attributes[f_nr]
                            .assigned_lambda_d_nr = lambda_d_u;
                    } else if prev != lambda_d_u && !self.first_pass {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "heterogeneous capture shapes per fn-ref struct field are not supported \
                             (this lambda's captured environment differs from the previously-assigned \
                              lambda's); split into two structs or unify the captures"
                        );
                    }
                    // #318 sink R2: the host being written must be
                    // rooted in a frame-local — writing a capturing
                    // closure into (a field of) an ARGUMENT claims the
                    // closure record into a store that outlives this
                    // frame, while the record's DbRefs point at this
                    // frame's captures (silent corruption on slot
                    // reuse once the frame dies).
                    if !self.first_pass
                        && let Some(base) = ref_code.base_var()
                        && self.vars.is_argument(base)
                    {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "cannot store a capturing closure into a struct received as an \
                             argument — the closure references state owned by this \
                             function's frame, which the argument's struct outlives; \
                             construct the closure in the frame that owns the captured \
                             state (#318)"
                        );
                        return Value::Null;
                    }
                }
                emit_fn_ref_field_write(self, d_nr, f_nr, ref_code, pos_val, &val_code)
            }
            Type::Tuple(ref elems) => {
                // Plan-06 phase 4d: tuple struct field assignment.
                // Storage layout matches the synthetic `__tuple<…>`
                // struct's element positions (registered via
                // `tuple_def`) so each element write goes through
                // the same `OpSetInt`/`OpSetFloat`/etc. opcodes used
                // for ordinary struct fields.  Recurses for nested
                // Tuple element types — see `emit_tuple_set_ops`.
                let elems_vec = elems.clone();
                let host_field_pos = if let Value::Int(p) = pos_val {
                    p as u16
                } else {
                    0
                };
                let ops = self.emit_tuple_set_ops(&ref_code, host_field_pos, &elems_vec, val_code);
                v_block(ops, Type::Void, "tuple_field_set")
            }
            Type::Character => self.cl("OpSetCharacter", &[ref_code, pos_val, val_code]),
            Type::Reference(inner_tp, deps) => {
                if deps.is_empty() {
                    // The value is a 12-byte DbRef; OpSetInt would only read 4 bytes of it.
                    // Copy the struct bytes into the embedded field instead.
                    let type_nr = if self.first_pass {
                        Value::Int(i32::from(u16::MAX))
                    } else {
                        Value::Int(i32::from(self.data.def(inner_tp).known_type()))
                    };
                    let field_ref = self.cl("OpGetField", &[ref_code, pos_val, type_nr.clone()]);
                    // Note: the free-source high-bit for Issue #120 is set in
                    // copy_ref() (operators.rs), which is the path for struct
                    // field reassignment. This set_field_check path is for
                    // construction (initial field population).
                    self.cl("OpCopyRecord", &[val_code.clone(), field_ref, type_nr])
                } else {
                    // Plan-22 phase 02b (2026-05-12): auto-Reference field
                    // — store the source's 12-byte DbRef directly,
                    // sharing the underlying record.  Phase 02c sets
                    // non-empty deps for mutated Reference captures on
                    // closure records; today's user code paths keep
                    // empty deps and stay on the legacy OpCopyRecord
                    // path above.
                    self.cl("OpSetDbRef", &[ref_code, pos_val, val_code])
                }
            }
            Type::Enum(_, false, _) => self.cl("OpSetEnum", &[ref_code, pos_val, val_code]),
            Type::Enum(nr, true, _) => {
                // A struct-enum field holds an inline record; copy the source enum
                // into the field AT ITS OFFSET via a field sub-ref — exactly like
                // the Type::Reference arm above.  Copying into `ref_code` (the
                // struct base) instead landed a non-first struct-enum field at
                // offset 0, clobbering field 0 and reading a garbage discriminant
                // (#406: `Entry { key: a, value: b }` from enum variables).
                let type_nr = if self.first_pass {
                    // known_type() is u16::MAX until the enum registers in pass 2;
                    // emit the placeholder now (codegen re-runs in pass 2), matching
                    // the Type::Reference arm.
                    Value::Int(i32::from(u16::MAX))
                } else {
                    Value::Int(i32::from(self.data.def(nr).known_type()))
                };
                let field_ref = self.cl("OpGetField", &[ref_code, pos_val, type_nr.clone()]);
                self.cl("OpCopyRecord", &[val_code, field_ref, type_nr])
            }
            // @PLN17: store the boolean's u8 form (0/1/255) directly, like enum —
            // the old `if val {1} else {0}` forced 0/1 and dropped the null sentinel.
            Type::Boolean => self.cl("OpSetBoolean", &[ref_code, pos_val, val_code]),
            Type::Float => self.cl("OpSetFloat", &[ref_code, pos_val, val_code]),
            Type::Single => self.cl("OpSetSingle", &[ref_code, pos_val, val_code]),
            Type::Text(_) => self.cl("OpSetText", &[ref_code, pos_val, val_code]),
            _ => {
                if self.first_pass {
                    Value::Null
                } else {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Cannot assign to field '{}' of type {}",
                        self.data.attr_name(d_nr, f_nr),
                        self.data.attr_type(d_nr, f_nr).name(&self.data)
                    );
                    Value::Null
                }
            }
        };
        self.emit_field_constraint(set_op, ref_for_check, d_nr, f_nr, &nm)
    }

    /// Wrap a set operation with a constraint assertion if the field has one.
    fn emit_field_constraint(
        &mut self,
        set_op: Value,
        ref_for_check: Option<Value>,
        d_nr: u32,
        f_nr: usize,
        field_name: &str,
    ) -> Value {
        let Some(ref_val) = ref_for_check else {
            return set_op;
        };
        let check = self.data.def(d_nr).attributes()[f_nr].check.clone();
        let bound = Self::replace_record_ref(check, &ref_val);
        let msg = if let Value::Text(s) = &self.data.def(d_nr).attributes()[f_nr].check_message {
            Value::Text(s.clone())
        } else {
            Value::Text(format!(
                "field constraint failed on {}.{field_name}",
                self.data.def(d_nr).name()
            ))
        };
        let assert_dnr = self.data.def_nr("n_assert");
        let pos = self.lexer.pos();
        let assert_call = Value::Call(
            assert_dnr,
            vec![
                bound,
                msg,
                Value::Text(pos.file.clone()),
                Value::Int(pos.line as i32),
            ],
        );
        Value::Insert(vec![set_op, assert_call])
    }

    /// Append a `--show-types --trace` entry for the type at the
    /// current parse position.  No-op unless `trace_types` is set
    /// AND we are on the second pass (first-pass types are
    /// placeholders and would emit thousands of meaningless lines).
    pub(crate) fn record_type_trace(&mut self, t: &Type) {
        if !self.trace_types || self.first_pass {
            return;
        }
        let pos = self.lexer.pos();
        let fn_name = self.vars.name.clone();
        if fn_name.is_empty() {
            return;
        }
        let type_str = t.show(&self.data, &self.vars);
        self.trace_types_lines
            .push(format!("{fn_name}\t{}:{}\t{type_str}", pos.line, pos.pos));
    }

    fn cl(&mut self, op: &str, list: &[Value]) -> Value {
        let d_nr = self.data.def_nr(op);
        if d_nr == u32::MAX {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Internal error: missing built-in operation (report this as a bug)"
            );
            Value::Null
        } else {
            Value::Call(d_nr, list.to_vec())
        }
    }

    /// Try to find a matching defined operator. There can be multiple possible definitions for each operator.
    fn call_op(&mut self, code: &mut Value, op: &str, list: &[Value], types: &[Type]) -> Type {
        // A first-pass UNARY op on an operand whose type is still UNRESOLVED — an
        // Unknown-rooted value, e.g. `x = f()` where `f`'s return type isn't linked
        // yet (a cross-package fn resolved only after this body's first pass) — must
        // stay re-typeable.  Otherwise the `possible` loop below matches a concrete
        // built-in (`-x` → OpMinInt → integer) and locks the result var to integer;
        // pass 2 then re-resolves `x` to its real (e.g. float) type and the assignment
        // errors "cannot change type from integer to float".  Return Unknown so pass 2
        // refines it cleanly — the same re-typeable escape the generic-type-variable
        // arm below takes on the first pass.  Scoped to a single unknown operand (a
        // unary `-`/`~`/`!`): binary ops keep erroring so a genuine "No matching
        // operator '<' on 'unknown' and 'boolean'" still fires.  A truly-unresolvable
        // unary operand re-errors on pass 2 (this guard is first-pass only).
        // (@PLN102 transitive cross-package inference.)
        //
        // The same applies when EVERY operand is unresolved — `f() - g()` with both
        // callees defined lower in the file.  The `possible` loop matches the first
        // candidate (`OpMinInt`) and locks the result to integer; pass 2 re-resolves
        // to the real float return and the assignment errors "cannot change type from
        // integer to float" at a line that looks correct.  Requiring ALL operands to be
        // unknown is what keeps the diagnostic above intact: it has one KNOWN operand
        // (`boolean`), so it still reaches the error path.  One known operand is enough
        // to steer resolution, so only the no-information case defers to pass 2.
        if self.first_pass && !types.is_empty() && types.iter().all(Type::is_unknown) {
            return Type::Unknown(0);
        }
        // I8.1: if any operand is a generic type variable, skip the main operator loop
        // and go straight to the T-stub lookup.  The main loop would otherwise false-match
        // concrete operators (e.g. OpEqRef, OpEqBool) via implicit type conversions on T.
        let generic_name = types.iter().find_map(|t| self.generic_type_name(t));
        if let Some(tv_name) = generic_name {
            if self.first_pass {
                // Return the type variable type so assignments keep a consistent type
                // through the first pass (Type::Void would trigger "cannot change type").
                let tv_nr = self.data.def_nr(tv_name);
                return if tv_nr == u32::MAX {
                    Type::Unknown(0)
                } else {
                    Type::Reference(tv_nr, Deps::none())
                };
            }
            let op_method = format!("Op{}", rename(op));
            let stub_name = format!("t_{}{}_{}", tv_name.len(), tv_name, op_method);
            let stub_nr = self.data.def_nr(&stub_name);
            // Only use the T-stub if the CURRENT function's bounds declare this method.
            // Without this check, T-stubs from unrelated bounded generics (e.g., stdlib's
            // sum<T: Addable>) would leak into unbound generics like `fn bad<T>(x+y)`.
            if stub_nr != u32::MAX
                && self.context != u32::MAX
                && self.has_bound_for_method(&op_method)
            {
                let tp = self.call_nr(code, stub_nr, list, types, false, &[], None);
                if tp != Type::Null {
                    return tp;
                }
            }
        } else {
            // @PLN99 Arc A completion — a first-grade struct's OWN operator method
            // (`t_<len><Type>_Op<Name>`) must take precedence over the built-in `possible`
            // loop below, which otherwise wins via (a) reference-identity for `==`/`!=` on
            // two struct refs (OpEqRef), or (b) coercing an operand through a user
            // `T → builtin` conversion and using the built-in operator (`a - b` → OpMinInt).
            // A built-in `integer` never coerces itself away; a user type must not either.
            // Method-only lookup (NOT full `find_fn`, whose `possible` fallback would
            // pre-empt the coercion the loop legitimately does for mixed built-in operands).
            if let Some(first) = types.first() {
                let m = self
                    .data
                    .find_op_method(u16::MAX, &format!("Op{}", rename(op)), first);
                if m != u32::MAX {
                    let tp = self.call_nr(code, m, list, types, false, &[], None);
                    if tp != Type::Null {
                        return tp;
                    }
                }
            }
            let mut possible = Vec::new();
            for pos in self
                .data
                .get_possible(&format!("Op{}", rename(op)), &self.lexer)
            {
                possible.push(*pos);
            }
            for pos in possible {
                // `OpEqBool` truthiness-fallback guard lives in `call_nr` (the single
                // chokepoint all three resolution sub-paths share) — see there.
                let tp = self.call_nr(code, pos, list, types, false, &[], None);
                if tp != Type::Null {
                    // We cannot compare two different types of enums, both will be integers in the same range
                    if let (Some(Type::Enum(f, _, _)), Some(Type::Enum(s, _, _))) =
                        (types.first(), types.get(1))
                        && f != s
                    {
                        break;
                    }
                    return tp;
                }
            }
            // @PLN99 Arc A — a user-defined operator on a concrete struct is stored
            // as `t_<len><Type>_Op<Name>` / `n_Op<Name>`, never in the `possible`
            // map (`add_op` fills it only for prefix-named built-ins like `OpLtInt`).
            // Resolve it via `find_fn` — the same resolver the generic/method path
            // uses — so a DIRECT `a < b` on a user struct dispatches the user def,
            // not only inside a `<T: Ordered>` body. A type with no such def falls
            // through to the unchanged "No matching operator" error below.
            if let Some(first) = types.first() {
                let user_op = self
                    .data
                    .find_fn(u16::MAX, &format!("Op{}", rename(op)), first);
                if user_op != u32::MAX {
                    let tp = self.call_nr(code, user_op, list, types, false, &[], None);
                    if tp != Type::Null {
                        return tp;
                    }
                }
            }
        }
        // @PLN25 E2 — `!nullable` (a `__nullable<S>` value in boolean position)
        // means "is absent" (discriminant 0): no struct operator overload
        // matches, so lower it directly (mirrors the `== null` is-null lowering).
        // The common shape is `!v[oob]` ("out-of-bounds is null").
        if op == "Not"
            && types.len() == 1
            && let Type::Enum(syn, true, _) = &types[0]
            && self.data.def(*syn).name.starts_with("__nullable<")
        {
            let get_enum = self.cl("OpGetEnum", &[list[0].clone(), Value::Int(0)]);
            let disc = self.cl("OpConvIntFromEnum", &[get_enum]);
            *code = self.cl("OpEqInt", &[disc, Value::Int(0)]);
            return Type::Boolean;
        }
        // generic-specific error message for operators on T.
        let generic_name = types.iter().find_map(|t| self.generic_type_name(t));
        if let Some(tv_name) = generic_name {
            specific!(
                self.lexer,
                &self.lexer.peek(),
                Level::Error,
                "generic type {tv_name}: operator '{op}' requires a concrete type",
            );
        } else if types.len() > 1 {
            specific!(
                self.lexer,
                &self.lexer.peek(),
                Level::Error,
                "No matching operator '{op}' on '{}' and '{}'",
                types[0].name(&self.data),
                types[1].name(&self.data)
            );
        } else {
            specific!(
                self.lexer,
                &self.lexer.peek(),
                Level::Error,
                "No matching operator {op} on {}",
                types[0].name(&self.data)
            );
        }
        Type::Unknown(0)
    }

    /// Call a specific definition
    #[allow(clippy::too_many_arguments)]
    fn call_nr(
        &mut self,
        code: &mut Value,
        d_nr: u32,
        list: &[Value],
        types: &[Type],
        report: bool,
        arg_pos: &[Position],
        // The call NAME's position, when the caller knows it (a free-function call
        // via `call()`).  The @PLN102 arc-C steer points its caret here and carries a
        // `codeAction` suggestion, so the quick-fix replaces the right token; `None`
        // (a method / operator path) keeps the cursor caret and offers no quick-fix.
        name_pos: Option<&Position>,
    ) -> Type {
        // @PLN102 pre-freeze — `OpEqBool`/`OpNeBool` are BOOLEAN (in)equality; they must
        // not be the implicit truthiness fallback for mismatched types.  Without this,
        // `5 == "banana"` resolves as `OpEqBool(OpConvBoolFromInt(5),
        // OpConvBoolFromText("banana"))` = `true == true` = **true** (likewise
        // `float == text`, `char == float`, `bool == text`, and their `!=` twins).  These
        // reach here via THREE call_op sub-paths (find_op_method, the `possible` loop, and
        // find_fn — the last resolves the boolean's own operator), so the guard lives at
        // this single chokepoint they all share: refuse the boolean (in)equality op unless
        // both operands are genuinely boolean, returning "no match" so the caller falls
        // through to a numeric/same-type op or the "No matching operator" reject (as the
        // ordering operators `<`/`<=`/… already do).  A `null` operand is EXEMPT: a bare
        // `boolean?`/`integer?`/… `== null` null-check legitimately lowers through the
        // boolean op (no separate bool-null branch upstream), so only reject when BOTH
        // operands are concrete non-null and not both boolean.
        if matches!(self.data.def(d_nr).name(), "OpEqBool" | "OpNeBool") && types.len() >= 2 {
            let (a, b) = (types[0].base(), types[1].base());
            // Only a VALUE-vs-value mismatch is the truthiness bug: in-band scalars
            // plus enums (`Color.Green == 1` truthiness-coerces both).  A reference /
            // heap operand (`DT? == DT?`, a nullable struct-ref) legitimately reaches
            // OpEqBool as its null-ness comparison — blocking it there would strand it
            // with "No matching operator" (there is no OpEqRef coercion for it).  So
            // require BOTH operands to be value types before rejecting.
            let value = |t: &Type| {
                matches!(
                    t,
                    Type::Integer(_)
                        | Type::Float
                        | Type::Single
                        | Type::Text(_)
                        | Type::Character
                        | Type::Boolean
                        | Type::Enum(..)
                )
            };
            let both_bool = matches!(a, Type::Boolean) && matches!(b, Type::Boolean);
            // A boolean-vs-integer comparison is rejected UPSTREAM (parser/operators.rs)
            // with a bespoke "a boolean is true/false/null, not 0/1" message; leave that
            // pair to it (this guard would otherwise pre-empt it with the generic reject).
            let bool_int = (matches!(a, Type::Boolean) && matches!(b, Type::Integer(_)))
                || (matches!(a, Type::Integer(_)) && matches!(b, Type::Boolean));
            if value(a) && value(b) && !both_bool && !bool_int {
                return Type::Null;
            }
        }
        // @PLN102 pre-freeze — an enum compared to a RAW integer coerces the enum to
        // its INTERNAL discriminant (`OpConvIntFromEnum`, +1-biased so variant 0 is
        // disc 1), so `Color.Green == 1` leaks that encoding and reads a confusing
        // false (Green's disc is 2).  Reject the enum-vs-integer pair like the other
        // cross-type comparisons — `enum == enum` is untouched (BOTH sides convert, so
        // both operands are `Enum` here, not one enum + one integer; a bare `enum ==
        // null` is `Enum` + `Null`).  The internal is-absent lowering builds `OpEqInt`
        // via `self.cl` with an already-integer discriminant, bypassing this path.
        if matches!(self.data.def(d_nr).name(), "OpEqInt" | "OpNeInt") && types.len() >= 2 {
            let (a, b) = (types[0].base(), types[1].base());
            let enum_int = (matches!(a, Type::Enum(..)) && matches!(b, Type::Integer(_)))
                || (matches!(a, Type::Integer(_)) && matches!(b, Type::Enum(..)));
            if enum_int {
                return Type::Null;
            }
        }
        let mut all_types = Vec::from(types);
        if self.data.def_type(d_nr) == DefType::Dynamic {
            for a_nr in 0..self.data.attributes(d_nr) {
                let Type::Routine(r_nr) = self.data.attr_type(d_nr, a_nr) else {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Incorrect dynamic function {}",
                        self.data.def(d_nr).name()
                    );
                    return Type::Void;
                };
                if self.data.attr_type(r_nr, 0).is_equal(&types[0]) {
                    return self.call_nr(code, r_nr, list, types, report, arg_pos, name_pos);
                }
            }
            diagnostic!(
                self.lexer,
                Level::Error,
                "No matching function {}",
                self.data.def(d_nr).name()
            );
        } else if !matches!(self.data.def_type(d_nr), DefType::Function) {
            if report {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Unknown definition {}",
                    self.data.def(d_nr).name()
                );
            }
            return Type::Null;
        }
        let mut actual = self.process_call_args(d_nr, list, types, &mut all_types, report, arg_pos);
        if actual.is_empty() && !types.is_empty() {
            return Type::Null;
        }
        // @PLN86 §7.2 (F7) — gate an OVERRIDE of a `…#default`-locked parameter BEFORE
        // defaults fill the gaps: at this point a parameter slot is non-`Null` exactly
        // when the caller supplied it explicitly, so a defaulted parameter is invisible.
        if self.in_sandbox && !self.first_pass {
            self.record_param_lock_overrides(d_nr, &actual);
        }
        self.add_defaults(d_nr, &mut actual, &mut all_types);
        let tp = self.call_dependencies(d_nr, &all_types);
        *code = Value::Call(d_nr, actual);
        self.warn_text_len_byte_index(d_nr, code);
        // @PLN102 arc C step 3 — the recommended-idiom STEER.  A resolved call FROM
        // OWNED source (the entry project) to a `#superseded "Y"` symbol warns the
        // author toward `Y`; the old form keeps working (a never-break signpost, not
        // a removal).  The caller-provenance gate (`caller_source_is_owned`) means a
        // consumer re-parsing a dependency's source is NEVER nagged about the
        // dependency's internal old-idiom use — only whoever can act sees it.
        // Second-pass + `report` only, so it fires exactly once per call site;
        // `LOFT_NO_STEER` opts out; inert until a symbol is actually marked.
        if report
            && !self.first_pass
            && crate::keys::steer_enabled()
            && self.data.caller_source_is_owned()
        {
            let succ = self.data.def(d_nr).superseded().to_string();
            if !succ.is_empty() {
                let shown = self.data.def(d_nr).display_name().to_string();
                // Position on the call NAME when the caller supplied it, and carry the
                // successor as a structured suggestion so a `codeAction` can offer a
                // "Change to `Y`" quick-fix (step B).  Without a name position (method /
                // operator path) keep the cursor caret and no suggestion — a quick-fix
                // there could replace the wrong token.
                if let Some(pos) = name_pos {
                    diagnostic_at!(
                        self.lexer,
                        pos,
                        Level::Advice,
                        "`{shown}` is superseded — use `{succ}` (the old form keeps working)"
                    );
                    self.lexer.suggest_last(&succ);
                } else {
                    diagnostic!(
                        self.lexer,
                        Level::Warning,
                        "`{shown}` is superseded — use `{succ}` (the old form keeps working)"
                    );
                }
            }
        }
        tp
    }

    /// @PLN86 §7.2 (F7) — record, for a sandboxed call to `d_nr`, each argument that
    /// OVERRIDES a `…#default`-locked parameter.  A parameter is overridden when its slot
    /// was supplied explicitly (`actual[i]` present and non-`Null` — defaults have not
    /// filled the gaps yet at the call site) AND the value DIFFERS from the parameter's
    /// declared default; an argument equal to the default is exactly what the lock pins
    /// to, so it is free.  Admission (`param_lock_violations`) then rejects an override
    /// whose lock token the calling profile does not grant.
    fn record_param_lock_overrides(&mut self, d_nr: u32, actual: &[Value]) {
        if self.param_locks.is_empty() {
            return;
        }
        for i in 0..self.data.attributes(d_nr) {
            let Some(token) = self.param_locks.get(&(d_nr, i as u32)) else {
                continue;
            };
            let token = token.clone();
            let Some(arg) = actual.get(i) else { continue };
            if matches!(arg.unspan(), Value::Null) {
                continue; // not supplied → keeps its default, free
            }
            if arg.unspan() == self.data.def(d_nr).attributes()[i].value.unspan() {
                continue; // explicitly the default → not an override
            }
            let pos = self.lexer.peek_pos().clone();
            self.sandbox_param_overrides
                .entry(self.context)
                .or_default()
                .push((token, pos));
        }
    }

    /// Convert and validate each positional argument for a call.
    fn process_call_args(
        &mut self,
        d_nr: u32,
        list: &[Value],
        types: &[Type],
        all_types: &mut [Type],
        report: bool,
        arg_pos: &[Position],
    ) -> Vec<Value> {
        let mut actual = Vec::new();
        if types.is_empty() {
            return actual;
        }
        if list.len() > self.data.attributes(d_nr) {
            if report {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Too many parameters for {}",
                    self.data.def(d_nr).name()
                );
            }
            return actual;
        }
        // @PLN102 gate-2 (N-Store) at the CALL-ARG site — constant across the args: the callee
        // name (for the diagnostic) and whether the call-arg check applies. Null-transparent fns
        // (`min`/`max`/`clamp`/`abs`/… — `is_null_transparent`) PROPAGATE null via a runtime guard
        // (`wrap_null_transparent`), so a nullable arg into their non-null param is intentional, not
        // an unsound store — exempt them (operators already dodge this path via the nullable-op swap).
        let callee_name = self.data.def(d_nr).original_name();
        let callarg_nstore =
            crate::keys::callarg_nstore_enabled() && !Self::is_null_transparent(&callee_name);
        for (nr, a_code) in list.iter().enumerate() {
            let tp = self.data.attr_type(d_nr, nr);
            let Some(actual_type) = types.get(nr) else {
                continue;
            };
            let mut actual_code = a_code.clone();
            if let (Type::Vector(to_tp, _), Type::Vector(a_tp, _)) = (&tp, actual_type)
                && a_tp.is_unknown()
                && !to_tp.is_unknown()
            {
                self.change_var(&actual_code, &tp);
                actual.push(actual_code);
                continue;
            }
            // empty `[]` literal → create temp vector where parameter type is known.
            if matches!(&actual_code, Value::Insert(ops) if ops.len() <= 1)
                && let Type::Vector(elm_tp, dep) = &tp
            {
                let vec = self.create_unique("vec", &Type::Vector(elm_tp.clone(), dep.clone()));
                let mut ls = self.vector_db(elm_tp, vec);
                ls.push(Value::Var(vec));
                actual.push(v_block(ls, tp.clone(), "empty_vector_arg"));
                all_types[nr] = tp.clone();
                continue;
            }
            // L4: reject non-variable expressions passed to `&` parameters (except &text
            // which has its own work-text copy handling in convert()).  The `&` modifier
            // means "mutations propagate back to the caller" — passing a literal means
            // the mutations are silently discarded, which is almost certainly a bug.
            // also accept "addressable" expressions — vector element access
            // (`v[i]`), field access (`s.field`), and chains thereof — since these
            // produce a DbRef into existing mutable storage.
            if let Type::RefVar(inner) = &tp
                && !matches!(inner.as_ref(), Type::Text(_))
                && !matches!(&actual_code, Value::Var(_))
                && !Self::is_addressable(&actual_code, &self.data)
            {
                // Defer on pass 1 (#375): a field access on a struct whose
                // layout is not yet finalised — because one of its fields is a
                // forward / cross-package reference still resolving — lowers to
                // `Null` here, which is not addressable.  Erroring now would
                // abort pass 1 before the type resolves; on pass 2 the layout is
                // complete and the access lowers to `OpGetField` (addressable),
                // so the check passes.  A genuine literal-to-`&` is still an
                // error: it is non-addressable on pass 2 too, where this fires.
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Cannot pass a literal or expression to a '&' parameter — \
                         assign to a variable first"
                    );
                }
                actual.push(actual_code);
                continue;
            }
            if actual_type.is_unknown() && matches!(&tp, Type::Vector(_, _)) {
                self.change_var(&actual_code, &tp);
                actual.push(actual_code);
                continue;
            }
            if let (Type::Integer(_), Type::Enum(_, true, _)) = (&tp, actual_type) {
                let cd = if matches!(actual_code, Value::Enum(_, _)) {
                    actual_code
                } else {
                    self.cl("OpGetEnum", &[actual_code, Value::Int(0)])
                };
                actual.push(self.cl("OpConvIntFromEnum", &[cd]));
                continue;
            }
            // @PLN102 gate-2 (N-Store) at the CALL-ARG site — the last store site the teeth did
            // not cover (converges with the earlier routing-feedback f4 fix). `convert` below
            // leniently peels an `Optional`, so a nullable `τ?` (or a bare `null` under DN1) bound
            // silently into a non-null PARAMETER. Run the same `n_store_violation` check here
            // (identical Phase-1 warn/error split) so the param binding is held to the same rule as
            // an assignment / field / return. On a hard error (a narrow-width param) skip
            // `convert`'s generic diagnostic; otherwise fall through and let `convert` peel it.
            // The position anchors to the argument's own span (nstore-position-fix.md) so a TAIL
            // call reports at the call, not the next line.
            if report
                && callarg_nstore
                && self.n_store_violation(
                    actual_type,
                    &tp,
                    &format!("parameter {} of `{callee_name}`", nr + 1),
                    actual_code.span_pos(),
                )
            {
                actual.push(actual_code);
                continue;
            }
            if !self.convert(&mut actual_code, actual_type, &tp) {
                if report {
                    let context = format!(
                        "argument {} of call to {}",
                        nr + 1,
                        self.data.def(d_nr).original_name()
                    );
                    // `arg_pos[nr]` is the argument's start, captured in
                    // `parse_call`; the lexer cursor has drifted to `)` / `,`.
                    // Synthetic / reordered call paths pass an empty slice and
                    // fall back to the cursor.
                    let pos = arg_pos
                        .get(nr)
                        .cloned()
                        .unwrap_or_else(|| self.lexer.pos().clone());
                    self.validate_convert(&context, actual_type, &tp, &pos);
                } else if !self.can_convert(actual_type, &tp) {
                    return Vec::new();
                }
            }
            actual.push(actual_code);
        }
        actual
    }

    // Gather depended on variables from arguments of the given called routine.
    fn call_dependencies(&mut self, d_nr: u32, types: &[Type]) -> Type {
        let tp = self.data.def(d_nr).returned().clone();
        // for Reference returns (structs), filter out hidden return-mechanism
        // attributes from dep resolution. The struct owns its store independently —
        // hidden return-store buffers are implementation artifacts.
        // Text/Vector returns genuinely depend on their hidden work buffers.
        let attrs = self.data.def(d_nr).attributes();
        let filter_hidden = |d: &[u16]| -> Vec<u16> {
            d.iter()
                .copied()
                .filter(|&i| (i as usize) >= attrs.len() || !attrs[i as usize].hidden)
                .collect()
        };
        if let Type::Text(d) = tp {
            Type::Text(Deps::frame(Self::resolve_deps(types, d.as_attr_indices())))
        } else if let Type::Vector(to, d) = tp {
            Type::Vector(
                to,
                Deps::frame(Self::resolve_deps(types, d.as_attr_indices())),
            )
        } else if let Type::Sorted(to, key, d) = tp {
            Type::Sorted(
                to,
                key,
                Deps::frame(Self::resolve_deps(types, d.as_attr_indices())),
            )
        } else if let Type::Hash(to, key, d) = tp {
            Type::Hash(
                to,
                key,
                Deps::frame(Self::resolve_deps(types, d.as_attr_indices())),
            )
        } else if let Type::Index(to, key, d) = tp {
            Type::Index(
                to,
                key,
                Deps::frame(Self::resolve_deps(types, d.as_attr_indices())),
            )
        } else if let Type::Radix(to, key, d) = tp {
            Type::Radix(
                to,
                key,
                Deps::frame(Self::resolve_deps(types, d.as_attr_indices())),
            )
        } else if let Type::Reference(to, d) = tp {
            Type::Reference(
                to,
                Deps::frame(Self::resolve_deps(
                    types,
                    &filter_hidden(d.as_attr_indices()),
                )),
            )
        } else if let Type::Enum(to, true, d) = tp {
            Type::Enum(
                to,
                true,
                Deps::frame(Self::resolve_deps(
                    types,
                    &filter_hidden(d.as_attr_indices()),
                )),
            )
        } else {
            tp
        }
    }

    /// @PLN85 L1 — the def→frame dep conversion for FN-REF call results.
    /// A fn-ref/lambda call's declared return type carries the CALLEE's
    /// attr-space deps (e.g. a grown hidden work-buffer's index).  Read
    /// verbatim in the caller they alias arbitrary caller attrs/frame vars —
    /// the L1 leak: a lambda's hidden-buffer index 1 read as the CALLER's
    /// attr 1 (`__retbuf`) made `ref_return` believe the CallRef tail already
    /// rode the buffer, so no delivery was emitted and the store
    /// `fn_call_ref` allocates at runtime leaked on both backends.  Convert
    /// exactly as plain calls do (`call_dependencies`): map visible-param
    /// indices through the actual argument types; out-of-range (hidden /
    /// grown) indices drop — the adaptive fn-ref ABI allocates those buffers
    /// at runtime, so the value arrives OWNED.
    fn fnref_result_type(ret: Type, types: &[Type]) -> Type {
        match ret {
            Type::Text(d) => {
                Type::Text(Deps::frame(Self::resolve_deps(types, d.as_attr_indices())))
            }
            Type::Vector(to, d) => Type::Vector(
                to,
                Deps::frame(Self::resolve_deps(types, d.as_attr_indices())),
            ),
            Type::Reference(to, d) => Type::Reference(
                to,
                Deps::frame(Self::resolve_deps(types, d.as_attr_indices())),
            ),
            Type::Enum(to, true, d) => Type::Enum(
                to,
                true,
                Deps::frame(Self::resolve_deps(types, d.as_attr_indices())),
            ),
            other => other,
        }
    }

    /// THE def→frame dep converter (H2 / DEPS_INVENTORY): maps the
    /// callee's ATTR-INDEX deps through the actual argument types at a
    /// call site into caller FRAME var deps.  `d` must be attr-space
    /// (callers read it via `Deps::as_attr_indices`); the result is
    /// wrapped `Deps::frame` by `call_dependencies`.
    fn resolve_deps(types: &[Type], d: &[u16]) -> Vec<u16> {
        let mut dp = HashSet::new();
        for ar in d {
            if *ar as usize >= types.len() {
                continue;
            }
            if let Type::Text(ad)
            | Type::Vector(_, ad)
            | Type::Sorted(_, _, ad)
            | Type::Hash(_, _, ad)
            | Type::Index(_, _, ad)
            | Type::Radix(_, _, ad)
            | Type::Reference(_, ad)
            | Type::Enum(_, true, ad) = &types[*ar as usize]
            {
                for a in ad {
                    dp.insert(*a);
                }
            }
        }
        Vec::from_iter(dp)
    }

    fn add_defaults(&mut self, d_nr: u32, actual: &mut Vec<Value>, all_types: &mut Vec<Type>) {
        // @PLAN59 phase 2: the `__rref_N` recursive-self counter dance is
        // gone.  It existed to keep `__ref_N` numbering pass-stable so
        // `ref_return` could re-find its promoted attr BY NAME instead of
        // growing the attr count across passes.  With the signature-time
        // `__retbuf` the attr exists before any body parses and the
        // promotion re-find no longer depends on work-ref numbering.
        // Extend to full parameter count so we can fill gaps from named arguments.
        while actual.len() < self.data.attributes(d_nr) {
            actual.push(Value::Null);
            all_types.push(Type::Unknown(0));
        }
        {
            // Fill all missing (Null) parameter slots with defaults.
            for a_nr in 0..self.data.attributes(d_nr) {
                if actual[a_nr] != Value::Null {
                    continue;
                }
                let default = self.data.def(d_nr).attributes()[a_nr].value.clone();
                let tp = self.data.attr_type(d_nr, a_nr);
                // Arity: a slot with NO provided argument and NO explicit default is a
                // too-few-arguments error. loft's old "defaulted-null args" lenience (omit a trailing
                // arg → null/empty fill) was REMOVED (owner decision 2026-07-17): it was a footgun —
                // a real consumer hit it as both a stdlib SIGSEGV (a missing fn-typed arg fills a
                // broken `()`) and a silent-wrong (a missing scalar fills null). Skip the cases that
                // are NOT a user argument:
                //  · a NULLABLE param defaults to null (still optional);
                //  · a `= expr` default (value != Null) is filled below;
                //  · a COMPILER-inserted return slot — `hidden` (a `ref_return` out-buffer), a
                //    `__`-prefixed name (`__retbuf` / `__work_N` / `__tret`), OR an attr the RETURN
                //    VALUE depends on (a local promoted to a caller buffer, e.g. a returned view
                //    `return tv[0]` keeps the local's name, so `returned.depend()` names that index).
                // Pass 1 defers (a forward-ref `&` arg lowers to Null and only looks missing then);
                // the named-param feature made an internal Null slot normal, which lost this check.
                let (a_name, a_hidden) = {
                    let a = &self.data.def(d_nr).attributes()[a_nr];
                    (a.name.clone(), a.hidden)
                };
                let promoted = a_hidden
                    || a_name.starts_with("__")
                    || self
                        .data
                        .def(d_nr)
                        .returned
                        .depend()
                        .contains(&(a_nr as u16));
                if !self.first_pass
                    && default == Value::Null
                    && !matches!(tp, Type::Optional(_))
                    && !promoted
                {
                    let fname = self.data.def(d_nr).display_name().to_string();
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "missing argument for parameter '{a_name}' of `{fname}` — the call supplies \
                         too few arguments (add it, or give the parameter a default `= …`)"
                    );
                    continue;
                }
                if let Type::Vector(content, _) = &tp {
                    assert_eq!(
                        default,
                        Value::Null,
                        "Expect a null default on database references"
                    );
                    // #306: the attr's dep list is callee-internal (attr
                    // indices); inherited verbatim it reads as CALLER var
                    // numbers and mislabels the fresh buffer a borrow.
                    let buf_tp = Type::Vector(content.clone(), Deps::none());
                    let vr = self.vars.work_refs(&buf_tp, &mut self.lexer);
                    // @PLAN51 Cluster IV: tag this work-ref so parse_code's
                    // preamble emits Set(vr, Null) regardless of vr's
                    // typedef dep list.  Without it, if-tail / recursion
                    // shapes leave vr without a first_def → slot allocator
                    // skips → codegen panics.
                    self.vars.mark_caller_hidden_buf(vr);
                    self.data.vector_def(&mut self.lexer, content);
                    all_types[a_nr] = Type::Vector(content.clone(), Deps::frame1(vr));
                    actual[a_nr] = Value::Var(vr);
                } else if let Type::Reference(content, _) = tp {
                    assert_eq!(
                        default,
                        Value::Null,
                        "Expect a null default on database references"
                    );
                    // #306: strip the callee-internal dep list (see Vector arm).
                    let buf_tp = Type::Reference(content, Deps::none());
                    let vr = self.vars.work_refs(&buf_tp, &mut self.lexer);
                    self.vars.mark_caller_hidden_buf(vr);
                    all_types[a_nr] = Type::Reference(content, Deps::frame1(vr));
                    actual[a_nr] = Value::Var(vr);
                } else if let Type::Enum(content, true, _) = tp {
                    // @P301 — struct-enums are heap records like
                    // Reference/Vector, so a struct-enum return-slot
                    // promoted to a hidden caller arg by `ref_return`
                    // (parser/control.rs) needs a pre-allocated work-ref
                    // passed in.  Without this arm the hidden param
                    // stayed `Value::Null` → emitted as `()` natively
                    // (E0308: expected DbRef, found ()).  Mirrors the
                    // Reference arm above, keeping the struct-enum
                    // discriminator in the result type.
                    assert_eq!(
                        default,
                        Value::Null,
                        "Expect a null default on database references"
                    );
                    // #306: strip the callee-internal dep list (see Vector arm).
                    let buf_tp = Type::Enum(content, true, Deps::none());
                    let vr = self.vars.work_refs(&buf_tp, &mut self.lexer);
                    self.vars.mark_caller_hidden_buf(vr);
                    all_types[a_nr] = Type::Enum(content, true, Deps::frame1(vr));
                    actual[a_nr] = Value::Var(vr);
                } else if let Type::RefVar(vtp) = &tp {
                    let mut ls = Vec::new();
                    let vr = if matches!(**vtp, Type::Text(_)) {
                        let wv = self.vars.work_text(&mut self.lexer);
                        // clear the work buffer before each call so loop
                        // iterations start fresh (matches fn-ref path in control.rs).
                        ls.push(v_set(wv, Value::Text(String::new())));
                        if default != Value::Null
                            && if let Value::Text(t) = &default {
                                !t.is_empty()
                            } else {
                                true
                            }
                        {
                            ls.push(self.cl("OpAppendText", &[Value::Var(wv), default]));
                        }
                        wv
                    } else if self.first_pass {
                        // Defer on pass 1 (#375): this missing-slot filler runs
                        // when an arg lowered to `Null`.  A `&` field arg whose
                        // owning struct's layout is not yet finalised (a forward /
                        // cross-package field still resolving) lowers to `Null`
                        // here, so it looks like a missing parameter.  Erroring
                        // would abort pass 1; on pass 2 the access lowers to a real
                        // `OpGetField` (non-Null), this filler is skipped, and a
                        // genuinely-missing non-text `&` default still errors then.
                        0
                    } else {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "Unexpected reference type {}",
                            vtp.name(&self.data)
                        );
                        0
                    };
                    ls.push(self.cl("OpCreateStack", &[Value::Var(vr)]));
                    actual[a_nr] = v_block(
                        ls,
                        Type::Reference(self.data.def_nr("reference"), Deps::frame1(vr)),
                        "default ref",
                    );
                    all_types[a_nr] = tp.clone();
                } else {
                    // default expressions may reference earlier
                    // parameters by `Var(N)` slots (e.g. `b: integer = a * 2`
                    // produces a tree with `Var(0)`).  Substitute those
                    // references with the caller's actual argument values
                    // so the emitted code uses the caller's scope, not
                    // the callee's (which wouldn't resolve at the call
                    // site).  Only parameters 0..a_nr are earlier; no
                    // recursion into the current or later default.
                    let substituted = Self::substitute_param_refs(default, &actual[..a_nr]);
                    actual[a_nr] = substituted;
                    all_types[a_nr] = tp.clone();
                }
            }
        }
    }

    /// replace `Value::Var(from)` with `Value::Var(to)` throughout
    /// a default-expression tree.  Used by `parse_arguments` to rewrite
    /// internally-allocated slot numbers into stable argument indices
    /// before the default is stored on the function definition.
    pub(crate) fn remap_var_nr(val: Value, from: u16, to: u16) -> Value {
        match val {
            Value::Var(n) if n == from => Value::Var(to),
            Value::Call(op, xs) => Value::Call(
                op,
                xs.into_iter()
                    .map(|x| Self::remap_var_nr(x, from, to))
                    .collect(),
            ),
            Value::CallRef(op, xs) => Value::CallRef(
                op,
                xs.into_iter()
                    .map(|x| Self::remap_var_nr(x, from, to))
                    .collect(),
            ),
            Value::Set(v, inner) => {
                let v = if v == from { to } else { v };
                Value::Set(v, Box::new(Self::remap_var_nr(*inner, from, to)))
            }
            Value::Insert(ops) => Value::Insert(
                ops.into_iter()
                    .map(|x| Self::remap_var_nr(x, from, to))
                    .collect(),
            ),
            other => other,
        }
    }

    /// replace `Value::Var(i)` for `i < args.len()` with `args[i]`
    /// in a default-expression tree.  Used at call sites to transplant a
    /// default's earlier-parameter references into the caller's scope.
    fn substitute_param_refs(val: Value, args: &[Value]) -> Value {
        match val {
            Value::Var(n) if (n as usize) < args.len() => args[n as usize].clone(),
            Value::Call(op, xs) => Value::Call(
                op,
                xs.into_iter()
                    .map(|x| Self::substitute_param_refs(x, args))
                    .collect(),
            ),
            Value::CallRef(op, xs) => Value::CallRef(
                op,
                xs.into_iter()
                    .map(|x| Self::substitute_param_refs(x, args))
                    .collect(),
            ),
            Value::Set(v, inner) => {
                Value::Set(v, Box::new(Self::substitute_param_refs(*inner, args)))
            }
            Value::Insert(ops) => Value::Insert(
                ops.into_iter()
                    .map(|x| Self::substitute_param_refs(x, args))
                    .collect(),
            ),
            Value::Span(b) => {
                let (pos, inner) = *b;
                let new_inner = Self::substitute_param_refs(inner, args);
                Value::with_span(pos, new_inner)
            }
            other => other,
        }
    }
    // ********************
    // * Parser functions *
    // ********************

    /// Parse data from the current lexer.
    #[allow(clippy::too_many_lines)] // two-pass parser dispatch — splitting would lose context
    fn parse_file(&mut self) {
        let start_def = self.data.definitions();
        // #255 / @PLN9: file-level `#cwd` directive — opt this program out of the
        // program-relative default so a *relative* file path resolves against the
        // process cwd (CLI-tool semantics) rather than the program's own
        // directory.  Whole-program; must precede declarations.  At file top the
        // lexer's first token can only be this directive (`#rust`/`#native`/etc.
        // are declaration-scoped, consumed later by `parse_rust`).
        if self.lexer.has_token("#") {
            match self.lexer.has_identifier().as_deref() {
                Some("cwd") => {
                    self.database.program_relative = false;
                    let _ = self.lexer.has_token(";");
                }
                other => {
                    let name = other.unwrap_or("").to_string();
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Unknown file directive '#{name}' (expected '#cwd')"
                    );
                }
            }
        }
        // Tier-0 lazy auto-`use`: the file the lexer is on right now, captured
        // before the use-loop may switch away.  Scanned for `lib::` references
        // after the use-region (see the load loop below).
        let auto_use_scan_file = self.lexer.pos().file.clone();
        // A file that writes any `use` — or the stdlib, parsed with
        // `self.default` — is in *explicit* mode: the author manages their
        // libraries by hand, so a `lib::` to an un-`use`d library is a forgotten
        // `use`, not a request to auto-load.  Skip the pre-scan for those files.
        let mut had_use = self.default;
        // Use-region fixpoint.  Pre-scan explicit `use`s, then load manifest
        // `[dependencies]`.  Loading a manifest dep `switch_to_dep`s the lexer
        // ONTO it; a MULTI-FILE dependency lands on an entry file that still has
        // its own leading `use` statements — which the pending loop, unlike this
        // pre-scan, does NOT handle.  Loop until the cursor rests on a use-free
        // file with nothing pending, so every file the main definition-loop
        // parses has had its uses processed (otherwise a dependency's legitimate
        // top `use` is misread as "use after definitions" — and never imported).
        loop {
            while self.lexer.has_token("use") {
                if let Some(id) = self.lexer.has_identifier() {
                    had_use = true;
                    // @PLN22 Phase 3 — optional library alias: `use lib as m;` → `m::fn`.
                    let lib_alias = if self.lexer.has_token("as") {
                        self.lexer.has_identifier()
                    } else {
                        None
                    };
                    // Parse optional import spec: `::*` wildcard, a single
                    // `::name [as bind]`, or the grouped `::(a [as x], b, …)`.
                    // @PLN22 Phase 4 — multiple names MUST be grouped in parentheses;
                    // the flat top-level comma list (`use lib::a, b`) is dropped (it
                    // read poorly — `b` didn't visually bind to `lib::`).
                    let spec = if self.lexer.has_token("::") {
                        if self.lexer.has_token("*") {
                            Some(ImportSpec::Wildcard)
                        } else {
                            let grouped = self.lexer.has_token("(");
                            let mut names = Vec::new();
                            while let Some(name) = self.lexer.has_identifier() {
                                // @PLN22 Phase 3 — `Name as Alias` binds the imported
                                // name under the bare alias (works inside `(…)` too).
                                let bind = if self.lexer.has_token("as") {
                                    self.lexer.has_identifier().unwrap_or_else(|| name.clone())
                                } else {
                                    name.clone()
                                };
                                names.push((name, bind));
                                if !self.lexer.has_token(",") {
                                    break;
                                }
                            }
                            if grouped {
                                self.lexer.token(")");
                            } else if names.len() > 1 {
                                // @PLN22 Phase 4 — flat comma list dropped; the names
                                // are still bound (recovery) so the rest parses cleanly.
                                diagnostic!(
                                    self.lexer,
                                    Level::Error,
                                    "import multiple names with parentheses: `use {id}::(a, b, …)`"
                                );
                            }
                            if names.is_empty() {
                                diagnostic!(
                                    self.lexer,
                                    Level::Error,
                                    "Expected name, '*', or '(' after '::'"
                                );
                                None
                            } else {
                                Some(ImportSpec::Names(names))
                            }
                        }
                    } else {
                        None
                    };
                    if self.data.use_exists(&id) {
                        let lib_source = self.data.get_source(&id);
                        // @PLN22 Phase 3 — register the library alias for `m::` access.
                        if let Some(alias) = &lib_alias {
                            self.data.use_alias(alias, lib_source);
                        }
                        // Plain `use foo` (no spec) wildcard-imports all pub defs.
                        // `use foo as m;` (alias, no spec) does NOT — it only provides
                        // the `m::` qualifier (the disambiguation escape hatch).  An
                        // explicit `::` spec is honoured in either case.
                        let import_spec = match spec {
                            Some(s) => Some(s),
                            None if lib_alias.is_some() => None,
                            None => Some(ImportSpec::Wildcard),
                        };
                        if let Some(import_spec) = import_spec {
                            self.pending_imports.push(PendingImport {
                                for_source: self.data.source,
                                lib_source,
                                spec: import_spec,
                            });
                        }
                        if !self.lexer.has_token(";") {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "Missing ';' after 'use {id}' — use statements must end with a semicolon"
                            );
                        }
                        continue;
                    }
                    let f = self.lib_path(&id);
                    let f_exists = std::path::Path::new(&f).exists() || {
                        #[cfg(feature = "wasm")]
                        {
                            crate::wasm::virt_fs_get(&f).is_some()
                        }
                        #[cfg(not(feature = "wasm"))]
                        {
                            false
                        }
                    };
                    if f_exists {
                        let cur = &self.lexer.pos().file;
                        self.todo_files.push((cur.clone(), self.data.source));
                        self.data.use_add(&id);
                        // @PLN22 Phase 3 — register the library alias now that the lib's
                        // source exists (use_add set self.data.source to it).  The
                        // import itself is recorded on the second encounter (via
                        // todo_files re-parse with use_exists=true).
                        if let Some(alias) = &lib_alias {
                            self.data.use_alias(alias, self.data.source);
                        }
                        drop(spec);
                        self.switch_to_dep(&f);
                    } else {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "Library '{id}' not found — searched lib/, lib_dirs, and sibling packages"
                        );
                        self.lexer.has_token(";");
                    }
                }
            }
            // PKG.3: load transitive dependencies discovered during manifest reading.
            // Dependencies are queued by lib_path_manifest when it reads [dependencies].
            while !self.pending_pkg_deps.is_empty() {
                let deps = std::mem::take(&mut self.pending_pkg_deps);
                for (dep_id, parent_dir) in deps {
                    if self.data.use_exists(&dep_id) {
                        continue;
                    }
                    // First try the sibling package directory (same parent as the
                    // depending package), then fall back to the normal lib_path search.
                    let f = if let Some(entry) = self.lib_path_manifest(&parent_dir, &dep_id) {
                        entry
                    } else {
                        self.lib_path(&dep_id)
                    };
                    if std::path::Path::new(&f).exists() {
                        let cur = &self.lexer.pos().file;
                        self.todo_files.push((cur.clone(), self.data.source));
                        self.data.use_add(&dep_id);
                        self.switch_to_dep(&f);
                    }
                }
            }
            // Fixpoint exit: the cursor rests on a use-free file and no manifest
            // dependency is still queued.  `peek_token` (not `has_token`) so the
            // check never consumes a leading `use` that the next iteration must see.
            if !self.lexer.peek_token("use") && self.pending_pkg_deps.is_empty() {
                break;
            }
        }
        // Tier-0 lazy auto-`use`: scan this file for `lib::` references and load
        // any that name an unloaded, available library — here in the use-region,
        // before the defs-loop, exactly like an explicit `use` (so the two-pass
        // model never sees a redefinition).  Only when the lexer is still on the
        // scanned file (the use-loop has not switched away to an explicitly-used
        // library — that file re-parses via `todo_files` and scans itself then).
        // Resolve every path first (before a `switch` changes the lexer's cwd),
        // then load with the same `todo_files` + `use_add` + `switch` shape as
        // the `pending_pkg_deps` loop.  A name that is not a real library is
        // skipped and falls through to the normal "unknown" error.
        // `!had_use`: explicit-mode files (any `use`, and the stdlib) are skipped
        // entirely — the author manages their libraries by hand there.  Read +
        // scan each remaining file at most once (cache keyed by path).
        if !had_use && self.lexer.pos().file == auto_use_scan_file {
            let (refs, calls) = if let Some(c) = self.auto_use_scan_cache.get(&auto_use_scan_file) {
                c.clone()
            } else {
                let src = Self::read_source(&auto_use_scan_file);
                let pair = (
                    crate::libscan::scan_qualified_lib_refs(&src),
                    crate::libscan::scan_method_calls(&src),
                );
                self.auto_use_scan_cache
                    .insert(auto_use_scan_file.clone(), pair.clone());
                pair
            };
            // Tier-0: `lib::x` — the library is named directly.
            let mut to_load: Vec<String> = Vec::new();
            for name in refs {
                if self.data.use_exists(&name) || self.data.def_nr(&name) != u32::MAX {
                    continue;
                }
                if !to_load.contains(&name) {
                    to_load.push(name);
                }
            }
            // Tier-1: `obj.method(…)` — map the method to its providing package
            // via the trigger surface of the current package (+ trigger-enabled
            // deps), derived once and cached.
            if !calls.is_empty() {
                let map = self.trigger_map(&auto_use_scan_file);
                // Catalog fallback is built lazily — only read index.json once a
                // method misses the local (current package + deps) trigger map.
                let mut catalog: Option<std::collections::HashMap<String, String>> = None;
                for m in calls {
                    let pkg = if let Some(p) = map.get(&m) {
                        Some(p.clone())
                    } else {
                        if catalog.is_none() {
                            catalog = Some(self.catalog_trigger_map());
                        }
                        catalog.as_ref().and_then(|c| c.get(&m).cloned())
                    };
                    if let Some(pkg) = pkg
                        && !self.data.use_exists(&pkg)
                        && !to_load.contains(&pkg)
                    {
                        to_load.push(pkg);
                    }
                }
            }
            // Resolve every path first (before a `switch` changes the cwd), then
            // load with the proven todo_files + use_add + switch shape.
            let mut resolved: Vec<(String, String)> = Vec::new();
            for n in to_load {
                if self.data.use_exists(&n) {
                    continue;
                }
                let f = self.lib_path(&n);
                if std::path::Path::new(&f).exists() {
                    resolved.push((n, f));
                }
            }
            for (name, f) in resolved {
                if self.data.use_exists(&name) {
                    continue;
                }
                let cur = self.lexer.pos().file.clone();
                self.todo_files.push((cur, self.data.source));
                self.data.use_add(&name);
                self.switch_to_dep(&f);
            }
        }
        // Apply wildcard/selective imports queued for this source now that the while-use loop
        // has resolved all libraries.  Must run before the definitions loop so that imported
        // names are visible when function bodies and type annotations are parsed.
        self.apply_pending_imports();
        self.file += 1;
        self.line = 0;
        loop {
            let is_pub = self.lexer.has_token("pub");
            let before = self.data.definitions();
            if self.lexer.diagnostics().level() == Level::Fatal
                || (!self.parse_capability()
                    && !self.parse_enum()
                    && !self.parse_typedef()
                    && !self.parse_function()
                    && !self.parse_struct()
                    && !self.parse_interface()
                    && !self.parse_constant())
            {
                break;
            }
            // mark newly created definitions as pub-visible.
            if is_pub {
                for d_nr in before..self.data.definitions() {
                    self.data.def_mut(d_nr).pub_visible = true;
                }
            }
        }
        let res = self.lexer.peek();
        if res.has != LexItem::None && self.lexer.diagnostics().level() != Level::Fatal {
            if self.lexer.peek_token("use") {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "use statements must appear before all definitions"
                );
            } else {
                let token = match &res.has {
                    crate::lexer::LexItem::Token(s) | crate::lexer::LexItem::Identifier(s) => {
                        format!("'{s}'")
                    }
                    crate::lexer::LexItem::CString(s) => format!("\"{s}\""),
                    other => format!("{other:?}"),
                };
                diagnostic!(self.lexer, Level::Error, "Syntax error: unexpected {token}");
            }
        }
        // defer `Undefined type` errors to `resolve_deferred_unknowns`
        // so forward-references across cyclic intra-package `use` declarations
        // get a chance to resolve once both sides of the cycle are registered.
        typedef::actual_types_deferred(
            &mut self.data,
            &mut self.database,
            &mut self.lexer,
            start_def,
            Some(&mut self.deferred_unknown),
        );
        typedef::fill_all(
            &mut self.data,
            &mut self.database,
            &mut self.lexer,
            start_def,
        );
        self.database.finish();
        // Validate layouts of all registered types — catches late-
        // mutation bugs (e.g. P191's bookkeeping fields landing at
        // overlapping positions because finish_type already ran).
        // Skip when the parser has already reported errors: an
        // incomplete struct (e.g. a self-referential type that the
        // parser correctly rejected) can have unlaid fields whose
        // position == u16::MAX, and re-flagging that here just adds
        // noise on top of the real diagnostic.
        let already_failed = matches!(
            self.lexer.diagnostics().level(),
            Level::Error | Level::Fatal
        );
        if !already_failed {
            for issue in self.database.validate_all_layouts() {
                diagnostic!(self.lexer, Level::Error, "type layout: {}", issue);
            }
        }
        self.enum_fn();
        let lvl = self.lexer.diagnostics().level();
        if lvl == Level::Error || lvl == Level::Fatal {
            return;
        }
        // Parse all files left in the todo_files list, as they are halted to parse a use file.
        while let Some((t, s)) = self.todo_files.pop() {
            self.lexer.switch(&t);
            self.data.source = s;
            self.parse_file();
        }
    }

    /// Apply all pending imports whose target source matches the currently active source.
    fn apply_pending_imports(&mut self) {
        let cur = self.data.source;
        // Partition: imports targeting `cur` are applied now; others wait for their source.
        let mut to_apply = Vec::new();
        let mut remaining = Vec::new();
        for pi in self.pending_imports.drain(..) {
            if pi.for_source == cur {
                to_apply.push(pi);
            } else {
                remaining.push(pi);
            }
        }
        self.pending_imports = remaining;
        for pi in to_apply {
            // retain a copy so `resolve_deferred_unknowns` can re-apply
            // with overwrite semantics after a cyclic `use` has finished
            // registering the partner file's definitions.
            self.applied_imports.push(pi.clone());
            match pi.spec {
                ImportSpec::Wildcard => {
                    self.data.import_all(pi.lib_source, cur);
                }
                ImportSpec::Names(names) => {
                    for (name, bind) in &names {
                        if !self.data.import_name(pi.lib_source, cur, name, bind) {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "Name '{name}' not found in library"
                            );
                        }
                    }
                }
            }
        }
    }

    /// Read a source file's content for the Tier-0 auto-`use` pre-scan.
    /// Honours the wasm VirtFS; an empty string on a read error (the scan then
    /// finds nothing and normal resolution proceeds unchanged).
    fn read_source(filename: &str) -> String {
        #[cfg(feature = "wasm")]
        if let Some(c) = crate::wasm::virt_fs_get(filename) {
            return c;
        }
        std::fs::read_to_string(filename).unwrap_or_default()
    }

    /// Tier-1: build (once, cached) the `method name -> providing package` map
    /// from the current package's declared triggers — `[triggers] enabled` plus
    /// the text-methods derived from its source — and those of its
    /// trigger-enabled dependencies.  The package is located by walking up from
    /// `from_file` to the nearest `loft.toml`.
    fn trigger_map(&mut self, from_file: &str) -> std::collections::HashMap<String, String> {
        if let Some(m) = &self.auto_use_trigger_map {
            return m.clone();
        }
        let mut map = std::collections::HashMap::new();
        let mut dir = std::path::Path::new(from_file)
            .parent()
            .map(std::path::Path::to_path_buf);
        while let Some(d) = dir {
            let toml = d.join("loft.toml");
            if toml.exists() {
                Self::add_pkg_triggers(&toml, &d, &mut map);
                if let Some(man) = crate::manifest::read_manifest(&toml.to_string_lossy()) {
                    for (dep, _ver) in &man.dependencies {
                        if let Some(entry) = self.lib_path_manifest(&d.to_string_lossy(), dep)
                            && let Some(root) = std::path::Path::new(&entry)
                                .parent()
                                .and_then(|p| p.parent())
                        {
                            Self::add_pkg_triggers(&root.join("loft.toml"), root, &mut map);
                        }
                    }
                }
                break;
            }
            dir = d.parent().map(std::path::Path::to_path_buf);
        }
        self.auto_use_trigger_map = Some(map.clone());
        map
    }

    /// Add a package's derived text-method triggers (`method -> package`) to
    /// `map`, when the package opts in via `[triggers] enabled`.
    fn add_pkg_triggers(
        toml: &std::path::Path,
        pkg_root: &std::path::Path,
        map: &mut std::collections::HashMap<String, String>,
    ) {
        let Some(man) = crate::manifest::read_manifest(&toml.to_string_lossy()) else {
            return;
        };
        if !man.trigger_enabled {
            return;
        }
        let Some(name) = man.name else { return };
        let entry = man.entry.unwrap_or_else(|| format!("src/{name}.loft"));
        let src = std::fs::read_to_string(pkg_root.join(&entry)).unwrap_or_default();
        for mt in crate::triggers::derive_triggers(&src).methods {
            map.entry(mt.name).or_insert_with(|| name.clone());
        }
    }

    /// Tier-1 lazy *catalog* fallback (`method -> package`), read once from the
    /// cached registry `index.json`.  This is what makes `line.matches(p)` work
    /// against a package the user has NOT declared as a dependency: the local
    /// `trigger_map` misses, so we look the method up across the whole catalog,
    /// get the package name, and hand it to `lib_path` — which resolves it via
    /// the same lockfile → installed → auto-install chain an explicit `use`
    /// would take.  Ambiguity is dropped by `trigger_providers` (registry
    /// submission already rejects true collisions).  Cached (empty on absent
    /// catalog) so the file read happens at most once per parse.
    #[cfg(feature = "registry")]
    fn catalog_trigger_map(&mut self) -> std::collections::HashMap<String, String> {
        if let Some(m) = &self.auto_use_catalog_map {
            return m.clone();
        }
        let mut map = std::collections::HashMap::new();
        let (idx_path, _, _) = crate::registry_index::index_paths();
        if let Ok(content) = std::fs::read_to_string(&idx_path)
            && let Ok(index) = crate::registry_index::parse_index(&content)
        {
            map = crate::registry_index::trigger_providers(&index)
                .into_iter()
                .collect();
        }
        self.auto_use_catalog_map = Some(map.clone());
        map
    }

    /// No-op when the registry feature is off — there is no cached catalog to
    /// consult, so Tier-1 resolves only via declared dependencies.
    #[cfg(not(feature = "registry"))]
    #[allow(clippy::unused_self)]
    fn catalog_trigger_map(&mut self) -> std::collections::HashMap<String, String> {
        std::collections::HashMap::new()
    }

    fn lib_path(&mut self, id: &str) -> String {
        // Under the `wasm` feature, VIRT_FS wins over filesystem lookups.
        #[cfg(feature = "wasm")]
        if crate::wasm::virt_fs_get(&format!("{id}.loft")).is_some() {
            return format!("{id}.loft");
        }

        let cur_script = self.lexer.pos().file.replace(other_sep(), sep_str());
        let cur_dir_owned = script_dir(&cur_script).to_string();
        let base_dir_owned = tests_base_dir(&cur_dir_owned).to_string();
        let cur_dir = cur_dir_owned.as_str();
        let base_dir = base_dir_owned.as_str();

        // Try each strategy in order; first one that finds the file wins.
        // `f` starts as the cheapest guess (project-local `lib/<id>.loft` or
        // `<id>.loft`); subsequent strategies overwrite only when `f` does
        // not yet resolve to an existing file.
        // Dep-shadowing guard: a name declared under [dependencies] in the
        // current package's manifest must resolve as a dependency — never to
        // a same-named `.loft` file inside the declaring package.  Without
        // this, `use server` in a package that also contains `server.loft`
        // loads the package file (the package root sits in `lib_dirs` for
        // intra-package `use`), silently shadowing the library: its types
        // vanish and every consumer in the package breaks.
        let shadow_root = self
            .package_declared_deps(cur_dir)
            .filter(|(_, deps)| deps.contains(id))
            .map(|(root, _)| root);
        let blocked = |candidate: &str| {
            shadow_root.as_deref().is_some_and(|root| {
                // Canonicalize so relative candidates (cwd inside the
                // package) and the canonical root compare in one space.
                let cand = std::path::Path::new(candidate);
                cand.canonicalize()
                    .unwrap_or_else(|_| cand.to_path_buf())
                    .starts_with(root)
            })
        };

        let mut f = Self::probe_project_lib(id);
        if blocked(&f) {
            f = format!("{id}.loft");
        }
        Self::probe_dir_lib(id, cur_dir, &mut f);
        Self::probe_dir_lib(id, base_dir, &mut f);
        if blocked(&f) {
            f = format!("{id}.loft");
        }
        self.probe_manifest_path_dep(id, cur_dir, &mut f);
        self.probe_sibling_package(id, cur_dir, &mut f);
        Self::probe_script_sibling_dir(id, &cur_script, &mut f);
        if blocked(&f) {
            f = format!("{id}.loft");
        }
        self.probe_cmdline_lib_dirs(id, &mut f);
        if blocked(&f) {
            f = format!("{id}.loft");
        }
        self.probe_cmdline_lib_dirs_manifest(id, &mut f);
        Self::probe_loft_lib_flat(id, &mut f);
        self.probe_loft_lib_manifest(id, &mut f);
        self.probe_user_installed(id, &mut f);
        self.probe_sidecar_lockfile(id, &mut f);
        self.probe_project_lockfile(id, &mut f);
        self.probe_registry_installed(id, &mut f);
        self.probe_auto_install(id, &mut f);
        Self::probe_cur_dir_flat(id, cur_dir, &mut f);
        Self::probe_base_dir_flat(id, base_dir, &mut f);
        if blocked(&f) {
            f = format!("{id}.loft");
        }
        f
    }

    /// The package context owning `cur_dir`: the nearest ancestor directory
    /// holding a `loft.toml`, plus that manifest's declared dependency names.
    /// Cached per directory (manifests don't change mid-parse).
    fn package_declared_deps(
        &mut self,
        cur_dir: &str,
    ) -> Option<(String, std::collections::HashSet<String>)> {
        if let Some(cached) = self.pkg_dep_cache.get(cur_dir) {
            return cached.clone();
        }
        let mut found = None;
        let start = if cur_dir.is_empty() { "." } else { cur_dir };
        let mut search = std::path::Path::new(start).canonicalize().ok();
        while let Some(dir) = search {
            let manifest_path = dir.join("loft.toml");
            if manifest_path.exists() {
                if let Some(manifest) =
                    crate::manifest::read_manifest(&manifest_path.to_string_lossy())
                {
                    let deps: std::collections::HashSet<String> = manifest
                        .dependencies
                        .iter()
                        .map(|(name, _)| name.clone())
                        .collect();
                    found = Some((dir.to_string_lossy().to_string(), deps));
                }
                break;
            }
            search = dir.parent().map(std::path::Path::to_path_buf);
        }
        self.pkg_dep_cache
            .insert(cur_dir.to_string(), found.clone());
        found
    }

    /// The root project's declared version constraint for dependency `id`, if
    /// any — read once (cached) from the main script's nearest-ancestor
    /// `loft.toml` via `source_dir`.  Threaded into source-level auto-installs
    /// so a root pin overrides the default "resolve newest", including for a
    /// package pulled transitively by a lib that didn't pin it itself.  This is
    /// what makes `glb = "=0.1.0"` (exact) — or any range — an honoured option.
    #[cfg(feature = "registry")]
    fn root_dep_constraint(&mut self, id: &str) -> Option<String> {
        if self.root_dep_pins.is_none() {
            let mut map = std::collections::HashMap::new();
            if let Some(root) = Self::find_project_root(&self.database.source_dir) {
                let manifest_path = root.join("loft.toml");
                if let Some(manifest) =
                    crate::manifest::read_manifest(&manifest_path.to_string_lossy())
                {
                    for (name, req) in manifest.dependencies {
                        map.insert(name, req);
                    }
                }
            }
            self.root_dep_pins = Some(map);
        }
        self.root_dep_pins.as_ref().and_then(|m| m.get(id).cloned())
    }

    /// Initial guess: the project-supplied `lib/<id>.loft`, falling back to
    /// `<id>.loft` in the current working directory.
    fn probe_project_lib(id: &str) -> String {
        let f = format!("lib{0}{id}.loft", sep_str());
        if std::path::Path::new(&f).exists() {
            f
        } else {
            format!("{id}.loft")
        }
    }

    /// `<dir>/lib/<id>.loft` — a `lib/` directory relative to `dir`
    /// (called for the script's own dir, then for the base dir when the
    /// script lives inside a `/tests/` tree).
    fn probe_dir_lib(id: &str, dir: &str, f: &mut String) {
        if !dir.is_empty() && !std::path::Path::new(f).exists() {
            *f = format!("{dir}{0}lib{0}{id}.loft", sep_str());
        }
    }

    /// #337 / PACKAGES.md resolution step 2 — `[dependencies] <id> =
    /// { path = "…" }` in the consuming package's `loft.toml`.  Walk up
    /// from `cur_dir` to the owning manifest; if it declares `id` as a
    /// path dependency, resolve the path relative to the manifest's
    /// directory and locate the dep package's entry file (its own
    /// `[library] entry`, defaulting to `src/<id>.loft`).  Registers the
    /// dep's manifest so its `#native` symbols resolve, mirroring
    /// `probe_sibling_package`.
    fn probe_manifest_path_dep(&mut self, id: &str, cur_dir: &str, f: &mut String) {
        if std::path::Path::new(f).exists() || cur_dir.is_empty() {
            return;
        }
        let mut search_dir = std::path::Path::new(cur_dir).to_path_buf();
        loop {
            let manifest_path = search_dir.join("loft.toml");
            if manifest_path.exists() {
                let dep_rel = crate::manifest::read_manifest(&manifest_path.to_string_lossy())
                    .and_then(|m| {
                        m.dependencies.iter().find_map(|(name, value)| {
                            (name == id)
                                .then(|| crate::manifest::extract_path_dep(value))
                                .flatten()
                                .map(str::to_string)
                        })
                    });
                if let Some(rel) = dep_rel {
                    let pkg_root = search_dir.join(rel);
                    let dep_manifest = pkg_root.join("loft.toml");
                    let entry = dep_manifest
                        .exists()
                        .then(|| crate::manifest::read_manifest(&dep_manifest.to_string_lossy()))
                        .flatten()
                        .and_then(|m| m.entry)
                        .unwrap_or_else(|| format!("src{}{id}.loft", sep_str()));
                    let file = pkg_root.join(entry);
                    if file.exists() {
                        *f = file.to_string_lossy().to_string();
                        if dep_manifest.exists() {
                            self.register_native_manifest(&dep_manifest, &pkg_root);
                        }
                    }
                }
                break;
            }
            let Some(p) = search_dir.parent() else {
                break;
            };
            search_dir = p.to_path_buf();
        }
    }

    /// Walk up from `cur_dir` looking for a `loft.toml`; on hit, the package's
    /// parent directory may contain sibling packages.  When the sibling is
    /// found directly (not via `lib_path_manifest`), the sibling's own
    /// `loft.toml` must be registered so its `#native` symbols resolve.
    fn probe_sibling_package(&mut self, id: &str, cur_dir: &str, f: &mut String) {
        if std::path::Path::new(f).exists() || cur_dir.is_empty() {
            return;
        }
        let mut search_dir = std::path::Path::new(cur_dir).to_path_buf();
        loop {
            if search_dir.join("loft.toml").exists() {
                if let Some(parent) = search_dir.parent()
                    && let Some(path) = Self::find_sibling_file(parent, id)
                {
                    *f = path.to_string_lossy().to_string();
                    let pkg_root = parent.join(id);
                    let manifest = pkg_root.join("loft.toml");
                    if manifest.exists() {
                        self.register_native_manifest(&manifest, &pkg_root);
                    }
                }
                break;
            }
            let Some(p) = search_dir.parent() else {
                break;
            };
            search_dir = p.to_path_buf();
        }
    }

    /// Sibling lookup: prefer `<parent>/<id>/src/<id>.loft`, fall back to
    /// flat `<parent>/<id>.loft`.
    fn find_sibling_file(parent: &std::path::Path, id: &str) -> Option<std::path::PathBuf> {
        let nested = parent.join(id).join("src").join(format!("{id}.loft"));
        if nested.exists() {
            return Some(nested);
        }
        let flat = parent.join(format!("{id}.loft"));
        flat.exists().then_some(flat)
    }

    /// A directory named after the current script (minus the `.loft` suffix).
    fn probe_script_sibling_dir(id: &str, cur_script: &str, f: &mut String) {
        if !std::path::Path::new(f).exists() && cur_script.len() >= 5 {
            *f = format!(
                "{}{}{id}.loft",
                &cur_script[0..cur_script.len() - 5],
                sep_str()
            );
        }
    }

    /// `--lib` / `--project` command-line flag directories, flat layout.
    /// Registers any discovered `loft.toml` in the file's ancestry.
    fn probe_cmdline_lib_dirs(&mut self, id: &str, f: &mut String) {
        if std::path::Path::new(f).exists() {
            return;
        }
        let lib_dirs = self.lib_dirs.clone();
        for l in &lib_dirs {
            let candidate = format!("{l}{}{id}.loft", sep_str());
            if std::path::Path::new(&candidate).exists() {
                f.clone_from(&candidate);
                self.register_manifest_in_ancestors(&candidate);
                break;
            }
        }
    }

    /// Scan ancestor directories for a `loft.toml` and register the first one
    /// found.  Called when a file was resolved directly (not through
    /// `lib_path_manifest`), so its owning package's native crate would
    /// otherwise never be registered.
    fn register_manifest_in_ancestors(&mut self, candidate: &str) {
        let mut search = std::path::Path::new(candidate)
            .parent()
            .map(std::path::Path::to_path_buf);
        while let Some(dir) = search {
            let manifest = dir.join("loft.toml");
            if manifest.exists() {
                self.register_native_manifest(&manifest, &dir);
                return;
            }
            search = dir.parent().map(std::path::Path::to_path_buf);
        }
    }

    /// `--lib` / `--project` directories, packaged layout
    /// (`<dir>/<id>/src/<id>.loft`).
    fn probe_cmdline_lib_dirs_manifest(&mut self, id: &str, f: &mut String) {
        if std::path::Path::new(f).exists() {
            return;
        }
        let lib_dirs = self.lib_dirs.clone();
        for l in &lib_dirs {
            if let Some(entry) = self.lib_path_manifest(l, id) {
                *f = entry;
                return;
            }
        }
    }

    /// `LOFT_LIB` env var, flat layout (`<dir>/<id>.loft`).
    fn probe_loft_lib_flat(id: &str, f: &mut String) {
        if std::path::Path::new(f).exists() {
            return;
        }
        let Some(v) = env::var_os("LOFT_LIB") else {
            return;
        };
        for l in env::split_paths(&v) {
            let candidate = l.join(format!("{id}.loft"));
            if candidate.exists() {
                *f = candidate.to_string_lossy().replace(other_sep(), sep_str());
                return;
            }
        }
    }

    /// `LOFT_LIB` env var, packaged layout (via `lib_path_manifest`).
    fn probe_loft_lib_manifest(&mut self, id: &str, f: &mut String) {
        if std::path::Path::new(f).exists() {
            return;
        }
        let Some(v) = env::var_os("LOFT_LIB") else {
            return;
        };
        for l in env::split_paths(&v) {
            let l = l.to_string_lossy().replace(other_sep(), sep_str());
            if let Some(entry) = self.lib_path_manifest(&l, id) {
                *f = entry;
                return;
            }
        }
    }

    /// `~/.loft/lib/<id>/src/<id>.loft` — packages installed via `loft install`.
    fn probe_user_installed(&mut self, id: &str, f: &mut String) {
        if std::path::Path::new(f).exists() {
            return;
        }
        let home = env::var("HOME")
            .or_else(|_| env::var("USERPROFILE"))
            .unwrap_or_default();
        if home.is_empty() {
            return;
        }
        let user_lib = format!("{home}/.loft/lib");
        if let Some(entry) = self.lib_path_manifest(&user_lib, id) {
            *f = entry;
        }
    }

    /// @PLAN12 Phase 6.7 — per-invocation advisory classifier.
    ///
    /// Called by each lockfile-based probe (sidecar / project /
    /// registry-installed) after resolving a `(name, version)`
    /// tuple.  Loads the cached advisory feed (lazily, once per
    /// process), classifies the tuple, and emits severity-tiered
    /// output:
    ///
    /// - `security_critical` → loud `error:` block; **continue
    ///   running** (the user may be running their code precisely to
    ///   test the upgrade, and security fixes can introduce
    ///   breaking changes; refusing here is worse than warning).
    ///   Opt-in refusal: `LOFT_STRICT_SECURITY=1` (env var) OR
    ///   `--strict-security` (CLI flag, intended for CI gates) — in
    ///   either, critical aborts with `process::exit(3)` UNLESS
    ///   `LOFT_SECURITY_OVERRIDE=<advisory-id>` is set (comma-
    ///   separated for multiple).
    /// - `security_high` → loud `warning:` block; continue.
    /// - `security_low` / `bug` → one-line warning.
    /// - `deprecated` → one-line note.
    ///
    /// The Cargo / npm precedent: `cargo audit` defaults to warn,
    /// `--deny warnings` is opt-in for CI.  Pure refusal blocks the
    /// user precisely when they're trying to ship a fix or assess
    /// the upgrade — the wrong default.
    ///
    /// Dedupes via `self.advisory_checked` so a package surfaced by
    /// multiple `use` paths (sidecar + auto-install re-probe) only
    /// reports once.  Feed loading is best-effort offline + cache-
    /// only: no network fetch fires during script execution, only
    /// during explicit `loft audit` / install commands.
    #[cfg(feature = "registry")]
    fn check_advisory(&mut self, name: &str, version: &str) {
        let key = (name.to_string(), version.to_string());
        if !self.advisory_checked.insert(key) {
            return;
        }
        let Some(feed) = Self::advisory_feed() else {
            return;
        };
        let hits = crate::registry_advisories::classify(name, version, feed);
        if hits.is_empty() {
            return;
        }
        let strict = std::env::var("LOFT_STRICT_SECURITY").is_ok();
        let overrides: std::collections::HashSet<String> = std::env::var("LOFT_SECURITY_OVERRIDE")
            .unwrap_or_default()
            .split(',')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        for hit in &hits {
            use crate::registry_advisories::Severity;
            let overridden = overrides.contains(&hit.advisory_id);
            match hit.severity {
                Severity::SecurityCritical => {
                    // Loud error block — same in strict and non-strict
                    // mode.  The DIFFERENCE is only whether we then
                    // refuse to proceed.
                    eprintln!(
                        "error: {} {} was yanked for a security vulnerability",
                        hit.package, hit.version
                    );
                    eprintln!("  advisory: {}", hit.advisory_id);
                    eprintln!("  summary:  {}", hit.summary);
                    if let Some(fix) = &hit.fixed_in {
                        eprintln!(
                            "  fix:      {} >= {} (run `loft install {}@{}`)",
                            hit.package, fix, hit.package, fix
                        );
                    }
                    if strict && !overridden {
                        eprintln!("  refused under LOFT_STRICT_SECURITY=1");
                        eprintln!(
                            "  override (audit-trail required): LOFT_SECURITY_OVERRIDE={}",
                            hit.advisory_id
                        );
                        std::process::exit(3);
                    }
                    if strict && overridden {
                        eprintln!(
                            "[security] override applied: {} ({} {})",
                            hit.advisory_id, hit.package, hit.version
                        );
                    }
                    // Non-strict mode: error block printed, but we
                    // proceed.  User retains the right to run their
                    // code while they investigate / fix.
                }
                Severity::SecurityHigh => {
                    eprintln!(
                        "warning: {} {} has a known security issue",
                        hit.package, hit.version
                    );
                    eprintln!("  advisory: {}", hit.advisory_id);
                    eprintln!("  summary:  {}", hit.summary);
                    if let Some(fix) = &hit.fixed_in {
                        eprintln!("  fix:      {} >= {}", hit.package, fix);
                    }
                }
                Severity::SecurityLow | Severity::Bug => {
                    eprintln!(
                        "warning: {} {} — {} (advisory {})",
                        hit.package, hit.version, hit.summary, hit.advisory_id
                    );
                }
                Severity::Deprecated => {
                    eprintln!(
                        "note: {} {} is deprecated — {} (advisory {})",
                        hit.package, hit.version, hit.summary, hit.advisory_id
                    );
                }
            }
        }
    }

    /// No-op when registry feature is off.
    #[cfg(not(feature = "registry"))]
    #[allow(clippy::unused_self, dead_code)]
    fn check_advisory(&mut self, _name: &str, _version: &str) {}

    /// Lazy process-global advisory feed loader.  Cached for the
    /// duration of the process; offline-respecting; never refetches
    /// from network mid-parse (the explicit `loft audit` flow does
    /// that).  Returns `None` when the registry doesn't yet host an
    /// advisory feed (current real state — HTTP 404 soft-fails to
    /// "no advisories").
    #[cfg(feature = "registry")]
    fn advisory_feed() -> Option<&'static crate::registry_advisories::AdvisoryFeed> {
        use std::sync::OnceLock;
        static FEED: OnceLock<Option<crate::registry_advisories::AdvisoryFeed>> = OnceLock::new();
        FEED.get_or_init(|| {
            let opts = crate::registry_advisories::LoadOptions {
                allow_unsigned: true,
                // Mid-parse, never hit the network — always cache-only.
                // `loft audit` does the freshness refresh on demand.
                offline: true,
                refresh: false,
            };
            crate::registry_advisories::load_or_fetch(&opts).unwrap_or(None)
        })
        .as_ref()
    }

    /// @PLAN12 Phase 6.6 — walk-up project root detection.
    ///
    /// From the script's directory, walks up to `/` looking for a
    /// `loft.toml`.  Returns the dir containing it (the project
    /// root) — used by:
    /// - `probe_project_lockfile` to find the lockfile that pins
    ///   manifest-declared deps.
    /// - `probe_auto_install` to redirect the lockfile WRITE path
    ///   to the project root, so auto-installs in a project
    ///   context update the project's lockfile rather than cwd's.
    ///
    /// Returns `None` for script-mode invocations (no `loft.toml`
    /// anywhere in the parent chain).  Script mode falls back to
    /// cwd's `loft.lock` (existing behaviour) or to the sidecar
    /// (when `loft pin <script>` has been run).
    #[cfg(feature = "registry")]
    fn find_project_root(script_path: &str) -> Option<std::path::PathBuf> {
        let p = std::path::Path::new(script_path);
        if script_path.is_empty() {
            return None;
        }
        let start_dir = if p.is_dir() {
            p.to_path_buf()
        } else {
            let parent = p.parent()?;
            if parent.as_os_str().is_empty() {
                std::env::current_dir().ok()?
            } else {
                parent.to_path_buf()
            }
        };
        // Canonicalize so the walk-up doesn't terminate prematurely
        // on relative `./` prefixes that loop on themselves.
        let abs_start = std::fs::canonicalize(&start_dir).unwrap_or(start_dir);
        let mut cur = abs_start.as_path();
        loop {
            if cur.join("loft.toml").exists() {
                return Some(cur.to_path_buf());
            }
            let parent = cur.parent()?;
            if parent == cur {
                return None;
            }
            cur = parent;
        }
    }

    /// @PLAN12 Phase 6.6 — project-mode lockfile resolution.
    ///
    /// Walks up from the script's directory looking for `loft.toml`;
    /// if found, reads the adjacent `loft.lock` and resolves `id`
    /// against it.  Means a script at `myproject/src/foo.loft`
    /// resolves registry libraries via `myproject/loft.lock` no
    /// matter where `loft` is invoked from (cwd-independent).
    ///
    /// Cargo-style: the project root owns the lockfile; cwd is
    /// irrelevant.  Inserted in the probe chain BEFORE
    /// `probe_registry_installed` (which uses cwd as a script-mode
    /// fallback when no project is found).
    #[cfg(feature = "registry")]
    fn probe_project_lockfile(&mut self, id: &str, f: &mut String) {
        if std::path::Path::new(f).exists() {
            return;
        }
        let cur_script = self.lexer.pos().file.replace(other_sep(), sep_str());
        let Some(project_root) = Self::find_project_root(&cur_script) else {
            return;
        };
        let lock_path = project_root.join("loft.lock");
        if !lock_path.exists() {
            return;
        }
        let lock = match crate::lockfile::read_lockfile(&lock_path) {
            Ok(Some(l)) => l,
            _ => return,
        };
        let version = match lock.packages.iter().find(|p| p.name == id) {
            Some(p) => p.version.clone(),
            None => return,
        };
        let install_dir = crate::registry_index::extract_dir(id, &version);
        let parent = match install_dir.parent().and_then(std::path::Path::to_str) {
            Some(p) => p.to_string(),
            None => return,
        };
        let versioned_name: String = match install_dir.file_name().and_then(std::ffi::OsStr::to_str)
        {
            Some(n) => n.to_string(),
            None => return,
        };
        if let Some(entry) = self.lib_path_manifest(&parent, &versioned_name) {
            self.check_advisory(id, &version);
            *f = entry;
        }
    }

    /// No-op when registry feature is off.
    #[cfg(not(feature = "registry"))]
    #[allow(clippy::unused_self)]
    fn probe_project_lockfile(&mut self, _id: &str, _f: &mut String) {}

    /// @PLAN12 Phase 6.6 — sidecar lockfile next to the script.
    ///
    /// `<script>.loft.lock` (e.g. `hello.loft.lock` next to `hello.loft`)
    /// pins the registry versions a one-file script uses.  Generated by
    /// `loft pin <script>`; takes precedence over the cwd `loft.lock`
    /// because the sidecar belongs TO the script, while cwd's lockfile
    /// belongs to wherever the user happens to be invoking from.
    ///
    /// Without the sidecar, single-file scripts inherit cwd's lockfile
    /// (or auto-install latest active).  With it, the script is
    /// reproducible regardless of cwd or registry-state drift.
    #[cfg(feature = "registry")]
    fn probe_sidecar_lockfile(&mut self, id: &str, f: &mut String) {
        if std::path::Path::new(f).exists() {
            return;
        }
        let cur_script = self.lexer.pos().file.replace(other_sep(), sep_str());
        if cur_script.is_empty() {
            return;
        }
        let sidecar = format!("{cur_script}.lock");
        if !std::path::Path::new(&sidecar).exists() {
            return;
        }
        let lock = match crate::lockfile::read_lockfile(std::path::Path::new(&sidecar)) {
            Ok(Some(l)) => l,
            _ => return,
        };
        let version = match lock.packages.iter().find(|p| p.name == id) {
            Some(p) => p.version.clone(),
            None => return,
        };
        let install_dir = crate::registry_index::extract_dir(id, &version);
        let parent = match install_dir.parent().and_then(std::path::Path::to_str) {
            Some(p) => p.to_string(),
            None => return,
        };
        let versioned_name: String = match install_dir.file_name().and_then(std::ffi::OsStr::to_str)
        {
            Some(n) => n.to_string(),
            None => return,
        };
        if let Some(entry) = self.lib_path_manifest(&parent, &versioned_name) {
            self.check_advisory(id, &version);
            *f = entry;
        }
    }

    /// No-op when registry feature is off.
    #[cfg(not(feature = "registry"))]
    #[allow(clippy::unused_self)]
    fn probe_sidecar_lockfile(&mut self, _id: &str, _f: &mut String) {}

    /// `~/.loft/registry/<id>-<version>/` — packages installed via `loft install`
    /// against the package registry.  Resolves the version via the cwd's
    /// `loft.lock` (written by `loft install`).  When loft.lock is absent or
    /// doesn't list `id`, this probe is a no-op and resolution falls through
    /// to the remaining strategies.
    ///
    /// @PLAN12 phase 3.5a wiring (2026-05-24): closes the "loft install →
    /// use installed package" loop.  Before this, `loft install crypto`
    /// downloaded + extracted correctly but `use crypto;` in a subsequent
    /// run still required a manual `--lib` flag.
    #[cfg(feature = "registry")]
    fn probe_registry_installed(&mut self, id: &str, f: &mut String) {
        // Use `loft::*` (not `crate::*`) because this module is compiled
        // into BOTH the loft library AND the loft binary; the binary
        // doesn't have `lockfile` / `registry_index` declared as `mod`,
        // but accesses them as deps via the `loft::` library path.
        if std::path::Path::new(f).exists() {
            return;
        }
        let cwd = match env::current_dir() {
            Ok(c) => c,
            Err(_) => return,
        };
        let lock_path = cwd.join("loft.lock");
        if !lock_path.exists() {
            return;
        }
        let lock = match crate::lockfile::read_lockfile(&lock_path) {
            Ok(Some(l)) => l,
            _ => return,
        };
        let version = match lock.packages.iter().find(|p| p.name == id) {
            Some(p) => p.version.clone(),
            None => return,
        };
        self.resolve_registry_installed(id, &version, f);
    }

    /// Point `f` at an installed registry package's extracted source, given the
    /// resolved `version`. The shared tail of every registry resolve path
    /// (`extract_dir` → `lib_path_manifest`). Separated (#634) so a transitive
    /// dep can be resolved straight from the install REPORT — a cache-internal
    /// auto-install writes no lockfile (it must not mutate the immutable cache),
    /// so a resolver that could only learn the version by reading a lockfile back
    /// would leave `use <dep>` unresolved even though the package is installed.
    #[cfg(feature = "registry")]
    fn resolve_registry_installed(&mut self, id: &str, version: &str, f: &mut String) {
        if !f.is_empty() && std::path::Path::new(f).exists() {
            return;
        }
        let install_dir = crate::registry_index::extract_dir(id, version);
        let Some(parent) = install_dir.parent().and_then(std::path::Path::to_str) else {
            return;
        };
        let parent = parent.to_string();
        let Some(versioned_name) = install_dir
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .map(str::to_string)
        else {
            return;
        };
        if let Some(entry) = self.lib_path_manifest(&parent, &versioned_name) {
            self.check_advisory(id, version);
            *f = entry;
        }
    }

    /// No-op when registry feature is off — registry-installed packages
    /// only resolve when the `loft install` machinery is compiled in.
    /// `self` is kept to match the `#[cfg(feature = "registry")]` method
    /// signature (it is a genuine method on the registry-enabled build).
    #[cfg(not(feature = "registry"))]
    #[allow(clippy::unused_self)]
    fn probe_registry_installed(&mut self, _id: &str, _f: &mut String) {}

    /// @PLAN12 Phase 6.6 — auto-install on `use`.
    ///
    /// When `id` doesn't resolve via any of the prior strategies
    /// (path-dep, sibling lookup, lockfile + cached registry install)
    /// AND `id` is a known package name in the registry catalog,
    /// fire `install_one` to fetch + extract + lockfile-update,
    /// then re-run the cached-registry-install probe.
    ///
    /// The Python comparison: `python my_script.py` with
    /// `import requests` works if `pip install requests` was done
    /// once.  Loft's equivalent — `loft my_script.loft` with
    /// `use gridmesh;` — Just Works on first run by doing the
    /// `pip install` step on the user's behalf.
    ///
    /// Off-switches: `LOFT_OFFLINE=1` and `LOFT_NO_AUTO_INSTALL=1`
    /// both suppress this probe.  Surprise reduction: every cold
    /// install prints `[registry] ...` lines (mirrors Cargo's
    /// "Downloading…" output); steady-state (cache hit, resolves
    /// via probe_registry_installed) is silent.
    #[cfg(feature = "registry")]
    fn probe_auto_install(&mut self, id: &str, f: &mut String) {
        if std::path::Path::new(f).exists() {
            return;
        }
        // Off-switches.
        if std::env::var("LOFT_OFFLINE").is_ok() {
            return;
        }
        if std::env::var("LOFT_NO_AUTO_INSTALL").is_ok() {
            return;
        }
        // Bootstrap state: the loft binary may not have an embedded
        // trust root yet (K_tmp → K_real rotation per
        // PKG_REGISTRY.md / REGISTRY_BOOTSTRAP.md).  Mirror what
        // `loft install` / `loft search` / `loft info` do — accept
        // unsigned indexes during the trust-bootstrap window.  Once
        // the production key is embedded, signed indexes verify
        // cleanly and this flag becomes a no-op for the happy path.
        //
        // Lockfile WRITE path: walk up from the script's directory
        // looking for `loft.toml`.  Found → project mode — write to
        // `<project_root>/loft.lock` so the project's manifest +
        // lockfile stay co-located regardless of where loft was
        // invoked from.  Not found → script mode — `lock_path: None`
        // falls back to cwd's `loft.lock` (the existing default).
        let cur_script = self.lexer.pos().file.replace(other_sep(), sep_str());
        // A transitive dep discovered while parsing an ALREADY-CACHED package
        // (`~/.loft/registry/<pkg>/src/...`) has no consumer project: the only
        // `loft.toml` the walk-up finds is the cached dep's own, so writing a
        // `loft.lock` there would mutate the immutable cache — a harmless stray
        // file on Unix, but an ENOENT that aborts the whole resolution on Windows
        // (nightly `moros_glb_cli_end_to_end`).  Install without recording.
        let in_registry_cache = std::fs::canonicalize(&cur_script)
            .ok()
            .zip(std::fs::canonicalize(crate::registry_index::cache_dir()).ok())
            .is_some_and(|(script, cache)| script.starts_with(&cache));
        let project_root = if in_registry_cache {
            None
        } else {
            Self::find_project_root(&cur_script)
        };
        let lock_path = project_root.as_ref().map(|p| p.join("loft.lock"));
        let opts = crate::install::InstallOptions {
            allow_unsigned: true,
            refresh: false,
            skip_lockfile: in_registry_cache,
            // LOFT_OFFLINE=1 makes resolution HERMETIC: a missing package
            // fails fast and deterministically instead of fetching — what a
            // test-spawned fixture (or an air-gapped box) wants.  Mirrors
            // the CLI paths (src/main.rs) that already honour it.
            offline: std::env::var_os("LOFT_OFFLINE").is_some(),
            allow_prerelease: false,
            lock_path,
        };
        // Honour the root project's declared constraint for `id` (if any), so a
        // consumer's pin — exact or ranged — wins over "resolve newest", even
        // when `id` is pulled transitively by a lib that didn't pin it.
        let pin = self.root_dep_constraint(id);
        match crate::install::auto_install_if_in_catalog(id, pin.as_deref(), &opts) {
            Ok(Some(report)) => {
                // Resolve `f` straight from the install report's version — no
                // lockfile round-trip (#634): a cache-internal install writes no
                // lockfile, so the lockfile-based probes below would leave a
                // freshly-installed transitive dep unresolved (`use <dep>` → "not
                // found") even though it is on disk. The report carries the exact
                // version, whether newly installed or already cached.
                if let Some((_, version)) = report
                    .installed
                    .iter()
                    .chain(report.skipped_cached.iter())
                    .find(|(n, _)| n == id)
                {
                    let version = version.clone();
                    self.resolve_registry_installed(id, &version, f);
                }
                // Fallbacks for a normal (non-cache) install that DID write a
                // lockfile: the project lockfile (where the new entry landed),
                // then cwd's lockfile (script mode).
                self.probe_project_lockfile(id, f);
                self.probe_registry_installed(id, f);
            }
            Ok(None) => {
                // `id` is not a registry package; let the remaining
                // resolution strategies handle it (or fail with
                // the standard "library not found" diagnostic).
            }
            Err(e) => {
                // Network failure, sig mismatch, or similar.
                // Print a notice but let resolution fall through —
                // the user may have a path-dep or sibling that
                // resolves anyway, or they may want the standard
                // error.
                eprintln!("[registry] auto-install failed for {id}: {e}");
            }
        }
    }

    /// No-op when registry feature is off.
    #[cfg(not(feature = "registry"))]
    #[allow(clippy::unused_self)]
    fn probe_auto_install(&mut self, _id: &str, _f: &mut String) {}

    /// Final fallback: beside the parsed file itself.
    fn probe_cur_dir_flat(id: &str, cur_dir: &str, f: &mut String) {
        if !cur_dir.is_empty() && !std::path::Path::new(f).exists() {
            *f = format!("{cur_dir}{0}{id}.loft", sep_str());
        }
    }

    /// Final fallback for scripts inside a `/tests/` tree.
    fn probe_base_dir_flat(id: &str, base_dir: &str, f: &mut String) {
        if !base_dir.is_empty() && !std::path::Path::new(f).exists() {
            *f = format!("{base_dir}{0}{id}.loft", sep_str());
        }
    }

    /// Register native crate info from a loft.toml manifest.
    /// Called when a .loft file was found directly via lib_dirs (not through
    /// lib_path_manifest), so the manifest's native crate registration would
    /// otherwise be skipped.
    fn register_native_manifest(
        &mut self,
        manifest_path: &std::path::Path,
        pkg_dir: &std::path::Path,
    ) {
        let Some(m) = manifest::read_manifest(manifest_path.to_str().unwrap_or("")) else {
            return;
        };
        let pkg_dir = pkg_dir.to_string_lossy().to_string();
        // Register the dlopen-side native lib path (interpreter mode).  The
        // `[library] native = "..."` form registers the cdylib for dlopen;
        // the separate `[native] crate = "..."` block (handled below)
        // registers the rlib for the native-compile path.  Both must run
        // here for sibling-package and ancestor-walk paths, otherwise
        // packages depended on by a no-native parent (e.g. an examples
        // package that uses `lib/server`) lose their native bindings in
        // interpreter mode.
        if let Some(ref stem) = m.native
            && let Some(path) = crate::extensions::resolve_native_lib(&pkg_dir, stem)
            && !self.pending_native_libs.contains(&path)
        {
            self.pending_native_libs.push(path);
            self.native_lib_regs.push((stem.clone(), pkg_dir.clone()));
        }
        // @PLN11 N3 Step 3 (default-native) / F2 — mirror `apply_manifest_side_effects`:
        // a normal loft library reached via THIS direct-resolution / sibling-package /
        // ancestor-walk path must ALSO be recorded as a native-compile candidate.
        // Without it the two resolution paths diverge: a library pulled in transitively
        // (or used directly *after* it was already loaded transitively, so the direct
        // `use` dedups) is loaded here but never recorded — so it never builds its own
        // cdylib and its direct calls interpret.  `native` (hand-written cdylib) takes
        // precedence — don't double-compile.
        if m.native.is_none() && !self.pending_native_compile.iter().any(|d| d == &pkg_dir) {
            self.pending_native_compile.push(pkg_dir.clone());
        }
        if let Some(ref crate_name) = m.native_crate {
            let rust_crate = crate_name.replace('-', "_");
            if !self
                .data
                .native_packages
                .iter()
                .any(|(c, _)| c == crate_name)
            {
                self.data
                    .native_packages
                    .push((crate_name.clone(), pkg_dir.clone()));
            }
            // @PLAN12 phase 2 step 2 (2026-05-24) — same manifest-driven
            // `native_symbols` + `def.native` population as
            // `apply_manifest_side_effects` (the legacy path).  The
            // sibling-package probe (used when a script reaches a library
            // via cwd ancestry rather than `--lib`) hits THIS function;
            // without the same population, `[native.functions]` entries
            // would be visible only to native codegen via the legacy
            // path's `native_symbols` lookup — the sibling path would
            // leave def.native empty and the interpreter dispatch path
            // would silently fall through to OpCall with an empty body,
            // returning null instead of dispatching to the cdylib.
            for (loft_name, rust_symbol) in &m.native_functions {
                self.data
                    .native_symbols
                    .insert(loft_name.clone(), rust_symbol.clone());
            }
            for (loft_name, rust_symbol) in &m.native_functions {
                let candidates = [format!("n_{loft_name}"), loft_name.clone()];
                for d_nr in 0..self.data.definitions() {
                    let def = self.data.def(d_nr);
                    if !def.native().is_empty() {
                        continue;
                    }
                    if !candidates.iter().any(|c| c == def.name()) {
                        continue;
                    }
                    if !def.position().file.starts_with(&pkg_dir) {
                        continue;
                    }
                    rust_symbol.clone_into(&mut self.data.definitions[d_nr as usize].native);
                }
            }
            // P266: same ownership-driven restriction as
            // `apply_manifest_side_effects` above — only map `#native`
            // symbols whose definition lives in THIS package's source
            // tree, so out-of-order manifest/source loading can't make
            // one package claim another's symbols.
            for d_nr in 0..self.data.definitions() {
                let def = self.data.def(d_nr);
                let sym = def.native();
                if sym.is_empty() {
                    continue;
                }
                if !def.position().file.starts_with(&pkg_dir) {
                    continue;
                }
                if self.data.native_symbol_crates.contains_key(sym) {
                    continue;
                }
                self.data
                    .native_symbol_crates
                    .insert(sym.to_string(), rust_crate.clone());
            }
        }
        // #453 — a `[wasm.bridge]` library must register its bridge crate +
        // routes + host_js even with NO `[native]`. This used to sit INSIDE the
        // `m.native_crate` guard above, so a browser-only bridge lib (no native
        // crate) reached through `--lib` / a `path` dep / the sibling walk
        // silently dropped its routes and `--html` failed with P269. Register it
        // ungated, mirroring `apply_manifest_side_effects` — the legacy resolution
        // path that always got this right. (lib_plan-29 W1c bridge crate + routes,
        // W2 host_js.)
        if let Some(ref bridge_crate) = m.wasm_bridge_crate {
            if !self
                .data
                .wasm_bridge_packages
                .iter()
                .any(|(c, _)| c == bridge_crate)
            {
                self.data
                    .wasm_bridge_packages
                    .push((bridge_crate.clone(), pkg_dir.clone()));
            }
            for (loft_sym, bridge_fn) in &m.wasm_bridge_routes {
                self.data
                    .wasm_bridge_routes
                    .insert(loft_sym.clone(), (bridge_crate.clone(), bridge_fn.clone()));
            }
        }
        if let Some(ref host_js_rel) = m.wasm_bridge_host_js {
            let abs = std::path::Path::new(&pkg_dir).join(host_js_rel);
            let abs_str = abs.to_string_lossy().to_string();
            if !self.data.wasm_bridge_host_js_files.contains(&abs_str) {
                self.data.wasm_bridge_host_js_files.push(abs_str);
            }
        }
    }

    /// Check whether `<dir>/<id>` contains a valid loft package layout.
    /// Reads `loft.toml` when present and validates the interpreter version
    /// requirement.  Emits a fatal diagnostic on version mismatch.
    /// Returns `Some(entry_path)` when the layout exists and the version passes,
    /// `None` otherwise.
    ///
    /// Legacy path: delegates to [`lib_path_manifest_resolve`] for pure
    /// resolution, then applies side-effects via [`apply_manifest_side_effects`].
    /// Phase A calls `lib_path_manifest_resolve`
    /// directly and builds the package graph explicitly.
    fn lib_path_manifest(&mut self, dir: &str, id: &str) -> Option<String> {
        let resolved = self.lib_path_manifest_resolve(dir, id)?;
        if let Some(m) = resolved.manifest.as_ref() {
            self.apply_manifest_side_effects(dir, &resolved.pkg_dir, m);
        }
        Some(resolved.entry)
    }

    /// Pure resolution of `<dir>/<id>` against disk + manifest.  No side
    /// effects on `self.data` / `self.lib_dirs` / `self.pending_*`; the only
    /// state touched is `self.lexer.diagnostics` on a version-mismatch fatal.
    ///
    /// This is the entry point used by Phase A of the package-mode driver
    /// , which needs to enumerate files + package edges without
    /// spilling symbol-table side-effects before pass-1 parsing begins.
    fn lib_path_manifest_resolve(&mut self, dir: &str, id: &str) -> Option<ResolvedPkg> {
        // @P296-sibling (Windows) — build the package paths with
        // `Path::join` rather than `format!("{dir}/{id}")`.  When `dir` is
        // an absolute Windows path (`D:\a\loft\loft\lib`, e.g. a `--lib`
        // argument built via `PathBuf::join`), interpolating a literal `/`
        // produces a mixed-separator path; `Path::join` uses the platform
        // separator consistently so nested `--lib` packages resolve on
        // Windows (the crystal_gold CI failure: "Library 'audience_crystal'
        // not found").  No behavioural change on Linux/macOS.
        // #408 — accept `--lib` pointing AT the package dir itself (its basename
        // matches `id` and it carries a loft.toml), not only at its parent.
        // Without this, `--lib path/to/crypto` looks for `path/to/crypto/crypto`,
        // fails, and `use crypto` falls through to an installed registry copy —
        // the shadowing that made a local lib's `[wasm.bridge]` routes unreachable
        // in `--html`. Falls back to `<dir>/<id>` for the normal parent-dir search.
        let dir_pb = std::path::Path::new(dir);
        let pkg_dir_pb = if dir_pb.file_name() == Some(std::ffi::OsStr::new(id))
            && dir_pb.join("loft.toml").is_file()
        {
            dir_pb.to_path_buf()
        } else {
            dir_pb.join(id)
        };
        if !pkg_dir_pb.is_dir() {
            return None;
        }
        let pkg_dir = pkg_dir_pb.to_string_lossy().into_owned();
        let manifest_pb = pkg_dir_pb.join("loft.toml");
        let nested_entry = || {
            pkg_dir_pb
                .join("src")
                .join(format!("{id}.loft"))
                .to_string_lossy()
                .into_owned()
        };
        let (entry, manifest) = if manifest_pb.exists() {
            let manifest_path = manifest_pb.to_string_lossy().into_owned();
            let m = manifest::read_manifest(&manifest_path)?;
            if let Some(ref req) = m.loft_version {
                let current = env!("CARGO_PKG_VERSION");
                match manifest::check_version(req, current) {
                    manifest::VersionCheck::Satisfied => {}
                    manifest::VersionCheck::Unsatisfied => {
                        diagnostic!(
                            self.lexer,
                            Level::Fatal,
                            "Package '{id}' requires loft {req} but interpreter is {current}"
                        );
                        return None;
                    }
                    // @PLN102 arc B: a constraint the loader cannot honour is
                    // rejected loudly, not silently treated as "any version".
                    manifest::VersionCheck::Malformed(why) => {
                        diagnostic!(
                            self.lexer,
                            Level::Fatal,
                            "Package '{id}' has an invalid loft version requirement '{req}': {why}"
                        );
                        return None;
                    }
                }
            }
            // @PLN102 arc B-semantic — the compatibility `contract` axis (a
            // monotone integer; increments on a silent breaking change, distinct
            // from the calendar release tag above).  Too-old is a hard reject;
            // drift (loft advanced past the tested epoch) WARNS — the arc-C
            // deprecation channel — and loads, so the fix is the author
            // republishing, never a silent wrong answer for the consumer.
            if let Some(ref creq) = m.contract {
                let cur = manifest::CONTRACT_VERSION;
                match manifest::check_contract(creq, cur) {
                    manifest::ContractCheck::Ok => {}
                    manifest::ContractCheck::TooOld { required_min } => {
                        diagnostic!(
                            self.lexer,
                            Level::Fatal,
                            "Package '{id}' requires loft contract >= {required_min} but this loft is contract {cur}"
                        );
                        return None;
                    }
                    manifest::ContractCheck::Drifted { tested_max } => {
                        diagnostic!(
                            self.lexer,
                            Level::Warning,
                            "Package '{id}' was tested against loft contract <= {tested_max} but this loft is contract {cur} — a breaking change since then may make it misbehave; ask its author to republish against contract {cur}"
                        );
                    }
                    manifest::ContractCheck::Malformed(why) => {
                        diagnostic!(
                            self.lexer,
                            Level::Fatal,
                            "Package '{id}' has an invalid loft contract requirement '{creq}': {why}"
                        );
                        return None;
                    }
                }
            }
            let entry = m.entry.as_ref().map_or_else(nested_entry, |e| {
                pkg_dir_pb.join(e).to_string_lossy().into_owned()
            });
            (entry, Some(m))
        } else {
            (nested_entry(), None)
        };
        if std::path::Path::new(&entry).exists() {
            Some(ResolvedPkg {
                pkg_dir,
                entry,
                manifest,
            })
        } else {
            None
        }
    }

    /// Apply the parser-state side effects that the legacy `lib_path_manifest`
    /// performs for a resolved package: native-lib registration,
    /// native-symbol / native-crate bookkeeping, sibling-dependency search
    /// paths (`lib_dirs`), and queued transitive package loads
    /// (`pending_pkg_deps`).
    fn apply_manifest_side_effects(&mut self, dir: &str, pkg_dir: &str, m: &manifest::Manifest) {
        // register native shared library path for loading after byte_code().
        // Pre-built location first, then auto-build from source (one home:
        // `extensions::resolve_native_lib`, shared with the warm-cache load).
        if let Some(ref stem) = m.native
            && let Some(path) = crate::extensions::resolve_native_lib(pkg_dir, stem)
            && !self.pending_native_libs.contains(&path)
        {
            self.pending_native_libs.push(path);
            self.native_lib_regs
                .push((stem.clone(), pkg_dir.to_string()));
        }
        // @PLN11 Arc N / N3 Step 3 — **default-native**.  Every `use`d normal loft
        // library is a native candidate: record the package dir; the driver marks +
        // builds + loads after scope analysis (see `pending_native_compile`).  No
        // opt-in, no flag — "libraries compile, scripts interpret" is the default.
        //   - `native` (a hand-written cdylib via `[library] native = ...`) takes
        //     precedence — don't double-compile.
        //   - `LOFT_NO_NATIVE_LIBS=1` is the interpret escape (handled in the driver:
        //     it clears `pending_native_compile`), used by the dev/edit loop + the
        //     parity reference until Step 4 (dev-interpret-on-edit) lands.
        //   - A build failure silently interprets (Step 2), so recording a library
        //     that can't compile native is harmless.
        // The legacy `[library] compile = "native"` opt-in is now redundant but still
        // accepted (a no-op): default-native already records it.
        if m.native.is_none() && !self.pending_native_compile.iter().any(|d| d == pkg_dir) {
            self.pending_native_compile.push(pkg_dir.to_string());
        }
        // PKG.4: register native function symbols and package crate info.
        if let Some(ref crate_name) = m.native_crate {
            let rust_crate = crate_name.replace('-', "_");
            if !self
                .data
                .native_packages
                .iter()
                .any(|(c, _)| c == crate_name)
            {
                self.data
                    .native_packages
                    .push((crate_name.clone(), pkg_dir.to_string()));
            }
            for (loft_name, rust_symbol) in &m.native_functions {
                self.data
                    .native_symbols
                    .insert(loft_name.clone(), rust_symbol.clone());
            }
            // @PLAN12 phase 2 step 2 (2026-05-24) — make
            // `[native.functions]` entries fully equivalent to
            // `#native "symbol"` annotations.  For each manifest
            // entry whose loft fn name matches a definition owned by
            // this package, populate `def.native` so the interpreter's
            // `wire_native_fns` (which keys off `def.native`) and the
            // bytecode dispatch (`state/codegen.rs:2207-2217`, which
            // also reads `def.native`) see the binding without
            // requiring the redundant `#native` annotation.  Native
            // codegen already consulted `native_symbols` directly so
            // this isn't strictly required for the native path, but
            // unifying the two paths is the whole point of step 2.
            //
            // Ownership check (`def.position.file.starts_with(pkg_dir)`)
            // mirrors the @P266-style guard below: only populate defs
            // physically inside this package's source tree.
            for (loft_name, rust_symbol) in &m.native_functions {
                let candidates = [format!("n_{loft_name}"), loft_name.clone()];
                for d_nr in 0..self.data.definitions() {
                    let def = self.data.def(d_nr);
                    if !def.native().is_empty() {
                        continue;
                    }
                    if !candidates.iter().any(|c| c == def.name()) {
                        continue;
                    }
                    if !def.position().file.starts_with(pkg_dir) {
                        continue;
                    }
                    rust_symbol.clone_into(&mut self.data.definitions[d_nr as usize].native);
                }
            }
            // P266: map only `#native` symbols whose definition lives in
            // THIS package's source tree, not every unmapped symbol in
            // the whole `data.definitions()` list.  The earlier
            // walk-and-claim shape over-assigned: when manifests were
            // registered out-of-order with their sources (e.g. lib/web
            // manifest registered before lib/server's source was parsed,
            // then lib/server's manifest registered with both packages'
            // defs already in the table), it left lib/web's symbols
            // pointing at `loft_server` and lib/server's symbols pointing
            // at `loft_web`.  Restricting by `position.file.starts_with(
            // pkg_dir)` makes the assignment ownership-driven instead of
            // call-order-driven.
            for d_nr in 0..self.data.definitions() {
                let def = self.data.def(d_nr);
                let sym = def.native();
                if sym.is_empty() {
                    continue;
                }
                if !def.position().file.starts_with(pkg_dir) {
                    continue;
                }
                if self.data.native_symbol_crates.contains_key(sym) {
                    continue;
                }
                self.data
                    .native_symbol_crates
                    .insert(sym.to_string(), rust_crate.clone());
            }
        }
        // lib_plan-29 W1c (2026-05-29) — register WASM bridge crate +
        // routes for `--html` builds.  Mirrors the `[native]` block
        // above, scoped to the browser-WASM target.
        if let Some(ref bridge_crate) = m.wasm_bridge_crate {
            if !self
                .data
                .wasm_bridge_packages
                .iter()
                .any(|(c, _)| c == bridge_crate)
            {
                self.data
                    .wasm_bridge_packages
                    .push((bridge_crate.clone(), pkg_dir.to_string()));
            }
            for (loft_sym, bridge_fn) in &m.wasm_bridge_routes {
                self.data
                    .wasm_bridge_routes
                    .insert(loft_sym.clone(), (bridge_crate.clone(), bridge_fn.clone()));
            }
        }
        // lib_plan-29 W2 (2026-05-29) — resolve `[wasm.bridge].host_js`
        // relative to the package root and register the absolute path.
        // The `--html` driver concatenates each registered file into
        // the HTML preamble.
        if let Some(ref host_js_rel) = m.wasm_bridge_host_js {
            let abs = std::path::Path::new(pkg_dir).join(host_js_rel);
            let abs_str = abs.to_string_lossy().to_string();
            if !self.data.wasm_bridge_host_js_files.contains(&abs_str) {
                self.data.wasm_bridge_host_js_files.push(abs_str);
            }
        }
        // PKG.3: register dirs for dependency resolution.
        //
        // For plain-version deps (`foo = "0.1"`) and the legacy
        // sibling-probe shape (no path declared): register the
        // package's parent dir `dir` so `<dir>/<dep_name>` resolves
        // via sibling lookup.
        //
        // @PLAN12 phase 3.5b (2026-05-24) — for path-deps
        // (`foo = { path = "../external/foo" }`), resolve the path
        // relative to this package's directory (`pkg_dir`), then
        // register the PARENT of the resolved location.  This lets
        // the existing sibling-probe at
        // `<resolved-parent>/<dep_name>/loft.toml` find the external
        // package.  Without this, the inline-table path field was
        // decorative — manifests like `lib/audience_crystal/loft.toml`
        // worked by coincidence (their `path = "../gridmesh"`
        // happened to point at a sibling in `lib/`, also covered by
        // the `--lib lib` cmdline arg).  This change makes the path
        // field actually do what it says, unlocking external library
        // extractions.
        for (dep_name, dep_value) in &m.dependencies {
            let resolved_parent = if let Some(path) = manifest::extract_path_dep(dep_value) {
                let dep_pkg_path = std::path::Path::new(pkg_dir).join(path);
                dep_pkg_path
                    .parent()
                    .map_or_else(|| dir.to_string(), |p| p.to_string_lossy().into_owned())
            } else {
                dir.to_string()
            };
            if !self.lib_dirs.contains(&resolved_parent) {
                self.lib_dirs.push(resolved_parent.clone());
            }
            if !self.data.use_exists(dep_name) {
                self.pending_pkg_deps
                    .push((dep_name.clone(), resolved_parent));
            }
        }
    }

    /// Plan-07 phase 5 suggestions.  Find a similar user-defined
    /// function name (typed without the `n_` prefix) that might be
    /// the correct spelling.  Returns `None` when no candidate is
    /// within Levenshtein distance 2.
    ///
    /// Uses the same `suggest_similar` primitive as the existing
    /// variable-suggestion path at `parser/objects.rs::known_var_or_type`.
    /// @PLN115 — the resolved identifier occurrences recorded during the last
    /// parse (empty unless `record_resolutions` was set).  Drives the LSP's
    /// precise, non-lexical navigation.
    #[must_use]
    pub fn resolutions(&self) -> &[crate::resolution::Occurrence] {
        &self.resolutions
    }

    /// @PLN115 — turn occurrence recording on (default off).  The LSP parse path
    /// (S3) flips this before `parse_source` so navigation can resolve by binding
    /// identity; every normal compile leaves it off, keeping `record` a dead branch.
    pub fn set_record_resolutions(&mut self, on: bool) {
        self.record_resolutions = on;
    }

    /// @PLN115 — record one resolved occurrence, gated: a single predictable
    /// branch when `record_resolutions` is off (every normal compile), so it is
    /// zero-cost there.  A pure side-append — it changes no parse decision.  Wired
    /// to the resolution chokepoints starting in S2 (`parse_var` locals).
    fn record(&mut self, pos: &Position, len: u16, res: crate::resolution::Resolution) {
        self.record_occurrence(pos, len, res, false);
    }

    /// @PLN115 tail — record a binding's DECLARATION occurrence (a parameter's
    /// signature name, a `for` / lambda binder).  Same gate as [`Self::record`],
    /// but flagged `declaration` so a consumer knows the binding's declaration is
    /// captured and a precise rename is complete.
    fn record_decl(&mut self, pos: &Position, len: u16, res: crate::resolution::Resolution) {
        self.record_occurrence(pos, len, res, true);
    }

    fn record_occurrence(
        &mut self,
        pos: &Position,
        len: u16,
        res: crate::resolution::Resolution,
        declaration: bool,
    ) {
        if self.record_resolutions {
            self.resolutions.push(crate::resolution::Occurrence {
                line: pos.line,
                col: pos.pos,
                len,
                res,
                declaration,
            });
        }
    }

    pub fn suggest_function_name(&self, name: &str) -> Option<String> {
        let candidates_owned: Vec<String> = self
            .data
            .user_fn_d_nrs()
            .iter()
            .filter_map(|&d_nr| {
                let n = self.data.def(d_nr).name();
                if let Some(stripped) = n.strip_prefix("n_") {
                    // Skip synthetic lambda names — they're not user-typeable.
                    if stripped.starts_with("__lambda_") {
                        None
                    } else {
                        Some(stripped.to_string())
                    }
                } else {
                    None
                }
            })
            .collect();
        let candidates: Vec<&str> = candidates_owned.iter().map(String::as_str).collect();
        crate::diagnostics::suggest_similar_capped(name, &candidates).map(String::from)
    }

    /// Plan-07 phase 5 suggestions.  Find a similar field name on
    /// the given struct definition.  Skips synthetic compiler-
    /// generated attributes (those starting with `_` or `#`).
    pub fn suggest_field_name(&self, struct_d_nr: u32, name: &str) -> Option<String> {
        if struct_d_nr == u32::MAX {
            return None;
        }
        let candidates_owned: Vec<&str> = self
            .data
            .def(struct_d_nr)
            .attributes
            .iter()
            .filter_map(|a| {
                if a.name.starts_with('_') || a.name.starts_with('#') {
                    None
                } else {
                    Some(a.name.as_str())
                }
            })
            .collect();
        crate::diagnostics::suggest_similar_capped(name, &candidates_owned).map(String::from)
    }

    /// Plan-07 phase 5 suggestions.  Find a similar type name —
    /// thin wrapper around `Data::suggest_type_name` so callers in
    /// the parser don't have to thread `self.data` explicitly.
    pub fn suggest_type_name(&self, name: &str) -> Option<String> {
        self.data.suggest_type_name(name)
    }

    // Determine if there need to be special enum functions that call enum_value variants.
    pub fn create_var(&mut self, name: &str, var_type: &Type) -> u16 {
        if self.context == u32::MAX {
            return u16::MAX;
        }
        self.vars.add_variable(name, var_type, &mut self.lexer)
    }

    fn create_unique(&mut self, name: &str, var_type: &Type) -> u16 {
        self.vars.unique(name, var_type, &mut self.lexer)
    }

    fn var_usages(&mut self, vnr: u16, plus: bool) {
        // @P387 — a captured-var number can arrive from a DIFFERENT var table
        // (a capturing lambda passed as a fn-value records its captures, which
        // the outer call then marks): such a number is out of range here.  Skip
        // it rather than index OOB — consistent with the `u16::MAX` guard.
        if vnr == u16::MAX || vnr >= self.vars.count() {
            return;
        }
        if plus {
            self.vars.in_use(vnr, true);
        } else if self.vars.uses(vnr) > 0 {
            self.vars.in_use(vnr, false);
        }
    }

    /// check whether a value is "addressable" — rooted in a Var and
    /// reached through field access (OpGetField) or vector element access
    /// (OpGetVector / OpVectorRef) chains.  Addressable values produce a
    /// DbRef into existing mutable storage, so they are safe to pass as
    /// `&` parameters.
    fn is_addressable(val: &Value, data: &Data) -> bool {
        // Plan-07 phase 1: unspan() so wraps on `[` / `.` (steps 1.11
        // / 1.12) don't hide an addressable shape from the `&` arg
        // check.
        match val.unspan() {
            Value::Var(_) => true,
            Value::Call(d_nr, args) => {
                let name = data.def(*d_nr).name();
                (name == "OpGetField" || name == "OpGetVector" || name == "OpVectorRef")
                    && !args.is_empty()
                    && Self::is_addressable(&args[0], data)
            }
            _ => false,
        }
    }

    /// @PLN87 #1 — is `val` a PLACE (an addressable lvalue: a variable, struct field,
    /// or vector/array element) versus a TEMPORARY (a literal, a computed value, or a
    /// call result)?  Broader than [`is_addressable`] (which is the narrower
    /// "produces a heap DbRef" test the `&`-ARGUMENT path needs): a place includes a
    /// SCALAR field/element (`s.x` → `OpGetInt(s, …)`), whose reference codegen is a
    /// later rung (L3/L4) and copies for now — but it is still a place, not a
    /// temporary.  Used to reject `&<temporary>` at a `&` binding.  The accessor
    /// allowlist (place-GETTERS only) keeps temporary-builders like `OpGetTextSub`
    /// and every arithmetic / `n_*` op out.
    fn is_amp_place(val: &Value, data: &Data) -> bool {
        match val.unspan() {
            Value::Var(_) => true,
            Value::Call(d_nr, args) => {
                let name = data.def(*d_nr).name();
                matches!(
                    name,
                    "OpGetField"
                        | "OpGetVector"
                        | "OpVectorRef"
                        | "OpGetInt"
                        | "OpGetFloat"
                        | "OpGetSingle"
                        | "OpGetByte"
                        | "OpGetCharacter"
                        | "OpGetBoolean"
                        | "OpGetEnum"
                        | "OpGetShort"
                        | "OpGetRecord"
                        | "OpGetRef"
                        | "OpGetDbRef"
                ) && !args.is_empty()
                    && Self::is_amp_place(&args[0], data)
            }
            _ => false,
        }
    }

    /// Plan-06 PRIORITY.md spine step 5 — par-result use-site analyser.
    ///
    /// After the function body is fully parsed, find each
    /// `let r = parallel_for(...)` (or fused `for x in input par(r=...,
    /// N) { body }` whose result-vector flows into the body), and walk
    /// the body for uses of `r`.  Each use is classified:
    ///
    /// - **Streaming-eligible**: the result is iterated once via `for x
    ///   in r { … }` — lowers to `Stitch::Queue` in spine step 5b.
    /// - **Materialising**: random access (`r[i]`), length-of-filter,
    ///   multi-pass, alias (`r2 = r`), passed as `vector<S>` arg, stored
    ///   in `vector<S>` field, returned from `-> vector<S>` fn.  These
    ///   require the materialised vector (today's path) and will fail
    ///   compilation in spine step 7 unless rewritten via the explicit
    ///   `par_to_vec(...)` helper (planned phase 11).
    ///
    /// Spine step 7 (DONE 2026-04-29): emits `Level::Error` on
    /// materialising uses.  The deprecation window from step 5
    /// (warning) closed once the audit in step 6 confirmed zero
    /// corpus sites trigger it.  Materialising par results now fail
    /// to compile.  Streaming-eligible patterns (single
    /// `for x in r {}` use) stay silent and keep working through the
    /// existing materialised path until step 8 lands the streaming
    /// rewrite.
    fn check_par_result_singlepass(&mut self) {
        let par_for_d_nr = self.data.def_nr("n_parallel_for");
        if par_for_d_nr == u32::MAX || self.context == u32::MAX {
            return;
        }
        let body = self.data.def(self.context).code().clone();
        // Find each (var, par_call_pos) where `Set(var, Call(n_parallel_for, ...))`.
        let mut par_results: Vec<u16> = Vec::new();
        Self::collect_par_assignments(&body, par_for_d_nr, &mut par_results);
        for v in par_results {
            // Walk the body counting uses of v.  Each use lands in a
            // category — `streaming` (Iter init position) or `other`
            // (everything else, treated as materialising for the warn
            // policy).
            let mut streaming = 0usize;
            let mut other = 0usize;
            Self::classify_var_uses(&body, v, &mut streaming, &mut other);
            // The `Set(v, par_call)` itself counts as a use.  Subtract
            // exactly one read (the par-call's appearance in arg-position
            // of the assignment is an artifact of how Set is encoded —
            // some encodings do, some don't; conservative: don't double
            // count by subtracting the assignment read).
            //
            // Heuristic: if `other > 0` AND the only `streaming` use is
            // 0, the result is purely materialising — warn.  If
            // `streaming` >= 1 and `other` >= 1, mixed-use — warn (one
            // read can't be both streamed and materialised).
            let var_name = self.vars.name(v).to_string();
            // Skip compiler-generated bindings: the fused for-par
            // desugar in `parse_parallel_for_loop` emits a hidden
            // `_par_results_N` var that's bound to a parallel_for
            // call and then indexed per iteration.  That's the
            // existing materialised path we WILL retire (step 8),
            // but warning on the compiler's own desugaring spams
            // every fused for-par site with output the user can't
            // act on.  Names starting with `_` are unique-counter
            // generated; user-typed bindings never start with `_`.
            if var_name.starts_with('_') {
                continue;
            }
            if streaming == 0 && other == 0 {
                // Result is bound but never read — par dispatched for
                // worker side effects only.  Suggest the explicit
                // form so the materialised result vector isn't
                // allocated for nothing.
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "par result '{var_name}' is never read — the materialised \
                     result vector is allocated but unused.  Use a fused `for x \
                     in input par(_=fn(x), N) {{}}` loop with discard policy \
                     (plan-06 spine step 3c) instead, or remove the assignment \
                     if the worker has no observable side effects."
                );
            } else if other > 0 {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "par result '{var_name}' is used in a materialising context \
                     (random access, multi-pass, or stored as vector<S>) — \
                     phase 10 of plan-06 will require single-pass consumption.  \
                     Either rewrite to a fused `for x in input par(r=fn(x), N) {{ body }}` \
                     loop, or call `par_to_vec(input, fn, N)` (planned phase 11) \
                     for an explicit materialised vector.  \
                     {streaming} streaming use(s), {other} materialising use(s) detected."
                );
            }
            // streaming == 1 && other == 0 — single-pass eligible.
            // Step 5b's lowering would rewrite to a fused for-par
            // dispatching through `run_parallel_queue` (no result
            // vector).  Deferred — bundles with step 8 (Concat
            // retirement) where the runtime question becomes
            // concrete and the rewrite has a clear destination IR.
            // For now this case is silent: existing code keeps
            // working through the materialised path.
        }
    }

    /// Plan-06 spine step 5 — find each `Set(v, Call(par_for_d_nr, ...))`
    /// occurrence in the IR tree and push `v` into `result`.  Recurses
    /// through every compound variant (Block / Loop / If / Insert /
    /// Iter / Span / ParFor) so a par assignment buried inside an `if`
    /// branch or a sub-block is still found.
    fn collect_par_assignments(val: &Value, par_for_d_nr: u32, result: &mut Vec<u16>) {
        match val {
            Value::Set(v, inner) => {
                if let Value::Call(d, _) = inner.as_ref().unspan()
                    && *d == par_for_d_nr
                {
                    result.push(*v);
                }
                Self::collect_par_assignments(inner, par_for_d_nr, result);
            }
            Value::Block(bl) | Value::Loop(bl) => {
                for op in &bl.operators {
                    Self::collect_par_assignments(op, par_for_d_nr, result);
                }
            }
            Value::Insert(ops) => {
                for op in ops {
                    Self::collect_par_assignments(op, par_for_d_nr, result);
                }
            }
            Value::If(c, t, e) => {
                Self::collect_par_assignments(c, par_for_d_nr, result);
                Self::collect_par_assignments(t, par_for_d_nr, result);
                Self::collect_par_assignments(e, par_for_d_nr, result);
            }
            Value::Iter(_, a, b, c) => {
                Self::collect_par_assignments(a, par_for_d_nr, result);
                Self::collect_par_assignments(b, par_for_d_nr, result);
                Self::collect_par_assignments(c, par_for_d_nr, result);
            }
            Value::Call(_, args) | Value::CallRef(_, args) => {
                for a in args {
                    Self::collect_par_assignments(a, par_for_d_nr, result);
                }
            }
            Value::Return(v) | Value::Drop(v) | Value::Yield(v) => {
                Self::collect_par_assignments(v, par_for_d_nr, result);
            }
            Value::BreakWith(_, v) | Value::TuplePut(_, _, v) => {
                Self::collect_par_assignments(v, par_for_d_nr, result);
            }
            Value::Span(b) => Self::collect_par_assignments(&b.1, par_for_d_nr, result),
            Value::ParFor(b) => {
                Self::collect_par_assignments(&b.input, par_for_d_nr, result);
                Self::collect_par_assignments(&b.worker, par_for_d_nr, result);
                Self::collect_par_assignments(&b.threads, par_for_d_nr, result);
                Self::collect_par_assignments(&b.body, par_for_d_nr, result);
            }
            _ => {}
        }
    }

    /// Plan-06 spine step 5 — classify each `Value::Var(v)` read in the
    /// IR tree.  A read inside the `init` arm of a `Value::Iter` (the
    /// collection being iterated) counts as **streaming**; every other
    /// read counts as **other** (materialising).
    ///
    /// Counts are accumulated into the caller's mutable refs.  The
    /// `Set(v, …)` site that introduced `v` is intentionally NOT a
    /// read (Set's first field is a write target, not a read).
    fn classify_var_uses(val: &Value, v: u16, streaming: &mut usize, other: &mut usize) {
        match val {
            Value::Var(u) if *u == v => {
                *other += 1;
            }
            Value::Var(_) => {}
            Value::Iter(_, init, next, extra) => {
                // Iter's init is the iterable expression.  A bare
                // `Var(v)` in init means `for x in v { … }` — count
                // as streaming and don't recurse into the var read.
                if let Value::Var(u) = init.as_ref()
                    && *u == v
                {
                    *streaming += 1;
                } else {
                    Self::classify_var_uses(init, v, streaming, other);
                }
                Self::classify_var_uses(next, v, streaming, other);
                Self::classify_var_uses(extra, v, streaming, other);
            }
            Value::Set(_, inner) => Self::classify_var_uses(inner, v, streaming, other),
            Value::Call(_, args) | Value::CallRef(_, args) => {
                for a in args {
                    Self::classify_var_uses(a, v, streaming, other);
                }
            }
            Value::Block(bl) | Value::Loop(bl) => {
                for op in &bl.operators {
                    Self::classify_var_uses(op, v, streaming, other);
                }
            }
            Value::Insert(ops) | Value::Tuple(ops) | Value::Parallel(ops) => {
                for op in ops {
                    Self::classify_var_uses(op, v, streaming, other);
                }
            }
            Value::If(c, t, e) => {
                Self::classify_var_uses(c, v, streaming, other);
                Self::classify_var_uses(t, v, streaming, other);
                Self::classify_var_uses(e, v, streaming, other);
            }
            Value::Return(inner) | Value::Drop(inner) | Value::Yield(inner) => {
                Self::classify_var_uses(inner, v, streaming, other);
            }
            Value::BreakWith(_, inner) | Value::TuplePut(_, _, inner) => {
                Self::classify_var_uses(inner, v, streaming, other);
            }
            Value::Span(b) => Self::classify_var_uses(&b.1, v, streaming, other),
            Value::ParFor(b) => {
                // The input position is streaming-equivalent (par
                // dispatcher iterates).  worker/threads/body are
                // ordinary expression contexts.
                if let Value::Var(u) = &b.input
                    && *u == v
                {
                    *streaming += 1;
                } else {
                    Self::classify_var_uses(&b.input, v, streaming, other);
                }
                Self::classify_var_uses(&b.worker, v, streaming, other);
                Self::classify_var_uses(&b.threads, v, streaming, other);
                Self::classify_var_uses(&b.body, v, streaming, other);
            }
            _ => {}
        }
    }

    /// After parsing a function body, check that each `&` (`RefVar`) argument is actually
    /// mutated somewhere in the body. If not, emit a compile error suggesting to drop the `&`.
    /// Also check for redundant `const` annotations on primitive parameters that are never
    /// written to — the `const` has no effect when the parameter is not modified.
    fn check_ref_mutations(&mut self, arguments: &[Argument]) {
        let code = self.data.def(self.context).code().clone();
        let mut written: HashSet<u16> = HashSet::new();
        // interprocedural param-write cache, local to this check.
        // Re-created per function-body check; small cost, avoids
        // persisting state across passes or across unrelated checks.
        let mut callee_cache: HashMap<u32, Vec<bool>> = HashMap::new();
        find_written_vars(&code, &self.data, &mut written, &mut callee_cache);
        // Enhancement: when a for-loop variable is FIELD-WRITTEN (OpSet*
        // through the loop var, not just loop-advance Set), also mark the
        // collection it iterates over as written.  The dep chain is:
        //   it: ref(T)[_vector_1]  →  _vector_1: vector<T>[items]
        // Only propagate for vars that have a field-level write (OpSet*,
        // OpCopyRecord, OpNewRecord etc.) — not plain Set (which is just
        // the loop-iterator advance).
        let mut field_written: HashSet<u16> = HashSet::new();
        find_field_written_vars(&code, &self.data, &mut field_written);
        let mut propagated: HashSet<u16> = HashSet::new();
        for &w in &field_written {
            if w < self.vars.next_var() {
                for dep in self.vars.tp(w).depend() {
                    propagated.insert(dep);
                    if dep < self.vars.next_var() {
                        for dep2 in self.vars.tp(dep).depend() {
                            propagated.insert(dep2);
                        }
                    }
                }
            }
        }
        written.extend(propagated);
        for (a_nr, a) in arguments.iter().enumerate() {
            if matches!(a.typedef, Type::RefVar(_))
                && !a.constant
                && !written.contains(&(a_nr as u16))
            {
                let src = self.vars.var_source(a_nr as u16);
                self.lexer.to(src);
                // T1.6: RefVar(Tuple) — downgrade to warning since elements are stack values;
                // other RefVar types are an error (the & serves no purpose and misleads).
                if matches!(a.typedef, Type::RefVar(ref inner) if matches!(**inner, Type::Tuple(_)))
                {
                    diagnostic!(
                        self.lexer,
                        Level::Warning,
                        "Parameter '{}' does not need to be a reference",
                        a.name
                    );
                } else {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Parameter '{}' has & but is never modified; remove the &",
                        a.name
                    );
                }
            }
            // warn when `const` is used on a primitive parameter that is never
            // written to — the annotation is redundant since the parameter would not
            // have been modified anyway.  Compound types (vector, reference, struct)
            // are exempt: `const` serves as read-only documentation on those.
            let base_tp = if let Type::RefVar(inner) = &a.typedef {
                inner.as_ref()
            } else {
                &a.typedef
            };
            if a.constant
                && !written.contains(&(a_nr as u16))
                && matches!(
                    base_tp,
                    Type::Integer(_) | Type::Float | Type::Single | Type::Boolean | Type::Character
                )
            {
                let src = self.vars.var_source(a_nr as u16);
                self.lexer.to(src);
                diagnostic!(
                    self.lexer,
                    Level::Warning,
                    "Parameter '{}' is const but is never modified; \
                     'const' has no effect on an unmodified primitive parameter",
                    a.name
                );
            }
        }
    }

    // <function> ::= 'fn' <identifier> '(' <attributes> ] [ '->' <type> ] (';' <rust> | <code>)
    pub fn null(&mut self, tp: &Type) -> Value {
        // @PLN25 slice (b): an `Optional(τ)`'s null is `τ`'s typed null (same sentinel) — peel.
        match tp.base() {
            Type::Integer(_) => self.cl("OpConvIntFromNull", &[]),
            // `character` is a 4-byte (char-domain) type: its null is `'\0'`
            // (`OpConvCharacterFromNull`), NOT the i64 integer sentinel.  Folding
            // it into `OpConvIntFromNull` (i64::MIN) made native emit `i64::MIN`
            // into an `i32` character return slot — rustc E0308 (Cluster D H4).
            Type::Character => self.cl("OpConvCharacterFromNull", &[]),
            Type::Boolean => {
                // @PLN17 spike: boolean is tri-state (255 = null sentinel).
                // Supersedes the #256 rejection — `null` on a boolean now emits the
                // 255 sentinel; truthiness contexts coerce it to false.
                self.cl("OpConvBoolFromNull", &[])
            }
            Type::Enum(tp, _, _) => self.cl(
                "OpConvEnumFromNull",
                &[Value::Int(i32::from(self.data.def(*tp).known_type()))],
            ),
            Type::Float => self.cl("OpConvFloatFromNull", &[]),
            Type::Single => self.cl("OpConvSingleFromNull", &[]),
            Type::Text(_) => self.cl("OpConvTextFromNull", &[]),
            Type::RefVar(tp) if matches!(**tp, Type::Text(_)) => self.cl("OpConvTextFromNull", &[]),
            Type::Reference(_, _) => self.cl("OpNullRefSentinel", &[]),
            _ => Value::Null,
        }
    }

    // For now, assume that returned texts are always related to internal variables
}

/// Directory portion of a normalised script path, or `""` if the path is bare.
fn script_dir(cur_script: &str) -> &str {
    cur_script.rfind(sep()).map_or("", |p| &cur_script[0..p])
}

/// If `cur_dir` lives inside a `/tests/` tree, return the ancestor above
/// `tests/`; otherwise `""`.  Used to locate the project-root `lib/`
/// directory when running a script from `tests/`.
fn tests_base_dir(cur_dir: &str) -> &str {
    let tests_infix = format!("{0}tests{0}", sep());
    if let Some(idx) = cur_dir.find(tests_infix.as_str()) {
        &cur_dir[..idx]
    } else {
        ""
    }
}

/// Walk a parsed expression looking for a `Value::FnRef` with a
/// non-MAX closure_var — the marker that the closure captures one or
/// P213: locate the capturing `Value::FnRef(d_nr, w, _)` inside `v` and
/// return `(d_nr, w)`.  Mirrors `capturing_fn_ref` (the bool variant)
/// but extracts the lambda's `d_nr` so callers can record it on the
/// host attribute via `Attribute::assigned_lambda_d_nr`.  Returns
/// `None` when no capturing FnRef is present.  Walks Block / Set /
/// Span wrappers built by `parser/vectors.rs` around the `OpDatabase`
/// allocation steps.
fn find_capturing_fn_ref(data: &Data, v: &Value) -> Option<(i32, u16)> {
    match v.unspan() {
        // `w != MAX` only appears in the second pass (`emit_lambda_code`
        // builds the closure-allocation block there).  In the FIRST
        // pass a capturing lambda still emits as a plain
        // `FnRef(d, MAX)`, so also accept a lambda whose def carries a
        // synthesized closure record — `synthesize_closure_record`
        // runs in both passes, making it the pass-1-visible capture
        // marker.  Without this, `assigned_lambda_d_nr` is never set
        // in pass 1 and `fill_database` lays the field out as the
        // legacy 4B int — no `<attr>__closure_rec` half (#313).
        Value::FnRef(d, w, _)
            if *w != u16::MAX || data.def(*d as u32).closure_record() != u32::MAX =>
        {
            Some((*d, *w))
        }
        // First pass only: `emit_lambda_code` emits a lambda as a bare
        // `Int(d_nr)` there (no closure-allocation block yet), so a
        // capturing lambda is recognisable only through its def's
        // closure record.  Inside the fn-ref write arm an Int IS the
        // lambda's d_nr by construction (the non-capturing write path
        // stores exactly this Int).
        Value::Int(d)
            if *d >= 0
                && (*d as usize) < data.definitions.len()
                && data.def(*d as u32).closure_record() != u32::MAX =>
        {
            Some((*d, u16::MAX))
        }
        Value::Block(bl) => bl
            .operators
            .iter()
            .find_map(|op| find_capturing_fn_ref(data, op)),
        Value::Set(_, rhs) => find_capturing_fn_ref(data, rhs),
        _ => None,
    }
}

/// P213: emit the IR for writing a fn-ref to a struct field.
///
/// Three cases:
/// - Inline non-capturing lambda (`Value::FnRef(d, MAX, _)`): write
///   `OpSetInt4(d_nr)`; the closure_rec vector stays empty.
/// - Inline capturing lambda (a `fn_ref_with_closure` Block ending in
///   `Value::FnRef(d, w, _)`): run the lambda's existing alloc_steps
///   (allocate the closure record in parent's Store under `var(w)`),
///   write `OpSetInt4(d_nr)`, then `OpAppendVector` deep-copies the
///   parent-Store closure record into element [0] of the host's
///   `__closure_rec` vector field.  Parent's record is freed by the
///   existing parent-scope `OpFreeRef(w)`; host owns its own copy.
/// - Non-inline source (`Var` / `Call` returning a fn-ref) or unrecognised
///   shape: emit a placeholder `OpSetInt4(0)` write and a parse-time
///   diagnostic so the user sees the limitation in the second pass.
fn emit_fn_ref_field_write(
    p: &mut Parser,
    d_nr: u32,
    f_nr: usize,
    ref_code: Value,
    pos_val: Value,
    val_code: &Value,
) -> Value {
    let unspanned = val_code.unspan().clone();
    match unspanned {
        Value::FnRef(d, w, _) if w == u16::MAX => {
            // Non-capturing inline lambda — just write the d_nr.
            p.cl("OpSetInt4", &[ref_code, pos_val, Value::Int(d)])
        }
        Value::Block(bl) if bl.name == "fn_ref_with_closure" => {
            // Capturing inline lambda.  bl.operators = [
            //   alloc_steps...,
            //   FnRef(d, w, _)  // last element
            // ].
            let last = bl.operators.last().cloned().unwrap_or(Value::Null);
            let Value::FnRef(d, w, _) = last.unspan() else {
                if !p.first_pass {
                    diagnostic!(
                        p.lexer,
                        Level::Error,
                        "internal: fn_ref_with_closure block did not end in FnRef"
                    );
                }
                return Value::Null;
            };
            let (lambda_d, w_var) = (*d, *w);
            let mut ops: Vec<Value> = bl
                .operators
                .iter()
                .take(bl.operators.len() - 1)
                .cloned()
                .collect();
            // Write the d_nr at the loft-attribute position (which maps
            // to the database-side `<attr>` field — the d_nr half).
            ops.push(p.cl(
                "OpSetInt4",
                &[ref_code.clone(), pos_val.clone(), Value::Int(lambda_d)],
            ));
            // Deep-copy the parent-Store closure record into the host's
            // `__closure_rec` vector at pos+4.  We need the host's
            // closure_rec field as a DbRef + the closure record's
            // known_type for `OpAppendVector`'s type parameter.
            if w_var != u16::MAX && f_nr != usize::MAX && !p.first_pass {
                let closure_rec_d = p.data.def(lambda_d as u32).closure_record();
                if closure_rec_d != u32::MAX {
                    let closure_kt = p.data.def(closure_rec_d).known_type();
                    let crec_pos = match &pos_val {
                        Value::Int(pi) => Value::Int(pi + 4),
                        _ => Value::Int(0),
                    };
                    // OpGetField(host_ref, pos+4, type_id) yields a DbRef
                    // pointing at the host's closure_rec field.
                    let crec_field = p.cl(
                        "OpGetField",
                        &[
                            ref_code.clone(),
                            crec_pos,
                            Value::Int(i32::from(closure_kt)),
                        ],
                    );
                    ops.push(p.cl(
                        "OpClaimChildRec",
                        &[
                            crec_field,
                            Value::Var(w_var),
                            Value::Int(i32::from(closure_kt)),
                        ],
                    ));
                }
            }
            v_block(ops, Type::Void, "fn_ref_field_set")
        }
        Value::Var(v) => {
            // P215: lift the deferred diagnostic when both sides are
            // non-capturing — target field is 4B int layout
            // (`assigned_lambda_d_nr == u32::MAX`) AND the source var
            // is not in `closure_vars` (presence in that map signals a
            // capturing-lambda assignment per
            // `parser/expressions.rs:1217`).  In that case writing
            // just the d_nr is lossless: the source's closure DbRef
            // component is the null sentinel and there's no
            // `__closure_rec` half on the target to receive it.
            //
            // Closure-record-internal fn-ref fields always satisfy the
            // target check (synthesize_closure_record never sets
            // `assigned_lambda_d_nr`), so capturing fn-ref names from
            // outer scopes now write through to the closure record
            // when the captured lambda itself is non-capturing —
            // which is the canonical P215 reproducer.
            // Like the read side, the split/legacy answer must come
            // from the database layout (`fn_ref_field_is_split`), not
            // the body-parse-order-dependent flag (#313).
            let target_is_4b = if (d_nr as usize) < p.data.definitions.len()
                && f_nr < p.data.def(d_nr).attributes().len()
            {
                !p.fn_ref_field_is_split(d_nr, f_nr)
            } else {
                false
            };
            let source_is_noncapturing =
                matches!(p.vars.tp(v), Type::Function(_, _, _)) && !p.closure_vars.contains_key(&v);
            if target_is_4b && source_is_noncapturing {
                return p.cl("OpSetInt4", &[ref_code, pos_val, Value::FnRefDnr(v)]);
            }
            if !p.first_pass {
                diagnostic!(
                    p.lexer,
                    Level::Error,
                    "only inline lambda literals can be stored in fn-ref struct fields; \
                     non-inline (variable / call) sources are not yet supported when the \
                     source closure may capture (P215-deferred)"
                );
            }
            // Emit a no-op so the rest of construction proceeds.
            p.cl("OpSetInt4", &[ref_code, pos_val, Value::Int(0)])
        }
        Value::Call(_, _) => {
            // P213-deferred: non-inline source.  Capturing-call
            // sources need closure-DbRef copy; deferred.
            if !p.first_pass {
                diagnostic!(
                    p.lexer,
                    Level::Error,
                    "only inline lambda literals can be stored in fn-ref struct fields in this release; \
                     bind the lambda directly inside the struct constructor"
                );
            }
            p.cl("OpSetInt4", &[ref_code, pos_val, Value::Int(0)])
        }
        other => {
            // Fallback (Null default-init etc.) — write 0.
            let d_nr_only = match other {
                Value::FnRef(d, _, _) => Value::Int(d),
                Value::Int(n) => Value::Int(n),
                Value::Null => Value::Int(0),
                v => v,
            };
            p.cl("OpSetInt4", &[ref_code, pos_val, d_nr_only])
        }
    }
}

fn merge_dependencies(a: &Type, b: &Type) -> Type {
    // Never (return/break/continue) defers to the other branch's type.
    if matches!(a, Type::Never) {
        return b.clone();
    }
    if matches!(b, Type::Never) {
        return a.clone();
    }
    if let (Type::Text(da), Type::Text(db)) = (a, b) {
        let mut d = HashSet::new();
        for v in da {
            d.insert(*v);
        }
        for v in db {
            d.insert(*v);
        }
        Type::Text(Deps::frame(d.into_iter().collect()))
    } else {
        a.clone()
    }
}

fn field_id(key: &[(String, bool)], name: &mut String) {
    for (k_nr, (k, asc)) in key.iter().enumerate() {
        if k_nr > 0 {
            *name += ",";
        }
        if !asc {
            *name += "-";
        }
        *name += k;
    }
    *name += "]>";
}

/// Collect all `Value::Var` indices reachable anywhere in `val`.
fn collect_vars_in(val: &Value, result: &mut HashSet<u16>) {
    match val {
        Value::Var(v) => {
            result.insert(*v);
        }
        Value::Set(_, body) => collect_vars_in(body, result),
        Value::Call(_, args) => {
            for a in args {
                collect_vars_in(a, result);
            }
        }
        Value::Block(b) | Value::Loop(b) => {
            for op in &b.operators {
                collect_vars_in(op, result);
            }
        }
        Value::Insert(list) => {
            for item in list {
                collect_vars_in(item, result);
            }
        }
        Value::If(c, t, e) => {
            collect_vars_in(c, result);
            collect_vars_in(t, result);
            collect_vars_in(e, result);
        }
        Value::Return(v) | Value::Drop(v) => collect_vars_in(v, result),
        Value::Iter(_, a, b, c) => {
            collect_vars_in(a, result);
            collect_vars_in(b, result);
            collect_vars_in(c, result);
        }
        Value::Span(b) => collect_vars_in(&b.1, result),
        _ => {}
    }
}

/// Recursively walk a Value IR tree and collect all variable indices that are written.
/// A variable is considered written if:
/// - It appears as the target of `Value::Set(v, ...)`,
/// - It is passed as a `RefVar`-typed argument to a `Value::Call`, or
/// - It appears anywhere in the first argument of a field-write operator (`OpSet*`),
///   which covers the pattern `v[idx].field = val` where `v: &vector<T>`.
/// - It flows into a callee whose own body mutates that parameter
///   (directly or transitively via further calls).  The
///   interprocedural lookup is memoised via `callee_cache`.
pub(crate) fn find_written_vars(
    code: &Value,
    data: &Data,
    written: &mut HashSet<u16>,
    callee_cache: &mut HashMap<u32, Vec<bool>>,
) {
    match code {
        Value::Set(v, body) => {
            written.insert(*v);
            find_written_vars(body, data, written, callee_cache);
        }
        Value::Call(fn_nr, args) => {
            let def = data.def(*fn_nr);
            let attrs = def.attributes();
            // Operators whose FIRST argument is mutated (collection / field writes).
            // vector ops folded in here so `c.items += other_vec` (where `c.items`
            // is `OpGetField(Var(c), …)`) correctly marks `c` as written via
            // collect_vars_in.  Previously the OpAppend*/OpClear* family only checked for
            // a bare `Value::Var` arg, missing the field-access shape.
            let first_arg_write = def.name().starts_with("OpSet")
                || def.name().starts_with("OpAppendStack")
                || def.name().starts_with("OpClearStack")
                || def.name() == "OpNewRecord"
                || def.name() == "OpAppendCopy"
                || def.name() == "OpAppendVector"
                || def.name() == "OpClearVector"
                || def.name() == "OpClearKeyed"
                || def.name() == "OpSetKeyed"
                // @P320: keyed-remove `coll[key] = null` lowers to
                // `OpHashRemove(coll, …)` (collections.rs::towards_set_hash_remove),
                // so it mutates its first arg just like OpSetKeyed/OpClearKeyed.
                // Without this a `&` param whose only mutation is a keyed remove
                // was wrongly rejected as "never modified".
                || def.name() == "OpHashRemove"
                || def.name() == "OpInsertVector"
                || def.name() == "OpRemoveVector";
            // OpCopyRecord(src, dst, type) writes through `dst` (arg[1]).
            // Used by struct field whole-replacement (`s.i = fresh`) where the
            // destination is `OpGetField(s, …)`.
            let second_arg_write = def.name() == "OpCopyRecord";
            for (i, arg) in args.iter().enumerate() {
                if i < attrs.len()
                    && matches!(attrs[i].typedef, Type::RefVar(_))
                    && let Value::Var(v) = arg
                {
                    written.insert(*v);
                }
                if i == 0 && first_arg_write {
                    collect_vars_in(arg, written);
                }
                if i == 1 && second_arg_write {
                    collect_vars_in(arg, written);
                }
                find_written_vars(arg, data, written, callee_cache);
            }
            // the callee may mutate one of its by-value parameters
            // through a field write (e.g. `fn add(self: Box, x) { self.items += [x] }`).
            // Look up its param-write effects and mark the corresponding
            // caller-side arg vars so `check_ref_mutations` sees them as
            // mutated.  Skip natives (`def.code == Value::Null`) — their
            // effects are already encoded by the OpSet*/OpAppend*/OpCopyRecord
            // patterns above.  Args are collected with `collect_vars_in` so
            // wrapped sources (field access, `OpCreateStack(Var(_))` from
            // the hoisted-preamble path) still propagate the mutation to
            // their root var.
            if *def.code() != Value::Null {
                let callee_writes = callee_param_writes(*fn_nr, data, callee_cache);
                for (i, arg) in args.iter().enumerate() {
                    if i < callee_writes.len() && callee_writes[i] {
                        collect_vars_in(arg, written);
                    }
                }
            }
        }
        Value::Block(block) | Value::Loop(block) => {
            for item in &block.operators {
                find_written_vars(item, data, written, callee_cache);
            }
        }
        Value::Insert(list) => {
            for item in list {
                find_written_vars(item, data, written, callee_cache);
            }
        }
        Value::If(cond, then, els) => {
            find_written_vars(cond, data, written, callee_cache);
            find_written_vars(then, data, written, callee_cache);
            find_written_vars(els, data, written, callee_cache);
        }
        Value::Return(v) | Value::Drop(v) => {
            find_written_vars(v, data, written, callee_cache);
        }
        // T1.5: TuplePut writes to the ref-tuple variable via its element assignment.
        Value::TuplePut(var_nr, _, inner) => {
            written.insert(*var_nr);
            find_written_vars(inner, data, written, callee_cache);
        }
        Value::Iter(_, create, next, extra) => {
            find_written_vars(create, data, written, callee_cache);
            find_written_vars(next, data, written, callee_cache);
            find_written_vars(extra, data, written, callee_cache);
        }
        Value::Span(b) => find_written_vars(&b.1, data, written, callee_cache),
        _ => {}
    }
}

/// for the given user-defined function, return a boolean per
/// parameter indicating whether its body writes that parameter
/// (directly or through a transitive call).  Results are memoised
/// in `cache`; a placeholder (all-false) is inserted before recursive
/// analysis so cycles are broken.  Caller should iterate to fixpoint
/// if precise transitive effects across recursion chains are needed;
/// for linear forwarding (the common case) one pass suffices.
fn callee_param_writes(fn_nr: u32, data: &Data, cache: &mut HashMap<u32, Vec<bool>>) -> Vec<bool> {
    if let Some(v) = cache.get(&fn_nr) {
        return v.clone();
    }
    let def = data.def(fn_nr);
    let n = def.attributes().len();
    // Break recursion: insert a placeholder before walking the body.
    cache.insert(fn_nr, vec![false; n]);
    if *def.code() == Value::Null || n == 0 {
        return vec![false; n];
    }
    let body = def.code().clone();
    let mut written: HashSet<u16> = HashSet::new();
    find_written_vars(&body, data, &mut written, cache);
    let result: Vec<bool> = (0..n).map(|i| written.contains(&(i as u16))).collect();
    // Monotone merge with any prior placeholder entry.
    let prev = cache.get(&fn_nr).cloned().unwrap_or_else(|| vec![false; n]);
    let merged: Vec<bool> = prev
        .iter()
        .zip(result.iter())
        .map(|(a, b)| *a || *b)
        .collect();
    cache.insert(fn_nr, merged.clone());
    merged
}

/// Like `find_written_vars` but only collects variables that are FIELD-written
/// (OpSet*, OpCopyRecord, OpNewRecord first-arg).  Excludes plain `Value::Set`
/// which includes loop-iterator advance — that's not a user-initiated mutation.
/// Used by check_ref_mutations to detect when a for-loop variable's field
/// writes should propagate back to the iterated `&` collection, and by the
/// @PLN101 value-struct copy-elision pass (`scopes::value_struct_copy`) to prove
/// a read-only view's base is never mutated under it.
pub(crate) fn find_field_written_vars(code: &Value, data: &Data, written: &mut HashSet<u16>) {
    match code {
        Value::Call(fn_nr, args) => {
            let def = data.def(*fn_nr);
            let first_arg_write = def.name().starts_with("OpSet")
                || def.name() == "OpNewRecord"
                || def.name() == "OpAppendCopy"
                || def.name() == "OpAppendVector"
                || def.name() == "OpClearVector"
                || def.name() == "OpClearKeyed"
                || def.name() == "OpSetKeyed"
                // @P320: keyed-remove `coll[key] = null` → `OpHashRemove(coll, …)`;
                // mirror the find_written_vars set so a keyed remove inside a
                // `for … in &coll` loop also counts as a mutation.
                || def.name() == "OpHashRemove"
                || def.name() == "OpInsertVector"
                || def.name() == "OpRemoveVector";
            let second_arg_write = def.name() == "OpCopyRecord";
            for (i, arg) in args.iter().enumerate() {
                if i == 0 && first_arg_write {
                    collect_vars_in(arg, written);
                }
                if i == 1 && second_arg_write {
                    collect_vars_in(arg, written);
                }
                find_field_written_vars(arg, data, written);
            }
        }
        Value::Set(_, body) => find_field_written_vars(body, data, written),
        Value::Block(block) | Value::Loop(block) => {
            for item in &block.operators {
                find_field_written_vars(item, data, written);
            }
        }
        Value::Insert(list) => {
            for item in list {
                find_field_written_vars(item, data, written);
            }
        }
        Value::If(cond, then, els) => {
            find_field_written_vars(cond, data, written);
            find_field_written_vars(then, data, written);
            find_field_written_vars(els, data, written);
        }
        Value::Return(v) | Value::Drop(v) => find_field_written_vars(v, data, written),
        Value::Iter(_, create, next, extra) => {
            find_field_written_vars(create, data, written);
            find_field_written_vars(next, data, written);
            find_field_written_vars(extra, data, written);
        }
        Value::Span(b) => find_field_written_vars(&b.1, data, written),
        _ => {}
    }
}

/// Map an operator token to its CamelCase name suffix used in `OpCamelCase` identifiers.
/// E.g. `"<"` → `"Lt"`, so the method name becomes `"OpLt"`.
/// Also used by I3.1 (`op <token>` sugar in interface bodies).
pub(crate) fn rename(op: &str) -> &str {
    match op {
        "*" => "Mul",
        "+" => "Add",
        "-" => "Min",
        "/" => "Div",
        "&" => "Land",
        "|" => "Lor",
        "^" => "Eor",
        "<<" => "SLeft",
        ">>" => "SRight",
        "==" => "Eq",
        "!=" => "Ne",
        "<" => "Lt",
        "<=" => "Le",
        ">" => "Gt",
        ">=" => "Ge",
        "%" => "Rem",
        "**" => "Pow",
        "!" => "Not",
        "~" => "BitNot",
        "+=" => "Append",
        _ => op,
    }
}

#[cfg(test)]
mod p269_native_backfill_tests {
    use super::*;

    /// P269 regression — with MORE THAN ONE native package registered,
    /// `backfill_native_symbol_crates` must still bind each unmapped `#native`
    /// symbol to its owning package (the registered package whose directory is a
    /// prefix of the def's source file).  The old `len() != 1` guard bailed and
    /// left them unmapped → a `--native` P269 compile error on a symbol the
    /// interpreter dispatched fine.  Two native packages (`imaging` +
    /// `p269_native_b`); we then clear the map to reproduce the crawler
    /// precondition (imaging's `#native` symbols left unbound by the per-manifest
    /// pass, which runs before the package source is parsed) so backfill is their
    /// only binding site.
    #[test]
    fn backfill_binds_symbols_under_multiple_native_packages() {
        let sep = crate::platform::sep_str();
        let mut p = Parser::new();
        p.parse_dir("default", true, true).unwrap();
        p.lib_dirs = vec![
            format!("tests{sep}lib"),
            format!("tests{sep}fixtures{sep}libs"),
        ];
        p.parse(
            &format!("tests{sep}lib{sep}p269_two_native_pkgs_main.loft"),
            false,
        );
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "unexpected parse errors: {:?}",
            p.diagnostics.lines()
        );

        // The bug trigger: more than one registered native package.
        assert!(
            p.data.native_packages.len() >= 2,
            "expected >= 2 native packages (imaging + p269_native_b), got {:?}",
            p.data.native_packages
        );

        // Collect imaging's #native symbols FROM the parsed defs (those whose
        // source lives under imaging's package dir), so the test never hardcodes
        // a library `n_*` symbol in src/ — the @PLAN12 extraction-hygiene gate
        // forbids that, since imaging is an extracted library.
        let imaging_dir = p
            .data
            .native_packages
            .iter()
            .find(|(c, _)| c == "loft-imaging")
            .map(|(_, d)| d.clone())
            .expect("imaging registered as a native package");
        let imaging_syms: Vec<String> = (0..p.data.definitions())
            .map(|d| p.data.def(d))
            .filter(|def| {
                !def.native().is_empty() && def.position().file.starts_with(imaging_dir.as_str())
            })
            .map(|def| def.native().to_string())
            .collect();
        assert!(
            !imaging_syms.is_empty(),
            "expected imaging to contribute #native symbols"
        );

        // Reproduce the precondition the crawler hit: those symbols are not yet
        // bound, so backfill is the only place they can bind.
        p.data.native_symbol_crates.clear();
        p.backfill_native_symbol_crates();

        for sym in &imaging_syms {
            assert_eq!(
                p.data.native_symbol_crates.get(sym).map(String::as_str),
                Some("loft_imaging"),
                "{sym} not bound to loft_imaging after backfill - map={:?}",
                p.data.native_symbol_crates
            );
        }
    }

    /// @PLN40 step 3 — `const` on a struct field sets `const_field` on that
    /// attribute and leaves plain fields untouched.  Non-vacuous: a no-op mark
    /// fails the first assert, an over-eager mark the second.
    #[test]
    fn pln40_const_field_flag_is_set() {
        let mut p = Parser::new();
        p.parse_dir("default", true, false).unwrap();
        p.parse_str(
            "struct Cell { const c_color: integer, height: integer }",
            "pln40_const_field_flag_is_set",
            false,
        );
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "unexpected parse errors: {:?}",
            p.diagnostics.lines()
        );
        let cell = p.data.def_nr("Cell");
        assert_ne!(cell, u32::MAX, "Cell def not found");
        let color = p.data.attr(cell, "c_color");
        let height = p.data.attr(cell, "height");
        assert_ne!(color, usize::MAX, "c_color attr missing");
        assert_ne!(height, usize::MAX, "height attr missing");
        assert!(
            p.data.def(cell).attributes()[color].const_field,
            "const field c_color must set const_field"
        );
        assert!(
            !p.data.def(cell).attributes()[height].const_field,
            "plain field height must NOT be const"
        );
    }

    /// @PLN26 phase 1 — two native packages exporting the SAME `#native` symbol
    /// must be detected: the C-ABI flat namespace (and the interpreter's
    /// symbol-keyed `BRIDGE_REGISTRY`) can't disambiguate them, so native
    /// codegen rejects the program with a `compile_error!`.  Parse a consumer of
    /// collide_a + collide_b (both export `collide_shared`) and check the
    /// detector reports it.
    #[test]
    fn native_symbol_collision_across_packages_detected() {
        let sep = crate::platform::sep_str();
        let mut p = Parser::new();
        p.parse_dir("default", true, true).unwrap();
        p.lib_dirs = vec![format!("tests{sep}lib")];
        p.parse(&format!("tests{sep}lib{sep}collide_main.loft"), false);
        let collisions = p.data.native_symbol_collisions();
        assert!(
            collisions
                .iter()
                .any(|(sym, srcs)| sym == "collide_shared" && srcs.len() >= 2),
            "expected a `collide_shared` collision across >= 2 sources, got {collisions:?}"
        );
    }
}

#[cfg(test)]
mod plan86_sandbox_designation_tests {
    use super::*;
    use crate::sandbox::parse_sandbox_config;

    /// @PLN86 step 1.2 — a host designation (`fn:<name>`) tags exactly the named
    /// function with its profile; other functions stay unrestricted; and the tag
    /// comes from the host policy, never the source (only `fn:scripted` is
    /// designated, yet `host` in the same file is untagged).
    #[test]
    fn fn_designation_tags_only_the_designated_function() {
        let mut p = Parser::new();
        p.set_sandbox_config(parse_sandbox_config(
            "[sandbox]\nmod-script = [\"fn:scripted\"]\n[profile.mod-script]\nallow = [\"math#read\"]\n",
        ));
        let dir = std::env::temp_dir();
        let path = dir.join(format!("plan86_designation_{}.loft", std::process::id()));
        std::fs::write(&path, "fn scripted() { }\nfn host() { }\n").unwrap();
        p.parse(path.to_str().unwrap(), false);
        let _ = std::fs::remove_file(&path);
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "unexpected parse errors: {:?}",
            p.diagnostics.lines()
        );
        let scripted = p.data.def_nr("n_scripted");
        let host = p.data.def_nr("n_host");
        assert_ne!(scripted, u32::MAX, "scripted fn registered");
        assert_ne!(host, u32::MAX, "host fn registered");
        assert_eq!(p.def_sandbox_profile(scripted), Some("mod-script"));
        assert_eq!(p.def_sandbox_profile(host), None);
    }
}

#[cfg(test)]
mod plan86_nesting_guard_tests {
    use super::*;
    use crate::sandbox::parse_sandbox_config;

    fn sandboxed_parser() -> Parser {
        let mut p = Parser::new();
        p.set_sandbox_config(parse_sandbox_config(
            "[sandbox]\nmod-script = [\"fn:scripted\"]\n[profile.mod-script]\nallow = [\"math#read\"]\n",
        ));
        p
    }

    fn parse_source(p: &mut Parser, src: &str) {
        // Process-global counter (not the `src` pointer) for a collision-free name
        // across concurrent test threads — see `parse_admit_libs`.
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "plan86_nest_{}_{}.loft",
            std::process::id(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        ));
        std::fs::write(&path, src).unwrap();
        p.parse(path.to_str().unwrap(), false);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn capability_declarations_register_and_resolve() {
        // @PLN86 P6.1 — a `capability` decl registers its dotted name; a
        // `group#right` link resolves against it, and an undeclared group does not
        // (the load error a mistyped link becomes).  The dotted name is the namespace.
        let mut p = Parser::new();
        parse_source(
            &mut p,
            "capability fs\ncapability cmd.move\nfn main() { }\n",
        );
        assert!(p.cap_is_declared("fs"), "fs should be declared");
        assert!(
            p.cap_is_declared("cmd.move"),
            "the dotted name cmd.move should be declared"
        );
        assert!(
            !p.cap_is_declared("typo"),
            "an undeclared group must not resolve"
        );
        // the `#right` suffix parses to exactly the three rights, nothing else.
        use crate::sandbox::Right;
        assert_eq!(Right::parse("read"), Some(Right::Read));
        assert_eq!(Right::parse("update"), Some(Right::Update));
        assert_eq!(Right::parse("append"), Some(Right::Append));
        assert_eq!(Right::parse("delete"), None);
    }

    #[test]
    fn field_capability_links_are_recorded() {
        // @PLN86 P6.4 — a `group#right` link after a struct field's type is recorded
        // per (struct, field); an unlinked field carries none; multiple links stack;
        // append rides on a collection field.
        let mut p = Parser::new();
        parse_source(
            &mut p,
            "capability stats\ncapability bag\n\
             struct Item { v: integer }\n\
             struct Entity { id: integer, health: integer stats#read stats#update, \
             loot: Item bag#read bag#append }\n\
             fn main() { }\n",
        );
        let e = p.data.def_nr("Entity");
        let links = |f: &str| -> Vec<String> { p.member_links(e, f).to_vec() };
        assert!(links("id").is_empty(), "an unlinked field carries no links");
        assert_eq!(links("health"), ["stats#read", "stats#update"]);
        assert_eq!(links("loot"), ["bag#read", "bag#append"]);
    }

    /// @PLN86 0.1 — hostile deep nesting inside a sandboxed def is a clean
    /// LOAD-time parse error, NOT a native stack overflow (rc=139).  The
    /// definitive check is the pair of asserts below (a depth error was emitted
    /// AND it is the nesting-depth diagnostic); the explicit stack size only has
    /// to be large enough that the process REACHES those asserts before the guard
    /// trips at 128 — a stack overflow aborts the whole process, it is not a
    /// catchable panic.
    ///
    /// Sizing: one nesting level costs ≈15 KB of native stack (measured; the
    /// `expression → operators → part → single` chain — the null-flow parse logic
    /// in #559 grew it from ≈10 KB), so 128 levels ≈ 1.8 MB uninstrumented.  The
    /// ASan gate inflates every frame ≈4.4× (redzones + shadow), pushing 128
    /// levels to ≈8–9 MB — which overflowed the former 8 MB thread here and turned
    /// the ASan gate red (a guard that fires correctly, starved of stack).  64 MB
    /// clears 128 ASan-inflated levels with margin, while a runaway (broken guard →
    /// 2000 levels ≈ 128 MB under ASan) still overflows, so a regressed guard is
    /// caught either by the asserts or by an overflow.
    #[test]
    fn deep_nesting_in_sandboxed_def_is_a_clean_error_not_a_crash() {
        let (has_error, has_msg) = std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let depth = 2000; // >> SANDBOX_MAX_PARSE_DEPTH and into the overflow zone
                let src = format!(
                    "fn scripted() {{ x = {}1{}; }}\n",
                    "(".repeat(depth),
                    ")".repeat(depth)
                );
                let mut p = sandboxed_parser();
                parse_source(&mut p, &src);
                let has_error = p.diagnostics.level() >= crate::diagnostics::Level::Error;
                let has_msg = p
                    .diagnostics
                    .lines()
                    .iter()
                    .any(|l| l.contains("nesting too deep"));
                (has_error, has_msg)
            })
            .unwrap()
            .join()
            .expect("the parse thread must not overflow/panic");
        assert!(has_error, "expected a depth error");
        assert!(has_msg, "expected the nesting-depth diagnostic");
    }

    /// The guard must not false-trip: ordinary nesting well below the limit —
    /// and many sibling expressions — parse cleanly (proves the depth counter is
    /// balanced: each sub-expression decrements, so siblings don't accumulate).
    #[test]
    fn ordinary_nesting_in_sandboxed_def_parses_clean() {
        use std::fmt::Write as _;
        let mut body = String::from("fn scripted() {\n");
        for i in 0..200 {
            let _ = writeln!(body, "  a{i} = ((({i})));"); // shallow, 200 siblings
        }
        body.push_str("}\n");
        let mut p = sandboxed_parser();
        parse_source(&mut p, &body);
        assert!(
            !p.diagnostics
                .lines()
                .iter()
                .any(|l| l.contains("nesting too deep")),
            "guard false-tripped on ordinary code: {:?}",
            p.diagnostics.lines()
        );
    }
}

#[cfg(test)]
mod plan86_reachable_set_tests {
    use super::*;
    use crate::sandbox::parse_sandbox_config;

    fn parse_with_sandbox(selectors: &[&str], src: &str) -> Parser {
        let list = selectors
            .iter()
            .map(|s| format!("\"{s}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let cfg = format!("[sandbox]\nmod = [{list}]\n[profile.mod]\nallow = [\"x#read\"]\n");
        let mut p = Parser::new();
        p.set_sandbox_config(parse_sandbox_config(&cfg));
        p.parse_dir("default", true, true).unwrap(); // `integer` et al. live in the stdlib
        // Process-global counter (not the `src` pointer) for a collision-free name
        // across concurrent test threads — see `parse_admit_libs`.
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "plan86_reach_{}_{}.loft",
            std::process::id(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        ));
        std::fs::write(&path, src).unwrap();
        p.parse(path.to_str().unwrap(), false);
        let _ = std::fs::remove_file(&path);
        p
    }

    /// @PLN86 1.3 — the closure descends into sandboxed defs (so their callees are
    /// reachable) but treats a trusted symbol as a LEAF: `untrusted_helper` is
    /// recorded yet NOT descended, so the `hidden_leaf` it alone reaches is
    /// excluded — the trust boundary §4 demands.
    #[test]
    fn closure_descends_sandboxed_defs_and_stops_at_trusted_leaves() {
        let src = "fn allowed_leaf() -> integer { 1 }\n\
                   fn hidden_leaf() -> integer { 2 }\n\
                   fn inner() -> integer { allowed_leaf() }\n\
                   fn untrusted_helper() -> integer { hidden_leaf() }\n\
                   fn scripted() -> integer { inner(); untrusted_helper() }\n";
        let p = parse_with_sandbox(&["fn:scripted", "fn:inner"], src);
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "parse errors: {:?}",
            p.diagnostics.lines()
        );
        let reach = p.sandbox_reachable_set();
        let id = |n: &str| p.data.def_nr(n);
        assert!(reach.contains(&id("n_scripted")));
        assert!(reach.contains(&id("n_inner"))); // sandboxed → descended
        assert!(reach.contains(&id("n_allowed_leaf"))); // reached via inner
        assert!(reach.contains(&id("n_untrusted_helper"))); // referenced → leaf
        assert!(
            !reach.contains(&id("n_hidden_leaf")),
            "a trusted leaf must not be descended"
        );
    }

    /// L4 — a fn-ref passed as a `fn(...)`-typed argument (emitted as a bare
    /// `Int(def_nr)`) is in the set, so an indirect call can't escape admission.
    #[test]
    fn fnref_passed_as_argument_is_reachable_l4() {
        let src = "fn target(n: integer) -> integer { n }\n\
                   fn apply(f: fn(integer) -> integer, n: integer) -> integer { f(n) }\n\
                   fn scripted() -> integer { apply(target, 5) }\n";
        let p = parse_with_sandbox(&["fn:scripted"], src);
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "parse errors: {:?}",
            p.diagnostics.lines()
        );
        assert!(
            p.sandbox_reachable_set()
                .contains(&p.data.def_nr("n_target")),
            "fn-ref argument must be reachable (L4)"
        );
    }

    /// L4 — a fn-ref laundered through a `fn(...)`-typed local (`f = target`,
    /// emitted as `Set(f, Int(def_nr))`) is still in the set.
    #[test]
    fn fnref_laundered_through_a_variable_is_reachable_l4() {
        let src = "fn target(n: integer) -> integer { n }\n\
                   fn apply(f: fn(integer) -> integer, n: integer) -> integer { f(n) }\n\
                   fn scripted() -> integer { f = target; apply(f, 5) }\n";
        let p = parse_with_sandbox(&["fn:scripted"], src);
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "parse errors: {:?}",
            p.diagnostics.lines()
        );
        assert!(
            p.sandbox_reachable_set()
                .contains(&p.data.def_nr("n_target")),
            "fn-ref laundered through a variable must be reachable (L4)"
        );
    }

    /// @PLN86 1.4 — a sandboxed def reaching an EXTERNAL native bridge (its
    /// `#native` symbol owned by a native package) is flagged; a `#native` symbol
    /// with no external-package owner (a built-in op) is not.
    /// `native_symbol_crates` is the discriminator `backfill_native_symbol_crates`
    /// fills for package-owned symbols.
    #[test]
    fn reachable_external_ffi_bridge_is_flagged_local_native_is_not() {
        let src = "fn ext_fn() -> integer; #native \"ext_sym\"\n\
                   fn local_native() -> integer; #native \"local_sym\"\n\
                   fn scripted() -> integer { ext_fn(); local_native() }\n";
        let mut p = parse_with_sandbox(&["fn:scripted"], src);
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "parse errors: {:?}",
            p.diagnostics.lines()
        );
        // Simulate `ext_sym` being owned by an external native package — exactly
        // what `backfill_native_symbol_crates` records for a def under a
        // `native_packages` dir.  `local_sym` is left unowned.
        p.data
            .native_symbol_crates
            .insert("ext_sym".to_string(), "extcrate".to_string());
        let bridges = p.sandbox_ffi_bridges();
        assert!(
            bridges.contains(&p.data.def_nr("n_ext_fn")),
            "reachable external FFI bridge must be flagged, got {bridges:?}"
        );
        assert!(
            !bridges.contains(&p.data.def_nr("n_local_native")),
            "a #native symbol with no external-package owner must NOT be flagged"
        );
    }

    /// @PLN86 — a `group#right` call-gate link parses off the signature onto a def
    /// and is readable; the read/update distinction round-trips, and an unlinked def
    /// reads as `None`.
    #[test]
    fn cap_annotation_is_parsed_and_readable() {
        let src = "fn reader() -> integer collections#read;\n#native\n\
                   fn writer() -> integer collections#update;\n#native\n\
                   fn plain() -> integer { 0 }\n";
        let p = parse_with_sandbox(&[], src);
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "parse errors: {:?}",
            p.diagnostics.lines()
        );
        assert_eq!(
            p.def_cap_group(p.data.def_nr("n_reader")),
            Some("collections#read")
        );
        assert_eq!(
            p.def_cap_group(p.data.def_nr("n_writer")),
            Some("collections#update")
        );
        assert_eq!(p.def_cap_group(p.data.def_nr("n_plain")), None);
    }
}

#[cfg(test)]
mod plan86_admission_tests {
    use super::*;
    use crate::sandbox::{CapViolation, TotalityViolation, parse_sandbox_config};

    fn quoted(items: &[&str]) -> String {
        items
            .iter()
            .map(|s| format!("\"{s}\""))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Load the stdlib + parse `src` under the literal `[sandbox]` config `cfg`.
    fn parse_admit_cfg(cfg: &str, src: &str) -> Parser {
        let mut p = Parser::new();
        p.set_sandbox_config(parse_sandbox_config(cfg));
        p.parse_dir("default", true, true).unwrap();
        // A process-global counter, not the `src` pointer: cargo runs tests as
        // threads in ONE process, so a deterministic name (the literal's address)
        // can collide across concurrent threads — Windows' strict file locking
        // then fails the write where Unix tolerates it.
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "plan86_admit_{}_{}.loft",
            std::process::id(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        ));
        std::fs::write(&path, src).unwrap();
        p.parse(path.to_str().unwrap(), false);
        let _ = std::fs::remove_file(&path);
        p
    }

    /// #631 — parse `src` from a temp file whose BASENAME is handed to `cfg_for`, so
    /// a test can designate the file by a path selector and allow-list the library
    /// the source actually lands in (both are derived from the generated name).
    fn parse_admit_file(cfg_for: impl Fn(&str, &str) -> String, src: &str) -> Parser {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let stem = format!(
            "plan86file_{}_{}",
            std::process::id(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        );
        let path = std::env::temp_dir().join(format!("{stem}.loft"));
        let mut p = Parser::new();
        // `def_library` is the file stem, so stem doubles as the library name.
        p.set_sandbox_config(parse_sandbox_config(&cfg_for(
            &format!("{stem}.loft"),
            &stem,
        )));
        p.parse_dir("default", true, true).unwrap();
        std::fs::write(&path, src).unwrap();
        p.parse(path.to_str().unwrap(), false);
        let _ = std::fs::remove_file(&path);
        p
    }

    /// #631 — an entry point that fills its result through a private helper.  The
    /// mutation escapes `build`, so it is the raw write the rule exists to reject;
    /// moving it into `push` is the laundering.
    const LAUNDER_SRC: &str = "struct S { v: vector<integer> }\n\
                               fn push(s: S, x: integer) { s.v += [x]; }\n\
                               fn build(n: integer) -> S { out = S { v: [] }; \
                               for i in 0..n { push(out, i); } return out }\n";

    fn parse_admit_libs(
        designations: &[&str],
        allow_libs: &[&str],
        allow: &[&str],
        src: &str,
    ) -> Parser {
        let cfg = format!(
            "[sandbox]\nmod = [{}]\n[profile.mod]\nallow_libs = [{}]\nallow = [{}]\n",
            quoted(designations),
            quoted(allow_libs),
            quoted(allow),
        );
        parse_admit_cfg(&cfg, src)
    }

    /// Parse `src` with a `mod` profile carrying data-envelope bounds (`0` = omit).
    fn parse_admit_envelope(
        designations: &[&str],
        allow_libs: &[&str],
        data_budget: u64,
        max_input_n: u64,
        src: &str,
    ) -> Parser {
        let cfg = format!(
            "[sandbox]\nmod = [{}]\n[profile.mod]\nallow_libs = [{}]\ndata_budget = {}\n{}",
            quoted(designations),
            quoted(allow_libs),
            data_budget,
            if max_input_n == 0 {
                String::new()
            } else {
                format!("max_input_n = {max_input_n}\n")
            },
        );
        parse_admit_cfg(&cfg, src)
    }

    fn parse_admit(designations: &[&str], allow_caps: &[&str], src: &str) -> Parser {
        parse_admit_libs(designations, &[], allow_caps, src)
    }

    fn viol_symbol(v: &CapViolation) -> u32 {
        match v {
            CapViolation::UngrantedCap { symbol, .. }
            | CapViolation::UntaggedSymbol { symbol, .. }
            | CapViolation::ExternalFfi { symbol, .. } => *symbol,
        }
    }

    /// #631 — `allow_libs` naming the library the sandboxed code ITSELF lives in is
    /// rejected.  That policy makes every function in the module a trusted leaf, so
    /// the raw-write guard stops seeing its helpers and the data envelope stops
    /// counting their loops — both silently.  It is the policy the old diagnostic
    /// suggested first, and it is the whole mechanism behind the laundering below.
    #[test]
    fn allow_listing_the_sandboxed_code_s_own_library_is_rejected() {
        let p = parse_admit_file(
            |file, lib| {
                format!(
                    "[sandbox]\nmod = [\"fn:build\"]\n[profile.mod]\n\
                     allow_libs = [\"code\", \"{lib}\"]\n# file: {file}\n"
                )
            },
            LAUNDER_SRC,
        );
        let errors = p.sandbox_admission_errors();
        assert!(
            errors.iter().any(|e| e.contains("cannot vet itself")),
            "a self-allow-list must be rejected, got: {errors:?}"
        );
    }

    /// #631 — allow-listing a genuine HOST library stays admitted: the rejection
    /// above must key on the library holding sandboxed code, not on `allow_libs`.
    #[test]
    fn allow_listing_a_host_library_is_still_admitted() {
        let p = parse_admit_file(
            |file, _lib| {
                format!("[sandbox]\nmod = [\"{file}\"]\n[profile.mod]\nallow_libs = [\"code\"]\n")
            },
            "fn helper(n: integer) -> integer { n + 1 }\n\
             fn entry(n: integer) -> integer { helper(n) }\n",
        );
        assert!(
            p.sandbox_admission_errors().is_empty(),
            "a host-library allow-list must stay admitted: {:?}",
            p.sandbox_admission_errors()
        );
    }

    /// #631 shape 1 — the laundering itself.  With the module's own file designated,
    /// the escaping mutation is caught wherever it is written: moving `out.v += [i]`
    /// into a one-line helper must NOT change the verdict.
    #[test]
    fn a_same_library_helper_cannot_launder_an_escaping_raw_write() {
        let cfg = |file: &str, _lib: &str| {
            format!("[sandbox]\nmod = [\"{file}\"]\n[profile.mod]\nallow_libs = [\"code\"]\n")
        };
        let laundered = parse_admit_file(cfg, LAUNDER_SRC);
        let inline = parse_admit_file(
            cfg,
            "struct S { v: vector<integer> }\n\
             fn build(n: integer) -> S { out = S { v: [] }; \
             for i in 0..n { out.v += [i]; } return out }\n",
        );
        assert!(
            !inline.sandbox_raw_writes().is_empty(),
            "the inline form is the control — it must reject"
        );
        assert!(
            !laundered.sandbox_raw_writes().is_empty(),
            "the same mutation moved into a same-library helper must reject too"
        );
    }

    /// #631 shape 2 — the data envelope must count a designated file's helpers.  An
    /// entry point that delegates all its work reported `O(1)` while being `O(n)`, so
    /// a declared `data_budget` could be satisfied by a plugin that does not fit it.
    #[test]
    fn complexity_counts_loops_in_a_designated_file_s_helpers() {
        let p = parse_admit_file(
            |file, _lib| {
                format!("[sandbox]\nmod = [\"{file}\"]\n[profile.mod]\nallow_libs = [\"code\"]\n")
            },
            "fn render(doc: vector<integer>) -> integer { t = 0; \
             for i in 0..len(doc) { t += doc[i] ?? 0; } return t }\n\
             fn dispatch(doc: vector<integer>) -> integer { return render(doc) }\n",
        );
        assert!(
            p.sandbox_admission_errors().is_empty(),
            "should admit: {:?}",
            p.sandbox_admission_errors()
        );
        assert_eq!(
            p.sandbox_complexity_degree(),
            1,
            "`dispatch` has no loop of its own but reaches one — that is O(n), not O(1)"
        );
    }

    /// #631 — reaching an own-library helper names the fix that WORKS (designate it)
    /// and warns off the one that silently disables the guard (`allow_libs`).
    #[test]
    fn reaching_an_own_library_helper_suggests_designation_not_allow_libs() {
        let p = parse_admit_file(
            |_file, _lib| {
                "[sandbox]\nmod = [\"fn:build\"]\n[profile.mod]\nallow_libs = [\"code\"]\n"
                    .to_string()
            },
            LAUNDER_SRC,
        );
        let errors = p.sandbox_admission_errors();
        let msg = errors.join("\n");
        assert!(
            msg.contains("\"fn:push\"") && msg.contains("do NOT"),
            "the fix must lead with designation and warn off allow_libs, got: {msg}"
        );
    }

    /// @PLN86 2.3 — the admission convergence: a granted-cap reference admits, an
    /// ungranted-cap reference is rejected naming the group, an untagged symbol is
    /// rejected (deny-by-default).
    #[test]
    fn admission_grants_allowed_caps_and_rejects_ungranted_and_untagged() {
        let src = "fn cap_fs_read() -> integer fs#read;\n#native\n\
                   fn cap_coll_read() -> integer collections#read;\n#native\n\
                   fn cap_untagged() -> integer;\n#native\n\
                   fn ok() -> integer { cap_coll_read() }\n\
                   fn bad() -> integer { cap_fs_read() }\n\
                   fn uses_untagged() -> integer { cap_untagged() }\n";
        let p = parse_admit(
            &["fn:ok", "fn:bad", "fn:uses_untagged"],
            &["collections#read"],
            src,
        );
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "parse errors: {:?}",
            p.diagnostics.lines()
        );
        let v = p.sandbox_admit();
        // bad reaches fs.read which is not granted → named violation.
        assert!(
            v.contains(&CapViolation::UngrantedCap {
                from: p.data.def_nr("n_bad"),
                symbol: p.data.def_nr("n_cap_fs_read"),
                group: "fs#read".to_string(),
            }),
            "expected UngrantedCap(fs.read), got {v:?}"
        );
        // uses_untagged reaches an unclassified symbol → deny-by-default.
        assert!(
            v.contains(&CapViolation::UntaggedSymbol {
                from: p.data.def_nr("n_uses_untagged"),
                symbol: p.data.def_nr("n_cap_untagged"),
            }),
            "expected UntaggedSymbol, got {v:?}"
        );
        // ok reaches only collections.read (granted) → admitted clean.
        let coll = p.data.def_nr("n_cap_coll_read");
        assert!(
            !v.iter().any(|viol| viol_symbol(viol) == coll),
            "granted collections.read must admit clean, got {v:?}"
        );
    }

    /// @PLN86 2.3 / L4 — an indirect call through a fn-ref cannot escape: passing
    /// `cap_fs_read` to a higher-order fn still puts it in the reachable set, so
    /// admission rejects the ungranted `fs.read` group.
    #[test]
    fn admission_rejects_indirect_fnref_call_l4() {
        let src = "fn cap_fs_read(n: integer) -> integer fs#read;\n#native\n\
                   fn apply(f: fn(integer) -> integer, n: integer) -> integer { f(n) }\n\
                   fn sneaky() -> integer { apply(cap_fs_read, 5) }\n";
        let p = parse_admit(&["fn:sneaky", "fn:apply"], &["collections#read"], src);
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "parse errors: {:?}",
            p.diagnostics.lines()
        );
        let v = p.sandbox_admit();
        assert!(
            v.contains(&CapViolation::UngrantedCap {
                from: p.data.def_nr("n_sneaky"),
                symbol: p.data.def_nr("n_cap_fs_read"),
                group: "fs#read".to_string(),
            }),
            "L4 indirect fn-ref to fs.read must be rejected, got {v:?}"
        );
    }

    /// @PLN86 2.2 — the coverage lint lists an untagged public function and omits a
    /// tagged one (the work-list for tagging the stdlib/library surface).
    #[test]
    fn coverage_lint_lists_untagged_public_functions() {
        let src = "pub fn tagged_fn() -> integer math#read;\n#native\n\
                   pub fn untagged_fn() -> integer;\n#native\n";
        let p = parse_admit(&[], &[], src);
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "parse errors: {:?}",
            p.diagnostics.lines()
        );
        let untagged = p.untagged_public_symbols();
        assert!(
            untagged.contains(&p.data.def_nr("n_untagged_fn")),
            "an untagged public fn must be listed"
        );
        assert!(
            !untagged.contains(&p.data.def_nr("n_tagged_fn")),
            "a tagged public fn must NOT be listed"
        );
    }

    /// @PLN86 2.2 — the REAL stdlib fs/env surface gates correctly: `mtime`
    /// (tagged `fs.read`, bodiless so no untagged deps) is rejected naming the
    /// group when fs.read is not granted, while `env_variable` (`env`, granted)
    /// admits — and the tagged fs/env fns have left the coverage lint.
    #[test]
    fn stdlib_fs_env_caps_gate_real_functions() {
        let src = "fn reads_mtime() -> integer { mtime(\"x\") }\n\
                   fn reads_env() -> text { env_variable(\"X\") }\n";
        let p = parse_admit(&["fn:reads_mtime", "fn:reads_env"], &["env#read"], src);
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "parse errors: {:?}",
            p.diagnostics.lines()
        );
        // the fs/env surface is no longer flagged by the coverage lint
        let untagged = p.untagged_public_symbols();
        for n in [
            "n_content",
            "n_write",
            "n_env_variable",
            "n_mtime",
            "n_file",
        ] {
            assert!(
                !untagged.contains(&p.data.def_nr(n)),
                "{n} should be tagged (off the lint)"
            );
        }
        let v = p.sandbox_admit();
        // mtime (fs#read) is not granted → rejected naming the real group.
        assert!(
            v.contains(&CapViolation::UngrantedCap {
                from: p.data.def_nr("n_reads_mtime"),
                symbol: p.data.def_nr("n_mtime"),
                group: "fs#read".to_string(),
            }),
            "mtime must be rejected naming fs.read, got {v:?}"
        );
        // env_variable (env) is granted → admits clean.
        let envv = p.data.def_nr("n_env_variable");
        assert!(
            !v.iter().any(|x| viol_symbol(x) == envv),
            "granted env must admit, got {v:?}"
        );
    }

    /// @PLN86 — library-first admission: a wholesale-allowed library admits its
    /// UNTAGGED functions with no `#cap` tag (the common "include a whole
    /// library" case).  `now()` lives in the `files` stdlib module and is
    /// untagged; under `allow_libs=["files"]` it admits, without it it is an
    /// `UntaggedSymbol`.  Tags stay the fine-grained layer, not a requirement.
    #[test]
    fn wholesale_allowed_library_admits_untagged_functions() {
        // def_library identifies the stdlib module from the source file.
        let p0 = parse_admit(&[], &[], "fn ignore() -> integer { 1 }\n");
        assert_eq!(
            p0.def_library(p0.data.def_nr("n_mtime")).as_deref(),
            Some("files")
        );

        let src = "fn uses_now() -> integer { now() }\n";
        // (a) files allowed wholesale → untagged now() admits, no tag needed.
        let p = parse_admit_libs(&["fn:uses_now"], &["files"], &[], src);
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "parse errors: {:?}",
            p.diagnostics.lines()
        );
        assert!(
            p.sandbox_admit().is_empty(),
            "wholesale files must admit untagged now(), got {:?}",
            p.sandbox_admit()
        );
        // (b) nothing allowed → now() is rejected (deny-by-default).
        let p2 = parse_admit_libs(&["fn:uses_now"], &[], &[], src);
        let now = p2.data.def_nr("n_now");
        assert!(
            p2.sandbox_admit().iter().any(|x| viol_symbol(x) == now),
            "without allow_libs, untagged now() must be flagged, got {:?}",
            p2.sandbox_admit()
        );
    }

    /// @PLN86 2.5 — admission errors are actionable: each class names the
    /// position, the symbol, the rule, and BOTH fixes (wholesale lib + the
    /// fine-grained cap / native_ffi).
    #[test]
    fn admission_errors_name_symbol_rule_and_fix() {
        // UngrantedCap — mtime needs fs#read; only env#read is granted.
        let p = parse_admit(
            &["fn:reads_mtime"],
            &["env#read"],
            "fn reads_mtime() -> integer { mtime(\"x\") }\n",
        );
        let errs = p.sandbox_admission_errors();
        assert_eq!(errs.len(), 1, "{errs:?}");
        let e = &errs[0];
        eprintln!("CAP_DIAG: {e}");
        assert!(e.contains("mtime"), "names the symbol: {e}");
        assert!(e.contains("fs#read"), "names the group: {e}");
        assert!(e.contains("fix:"), "points at the fix: {e}");
        assert!(
            e.contains("`allow`") && e.contains("allow_libs"),
            "offers both fixes: {e}"
        );
        assert!(e.contains(".loft:"), "carries a source position: {e}");

        // UntaggedSymbol — now() in `files`, nothing allowed.
        let p2 = parse_admit_libs(
            &["fn:uses_now"],
            &[],
            &[],
            "fn uses_now() -> integer { now() }\n",
        );
        assert!(
            p2.sandbox_admission_errors()
                .iter()
                .any(|e| e.contains("now")
                    && e.contains("library `files`")
                    && e.contains("allow_libs")),
            "untagged error names the library + allow_libs: {:?}",
            p2.sandbox_admission_errors()
        );
    }

    /// @PLN86 2.5 — an external-FFI rejection names the crate and `native_ffi`.
    #[test]
    fn admission_error_for_external_ffi_names_crate() {
        let src = "fn ext_fn() -> integer; #native \"ext_sym\"\n\
                   fn calls_ext() -> integer { ext_fn() }\n";
        let mut p = parse_admit(&["fn:calls_ext"], &[], src);
        p.data
            .native_symbol_crates
            .insert("ext_sym".to_string(), "extcrate".to_string());
        assert!(
            p.sandbox_admission_errors()
                .iter()
                .any(|e| e.contains("ext_fn")
                    && e.contains("extcrate")
                    && e.contains("native_ffi")),
            "external-FFI error names the crate + native_ffi: {:?}",
            p.sandbox_admission_errors()
        );
    }

    /// @PLN86 3.1 — totality rejects an unbounded `while`, admits a bounded `for`.
    #[test]
    fn totality_rejects_while_admits_for() {
        // An UNBOUNDED `while` (a flag loop — no decreasing variant) is rejected;
        // bounded `while`s are admitted now and covered by `bounded_while_is_admitted`.
        let p = parse_admit_libs(
            &["fn:loops"],
            &["code"],
            &[],
            "fn loops(go: boolean) -> integer { x = 0; while go { x += 1 } x }\n",
        );
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "parse errors: {:?}",
            p.diagnostics.lines()
        );
        let t = p.sandbox_totality();
        assert!(
            t.iter()
                .any(|v| matches!(v, TotalityViolation::UnboundedLoop { .. })),
            "a `while` must be rejected, got {t:?}"
        );
        // the rendered error points at the bounded alternative
        assert!(
            p.sandbox_admission_errors()
                .iter()
                .any(|e| e.contains("while") && e.contains("for ")),
            "{:?}",
            p.sandbox_admission_errors()
        );

        let p2 = parse_admit_libs(
            &["fn:counts"],
            &["code"],
            &[],
            "fn counts() -> integer { s = 0; for i in 0..10 { s += i } s }\n",
        );
        assert!(
            p2.sandbox_totality().is_empty(),
            "a bounded `for` must be total, got {:?}",
            p2.sandbox_totality()
        );
    }

    /// @PLN86 3.2 — totality rejects recursion (self + mutual), admits acyclic.
    #[test]
    fn totality_rejects_recursion_admits_acyclic() {
        // self-recursion `f -> f`
        let p = parse_admit_libs(
            &["fn:rec"],
            &["code"],
            &[],
            "fn rec(n: integer) -> integer { rec(n + 1) }\n",
        );
        assert!(
            p.sandbox_totality()
                .iter()
                .any(|v| matches!(v, TotalityViolation::Recursion { .. })),
            "self-recursion must be rejected, got {:?}",
            p.sandbox_totality()
        );
        assert!(
            p.sandbox_admission_errors()
                .iter()
                .any(|e| e.contains("recursion") && e.contains("rec")),
            "{:?}",
            p.sandbox_admission_errors()
        );

        // mutual recursion `a -> b -> a`
        let p2 = parse_admit_libs(
            &["fn:a", "fn:b"],
            &["code"],
            &[],
            "fn a(n: integer) -> integer { b(n) }\nfn b(n: integer) -> integer { a(n) }\n",
        );
        assert!(
            p2.sandbox_totality()
                .iter()
                .any(|v| matches!(v, TotalityViolation::Recursion { .. })),
            "mutual recursion must be rejected, got {:?}",
            p2.sandbox_totality()
        );

        // acyclic call chain `top -> helper`
        let p3 = parse_admit_libs(
            &["fn:top", "fn:helper"],
            &["code"],
            &[],
            "fn helper(n: integer) -> integer { n + 1 }\n\
             fn top(n: integer) -> integer { helper(n) }\n",
        );
        assert!(
            !p3.sandbox_totality()
                .iter()
                .any(|v| matches!(v, TotalityViolation::Recursion { .. })),
            "an acyclic script must be total, got {:?}",
            p3.sandbox_totality()
        );
    }

    /// @PLN86 3.3 — total-op check: an explicit-abort op (`assert`) is excluded
    /// (it faults the script), while arithmetic stays total — a divide-by-zero is
    /// the interpreter's null sentinel, NOT a rejected op.
    #[test]
    fn totality_excludes_abort_ops_admits_total_arithmetic() {
        // assert faults → excluded as a partial op
        let p = parse_admit_libs(
            &["fn:checks"],
            &["code"],
            &[],
            "fn checks(n: integer) -> integer { assert(n > 0, \"pos\"); n }\n",
        );
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "parse errors: {:?}",
            p.diagnostics.lines()
        );
        assert!(
            p.sandbox_totality()
                .iter()
                .any(|v| matches!(v, TotalityViolation::PartialOp { .. })),
            "assert must be excluded as a partial op, got {:?}",
            p.sandbox_totality()
        );
        assert!(
            p.sandbox_admission_errors()
                .iter()
                .any(|e| e.contains("assert") && e.contains("fix")),
            "{:?}",
            p.sandbox_admission_errors()
        );

        // a divide-by-zero is total on the interpreter → NOT a partial op
        let p2 = parse_admit_libs(
            &["fn:divides"],
            &["code"],
            &[],
            "fn divides(a: integer, b: integer) -> integer { a / b ?? 0 }\n",
        );
        assert!(
            !p2.sandbox_totality()
                .iter()
                .any(|v| matches!(v, TotalityViolation::PartialOp { .. })),
            "arithmetic is total (not excluded), got {:?}",
            p2.sandbox_totality()
        );
    }

    /// @PLN86 — any designated def flags the program as carrying sandboxed code (which
    /// gates the load-time admission walk); a program with no sandboxed defs does not.
    /// The flag is backend-agnostic — it no longer forces interpret-only.
    #[test]
    fn has_sandboxed_defs_on_any_designation() {
        let p = parse_admit_libs(
            &["fn:scripted"],
            &["code"],
            &[],
            "fn scripted() -> integer { 1 }\n",
        );
        assert!(
            p.has_sandboxed_defs(),
            "a designated def must flag the program as sandboxed"
        );
        let p2 = parse_admit_libs(&[], &["code"], &[], "fn plain() -> integer { 1 }\n");
        assert!(!p2.has_sandboxed_defs(), "no sandboxed defs → not flagged");
    }

    /// @PLN86 3.4 — the worst-case complexity degree counts loop nesting, and
    /// composes across the acyclic call graph (a loop calling a looping fn → n²).
    #[test]
    fn complexity_degree_counts_loop_nesting_inter_procedural() {
        // no loop → O(1)
        let p0 = parse_admit_libs(
            &["fn:flat"],
            &["code"],
            &[],
            "fn flat() -> integer { 1 + 2 }\n",
        );
        assert_eq!(p0.sandbox_complexity_degree(), 0, "no loops → O(1)");

        // one loop → O(n)
        let p1 = parse_admit_libs(
            &["fn:one"],
            &["code"],
            &[],
            "fn one() -> integer { s = 0; for i in 0..10 { s += i } s }\n",
        );
        assert_eq!(p1.sandbox_complexity_degree(), 1, "one loop → O(n)");

        // nested loops → O(n^2)
        let p2 = parse_admit_libs(
            &["fn:nest"],
            &["code"],
            &[],
            "fn nest() -> integer { s = 0; for i in 0..10 { for j in 0..10 { s += i } } s }\n",
        );
        assert_eq!(p2.sandbox_complexity_degree(), 2, "nested loops → O(n^2)");
        assert!(
            p2.sandbox_complexity_report().contains("O(n^2)"),
            "{}",
            p2.sandbox_complexity_report()
        );

        // inter-procedural: a loop calling a looping fn → O(n^2)
        let p3 = parse_admit_libs(
            &["fn:outer", "fn:inner"],
            &["code"],
            &[],
            "fn inner() -> integer { s = 0; for j in 0..10 { s += j } s }\n\
             fn outer() -> integer { s = 0; for i in 0..10 { s += inner() } s }\n",
        );
        assert_eq!(
            p3.sandbox_complexity_degree(),
            2,
            "a loop calling a looping fn → O(n^2)"
        );
    }

    /// @PLN86 space budget — the SPACE degree counts only ACCUMULATING appends (a
    /// structure grown across a loop), so a pure-compute loop is O(1) space even
    /// when it is O(n) time, a transient buffer (reset each iteration) is O(1), and
    /// a vector built across nested loops is O(n^2).
    #[test]
    fn space_degree_counts_accumulating_appends_only() {
        // pure compute — O(n) TIME, O(1) SPACE (no heap grows)
        let p0 = parse_admit_libs(
            &["fn:spc_pure"],
            &["code"],
            &[],
            "fn spc_pure() -> integer { s = 0; for i in 0..10 { s += i } s }\n",
        );
        assert_eq!(p0.sandbox_complexity_degree(), 1, "time O(n)");
        assert_eq!(p0.sandbox_space_degree(), 0, "pure compute → O(1) space");

        // accumulate into a vector declared OUTSIDE the loop → O(n) SPACE
        let p1 = parse_admit_libs(
            &["fn:spc_build"],
            &["code"],
            &[],
            "fn spc_build() -> integer { r = []; for i in 0..10 { r += [i] } len(r) }\n",
        );
        assert_eq!(
            p1.sandbox_space_degree(),
            1,
            "accumulating vector → O(n) space"
        );

        // transient buffer — reset every iteration → O(1) SPACE
        let p2 = parse_admit_libs(
            &["fn:spc_trans"],
            &["code"],
            &[],
            "fn spc_trans() -> integer { c = 0; for i in 0..10 { b = []; b += [i]; c += len(b) } c }\n",
        );
        assert_eq!(
            p2.sandbox_space_degree(),
            0,
            "a buffer reset each iteration is transient → O(1) space, got {}",
            p2.sandbox_space_degree()
        );

        // accumulate across NESTED loops → O(n^2) SPACE; report names both axes
        let p3 = parse_admit_libs(
            &["fn:spc_grid"],
            &["code"],
            &[],
            "fn spc_grid() -> integer { r = []; for i in 0..10 { for j in 0..10 { r += [i] } } len(r) }\n",
        );
        assert_eq!(
            p3.sandbox_space_degree(),
            2,
            "nested accumulation → O(n^2) space"
        );
        assert!(
            p3.sandbox_complexity_report().contains("space O(n^2)"),
            "report names the space axis: {}",
            p3.sandbox_complexity_report()
        );
    }

    /// @PLN86 P7.1 (F9) — the space footprint reports `(degree, coeff)`: a per-element
    /// build loop accumulates ONE record's stride per appended element.
    #[test]
    fn space_footprint_reports_degree_and_record_coefficient() {
        // vector<integer>: one i64 element = 8 bytes per append.
        let pi = parse_admit_libs(
            &["fn:b"],
            &["code"],
            &[],
            "fn b() -> integer { r = []; for i in 0..10 { r += [i] } len(r) }\n",
        );
        assert_eq!(
            pi.sandbox_space_footprint(),
            (1, 8),
            "vector<integer> build → (degree 1, coeff 8)"
        );

        // vector<Mob>: coeff = the struct's record stride (one i64 field = 8 bytes).
        let ps = parse_admit_libs(
            &["fn:bs"],
            &["code"],
            &[],
            "struct Mob { hp: integer }\n\
             fn bs() -> integer { acc: vector<Mob> = []; \
              for i in 0..10 { acc += [Mob { hp: i }] } len(acc) }\n",
        );
        assert!(
            ps.diagnostics.level() < crate::diagnostics::Level::Error,
            "parse errors: {:?}",
            ps.diagnostics.lines()
        );
        // vector<Mob> stores DbRef slots (a reference element) → sizeof(DbRef)=12 bytes
        // per element in the backing (the Mob record body is a separate allocation, a
        // documented v1 under-count). Degree 1, coeff 12 (one slot per appended element).
        assert_eq!(
            ps.sandbox_space_footprint(),
            (1, 12),
            "vector<Mob> build → (degree 1, coeff 12 = DbRef backing slot)"
        );
    }

    /// @PLN86 §8 (F11) — the data-envelope reject: `coeff · max_input_n^degree` must
    /// fit `data_budget`, with `max_input_n` provable; over-budget / unprovable reject.
    #[test]
    fn data_budget_rejects_over_envelope_and_admits_under() {
        // degree 1, coeff 8 (vector<integer>); figure = 8 · max_input_n.
        let src = "fn build() -> integer { r = []; for i in 0..10 { r += [i] } len(r) }\n";

        // over budget: 8 · 1000 = 8000 > 4000 → rejected, naming the figure + budget.
        let over = parse_admit_envelope(&["fn:build"], &["code"], 4000, 1000, src);
        let errs = over.sandbox_admission_errors();
        assert!(
            errs.iter()
                .any(|e| e.contains("8000") && e.contains("data_budget 4000")),
            "over-budget must be rejected naming the figure: {errs:?}"
        );

        // under budget: 8 · 1000 = 8000 ≤ 40000 → admitted (no data-envelope error).
        let under = parse_admit_envelope(&["fn:build"], &["code"], 40000, 1000, src);
        assert!(
            !under
                .sandbox_admission_errors()
                .iter()
                .any(|e| e.contains("data_budget") || e.contains("peak heap")),
            "under-budget must admit: {:?}",
            under.sandbox_admission_errors()
        );

        // budget set but max_input_n unset, degree > 0 → unprovable → rejected.
        let unprov = parse_admit_envelope(&["fn:build"], &["code"], 40000, 0, src);
        assert!(
            unprov
                .sandbox_admission_errors()
                .iter()
                .any(|e| e.contains("cannot be bounded") && e.contains("max_input_n")),
            "an unset max_input_n with a growing footprint must be rejected: {:?}",
            unprov.sandbox_admission_errors()
        );

        // no data_budget → report-only (the same growing script admits).
        let nobudget = parse_admit_envelope(&["fn:build"], &["code"], 0, 0, src);
        assert!(
            !nobudget
                .sandbox_admission_errors()
                .iter()
                .any(|e| e.contains("peak heap")),
            "no data_budget → report-only, no reject: {:?}",
            nobudget.sandbox_admission_errors()
        );
    }

    /// @PLN86 §8 (F10) — under an active envelope, an uncapped dynamic string build is
    /// rejected (its bytes aren't a fixed record stride); a `max_string_len` cap admits.
    #[test]
    fn unbounded_string_build_rejected_unless_capped() {
        // grows `s` across a loop; max_input_n set so ONLY the string gate can fire.
        let src = "fn grow() -> text { s = \"\"; for i in 0..10 { s += \"x\" } s }\n";
        let base = "[sandbox]\nmod = [\"fn:grow\"]\n[profile.mod]\n\
                    allow_libs = [\"code\"]\ndata_budget = 1000000\nmax_input_n = 100\n";

        // uncapped (no max_string_len) → UnboundedAlloc rejection.
        let uncapped = parse_admit_cfg(base, src);
        assert!(
            uncapped.diagnostics.level() < crate::diagnostics::Level::Error,
            "parse errors: {:?}",
            uncapped.diagnostics.lines()
        );
        let errs = uncapped.sandbox_admission_errors();
        assert!(
            errs.iter()
                .any(|e| e.contains("string grows unboundedly") && e.contains("max_string_len")),
            "an uncapped string build must be rejected: {errs:?}"
        );

        // capped → admits (the cap bounds it).
        let capped_cfg = format!("{base}max_string_len = 256\n");
        let capped = parse_admit_cfg(&capped_cfg, src);
        assert!(
            !capped
                .sandbox_admission_errors()
                .iter()
                .any(|e| e.contains("string grows unboundedly")),
            "a max_string_len-capped string build must admit: {:?}",
            capped.sandbox_admission_errors()
        );
    }

    /// @PLN86 prevention #3 — TOTAL host capabilities: a `#cap`-tagged function that
    /// can reach an abort op (directly OR via a helper) is flagged as not total; a
    /// capability that validates and returns a clean value is clean.  The host-side
    /// mirror of the script-side 3.3 abort-op exclusion.
    #[test]
    fn capability_totality_flags_abort_reaching_caps() {
        let src = "\
            fn cap_bad(n: integer) -> integer game#read { assert(n > 0, \"pos\"); n }\n\
            fn cap_ok(n: integer) -> integer game#read { if n > 0 { n } else { 0 } }\n\
            fn helper(n: integer) -> integer { assert(n > 0, \"x\"); n }\n\
            fn cap_trans(n: integer) -> integer game#read { helper(n) }\n";
        let p = parse_admit_libs(&[], &["code"], &[], src);
        let v = p.sandbox_capability_totality_violations();
        let flagged: std::collections::HashSet<&str> =
            v.iter().map(|x| p.data.def(x.capability).name()).collect();
        // direct abort + transitive abort (via a non-cap helper) are both caught
        assert!(
            flagged.contains("n_cap_bad"),
            "cap_bad reaches assert: {flagged:?}"
        );
        assert!(
            flagged.contains("n_cap_trans"),
            "cap_trans reaches assert via helper: {flagged:?}"
        );
        // a total capability is NOT flagged; a non-cap helper is never enumerated
        assert!(
            !flagged.contains("n_cap_ok"),
            "cap_ok is total: {flagged:?}"
        );
        assert!(
            !flagged.contains("n_helper"),
            "non-cap helper is not a capability: {flagged:?}"
        );
        // the message is actionable
        let bad = v
            .iter()
            .find(|x| p.data.def(x.capability).name() == "n_cap_bad")
            .unwrap();
        let msg = crate::sandbox::describe_cap_totality_violation(&p.data, bad);
        assert!(
            msg.contains("not total") && msg.contains("cap_bad"),
            "actionable message: {msg}"
        );
    }

    /// @PLN86 prevention #4 — adversarial admission ESCAPE suite.  A battery that
    /// TRIES to break out across every dimension (capability / totality / raw-write);
    /// each must be rejected at load.  Positive controls prove the suite is not
    /// vacuously rejecting everything — a sandbox that admits nothing is useless.
    /// "No unknown holes" is unprovable, so this is how confidence is earned.
    #[test]
    fn admission_escape_suite_rejects_every_breakout() {
        // Parse + admit; ASSERT the probe parsed (a parse error would make the
        // admission check vacuous — a silent pass), then return the errors.
        fn adm(sel: &[&str], libs: &[&str], caps: &[&str], src: &str) -> Vec<String> {
            let p = parse_admit_libs(sel, libs, caps, src);
            assert!(
                p.diagnostics.level() < crate::diagnostics::Level::Error,
                "probe did not parse (vacuous):\n{src}\n{:?}",
                p.diagnostics.lines()
            );
            p.sandbox_admission_errors()
        }
        // A forbidden #cap native, declared in-source and NOT in an allowed library,
        // so the fine-grained capability gate applies to it.
        let secret = "capability danger\nfn secret() -> integer danger#read;\n#native\n";

        // ===== ESCAPES — admission MUST reject (≥1 error). =====
        let escapes: Vec<(&str, Vec<String>)> = vec![
            (
                "cap: direct ungranted call",
                adm(
                    &["fn:evil"],
                    &["code"],
                    &[],
                    &format!("{secret}fn evil() -> integer {{ secret() }}\n"),
                ),
            ),
            (
                "cap: indirect fn-ref (L4)",
                adm(
                    &["fn:evil"],
                    &["code"],
                    &[],
                    &format!("{secret}fn evil() -> integer {{ f = secret; f() }}\n"),
                ),
            ),
            (
                "cap: via sandboxed helper",
                adm(
                    &["fn:evil", "fn:helper"],
                    &["code"],
                    &[],
                    &format!(
                        "{secret}fn helper() -> integer {{ secret() }}\nfn evil() -> integer {{ helper() }}\n"
                    ),
                ),
            ),
            (
                "totality: unbounded while",
                adm(
                    &["fn:evil"],
                    &["code"],
                    &[],
                    "fn evil() { while true { } }\n",
                ),
            ),
            (
                "totality: self-recursion",
                adm(
                    &["fn:evil"],
                    &["code"],
                    &[],
                    "fn evil() -> integer { evil() }\n",
                ),
            ),
            (
                "totality: mutual recursion",
                adm(
                    &["fn:a", "fn:b"],
                    &["code"],
                    &[],
                    "fn a() -> integer { b() }\nfn b() -> integer { a() }\n",
                ),
            ),
            (
                "totality: abort op assert",
                adm(
                    &["fn:evil"],
                    &["code"],
                    &[],
                    "fn evil() { assert(false, \"x\") }\n",
                ),
            ),
            (
                "totality: abort op panic",
                adm(&["fn:evil"], &["code"], &[], "fn evil() { panic(\"x\") }\n"),
            ),
            (
                "totality: while non-constant step",
                adm(
                    &["fn:evil"],
                    &["code"],
                    &[],
                    "fn evil(n: integer) { i = 0; j = 2; while i < n { i = i + j } }\n",
                ),
            ),
            (
                "totality: while conditional step",
                adm(
                    &["fn:evil"],
                    &["code"],
                    &[],
                    "fn evil(n: integer, c: boolean) { i = 0; while i < n { if c { i = i + 1 } } }\n",
                ),
            ),
            (
                "cap: fn-ref in a collection (L4 — caught at fn-ref creation site)",
                adm(
                    &["fn:evil"],
                    &["code"],
                    &[],
                    &format!("{secret}fn evil() -> integer {{ v = [secret]; v[0]() }}\n"),
                ),
            ),
            (
                "cap: fn-ref returned then called (L4 — caught at fn-ref creation site)",
                adm(
                    &["fn:evil", "fn:get"],
                    &["code"],
                    &[],
                    &format!(
                        "{secret}fn get() -> fn() -> integer {{ secret }}\nfn evil() -> integer {{ g = get(); g() }}\n"
                    ),
                ),
            ),
            (
                "cap: fn-ref in a struct field (L4 — caught at fn-ref creation site)",
                adm(
                    &["fn:evil"],
                    &["code", "prog"],
                    &[],
                    &format!(
                        "{secret}struct Holder {{ f: fn() -> integer }}\nfn evil() -> integer {{ h = Holder {{ f: secret }}; h.f() }}\n"
                    ),
                ),
            ),
            (
                "raw-write: field",
                adm(
                    &["fn:evil"],
                    &["code", "prog"],
                    &[],
                    "struct Ent { hp: integer }\nfn evil(e: Ent) -> integer { e.hp = 0; e.hp }\n",
                ),
            ),
            (
                "raw-write: index",
                adm(
                    &["fn:evil"],
                    &["code", "prog"],
                    &[],
                    "fn evil(v: vector<integer>) -> integer { v[0] = 9; v[0] }\n",
                ),
            ),
            (
                // @PLN102 F6 — `r = &v` ALIASES the param `v` (proven: `r[0]=99` mutates the
                // caller's `v[0]`), so laundering a host-vector write through a `&`-bound
                // local must ALSO be rejected — the `Type::Vector => owned` gate must not
                // treat an alias of a parameter as script-owned.
                "raw-write: & alias launders param",
                adm(
                    &["fn:evil"],
                    &["code", "prog"],
                    &[],
                    "fn evil(v: vector<integer>) -> integer { r = &v; r[0] = 9; r[0] }\n",
                ),
            ),
            (
                "raw-write: nested field",
                adm(
                    &["fn:evil"],
                    &["code", "prog"],
                    &[],
                    "struct In { hp: integer }\nstruct Ent { it: In }\nfn evil(e: Ent) -> integer { e.it.hp = 0; e.it.hp }\n",
                ),
            ),
            (
                // @PLN86 D-cap-2 — a lambda BODY reaching an ungranted host cap. The
                // admission walk now DESCENDS into the lambda def (it is marked
                // sandboxed under the enclosing profile), so the reach is checked and
                // rejected — instead of the lambda escaping, OR being wholesale-rejected
                // as an untagged leaf without naming the real reach.
                "cap: lambda body reaches ungranted host (D-cap-2)",
                adm(
                    &["fn:evil"],
                    &["code"],
                    &[],
                    &format!(
                        "{secret}fn evil() -> integer {{ v = [1].map(|y| {{ secret() }}); v[0] }}\n"
                    ),
                ),
            ),
        ];
        for (name, e) in &escapes {
            assert!(!e.is_empty(), "ESCAPE NOT REJECTED — {name}");
        }

        // ===== CONTROLS — admission MUST admit (no errors). =====
        let controls: Vec<(&str, Vec<String>)> = vec![
            (
                "bounded for + arithmetic",
                adm(
                    &["fn:ok"],
                    &["code"],
                    &[],
                    "fn ok() -> integer { s = 0; for i in 0..10 { s += i } s }\n",
                ),
            ),
            (
                "bounded while (constant variant)",
                adm(
                    &["fn:ok"],
                    &["code"],
                    &[],
                    "fn ok() -> integer { i = 0; while i < 10 { i = i + 1 } i }\n",
                ),
            ),
            (
                "struct construction (not a write)",
                adm(
                    &["fn:ok"],
                    &["code", "prog"],
                    &[],
                    "struct Pt { x: integer }\nfn ok() -> Pt { Pt { x: 1 } }\n",
                ),
            ),
            (
                "local variable writes",
                adm(
                    &["fn:ok"],
                    &["code"],
                    &[],
                    "fn ok() -> integer { x = 5; x = x + 1; x }\n",
                ),
            ),
            (
                "granted capability",
                adm(
                    &["fn:ok"],
                    &["code"],
                    &["danger#read"],
                    &format!("{secret}fn ok() -> integer {{ secret() }}\n"),
                ),
            ),
            (
                // @PLN86 D-cap-2 — a lambda whose body touches ONLY script-owned data is
                // now usable in sandboxed code: the admission walk descends into the
                // lambda def and finds no host reach, so it admits (previously EVERY
                // lambda was rejected wholesale as an untagged `__lambda_N` leaf).
                "script-only lambda is usable (D-cap-2)",
                adm(
                    &["fn:ok"],
                    &["code"],
                    &[],
                    "fn ok() -> integer { s = 0; for x in [1,2,3].map(|y| { y * 2 }) { s += x } s }\n",
                ),
            ),
            (
                // @PLN86 D-cap-3 — a write to a SCRIPT-OWNED vector element is Cap-Own. A local
                // vector never aliases host state (every whole-value bind copies), so `v[i] = …`
                // on a local is admitted — the twin of the `raw-write: index` ESCAPE above, whose
                // `v[i] = …` is on a PARAMETER root (which DOES mutate the caller, so it rejects).
                "script-owned vector element write (D-cap-3)",
                adm(
                    &["fn:ok"],
                    &["code"],
                    &[],
                    "fn ok() -> integer { v = [1, 2, 3]; v[0] = 9; v[0] }\n",
                ),
            ),
        ];
        for (name, e) in &controls {
            assert!(e.is_empty(), "CLEAN SCRIPT REJECTED — {name}: {e:?}");
        }
    }

    /// @PLN86 P8.2 (F13) — the RED/GREEN ACCESS corpus: the committed battery over the
    /// capability access model (function call gate, field read/update/append, parameter
    /// `#default` lock, the `files` library split, undeclared links).  Every RED is
    /// paired with a GREEN TWIN — the SAME code with the grant added must ADMIT — so a
    /// rejection is proven to be the rule firing, not a parse error or an unrelated
    /// reject (non-vacuity, the escape-suite discipline).  Plus standalone REDs
    /// (read-only-by-default) and GREENs (construction is unrestricted, reads are free).
    #[test]
    fn access_corpus_red_green() {
        // parse + assert the probe parsed (else admission is vacuous) → errors.
        fn adm(sel: &[&str], libs: &[&str], caps: &[&str], src: &str) -> Vec<String> {
            let p = parse_admit_libs(sel, libs, caps, src);
            assert!(
                p.diagnostics.level() < crate::diagnostics::Level::Error,
                "probe did not parse (vacuous):\n{src}\n{:?}",
                p.diagnostics.lines()
            );
            p.sandbox_admission_errors()
        }
        // A RED (ungranted) rejects naming `token`; its GREEN twin (granted) does NOT —
        // proving the rejection is the access rule, not an incidental failure.
        let twin = |name: &str,
                    libs: &[&str],
                    token: &str,
                    ungranted: &[&str],
                    granted: &[&str],
                    src: &str| {
            let red = adm(&["fn:f"], libs, ungranted, src);
            assert!(
                red.iter().any(|e| e.contains(token)),
                "{name}: RED must be rejected naming `{token}`: {red:?}"
            );
            let green = adm(&["fn:f"], libs, granted, src);
            assert!(
                !green.iter().any(|e| e.contains(token)),
                "{name}: GREEN twin (granted) must admit — non-vacuity: {green:?}"
            );
        };

        // ── call gate: an `fs#update` function under an `fs#read`-only grant ──
        twin(
            "call gate fs#update under fs#read",
            &["code"],
            "fs#update",
            &["fs#read"],
            &["fs#read", "fs#update"],
            "fn host_write() -> integer fs#update;\n#native\n\
             fn f() -> integer { host_write() }\n",
        );
        // ── field READ of a private (`#read`-linked) field ──
        twin(
            "field read (private)",
            &["code"],
            "secret#read",
            &[],
            &["secret#read"],
            "capability secret\nstruct P { hidden: text secret#read }\n\
             fn f(p: P) -> text { p.hidden }\n",
        );
        // ── field UPDATE of an `#update`-linked field ──
        twin(
            "field update",
            &["code"],
            "stats#update",
            &[],
            &["stats#update"],
            "capability stats\nstruct M { hp: integer stats#update }\n\
             fn f(m: M) { m.hp = 0 }\n",
        );
        // ── field APPEND to a `#append`-linked collection field ──
        twin(
            "field append",
            &["code"],
            "bag#append",
            &[],
            &["bag#append"],
            "capability bag\nstruct I { items: vector<integer> bag#append }\n\
             fn f(i: I, x: integer) { i.items += [x] }\n",
        );
        // ── parameter `#default` lock: overriding a pinned argument ──
        twin(
            "param #default lock override",
            &["code"],
            "spawn.count#default",
            &["world#append"],
            &["world#append", "spawn.count#default"],
            "capability world\ncapability spawn.count\n\
             fn spawn(kind: text, count: integer = 1 spawn.count#default) -> integer world#append;\n#native\n\
             fn f() -> integer { spawn(\"g\", 5) }\n",
        );

        // ── @PLN86 D-cap-2: a lambda BODY reaching a host cap — gated by descending
        //    into the lambda def (marked sandboxed under the enclosing profile) ──
        twin(
            "lambda body reaches host cap (D-cap-2)",
            &["code"],
            "danger#read",
            &[],
            &["danger#read"],
            "capability danger\nfn secret() -> integer danger#read;\n#native\n\
             fn f() -> integer { v = [1].map(|y| { secret() }); v[0] }\n",
        );

        // ── undeclared capability link (typo) — RED + corrected-spelling GREEN twin ──
        let typo = adm(
            &["fn:f"],
            &["code"],
            &["helth#update"],
            "capability health\nstruct M { hp: integer helth#update }\nfn f(m: M) { m.hp = 0 }\n",
        );
        assert!(
            typo.iter()
                .any(|e| e.contains("undeclared capability `helth`")),
            "undeclared link must reject naming the group: {typo:?}"
        );
        let fixed = adm(
            &["fn:f"],
            &["code"],
            &["health#update"],
            "capability health\nstruct M { hp: integer health#update }\nfn f(m: M) { m.hp = 0 }\n",
        );
        assert!(
            !fixed.iter().any(|e| e.contains("undeclared")),
            "the corrected spelling must admit (non-vacuity): {fixed:?}"
        );

        // ===== standalone REDs — read-only-by-default (no grant admits them) =====
        // an UNLINKED host field stays read-only even when a capability is granted.
        let unlinked = adm(
            &["fn:f"],
            &["code"],
            &["stats#update"],
            "struct M { hp: integer }\nfn f(m: M) { m.hp = 0 }\n",
        );
        assert!(
            !unlinked.is_empty(),
            "an unlinked host field must stay read-only: {unlinked:?}"
        );
        // append-only: `=` (update) on a field with `#append` but NO `#update` is rejected
        // even with append granted — append-only is exactly this.
        let append_only_update = adm(
            &["fn:f"],
            &["code"],
            &["bag#append"],
            "capability bag\nstruct I { items: vector<integer> bag#append }\n\
             fn f(i: I) { i.items = [1] }\n",
        );
        assert!(
            !append_only_update.is_empty(),
            "update via `=` on an append-only field must be rejected: {append_only_update:?}"
        );

        // ===== standalone GREENs — the model must not over-reject =====
        // CONSTRUCTION is unrestricted (the position-1 decision — no construction gate).
        let construct = adm(
            &["fn:f"],
            &["code", "prog"],
            &[],
            "struct Pt { x: integer }\nfn f() -> Pt { Pt { x: 1 } }\n",
        );
        assert!(
            construct.is_empty(),
            "construction must admit (no construction gate): {construct:?}"
        );
        // reading an UNLINKED field is free (read is default-allow).
        let free_read = adm(
            &["fn:f"],
            &["code"],
            &[],
            "struct P { name: text }\nfn f(p: P) -> text { p.name }\n",
        );
        assert!(
            free_read.is_empty(),
            "reading an unlinked field is free: {free_read:?}"
        );
        // the REAL stdlib `files` split: `mtime` (fs#read) admits under an `fs#read` grant.
        let real_read = adm(
            &["fn:f"],
            &["code"],
            &["fs#read"],
            "fn f() -> integer { mtime(\"x\") }\n",
        );
        assert!(
            real_read.is_empty(),
            "real stdlib mtime (fs#read) must admit under fs#read: {real_read:?}"
        );
    }

    /// @PLN86 2.4 — no-raw-write rejects a field/index assignment to heap data,
    /// while struct construction + local-variable writes stay clean.
    #[test]
    fn no_raw_write_rejects_field_index_admits_construction() {
        // field write → rejected, with an actionable message
        let p = parse_admit_libs(
            &["fn:scripted"],
            &["code", "prog"],
            &[],
            "struct Ent { health: integer }\n\
             fn scripted(e: Ent) -> integer { e.health = 0; e.health }\n",
        );
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "parse errors: {:?}",
            p.diagnostics.lines()
        );
        assert_eq!(
            p.sandbox_raw_writes().len(),
            1,
            "a field write must be flagged, got {:?}",
            p.sandbox_raw_writes()
        );
        assert!(
            p.sandbox_admission_errors()
                .iter()
                .any(|e| e.contains("raw write") && e.contains("fix")),
            "{:?}",
            p.sandbox_admission_errors()
        );

        // index write → rejected
        let p2 = parse_admit_libs(
            &["fn:scripted"],
            &["code", "prog"],
            &[],
            "fn scripted(v: vector<integer>) -> integer { v[0] = 9; v[0] }\n",
        );
        assert!(
            !p2.sandbox_raw_writes().is_empty(),
            "an index write must be flagged, got {:?}",
            p2.sandbox_raw_writes()
        );

        // struct construction + local writes → admitted (no raw write)
        let p3 = parse_admit_libs(
            &["fn:scripted"],
            &["code", "prog"],
            &[],
            "struct Ent { health: integer }\n\
             fn scripted() -> integer { s = 0; for i in 0..3 { s += i } e = Ent { health: s }; e.health }\n",
        );
        assert!(
            p3.sandbox_raw_writes().is_empty(),
            "construction + local writes must be clean, got {:?}",
            p3.sandbox_raw_writes()
        );
    }

    /// @PLN86 2.4 (ownership-aware) — a mod may MUTATE the data it owns (a local of
    /// a script-defined struct — the dogfood `e.alive = false` pattern), but NOT
    /// host data (a parameter — the type also catches aliasing).
    #[test]
    fn no_raw_write_admits_script_owned_struct_mutation() {
        // local script-owned struct mutation → admitted (no violation at all)
        let p = parse_admit_libs(
            &["fn:scripted"],
            &["code"],
            &[],
            "struct Mob { hp: integer }\n\
             fn scripted() -> integer { m = Mob { hp: 5 }; m.hp = 0; m.hp }\n",
        );
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "parse errors: {:?}",
            p.diagnostics.lines()
        );
        assert!(
            p.sandbox_raw_writes().is_empty(),
            "mutating a script-owned struct must be allowed, got {:?}",
            p.sandbox_raw_writes()
        );
        assert!(
            p.sandbox_admission_errors().is_empty(),
            "a self-owned mutation script must fully admit, got {:?}",
            p.sandbox_admission_errors()
        );

        // the SAME write to a PARAMETER (host data) is still rejected
        let p2 = parse_admit_libs(
            &["fn:scripted"],
            &["code"],
            &[],
            "struct Mob { hp: integer }\n\
             fn scripted(m: Mob) -> integer { m.hp = 0; m.hp }\n",
        );
        assert!(
            !p2.sandbox_raw_writes().is_empty(),
            "a write to a parameter (host data) must be rejected, got {:?}",
            p2.sandbox_raw_writes()
        );
    }

    /// @PLN86 P6.4 (F4) — a sandboxed read of a `#read`-linked HOST field needs the
    /// grant; an unlinked field is freely readable (read default-allow).
    #[test]
    fn field_read_gates_private_field_admits_unlinked() {
        let src = "capability secret\n\
                   struct Player { name: text, hidden: text secret#read }\n\
                   fn peek(p: Player) -> text { p.hidden }\n\
                   fn nameof(p: Player) -> text { p.name }\n";
        // ungranted: reading `hidden` (secret#read) is rejected; `name` (unlinked) is fine.
        let p = parse_admit_libs(&["fn:peek", "fn:nameof"], &["code"], &[], src);
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "parse errors: {:?}",
            p.diagnostics.lines()
        );
        let errs = p.sandbox_admission_errors();
        assert!(
            errs.iter().any(|e| e.contains("secret#read")),
            "a private field read must be rejected: {errs:?}"
        );
        // granted: the same read admits clean.
        let p2 = parse_admit_libs(&["fn:peek", "fn:nameof"], &["code"], &["secret#read"], src);
        assert!(
            !p2.sandbox_admission_errors()
                .iter()
                .any(|e| e.contains("secret#read")),
            "granted secret#read must admit the read: {:?}",
            p2.sandbox_admission_errors()
        );
    }

    /// @PLN86 P6.4 (F5) — a write to an `#update`-linked host field admits iff the
    /// token is granted; a field with NO update link stays read-only (the coarse 2.4
    /// reject), generalising the all-or-nothing no-raw-write.
    #[test]
    fn field_update_gates_writable_fields_unlinked_read_only() {
        let src = "capability stats\n\
                   struct Mob { hp: integer stats#update, id: integer }\n\
                   fn hurt(m: Mob) -> integer { m.hp = 0; m.hp }\n";
        // ungranted: writing `hp` (stats#update) is rejected.
        let p = parse_admit_libs(&["fn:hurt"], &["code"], &[], src);
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "parse errors: {:?}",
            p.diagnostics.lines()
        );
        assert!(
            p.sandbox_admission_errors()
                .iter()
                .any(|e| e.contains("stats#update")),
            "an ungranted field write must be rejected: {:?}",
            p.sandbox_admission_errors()
        );
        // granted: the same write admits clean.
        let p2 = parse_admit_libs(&["fn:hurt"], &["code"], &["stats#update"], src);
        assert!(
            p2.sandbox_admission_errors().is_empty(),
            "granted stats#update must admit the write: {:?}",
            p2.sandbox_admission_errors()
        );
        // a field with NO update link stays read-only even when a cap is granted.
        let src2 = "struct Mob { hp: integer }\n\
                    fn hurt(m: Mob) -> integer { m.hp = 0; m.hp }\n";
        let p3 = parse_admit_libs(&["fn:hurt"], &["code"], &["stats#update"], src2);
        assert!(
            !p3.sandbox_admission_errors().is_empty(),
            "an unlinked host field must stay read-only: {:?}",
            p3.sandbox_admission_errors()
        );
    }

    /// @PLN86 P6.4 (F6) — `e.f += x` growing an `#append`-linked collection field admits
    /// iff the token is granted; an append to a field with no append link falls to the
    /// `#update`/coarse path.  This is what makes append-only expressible.
    #[test]
    fn field_append_gates_collection_grow() {
        let src = "capability bag\n\
                   struct Inv { items: vector<integer> bag#append }\n\
                   fn add(i: Inv, x: integer) { i.items += [x] }\n";
        // ungranted: appending to `items` (bag#append) is rejected.
        let p = parse_admit_libs(&["fn:add"], &["code"], &[], src);
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "parse errors: {:?}",
            p.diagnostics.lines()
        );
        assert!(
            p.sandbox_admission_errors()
                .iter()
                .any(|e| e.contains("bag#append")),
            "an ungranted append must be rejected: {:?}",
            p.sandbox_admission_errors()
        );
        // granted: the same append admits clean.
        let p2 = parse_admit_libs(&["fn:add"], &["code"], &["bag#append"], src);
        assert!(
            p2.sandbox_admission_errors().is_empty(),
            "granted bag#append must admit the grow: {:?}",
            p2.sandbox_admission_errors()
        );
    }

    /// @PLN86 §7.2 (F7) — a parameter the host pinned with `…#default` is forced to its
    /// default unless the modder holds the lock: omitting the argument (`spawn("g")`) is
    /// free, but OVERRIDING it (`spawn("g", 5)`) needs `spawn.count#default` granted.
    #[test]
    fn param_default_lock_gates_override() {
        // `world#append` is the call gate (needed to call spawn at all); `count` is then
        // pinned to its default `1` by `spawn.count#default`.
        let src = "capability world\n\
                   capability spawn.count\n\
                   fn spawn(kind: text, count: integer = 1 spawn.count#default) \
                       -> integer world#append;\n#native\n\
                   fn bare() -> integer { spawn(\"goblin\") }\n\
                   fn override_count() -> integer { spawn(\"goblin\", 5) }\n";
        // call-gate granted, lock NOT granted: the bare call admits, the override is rejected.
        let p = parse_admit_libs(
            &["fn:bare", "fn:override_count"],
            &["code"],
            &["world#append"],
            src,
        );
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "parse errors: {:?}",
            p.diagnostics.lines()
        );
        let errs = p.sandbox_admission_errors();
        assert!(
            errs.iter().any(|e| e.contains("spawn.count#default")),
            "overriding a pinned parameter must be rejected: {errs:?}"
        );
        // the bare call (uses the default) must NOT be flagged — only the override is.
        let bare = p.data.def_nr("n_bare");
        assert!(
            !p.sandbox_param_overrides.contains_key(&bare),
            "a defaulted parameter is not an override: {:?}",
            p.sandbox_param_overrides.get(&bare)
        );
        // lock granted: the override admits clean.
        let p2 = parse_admit_libs(
            &["fn:bare", "fn:override_count"],
            &["code"],
            &["world#append", "spawn.count#default"],
            src,
        );
        assert!(
            !p2.sandbox_admission_errors()
                .iter()
                .any(|e| e.contains("spawn.count#default")),
            "granted spawn.count#default must admit the override: {:?}",
            p2.sandbox_admission_errors()
        );
    }

    /// @PLN86 P6.8 (F8) — a `group#right` link in the main program whose group was never
    /// declared as a `capability` is a clean LOAD error (today it would silently deny, the
    /// group never matching a grant); a correctly-declared link is not flagged.
    #[test]
    fn undeclared_capability_link_is_a_load_error() {
        // a field link to `helth` — a typo for the `health` capability actually declared.
        let typo = "capability health\n\
                    struct Mob { hp: integer helth#update }\n\
                    fn hurt(m: Mob) { m.hp = 0 }\n";
        let p = parse_admit_libs(&["fn:hurt"], &["code"], &["helth#update"], typo);
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "parse errors: {:?}",
            p.diagnostics.lines()
        );
        let errs = p.sandbox_admission_errors();
        assert!(
            errs.iter()
                .any(|e| e.contains("undeclared capability `helth`")),
            "an undeclared capability group must be a load error: {errs:?}"
        );
        // the correctly-spelled link resolves against its declaration — not flagged.
        let ok = "capability health\n\
                  struct Mob { hp: integer health#update }\n\
                  fn hurt(m: Mob) { m.hp = 0 }\n";
        let p2 = parse_admit_libs(&["fn:hurt"], &["code"], &["health#update"], ok);
        assert!(
            !p2.sandbox_admission_errors()
                .iter()
                .any(|e| e.contains("undeclared capability")),
            "a declared link must not be flagged: {:?}",
            p2.sandbox_admission_errors()
        );
    }

    /// @PLN86 L4 — a fn-ref to a forbidden capability hidden where `referenced_defs`
    /// can't see it (a COLLECTION element — neither a call nor an assignment) is
    /// still capability-checked.  `mtime` (tagged `fs.read`) as a VALUE inside
    /// `[mtime]` must be rejected exactly as a direct `mtime(..)` call would —
    /// because it is recorded at the fn-ref CREATION site, not by an IR walk that
    /// only catches call-args + assignments.
    #[test]
    fn l4_fn_ref_in_collection_cannot_escape_capability() {
        let p = parse_admit_libs(
            &["fn:scripted"],
            &["code"],
            &["env"], // fs.read NOT granted
            "fn scripted() -> integer { handlers = [mtime]; len(handlers) }\n",
        );
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "parse errors: {:?}",
            p.diagnostics.lines()
        );
        let scripted = p.data.def_nr("n_scripted");
        let mtime = p.data.def_nr("n_mtime");
        // recorded at creation — the L4 mechanism
        assert!(
            p.sandbox_fn_refs
                .get(&scripted)
                .is_some_and(|s| s.contains(&mtime)),
            "the fn-ref to mtime must be recorded for L4, got {:?}",
            p.sandbox_fn_refs.get(&scripted)
        );
        // and admission rejects it naming the capability — no escape
        let v = p.sandbox_admit();
        assert!(
            v.iter().any(|x| viol_symbol(x) == mtime),
            "a collection-hidden fn-ref to mtime must be capability-checked (L4), got {v:?}"
        );
    }

    /// @PLN86 3.1 — does the sandboxed program have an unbounded-`while` violation?
    fn has_unbounded_while(p: &Parser) -> bool {
        p.sandbox_totality()
            .iter()
            .any(|v| matches!(v, TotalityViolation::UnboundedLoop { .. }))
    }

    /// @PLN86 3.1 — a `while` carrying a compiler-checked DECREASING VARIANT is
    /// admitted: an int counter against a stable bound, stepped by a constant every
    /// iteration (counting up, counting down, or guarded by `&& i < N`).
    #[test]
    fn bounded_while_is_admitted() {
        for (label, src) in [
            (
                "count up to a param bound",
                "fn scripted(n: integer) -> integer { i = 0; s = 0; \
                 while i < n { s = s + i; i = i + 1; } s }\n",
            ),
            (
                "count down to zero",
                "fn scripted() -> integer { j = 10; s = 0; \
                 while j > 0 { s = s + j; j = j - 1; } s }\n",
            ),
            (
                "guard counter in a conjunction",
                "fn scripted(flag: boolean) -> integer { g = 0; s = 0; \
                 while flag && (g < 2000) { s = s + g; g = g + 1; } s }\n",
            ),
        ] {
            let p = parse_admit_libs(&["fn:scripted"], &["code"], &[], src);
            assert!(
                p.diagnostics.level() < crate::diagnostics::Level::Error,
                "{label}: parse errors {:?}",
                p.diagnostics.lines()
            );
            assert!(
                !has_unbounded_while(&p),
                "{label}: a bounded while must be admitted, got {:?}",
                p.sandbox_totality()
            );
        }
    }

    /// @PLN86 3.1 — SOUNDNESS: a `while` whose termination cannot be PROVEN is
    /// rejected.  Each of these is either a genuine non-terminator or one the
    /// conservative recognizer must refuse (unsoundness here would let a script
    /// hang the host).
    #[test]
    fn unprovable_while_is_rejected() {
        for (label, src) in [
            (
                "flag loop (no variant)",
                "fn scripted(running: boolean) -> integer { s = 0; \
                 while running { s = s + 1; } s }\n",
            ),
            (
                "no step (counter never moves)",
                "fn scripted(n: integer) -> integer { i = 0; s = 0; \
                 while i < n { s = s + i; } s }\n",
            ),
            (
                "conditional step (may not run every iteration)",
                "fn scripted(n: integer) -> integer { i = 0; \
                 while i < n { if i > 5 { i = i + 1; } } i }\n",
            ),
            (
                "cancelling steps (net non-monotonic)",
                "fn scripted(n: integer) -> integer { i = 0; \
                 while i < n { i = i + 1; i = i - 1; } i }\n",
            ),
            (
                "non-constant step (cannot prove > 0)",
                "fn scripted(n: integer, k: integer) -> integer { i = 0; \
                 while i < n { i = i + k; } i }\n",
            ),
            (
                "moving bound (races away)",
                "fn scripted(n: integer) -> integer { i = 0; m = n; \
                 while i < m { i = i + 1; m = m + 1; } i }\n",
            ),
        ] {
            let p = parse_admit_libs(&["fn:scripted"], &["code"], &[], src);
            assert!(
                p.diagnostics.level() < crate::diagnostics::Level::Error,
                "{label}: parse errors {:?}",
                p.diagnostics.lines()
            );
            assert!(
                has_unbounded_while(&p),
                "{label}: an unprovable while must be rejected"
            );
        }
    }
}
