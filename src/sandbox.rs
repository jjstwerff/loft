// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN86 step 1.1 — the sandbox **policy model**: capability-group allow-lists
//! plus the coarse bans, and the `loft.toml` `[sandbox]` / `[profile.*]` parser
//! that builds them.
//!
//! Nothing here executes — it is policy DATA the compile-time admission walk
//! reads (`SandboxProfile::allows` is the central capability check used by the
//! group-membership admission, @PLN86 step 2.3).  Deny-by-default: a capability
//! group is permitted only when an `allow_caps` entry is a dotted-segment prefix
//! of it.

use crate::data::{Data, DefType, Position, Type, Value};
use std::collections::{HashMap, HashSet};

/// A named admission profile: which capability **groups** a sandboxed script may
/// reach, plus the always-on coarse bans (never native/rustc, never a `[native]`
/// cdylib — both RCE surfaces).  Defaults are the safe ones: deny every
/// capability, interpret-only, no FFI.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SandboxProfile {
    /// Libraries allowed **wholesale** (e.g. `"text"`, `"crypto"`).  Every symbol
    /// from an allow-listed library admits with NO `#cap` tag — a complete,
    /// host-vetted library is included as a unit.  Tags (`allow_caps`) are only
    /// for carving a library in half ("include `files`, but reads only").
    pub allow_libs: Vec<String>,
    /// Allowed capability-group prefixes (e.g. `"game"`, `"collections.read"`).
    /// The fine-grained layer, used when a library is NOT allowed wholesale.
    pub allow_caps: Vec<String>,
    /// May this profile reach a vetted `[native]` cdylib bridge?  Default false.
    /// (Interpret-only is NOT a per-profile choice — it is unconditional for ALL
    /// sandboxed code, enforced by `Parser::sandbox_forces_interpret`: generating
    /// + compiling Rust on the host is RCE by construction.)
    pub native_ffi: bool,
}

impl SandboxProfile {
    /// True iff library `lib` is allow-listed wholesale (exact name match).  A
    /// match admits every symbol from that library — including its untagged
    /// functions and its native bridges — because the host vetted it as a unit.
    #[must_use]
    pub fn allows_lib(&self, lib: &str) -> bool {
        !lib.is_empty() && self.allow_libs.iter().any(|l| l == lib)
    }

    /// True iff a symbol tagged with capability `group` is allowed by this
    /// profile.  **Deny-by-default**: an empty / untagged group is never allowed,
    /// and a group is allowed only when some `allow_caps` entry is a
    /// dotted-segment prefix of it — so `"game"` allows `"game.read"` but NOT
    /// `"gameover"`, and `"game.read"` is exact (does not match `"game.write"`).
    #[must_use]
    pub fn allows(&self, group: &str) -> bool {
        if group.is_empty() {
            return false;
        }
        self.allow_caps.iter().any(|a| cap_prefix_match(a, group))
    }
}

/// Dotted-segment prefix match: `allow` matches `group` iff `group == allow` or
/// `group` continues `allow` at a `.` boundary.  So `"game"` ⊇ `"game.read"` but
/// not `"gameover"`; `"game.read"` is exact.
#[must_use]
pub fn cap_prefix_match(allow: &str, group: &str) -> bool {
    if allow.is_empty() {
        return false;
    }
    group == allow
        || (group.len() > allow.len()
            && group.starts_with(allow)
            && group.as_bytes()[allow.len()] == b'.')
}

/// The whole sandbox configuration from a program's `loft.toml`: the named
/// profiles plus the designations mapping source selectors (file globs /
/// `fn:<name>`) to a profile.  The host owns this — a script cannot designate
/// itself (@PLN86 §1).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SandboxConfig {
    /// profile name -> profile
    pub profiles: HashMap<String, SandboxProfile>,
    /// (selector, profile-name) — which sources compile under which profile.
    /// A selector is a file glob (`"mods/**/*.loft"`) or `"fn:<name>"`.
    pub designations: Vec<(String, String)>,
}

impl SandboxConfig {
    /// True when this config designates any sandboxed source at all.
    #[must_use]
    pub fn is_active(&self) -> bool {
        !self.designations.is_empty()
    }

    /// The profile a selector maps to, if designated.
    #[must_use]
    pub fn profile_for_selector(&self, selector: &str) -> Option<&SandboxProfile> {
        self.designations
            .iter()
            .find(|(s, _)| s == selector)
            .and_then(|(_, p)| self.profiles.get(p))
    }

    /// The profile NAME a `fn:<name>` designation maps to, if the function is
    /// designated sandboxed.  The compiler resolves each parsed function against
    /// this (@PLN86 step 1.2); file-glob selectors are a later addition.
    #[must_use]
    pub fn fn_designation(&self, fn_name: &str) -> Option<&str> {
        let selector = format!("fn:{fn_name}");
        self.designations
            .iter()
            .find(|(s, _)| *s == selector)
            .map(|(_, p)| p.as_str())
    }
}

/// Parse the `[sandbox]` + `[profile.*]` sections of a `loft.toml` body into a
/// [`SandboxConfig`].  Mirrors `manifest::read_manifest`'s hand-rolled line
/// reader (loft.toml is not full TOML); unknown sections/keys are ignored so the
/// sandbox config coexists with the package manifest in one file.
///
/// - `[sandbox]` — `profile-name = ["selector", …]` adds a designation per
///   selector.
/// - `[profile.<name>]` — `allow_libs = ["l", …]`, `allow_caps = ["g", …]`,
///   `native_ffi = false`.  (Interpret-only is unconditional, not a key here.)
#[must_use]
pub fn parse_sandbox_config(content: &str) -> SandboxConfig {
    let mut config = SandboxConfig::default();
    let mut section = String::new();
    for line in content.lines() {
        let line = strip_comment(line).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = name.trim().to_string();
            // A named profile section pre-registers the profile so an empty
            // `[profile.x]` (deny-all) still exists to be designated.
            if let Some(p) = section.strip_prefix("profile.") {
                config.profiles.entry(p.to_string()).or_default();
            }
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if section == "sandbox" {
            // `profile = ["selector", …]` — each selector designates that profile.
            for selector in parse_str_list(value) {
                config.designations.push((selector, key.to_string()));
            }
        } else if let Some(name) = section.strip_prefix("profile.") {
            let profile = config.profiles.entry(name.to_string()).or_default();
            match key {
                "allow_libs" => profile.allow_libs = parse_str_list(value),
                "allow_caps" => profile.allow_caps = parse_str_list(value),
                // `backend` is accepted-and-ignored: interpret-only is unconditional
                // for sandboxed code, not a per-profile setting (see SandboxProfile).
                "native_ffi" => profile.native_ffi = value.trim() == "true",
                _ => {}
            }
        }
    }
    config
}

/// Drop a trailing `# comment` (loft.toml line comments).  Naive — fine for the
/// manifest's simple values, which never contain a literal `#`.
fn strip_comment(line: &str) -> &str {
    line.split_once('#').map_or(line, |(head, _)| head)
}

/// Parse a value that is either a TOML array `["a", "b"]` or a bare/quoted
/// scalar into a trimmed, unquoted, non-empty list.
fn parse_str_list(value: &str) -> Vec<String> {
    let inner = value
        .trim()
        .strip_prefix('[')
        .and_then(|v| v.strip_suffix(']'))
        .unwrap_or(value);
    inner
        .split(',')
        .map(|s| s.trim().trim_matches('"').trim())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// @PLN86 step 1.3 — the **sandbox-reachable set**: the transitive closure of
/// references (calls + fn-ref literals) from every sandboxed def, **descending
/// only into other sandboxed defs**.  A trusted / allow-listed symbol is a
/// *leaf* — recorded (the admission walk capability-checks it) but not descended,
/// because its body is the host's contract (§4 / L-host).  `sandboxed` maps each
/// sandboxed def_nr to its profile (the parser's `def_sandbox`).
///
/// Returns every reachable def_nr: the sandboxed entries, the sandboxed defs
/// descended into, and the trusted leaves they reference.  Step 2.3 capability-
/// checks the non-sandboxed members.
#[must_use]
pub fn reachable_set(
    data: &Data,
    sandboxed: &HashMap<u32, String>,
    fn_refs: &HashMap<u32, HashSet<u32>>,
) -> HashSet<u32> {
    let mut reachable = HashSet::new();
    let mut worklist: Vec<u32> = sandboxed.keys().copied().collect();
    let mut descended = HashSet::new();
    while let Some(d) = worklist.pop() {
        if !descended.insert(d) {
            continue;
        }
        reachable.insert(d);
        let mut refs = HashSet::new();
        referenced_defs(data, d, &mut refs);
        add_recorded_fn_refs(fn_refs, d, &mut refs);
        for r in refs {
            reachable.insert(r);
            // Descend ONLY into sandboxed defs; trusted symbols are leaves.
            if sandboxed.contains_key(&r) {
                worklist.push(r);
            }
        }
    }
    reachable
}

/// @PLN86 L4 — fold `f`'s parse-recorded fn-ref targets into its reference set.
/// These are the functions `f` named as a VALUE (caught at the creation site), so
/// an indirect call through a fn-ref returned / stored in a field / put in a
/// collection cannot escape the admission walk.
fn add_recorded_fn_refs(fn_refs: &HashMap<u32, HashSet<u32>>, f: u32, out: &mut HashSet<u32>) {
    if let Some(targets) = fn_refs.get(&f) {
        out.extend(targets.iter().copied());
    }
}

/// Collect every def_nr referenced in `fn_dnr`'s body — both direct calls AND
/// fn-ref literals.  L4 hazard (verified in the IR): a non-capturing fn-ref is
/// emitted as a bare `Value::Int(def_nr)`, indistinguishable from an integer
/// literal except by **type context** — so an `Int`/`Long` is a reference only
/// where a `Function` type is expected: a `fn(...)`-typed call argument or an
/// assignment to a `fn(...)`-typed variable.  Missing this lets
/// `f = read_file; f(x)` escape admission.
///
/// RESIDUAL (tracked, closes before step 2.3 capability admission relies on this
/// set): a fn-ref flowing through a `Function`-typed RETURN value, struct field,
/// or collection element is not yet detected.  Until then the design's host
/// contract (§4 / L-host: trusted APIs do not hand untrusted code arbitrary
/// fn-refs) covers the trusted-boundary case; the sandboxed-internal vectors are
/// the follow-up.
fn referenced_defs(data: &Data, fn_dnr: u32, out: &mut HashSet<u32>) {
    let def = data.def(fn_dnr);
    let func = &def.variables;
    def.code.walk(&mut |v| match v {
        Value::Call(d, args) => {
            out.insert(*d);
            // A `fn(...)`-typed parameter receives its argument as a fn-ref.
            let callee = &data.def(*d).variables;
            let params = callee.arguments();
            for (i, arg) in args.iter().enumerate() {
                if let Some(slot) = params.get(i)
                    && matches!(callee.tp(*slot), Type::Function(..))
                    && let Some(r) = fnref_target(arg)
                {
                    out.insert(r);
                }
            }
        }
        Value::Set(slot, value) => {
            // Assigning a fn-ref into a `fn(...)`-typed local.
            if matches!(func.tp(*slot), Type::Function(..))
                && let Some(r) = fnref_target(value)
            {
                out.insert(r);
            }
        }
        Value::FnRef(d, _, _) if *d >= 0 => {
            out.insert(*d as u32);
        }
        _ => {}
    });
}

/// Extract the referenced function def_nr from a value used in a `Function`-typed
/// position — a fn-ref is carried as `FnRef`, a bare `Int`, or a `Long` def_nr.
fn fnref_target(v: &Value) -> Option<u32> {
    match v.unspan() {
        Value::Int(d) | Value::FnRef(d, _, _) if *d >= 0 => Some(*d as u32),
        Value::Long(d) if *d >= 0 => Some(*d as u32),
        _ => None,
    }
}

/// @PLN86 step 1.4 — the external `[native]` cdylib bridges a sandboxed program
/// reaches: defs in `reachable` whose `#native` symbol is owned by an external
/// native package (`native_symbol_crates`), NOT a built-in interpreter
/// primitive.  Built-in stdlib functions use inline `#rust` bodies — their
/// `native()` is empty — and built-in `#native` ops live in `default/`, not under
/// a `native_packages` dir, so neither enters the crate map; only an external
/// cdylib bridge does.  Generating Rust + dlopen'ing a cdylib is RCE by
/// construction, so unless a profile sets `native_ffi = true` a non-empty result
/// is an admission rejection (consumed by the §4 walk).  Sorted for stable
/// diagnostics.
#[must_use]
pub fn reachable_ffi_bridges(data: &Data, reachable: &HashSet<u32>) -> Vec<u32> {
    let mut bridges: Vec<u32> = reachable
        .iter()
        .copied()
        .filter(|d| {
            let sym = data.def(*d).native();
            !sym.is_empty() && data.native_symbol_crates.contains_key(sym)
        })
        .collect();
    bridges.sort_unstable();
    bridges
}

/// @PLN86 step 2.3 — a capability-admission violation.  `from` is the sandboxed
/// def that makes the reference, `symbol` the trusted def it reaches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapViolation {
    /// `symbol`'s `#cap` group is not a prefix of any `allow_caps` entry.
    UngrantedCap {
        from: u32,
        symbol: u32,
        group: String,
    },
    /// `symbol` carries no `#cap` group — deny-by-default: an unclassified symbol
    /// cannot be proven safe.  Until 2.2 tags the surface this fires for every
    /// untagged stdlib symbol, which is correct (nothing is admitted yet).
    UntaggedSymbol { from: u32, symbol: u32 },
    /// `symbol` is an external native cdylib bridge and the profile does not set
    /// `native_ffi` (dlopen is RCE by construction).
    ExternalFfi {
        from: u32,
        symbol: u32,
        crate_name: String,
    },
}

/// @PLN86 — the library a def belongs to, the wholesale-admission key for
/// `allow_libs`.  Derived from the def's source file: the basename minus a
/// leading `NN_` number prefix and the extension.  Stdlib modules →
/// `code`/`files`/`text`/`json`; a single-file external lib → its package name
/// (`crypto`).  `None` for a synthetic / empty source.
#[must_use]
pub fn def_library(data: &Data, def_nr: u32) -> Option<String> {
    let def = data.def(def_nr);
    let file = def.position().file.as_str();
    let base = std::path::Path::new(file).file_stem()?.to_str()?;
    // Strip a leading `\d+_` (stdlib module naming: `01_code` -> `code`).
    let name = match base.find('_') {
        Some(i) if i > 0 && base.as_bytes()[..i].iter().all(u8::is_ascii_digit) => &base[i + 1..],
        _ => base,
    };
    (!name.is_empty()).then(|| name.to_string())
}

/// @PLN86 step 2.3 — the capability-admission walk, **library-first**.  Each
/// sandboxed def is admitted under ITS OWN profile; every trusted symbol it
/// references (a non-sandboxed leaf, §4) is admitted if **either**:
///   1. its **library** is allow-listed wholesale (`allow_libs`) — a complete,
///      host-vetted library is included as a unit, NO `#cap` tag needed; or
///   2. its `#cap` **group** is allow-listed (`allow_caps`) — the fine-grained
///      layer, for carving a library in half ("include `files`, reads only").
///
/// A symbol in no allowed library and with no allowed cap is the violation.  A
/// reference to another sandboxed def is fine (admitted under its own profile),
/// so the union of per-def checks covers the whole reachable closure.  This is
/// L4-complete: `referenced_defs` resolves indirect fn-refs (`f = read_file`) to
/// their target, so an indirect call cannot escape.
///
/// Returns every violation, deterministically ordered, for diagnostics (2.5).
#[must_use]
pub fn admit_capabilities(
    data: &Data,
    config: &SandboxConfig,
    sandboxed: &HashMap<u32, String>,
    fn_refs: &HashMap<u32, HashSet<u32>>,
) -> Vec<CapViolation> {
    let mut violations = Vec::new();
    let mut entries: Vec<u32> = sandboxed.keys().copied().collect();
    entries.sort_unstable();
    for from in entries {
        let profile = config.profiles.get(&sandboxed[&from]);
        let mut refs = HashSet::new();
        referenced_defs(data, from, &mut refs);
        add_recorded_fn_refs(fn_refs, from, &mut refs);
        let mut refs: Vec<u32> = refs.into_iter().collect();
        refs.sort_unstable();
        for symbol in refs {
            // Another sandboxed def: admitted under its own profile, not a leaf.
            if sandboxed.contains_key(&symbol) {
                continue;
            }
            // LIBRARY-FIRST: a wholesale-allowed library admits every symbol in
            // it — untagged functions AND native bridges — the host vetted it.
            if let Some(lib) = def_library(data, symbol)
                && profile.is_some_and(|p| p.allows_lib(&lib))
            {
                continue;
            }
            let def = data.def(symbol);
            // Not in an allowed library — apply the fine-grained gates.
            // 1.4: an external FFI bridge is gated by `native_ffi`, not caps.
            let native = def.native();
            if !native.is_empty()
                && let Some(crate_name) = data.native_symbol_crates.get(native)
            {
                if !profile.is_some_and(|p| p.native_ffi) {
                    violations.push(CapViolation::ExternalFfi {
                        from,
                        symbol,
                        crate_name: crate_name.clone(),
                    });
                }
                continue;
            }
            // 2.3: capability check (deny-by-default).
            let cap = def.cap();
            if cap.is_empty() {
                violations.push(CapViolation::UntaggedSymbol { from, symbol });
            } else if !profile.is_some_and(|p| p.allows(cap)) {
                violations.push(CapViolation::UngrantedCap {
                    from,
                    symbol,
                    group: cap.to_string(),
                });
            }
        }
    }
    violations
}

/// @PLN86 step 2.2 — the capability-coverage lint (L3-cap): every public function
/// the host exposes must carry a `#cap "group"`, or admission can only deny-by-
/// default reject it.  Returns the public, non-synthetic function defs that lack
/// a group, sorted — the work-list for tagging the stdlib / library surface.  The
/// END state is an empty result; until the surface is tagged this lists it.
#[must_use]
pub fn untagged_public_symbols(data: &Data) -> Vec<u32> {
    let mut out: Vec<u32> = (0..data.definitions())
        .filter(|&d| {
            let def = data.def(d);
            def.pub_visible
                && matches!(def.def_type(), DefType::Function)
                && def.synthetic().is_none()
                && def.cap().is_empty()
        })
        .collect();
    out.sort_unstable();
    out
}

impl CapViolation {
    /// The sandboxed def that makes the offending reference.
    #[must_use]
    pub fn from(&self) -> u32 {
        match self {
            Self::UngrantedCap { from, .. }
            | Self::UntaggedSymbol { from, .. }
            | Self::ExternalFfi { from, .. } => *from,
        }
    }

    /// The trusted def it reaches.
    #[must_use]
    pub fn symbol(&self) -> u32 {
        match self {
            Self::UngrantedCap { symbol, .. }
            | Self::UntaggedSymbol { symbol, .. }
            | Self::ExternalFfi { symbol, .. } => *symbol,
        }
    }
}

/// A def's human-readable name for diagnostics — the stored `n_<fn>` global
/// prefix stripped; method names (`t_<len><Type>_<m>`) are left as-is.
fn display_name(data: &Data, def_nr: u32) -> String {
    let n = data.def(def_nr).name();
    n.strip_prefix("n_").unwrap_or(n).to_string()
}

/// @PLN86 step 2.5 — the source position of the FIRST reference to `symbol` in
/// `from`'s body, for pointing the diagnostic at the exact construct.  Tracks the
/// nearest enclosing `Span` as it descends.  `None` (caller falls back to the
/// function's own position) when the reference is a bare `Int` fn-ref with no
/// span — rare, and the function position is still actionable.
#[must_use]
pub fn reference_position(data: &Data, from: u32, symbol: u32) -> Option<Position> {
    fn scan(v: &Value, target: u32, cur: Option<&Position>, found: &mut Option<Position>) {
        if found.is_some() {
            return;
        }
        if let Value::Span(b) = v {
            scan(&b.1, target, Some(&b.0), found);
            return;
        }
        match v {
            Value::Call(d, _) if *d == target => {
                *found = cur.cloned();
                return;
            }
            Value::FnRef(d, _, _) if *d >= 0 && *d as u32 == target => {
                *found = cur.cloned();
                return;
            }
            _ => {}
        }
        v.for_each_child(&mut |c| scan(c, target, cur, found));
    }
    let mut found = None;
    scan(&data.def(from).code, symbol, None, &mut found);
    found
}

/// @PLN86 step 2.5 — turn a `CapViolation` into a correct, specific, actionable
/// admission error (§6): it names the construct's position, the symbol, the rule
/// it breaks, the profile's allowed set, AND the fix — *before* the script runs.
/// Under library-first every fix offers BOTH the wholesale option (allow the
/// library) and the fine-grained one (the cap / `native_ffi`).
#[must_use]
pub fn describe_violation(
    data: &Data,
    config: &SandboxConfig,
    sandboxed: &HashMap<u32, String>,
    v: &CapViolation,
) -> String {
    let from = v.from();
    let symbol = v.symbol();
    let from_name = display_name(data, from);
    let sym_name = display_name(data, symbol);
    let lib = def_library(data, symbol);
    let libhint = lib.as_deref().unwrap_or("<unknown>");
    let pos =
        reference_position(data, from, symbol).unwrap_or_else(|| data.def(from).position().clone());
    let profile = sandboxed.get(&from).and_then(|n| config.profiles.get(n));
    let allowed_libs = profile.map(|p| p.allow_libs.join(", ")).unwrap_or_default();
    let allowed_caps = profile.map(|p| p.allow_caps.join(", ")).unwrap_or_default();
    match v {
        CapViolation::UngrantedCap { group, .. } => format!(
            "{pos}: sandboxed `{from_name}` reaches `{sym_name}`, which needs capability \
             `{group}` — not granted.\n  fix: add `{group}` to `allow_caps`, or add its \
             library `{libhint}` to `allow_libs`.\n  allowed capabilities: [{allowed_caps}]"
        ),
        CapViolation::UntaggedSymbol { .. } => format!(
            "{pos}: sandboxed `{from_name}` reaches `{sym_name}` (library `{libhint}`), which \
             is neither an allowed library nor a granted capability.\n  fix: add `{libhint}` \
             to `allow_libs` to include the whole library, or tag `{sym_name}` with a `#cap` \
             group and grant it.\n  allowed libraries: [{allowed_libs}]; allowed \
             capabilities: [{allowed_caps}]"
        ),
        CapViolation::ExternalFfi { crate_name, .. } => format!(
            "{pos}: sandboxed `{from_name}` reaches `{sym_name}`, a native FFI bridge from \
             crate `{crate_name}` — native code is not permitted (`native_ffi = false`).\n  \
             fix: add its library `{libhint}` to `allow_libs` (you vet its native code), or \
             set `native_ffi = true` for this profile."
        ),
    }
}

/// @PLN86 step P3 — a totality violation: a sandboxed script that cannot be
/// proven to terminate.  An admitted script always runs to completion, so these
/// are rejected at LOAD (never a runtime hang).
#[derive(Debug, Clone, PartialEq)]
pub enum TotalityViolation {
    /// An unbounded `while` loop in `def` (recorded at parse).  v1 admits only
    /// `for` over a finite collection / range; a `while` has no proven bound.
    UnboundedLoop { def: u32, position: Position },
    /// A cycle in the sandboxed call graph — recursion.  v1 is acyclic-only; the
    /// `cycle` lists the sandboxed defs forming the back-edge.
    Recursion { cycle: Vec<u32> },
    /// `def` reaches a **partial op** that faults the script (3.3): `assert` /
    /// `panic` / `log_fatal` abort, which would break the host.  The arithmetic
    /// ops are NOT here — the interpreter makes them total (div/mod-by-zero →
    /// null, OOB → null, overflow wraps), and the sandbox is interpret-only, so
    /// they never fault.  Only the explicit-abort ops are excluded.
    PartialOp {
        def: u32,
        op: u32,
        position: Option<Position>,
    },
}

/// @PLN86 step 3.3 — the explicit-abort ops excluded from sandboxed code: they
/// terminate the script (a fault the host must never see), and unlike the
/// arithmetic ops cannot be given a total result.  By stored name so the check
/// is a set membership over the reachable refs.
pub(crate) const ABORT_OPS: &[&str] = &["n_assert", "n_panic", "n_log_fatal"];

/// The sandboxed defs that `from` calls — the edges of the sandboxed call graph
/// (trusted callees are not followed; they are total by the host contract).
fn sandboxed_calls(
    data: &Data,
    sandboxed: &HashMap<u32, String>,
    fn_refs: &HashMap<u32, HashSet<u32>>,
    from: u32,
) -> Vec<u32> {
    let mut refs = HashSet::new();
    referenced_defs(data, from, &mut refs);
    add_recorded_fn_refs(fn_refs, from, &mut refs); // L4 — indirect calls are edges too
    let mut calls: Vec<u32> = refs
        .into_iter()
        .filter(|r| sandboxed.contains_key(r))
        .collect();
    calls.sort_unstable();
    calls
}

/// @PLN86 step 3.2 — every cycle in the sandboxed call graph (recursion).  A
/// standard colour DFS: a back-edge to a node still on the current path is a
/// cycle; the path slice from that node is the cycle.  Deduped by node-set so a
/// cycle is reported once.  Self-recursion (`f` calls `f`) is a length-1 cycle.
#[must_use]
pub fn recursion_cycles(
    data: &Data,
    sandboxed: &HashMap<u32, String>,
    fn_refs: &HashMap<u32, HashSet<u32>>,
) -> Vec<Vec<u32>> {
    #[allow(clippy::too_many_arguments)]
    fn dfs(
        n: u32,
        data: &Data,
        sandboxed: &HashMap<u32, String>,
        fn_refs: &HashMap<u32, HashSet<u32>>,
        on_path: &mut HashSet<u32>,
        done: &mut HashSet<u32>,
        path: &mut Vec<u32>,
        cycles: &mut Vec<Vec<u32>>,
        seen: &mut HashSet<Vec<u32>>,
    ) {
        if on_path.contains(&n) {
            if let Some(start) = path.iter().position(|&x| x == n) {
                let cycle = path[start..].to_vec();
                let mut key = cycle.clone();
                key.sort_unstable();
                if seen.insert(key) {
                    cycles.push(cycle);
                }
            }
            return;
        }
        if !done.insert(n) {
            return;
        }
        on_path.insert(n);
        path.push(n);
        for callee in sandboxed_calls(data, sandboxed, fn_refs, n) {
            dfs(
                callee, data, sandboxed, fn_refs, on_path, done, path, cycles, seen,
            );
        }
        path.pop();
        on_path.remove(&n);
    }

    let mut cycles = Vec::new();
    let (mut on_path, mut done, mut path, mut seen) =
        (HashSet::new(), HashSet::new(), Vec::new(), HashSet::new());
    let mut starts: Vec<u32> = sandboxed.keys().copied().collect();
    starts.sort_unstable();
    for n in starts {
        dfs(
            n,
            data,
            sandboxed,
            fn_refs,
            &mut on_path,
            &mut done,
            &mut path,
            &mut cycles,
            &mut seen,
        );
    }
    cycles
}

/// @PLN86 3.1 — is `while cond { body }` PROVABLY terminating?  SOUND and
/// conservative: true ONLY when a decreasing integer variant is found — an integer
/// counter compared in `cond` against a STABLE bound, stepped monotonically toward
/// it by a positive integer CONSTANT on EVERY iteration (an unconditional
/// top-level `c = c + k` / `c = c - k`) and modified no other way.  Flag loops,
/// non-integer / field variants, conditional or non-constant steps, and
/// call/expression bounds all return false → rejected.  Unsoundness here is a HANG
/// (the host's whole point is that it can't be hung), so when in any doubt: false.
///
/// The IR contract (verified): `&&` parses as `if A { B } else false`; `>` / `>=`
/// normalise to a SWAPPED `OpLtInt` / `OpLeInt` (counter on the right); `+`/`-` on
/// integers are `OpAddInt` / `OpMinInt`.
#[must_use]
pub fn while_is_bounded(data: &Data, cond: &Value, body: &Value) -> bool {
    let mut candidates: Vec<(u16, bool)> = Vec::new();
    collect_variant_candidates(data, cond, body, &mut candidates);
    candidates
        .iter()
        .any(|&(counter, increasing)| counter_progresses(data, body, counter, increasing))
}

/// Collect `(counter, increasing)` candidates from `cond`: `counter < bound` /
/// `counter <= bound` (increasing) or `bound < counter` / `bound <= counter`
/// (decreasing) with a stable bound, plus BOTH conjuncts of an `&&` (parsed as
/// `if A { B } else false`) — either can bound the loop.  `||` (`if A { true }
/// else B`, else ≠ false) yields nothing: neither side alone bounds it.
fn collect_variant_candidates(data: &Data, cond: &Value, body: &Value, out: &mut Vec<(u16, bool)>) {
    match cond.unspan() {
        Value::If(a, b, els) if matches!(els.unspan(), Value::Boolean(false)) => {
            collect_variant_candidates(data, a, body, out);
            collect_variant_candidates(data, b, body, out);
        }
        Value::Call(op, args) if args.len() == 2 => {
            let name = data.def(*op).name();
            if name == "OpLtInt" || name == "OpLeInt" {
                if let Value::Var(c) = args[0].unspan()
                    && is_stable_bound(body, &args[1], *c)
                {
                    out.push((*c, true)); // counter < bound → counts up
                }
                if let Value::Var(c) = args[1].unspan()
                    && is_stable_bound(body, &args[0], *c)
                {
                    out.push((*c, false)); // bound < counter → counts down
                }
            }
        }
        _ => {}
    }
}

/// A bound is stable iff it's an integer literal or a variable (not the counter)
/// never assigned in the body — so it cannot move away from the counter.  A call
/// or expression bound is conservatively unstable (hoist it to a local first).
fn is_stable_bound(body: &Value, bound: &Value, counter: u16) -> bool {
    match bound.unspan() {
        Value::Int(_) | Value::Long(_) => true,
        Value::Var(b) => *b != counter && !contains_assignment_to(body, *b),
        _ => false,
    }
}

/// Every assignment to `counter` in `body` must be a TOP-LEVEL (unconditional),
/// monotonic, constant step in the right direction; at least one must exist.  A
/// nested (conditional) or wrong-direction or non-constant assignment fails it.
fn counter_progresses(data: &Data, body: &Value, counter: u16, increasing: bool) -> bool {
    let ops: &[Value] = match body.unspan() {
        Value::Block(b) => &b.operators,
        other => std::slice::from_ref(other),
    };
    let mut stepped = false;
    for stmt in ops {
        if let Value::Set(v, rhs) = stmt.unspan()
            && *v == counter
        {
            if is_monotonic_step(data, rhs, counter, increasing) {
                stepped = true;
            } else {
                return false; // a top-level non-step assignment to the counter
            }
            continue;
        }
        // any other top-level statement must not touch the counter at all (a
        // conditional / nested step can't be proven to run every iteration).
        if contains_assignment_to(stmt, counter) {
            return false;
        }
    }
    stepped
}

/// `rhs` is `counter + k` (increasing) or `counter - k` (decreasing) with `k` a
/// positive integer constant.
fn is_monotonic_step(data: &Data, rhs: &Value, counter: u16, increasing: bool) -> bool {
    let Value::Call(op, args) = rhs.unspan() else {
        return false;
    };
    if args.len() != 2 {
        return false;
    }
    let pos_int = |v: &Value| matches!(v.unspan(), Value::Int(k) if *k > 0);
    let is_counter = |v: &Value| matches!(v.unspan(), Value::Var(c) if *c == counter);
    match (increasing, data.def(*op).name()) {
        // c + k  or  k + c
        (true, "OpAddInt") => {
            (is_counter(&args[0]) && pos_int(&args[1]))
                || (pos_int(&args[0]) && is_counter(&args[1]))
        }
        // c - k  (subtraction is not commutative)
        (false, "OpMinInt") => is_counter(&args[0]) && pos_int(&args[1]),
        _ => false,
    }
}

/// Does any node in `value` assign to `var` (a `Set(var, _)`)?
fn contains_assignment_to(value: &Value, var: u16) -> bool {
    let mut found = false;
    value.walk(&mut |v| {
        if let Value::Set(v_nr, _) = v
            && *v_nr == var
        {
            found = true;
        }
    });
    found
}

/// @PLN86 step P3 — the totality admission walk: a sandboxed script is total iff
/// it has no unbounded loop (3.1) and no recursion cycle (3.2).  `unbounded_loops`
/// is the parser's `sandbox_unbounded_loops` (while-loops recorded at parse).
/// Returns every violation, deterministically ordered.
#[must_use]
pub fn admit_totality(
    data: &Data,
    sandboxed: &HashMap<u32, String>,
    unbounded_loops: &HashMap<u32, Position>,
    fn_refs: &HashMap<u32, HashSet<u32>>,
) -> Vec<TotalityViolation> {
    let mut violations = Vec::new();
    // 3.1 — unbounded loops, ordered by def.
    let mut looped: Vec<u32> = unbounded_loops
        .keys()
        .copied()
        .filter(|d| sandboxed.contains_key(d))
        .collect();
    looped.sort_unstable();
    for def in looped {
        violations.push(TotalityViolation::UnboundedLoop {
            def,
            position: unbounded_loops[&def].clone(),
        });
    }
    // 3.2 — recursion cycles.
    for cycle in recursion_cycles(data, sandboxed, fn_refs) {
        violations.push(TotalityViolation::Recursion { cycle });
    }
    // 3.3 — reachable explicit-abort ops (assert / panic / log_fatal).  The
    // arithmetic ops are total on the interpreter, so they are NOT rejected; only
    // the ops that fault the script are excluded.
    let mut entries: Vec<u32> = sandboxed.keys().copied().collect();
    entries.sort_unstable();
    for def in entries {
        let mut refs = HashSet::new();
        referenced_defs(data, def, &mut refs);
        add_recorded_fn_refs(fn_refs, def, &mut refs); // L4 — fn-ref'd abort ops too
        let mut refs: Vec<u32> = refs.into_iter().collect();
        refs.sort_unstable();
        for op in refs {
            if !sandboxed.contains_key(&op) && ABORT_OPS.contains(&data.def(op).name()) {
                violations.push(TotalityViolation::PartialOp {
                    def,
                    op,
                    position: reference_position(data, def, op),
                });
            }
        }
    }
    violations
}

/// @PLN86 step 2.5 / P3 — render a totality violation as an actionable admission
/// error: the construct, the rule, and how to bound it.
#[must_use]
pub fn describe_totality_violation(data: &Data, v: &TotalityViolation) -> String {
    match v {
        TotalityViolation::UnboundedLoop { def, position } => {
            let name = display_name(data, *def);
            format!(
                "{position}: sandboxed `{name}` contains a `while` loop with no provable \
                 bound — a sandboxed script must be provably total.\n  fix: use a bounded \
                 `for x in <collection>` / `for i in 0..N` loop; or give the `while` a \
                 decreasing integer variant — an `int` counter compared against a constant or \
                 unchanging bound and stepped by a constant every iteration, e.g. \
                 `i = 0; while i < n {{ … i = i + 1 }}` (a guard `&& i < N` works too)."
            )
        }
        TotalityViolation::Recursion { cycle } => {
            let chain = cycle
                .iter()
                .map(|&d| display_name(data, d))
                .collect::<Vec<_>>()
                .join(" → ");
            let tail = display_name(data, cycle[0]);
            format!(
                "sandboxed recursion is not permitted (the script must be provably total): \
                 the call cycle `{chain} → {tail}` is unbounded.\n  fix: replace the \
                 recursion with a bounded `for` loop over the work to do."
            )
        }
        TotalityViolation::PartialOp { def, op, position } => {
            let from = display_name(data, *def);
            let op_name = display_name(data, *op);
            let at = position
                .as_ref()
                .map_or_else(String::new, |p| format!("{p}: "));
            format!(
                "{at}sandboxed `{from}` calls `{op_name}`, which faults the script — a \
                 sandboxed script may never abort the host.\n  fix: handle the condition \
                 without `{op_name}` (e.g. `if !ok {{ return <default> }}`, or `a / b ?? 0` \
                 for a divide-by-zero — arithmetic faults are already total/null)."
            )
        }
    }
}

/// @PLN86 step 3.4 — one def's intrinsic complexity: the deepest loop nesting at
/// which plain work happens (`max_leaf`), and every call with the loop nesting it
/// sits under (`(nesting, callee)`).  A pure single-def walk — the inter-procedural
/// composition is `complexity_degree`.  `Loop` (incl. comprehensions) / `Iter` /
/// `ParFor` each add one nesting level; an admitted script has no `while` (3.1) so
/// every loop is bounded.
fn intrinsic_complexity(data: &Data, f: u32) -> (u32, Vec<(u32, u32)>) {
    fn scan(v: &Value, nesting: u32, max_leaf: &mut u32, calls: &mut Vec<(u32, u32)>) {
        let child_nesting = match v {
            Value::Loop(_) | Value::Iter(..) | Value::ParFor(_) => nesting + 1,
            _ => nesting,
        };
        if let Value::Call(g, _) = v {
            calls.push((nesting, *g));
        } else {
            *max_leaf = (*max_leaf).max(nesting);
        }
        v.for_each_child(&mut |c| scan(c, child_nesting, max_leaf, calls));
    }
    let (mut max_leaf, mut calls) = (0, Vec::new());
    scan(&data.def(f).code, 0, &mut max_leaf, &mut calls);
    (max_leaf, calls)
}

/// @PLN86 step 3.4 — the polynomial DEGREE of `f`'s worst-case step count in the
/// input size `n` (the largest collection / range a loop iterates): the max over
/// `f`'s body of `loop_nesting + degree(callee)`.  A loop calling a fn that loops
/// composes (O(n²)).  The call graph is acyclic (3.2), so this terminates; the
/// `in_progress` guard is a defensive backstop.  Trusted-leaf ops count as O(1)
/// (degree 0) per call in v1.
fn complexity_degree(
    data: &Data,
    sandboxed: &HashMap<u32, String>,
    f: u32,
    memo: &mut HashMap<u32, u32>,
    in_progress: &mut HashSet<u32>,
) -> u32 {
    if let Some(&d) = memo.get(&f) {
        return d;
    }
    if !in_progress.insert(f) {
        return 0; // cycle (rejected by 3.2) — break defensively
    }
    let (max_leaf, calls) = intrinsic_complexity(data, f);
    let mut degree = max_leaf;
    for (nesting, callee) in calls {
        let callee_degree = if sandboxed.contains_key(&callee) {
            complexity_degree(data, sandboxed, callee, memo, in_progress)
        } else {
            0
        };
        degree = degree.max(nesting + callee_degree);
    }
    in_progress.remove(&f);
    memo.insert(f, degree);
    degree
}

/// @PLN86 step 3.4 — the worst-case complexity DEGREE over all sandboxed entries:
/// the script's step count is `O(n^degree)` in the largest input size.  An
/// admitted script is total (3.1–3.3), so this is finite and computable; the host
/// reads it to **bound the inputs** so no single frame stalls (L5).
#[must_use]
pub fn sandbox_complexity_degree(data: &Data, sandboxed: &HashMap<u32, String>) -> u32 {
    let (mut memo, mut in_progress) = (HashMap::new(), HashSet::new());
    sandboxed
        .keys()
        .map(|&f| complexity_degree(data, sandboxed, f, &mut memo, &mut in_progress))
        .max()
        .unwrap_or(0)
}

/// @PLN86 step 3.4 — the human-readable complexity report (the L5 host hint).
#[must_use]
pub fn complexity_report(degree: u32) -> String {
    let big_o = match degree {
        0 => "O(1)".to_string(),
        1 => "O(n)".to_string(),
        d => format!("O(n^{d})"),
    };
    format!(
        "worst-case complexity: {big_o} (n = the largest input collection / range; \
         bound n to keep each frame within budget)"
    )
}

/// @PLN86 step 2.4 — a no-raw-write violation: a sandboxed def directly mutates
/// heap data (`x.field = v` / `v[i] = v`).  An admitted script may mutate host
/// data only through allow-listed `*.write` operations, never a raw assignment,
/// so the host's invariants are preserved.
#[derive(Debug, Clone, PartialEq)]
pub struct RawWriteViolation {
    pub def: u32,
    pub position: Position,
}

/// @PLN86 step 2.4 — the no-raw-write admission: every sandboxed def that the
/// parser recorded a raw field/index write for (`sandbox_raw_writes`).  The
/// check is at parse (the parser knows the LHS is a field/index target, distinct
/// from a bare local or a struct literal), so this just surfaces the record.
#[must_use]
pub fn raw_write_violations(
    sandboxed: &HashMap<u32, String>,
    raw_writes: &HashMap<u32, Position>,
) -> Vec<RawWriteViolation> {
    let mut out: Vec<RawWriteViolation> = raw_writes
        .iter()
        .filter(|(d, _)| sandboxed.contains_key(d))
        .map(|(&def, position)| RawWriteViolation {
            def,
            position: position.clone(),
        })
        .collect();
    out.sort_by_key(|v| v.def);
    out
}

/// @PLN86 step 2.4 — render a no-raw-write violation as an actionable error.
#[must_use]
pub fn describe_raw_write_violation(data: &Data, v: &RawWriteViolation) -> String {
    let from = display_name(data, v.def);
    format!(
        "{}: sandboxed `{from}` performs a raw write to heap data (`x.field = …` / \
         `v[i] = …`) — a sandboxed script may not directly mutate host fields/elements, \
         which could break an invariant.\n  fix: mutate only through an allow-listed write \
         operation (a `*.write` capability such as `damage(e, 10)`); construct new values \
         freely.",
        v.position
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_match_is_dotted_segment() {
        assert!(cap_prefix_match("game", "game")); // exact
        assert!(cap_prefix_match("game", "game.read")); // child
        assert!(cap_prefix_match("game", "game.entity.write")); // grandchild
        assert!(!cap_prefix_match("game", "gameover")); // NOT a string prefix
        assert!(!cap_prefix_match("game.read", "game.write")); // sibling
        assert!(!cap_prefix_match("game.read", "game")); // parent is not allowed by child
        assert!(!cap_prefix_match("", "game")); // empty allow never matches
    }

    #[test]
    fn allows_is_deny_by_default() {
        let p = SandboxProfile {
            allow_caps: vec!["game.read".into(), "math".into(), "collections.read".into()],
            ..SandboxProfile::default()
        };
        assert!(p.allows("game.read"));
        assert!(p.allows("math")); // exact whole-lib
        assert!(p.allows("math.trig")); // child of an allowed prefix
        assert!(p.allows("collections.read"));
        assert!(!p.allows("collections.write")); // sibling — denied
        assert!(!p.allows("fs.read")); // not listed — denied
        assert!(!p.allows("game.write")); // sibling — denied
        assert!(!p.allows("")); // untagged — denied
        // an empty profile denies everything
        assert!(!SandboxProfile::default().allows("math"));
    }

    #[test]
    fn parses_profiles_and_designations() {
        let toml = r#"
[package]
name = "ztgame"

[sandbox]
mod-script = ["mods/**/*.loft", "fn:player_eval"]

[profile.mod-script]
backend = "interpret"
native_ffi = false
allow_caps = ["game.read", "game.write", "math", "collections.read"]
"#;
        let cfg = parse_sandbox_config(toml);
        assert!(cfg.is_active());
        assert_eq!(cfg.designations.len(), 2);
        assert_eq!(
            cfg.designations[0],
            ("mods/**/*.loft".to_string(), "mod-script".to_string())
        );
        assert_eq!(
            cfg.designations[1],
            ("fn:player_eval".to_string(), "mod-script".to_string())
        );
        let p = cfg.profiles.get("mod-script").expect("profile present");
        assert!(!p.native_ffi);
        assert!(p.allows("game.write"));
        assert!(p.allows("collections.read"));
        assert!(!p.allows("fs.read")); // not granted
        assert!(!p.allows("collections.write")); // sibling denied
        // resolve a designation to its profile
        assert!(
            cfg.profile_for_selector("mods/**/*.loft")
                .is_some_and(|p| p.allows("math"))
        );
    }

    #[test]
    fn fn_designation_resolves_only_designated_functions() {
        let cfg = parse_sandbox_config(
            "[sandbox]\nmod-script = [\"fn:player_eval\", \"fn:on_tick\"]\n[profile.mod-script]\n",
        );
        assert_eq!(cfg.fn_designation("player_eval"), Some("mod-script"));
        assert_eq!(cfg.fn_designation("on_tick"), Some("mod-script"));
        assert_eq!(cfg.fn_designation("host_update"), None); // not designated
        assert_eq!(cfg.fn_designation(""), None);
    }

    #[test]
    fn empty_profile_section_registers_a_deny_all_profile() {
        let cfg = parse_sandbox_config("[sandbox]\nlocked = [\"a.loft\"]\n[profile.locked]\n");
        let p = cfg.profiles.get("locked").expect("registered");
        assert!(p.allow_caps.is_empty());
        assert!(p.allow_libs.is_empty());
        assert!(!p.allows("game.read")); // deny-all by default
        assert!(!p.native_ffi); // safe default
    }

    #[test]
    fn no_sandbox_section_is_inactive() {
        let cfg = parse_sandbox_config("[package]\nname = \"x\"\n");
        assert!(!cfg.is_active());
        assert!(cfg.profiles.is_empty());
    }
}
