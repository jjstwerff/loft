// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I77 — Registry / manifest / lockfile resolution

//! Minimal `loft.toml` manifest reader for external package support (T2-11).
//! Phase 1: pure-loft package layout.  A simple line-scanner is sufficient
//! for the three fields needed here; no full TOML parser is required.

/// @PLN100 Slice 2 — one declared build target from `[build.target.<name>]`: a
/// named (shape × triple × feature-set) the `loft build` driver can produce, with
/// the toolchain it `requires`.  Overlays a built-in target of the same name
/// (`native` / `html` / `wasi`) or adds a new one; empty fields keep the built-in
/// default (`build_phase::resolve_target`).
#[derive(Debug, Default, Clone)]
pub struct BuildTarget {
    /// The target's name — the key in `[build.target.<name>]` and what
    /// `loft build <name>` / `default-targets` refer to.
    pub name: String,
    /// The compile shape that names the driver action: `native` (→ `--native`),
    /// `html` (→ `--html`), or `wasi` (→ `--native-wasm`).  `None` → defaults to
    /// the built-in of the same name, else `native`.
    pub shape: Option<String>,
    /// The rustc/rustup target triple (informational + the default entry of
    /// `requires.rust-targets`).
    pub triple: Option<String>,
    /// Cargo features the shape's runtime rlib is built with (`[…] features = [...]`).
    pub features: Vec<String>,
    /// `[build.target.<name>.requires] rust-targets = [...]` — rustup targets that
    /// must be installed for this target to build.
    pub requires_rust_targets: Vec<String>,
    /// `[build.target.<name>.requires] tools = [...]` — external tools that must be
    /// on `PATH` (e.g. `wasm-opt`).
    pub requires_tools: Vec<String>,
}

/// Content of a library's `loft.toml` manifest file.
#[derive(Debug, Default)]
pub struct Manifest {
    /// Package name from `[package] name = "..."`.
    pub name: Option<String>,
    /// Package version from `[package] version = "..."`.
    pub version: Option<String>,
    /// Entry `.loft` file path, relative to the package root.
    /// Defaults to `src/<name>.loft` when absent.
    pub entry: Option<String>,
    /// Interpreter version requirement from the `[package]` section,
    /// e.g. `">=1.0"`.  `None` means no constraint.
    pub loft_version: Option<String>,
    /// GitHub repository that publishes this package's releases, from
    /// `[package] repository = "..."`.  Either a bare repo name under the
    /// `loft-lang` org (`"loft-libs-core"`) or a full `"owner/repo"`.  When
    /// set, the package lives in a **monorepo**, so `loft package` emits the
    /// `<name>-v<version>` release-tag scheme; absent → the legacy
    /// one-repo-per-package `loft-<name>` + `v<version>` fallback.
    pub repository: Option<String>,
    /// One-line package description from `[package] description = "..."`.  The
    /// official source for the registry catalog (`loft search` / `loft api
    /// --registry`); registry tooling prefers this over scraping the README.
    pub description: Option<String>,
    /// Native shared-library stem from `[library] native = "..."`.
    /// `None` for pure-loft packages.  The interpreter resolves this to the
    /// platform-correct filename (`lib<stem>.so` / `.dylib` / `.dll`).
    pub native: Option<String>,
    /// @PLN11 Arc N / N3 — `[library] compile = "native"` opts a **normal** loft
    /// library (no hand-written `#native` / Rust crate) into auto-compilation to a
    /// shared-store cdylib: its native-compilable public functions are marked
    /// native and dispatched (`use <lib>` then runs the library native, the script
    /// interprets).  `None`/absent → the library is interpreted as before.  (B-phase
    /// makes this automatic; today it is the explicit opt-in.)
    pub compile: Option<String>,
    /// PKG.3: package dependencies from `[dependencies]` section.
    /// Key = package name, value = version requirement or path.
    pub dependencies: Vec<(String, String)>,
    /// PKG.4: Rust crate directory name from `[native] crate = "..."`.
    pub native_crate: Option<String>,
    /// @PLN18 — `[native] in_binary = true`: this library's `#native` symbols
    /// live INSIDE the loft binary (registered in `src/native.rs`), not in a
    /// cdylib.  Readers: the auto-native driver (skips the doomed cdylib
    /// compile + its warning) and the extraction-hygiene gate (the symbols
    /// are sanctioned in `src/**`).  One mark, every consumer.
    pub native_in_binary: bool,
    /// @PLN21 Phase 2 — `[native] runtime-libs = "libGL.so.1, libasound.so.2"`:
    /// the system shared libraries this package's cdylib needs at RUNTIME (its
    /// `dlopen` NEEDED set beyond libc).  Turns a load failure into an actionable
    /// install hint; a missing one of these can NOT be fixed by building from
    /// source (the source cdylib links the same lib) — Phase 3 / decision C3.
    pub runtime_libs: Vec<String>,
    /// @PLN21 Phase 2 — `[native] build-deps = "libgl-dev, libasound2-dev"`:
    /// the system DEV packages needed to BUILD the cdylib from source (the
    /// fallback when no prebuilt matches).  Diagnoses a failed `auto_build_native`
    /// ("install `<dev-lib>`").  Distinct from `runtime_libs` (the `-dev` headers
    /// vs the runtime `.so`).
    pub build_deps: Vec<String>,
    /// PKG.4: loft function name → Rust symbol path from `[native.functions]`.
    pub native_functions: Vec<(String, String)>,
    /// PKG.5: WASM-specific overrides from `[native.wasm]`.
    pub native_wasm: Vec<(String, String)>,
    /// lib_plan-29 W1c: bridge crate from `[wasm.bridge] crate = "..."`.
    /// `None` for libraries with no browser-WASM bridge.  When set, the
    /// `--html` driver builds `<pkg>/wasm/` (cargo path-rel) to a
    /// `wasm32-unknown-unknown` rlib and links it via `--extern`.
    pub wasm_bridge_crate: Option<String>,
    /// lib_plan-29 W1c: loft `#native` symbol → bridge `pub fn` name from
    /// `[wasm.bridge.routes]`.  Read by `src/generation/mod.rs::output_
    /// native_direct_call` (replaces the hard-coded `WASM_BRIDGE_FNS`).
    pub wasm_bridge_routes: Vec<(String, String)>,
    /// lib_plan-29 W2: per-library JS host-imports file from
    /// `[wasm.bridge] host_js = "..."` (path relative to the package
    /// root).  The `--html` driver reads the file and concatenates it
    /// into the HTML preamble; the bundled JS is expected to push a
    /// registration callback onto `globalThis.LOFT_WASM_EXTENSIONS`
    /// (see `lib/imaging/wasm/host.js` for the canonical shape).
    pub wasm_bridge_host_js: Option<String>,
    /// `[triggers] enabled = true` (lib_plans/59-lazy-stdlib) — opt this
    /// library into lazy auto-load, making `use <pkg>;` optional.  This is the
    /// ONLY trigger metadata: when set, loft DERIVES the actual trigger surface
    /// (the `<pkg>::` namespace, the library's public text-methods, any public
    /// types) from the library's own source — nothing is hand-listed, so there
    /// is no parallel table to drift (same source-of-truth principle as the
    /// source-scanned `#native` symbols).  The resolver side that acts on this
    /// is separate, future work; today this is parsed metadata.
    pub trigger_enabled: bool,
    /// @PLN100 Slice 2 — `[build] default-targets = [...]`: which targets
    /// `loft build` makes when given no target names.  Empty → the driver falls
    /// back to `["native"]` so a zero-config project still builds.
    pub build_default_targets: Vec<String>,
    /// @PLN100 Slice 2 — the `[build.target.<name>]` entries.  Each overlays a
    /// built-in target (`native` / `html` / `wasi`) or declares a new one.
    pub build_targets: Vec<BuildTarget>,
}

impl Manifest {
    /// @PLN100 Slice 2 — the `[build.target.<name>]` entry for `name`, creating an
    /// empty one on first mention so later `[build.target.<name>.requires]` lines
    /// attach to the same target regardless of section order.
    fn build_target_mut(&mut self, name: &str) -> &mut BuildTarget {
        if let Some(pos) = self.build_targets.iter().position(|t| t.name == name) {
            return &mut self.build_targets[pos];
        }
        self.build_targets.push(BuildTarget {
            name: name.to_string(),
            ..Default::default()
        });
        self.build_targets
            .last_mut()
            .expect("just pushed a build target")
    }
}

/// Read and parse a `loft.toml` file at `path`.
/// Returns `None` when the file does not exist or cannot be read.
#[must_use]
pub fn read_manifest(path: &str) -> Option<Manifest> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut manifest = Manifest::default();
    let mut section = String::new();
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].to_string();
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim().trim_matches('"');
            // @PLN100 Slice 2 — the [build] section uses dynamic
            // [build.target.<name>] / [build.target.<name>.requires] headers the
            // literal match below cannot express, plus single-line array values.
            if section == "build" {
                if key == "default-targets" {
                    manifest.build_default_targets = parse_array(value);
                }
                continue;
            }
            if let Some(rest) = section.strip_prefix("build.target.") {
                if let Some(tname) = rest.strip_suffix(".requires") {
                    let t = manifest.build_target_mut(tname);
                    match key {
                        "rust-targets" => t.requires_rust_targets = parse_array(value),
                        "tools" => t.requires_tools = parse_array(value),
                        _ => {}
                    }
                } else {
                    let t = manifest.build_target_mut(rest);
                    match key {
                        "shape" => t.shape = Some(value.to_string()),
                        "triple" => t.triple = Some(value.to_string()),
                        "features" => t.features = parse_array(value),
                        _ => {}
                    }
                }
                continue;
            }
            match (section.as_str(), key) {
                ("package", "name") => manifest.name = Some(value.to_string()),
                ("package", "version") => manifest.version = Some(value.to_string()),
                ("package", "loft") => manifest.loft_version = Some(value.to_string()),
                ("package", "repository") => manifest.repository = Some(value.to_string()),
                ("package", "description") => manifest.description = Some(value.to_string()),
                ("library", "entry") => manifest.entry = Some(value.to_string()),
                ("library", "native") => manifest.native = Some(value.to_string()),
                ("library", "compile") => manifest.compile = Some(value.to_string()),
                ("dependencies", _) => {
                    manifest
                        .dependencies
                        .push((key.to_string(), value.to_string()));
                }
                ("native", "crate") => manifest.native_crate = Some(value.to_string()),
                ("native", "in_binary") => manifest.native_in_binary = value == "true",
                ("native", "runtime-libs") => {
                    manifest.runtime_libs = split_list(value);
                }
                ("native", "build-deps") => {
                    manifest.build_deps = split_list(value);
                }
                ("native.functions", _) => {
                    manifest
                        .native_functions
                        .push((key.to_string(), value.to_string()));
                }
                ("native.wasm", _) => {
                    manifest
                        .native_wasm
                        .push((key.to_string(), value.to_string()));
                }
                ("wasm.bridge", "crate") => manifest.wasm_bridge_crate = Some(value.to_string()),
                ("wasm.bridge", "host_js") => {
                    manifest.wasm_bridge_host_js = Some(value.to_string());
                }
                ("wasm.bridge.routes", _) => {
                    manifest
                        .wasm_bridge_routes
                        .push((key.to_string(), value.to_string()));
                }
                ("triggers", "enabled") => manifest.trigger_enabled = value == "true",
                _ => {}
            }
        }
    }
    Some(manifest)
}

/// @PLN100 Slice 2 — parse a single-line TOML array value into its string items:
/// `["random"]` → `["random"]`, `[ "a", "b" ]` → `["a", "b"]`.  Tolerates a value
/// with or without the surrounding brackets (a bare `"a, b"` is treated as a list
/// too).  Multi-line arrays are not supported (the line-scanner sees one line).
fn parse_array(value: &str) -> Vec<String> {
    let inner = value
        .trim()
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(value);
    inner
        .split(',')
        .map(|s| s.trim().trim_matches('"').trim())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// @PLN21 Phase 2 — parse a comma-separated manifest value into a trimmed,
/// non-empty list: `"libGL.so.1, libasound.so.2"` → `["libGL.so.1",
/// "libasound.so.2"]`.  Backs `[native] runtime-libs` / `build-deps`.
fn split_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Check whether the `required` version constraint is satisfied by `current`.
/// Only `>=X.Y` and `>=X.Y.Z` forms are supported (Phase 1 scope).
/// Returns `true` when `current >= required_version` or `required` is empty.
#[must_use]
pub fn check_version(required: &str, current: &str) -> bool {
    if required.is_empty() {
        return true;
    }
    let req = required.strip_prefix(">=").unwrap_or(required);
    version_ge(current, req)
}

/// Returns `true` when semantic version `a >= b`.
fn version_ge(a: &str, b: &str) -> bool {
    fn parse(s: &str) -> (u32, u32, u32) {
        let mut parts = s.splitn(3, '.');
        let major = parts.next().and_then(|x| x.parse().ok()).unwrap_or(0);
        let minor = parts.next().and_then(|x| x.parse().ok()).unwrap_or(0);
        let patch = parts.next().and_then(|x| x.parse().ok()).unwrap_or(0);
        (major, minor, patch)
    }
    parse(a) >= parse(b)
}

/// Extract the `path` value from an inline-table dependency value
/// of the form `{ path = "X" }` (with optional whitespace + other
/// fields).  Returns `None` for plain version strings (`"0.1"`,
/// `">=0.2"`) or other shapes.
///
/// @PLAN12 phase 3.5b (2026-05-24) — path-deps in `loft.toml`
/// `[dependencies]` were decorative before this; the manifest
/// parser stored the entire inline-table as an opaque "version"
/// string.  This helper extracts the path so the caller in
/// `src/parser/mod.rs::apply_manifest_side_effects` can resolve
/// it relative to the package directory and register the parent
/// in `lib_dirs`.
#[must_use]
pub fn extract_path_dep(value: &str) -> Option<&str> {
    let v = value.trim();
    let inner = v.strip_prefix('{')?.strip_suffix('}')?.trim();
    // Support multi-field inline tables like
    // `{ path = "X", version = "Y" }` by splitting on `,`.  Commas
    // inside the quoted path value would break this — accept the
    // limitation for now, paths typically don't contain commas.
    for part in inner.split(',') {
        let part = part.trim();
        if let Some(rhs) = part.strip_prefix("path") {
            let rhs = rhs.trim();
            let rhs = rhs.strip_prefix('=')?.trim();
            // Strip surrounding quotes.
            let path = rhs.trim_start_matches('"').trim_end_matches('"');
            return Some(path);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp(name: &str, content: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "loft_manifest_test_{}_{}.toml",
            name,
            std::process::id()
        ));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    #[test]
    fn parses_loft_version_requirement() {
        let p = write_temp("ver", "[package]\nloft = \">=1.0\"\n");
        let m = read_manifest(p.to_str().unwrap()).unwrap();
        assert_eq!(m.loft_version.as_deref(), Some(">=1.0"));
    }

    #[test]
    fn parses_custom_entry() {
        let p = write_temp("entry", "[library]\nentry = \"src/mylib.loft\"\n");
        let m = read_manifest(p.to_str().unwrap()).unwrap();
        assert_eq!(m.entry.as_deref(), Some("src/mylib.loft"));
    }

    #[test]
    fn version_current_passes() {
        assert!(check_version(">=0.1", "0.1.0"));
        assert!(check_version(">=1.0", "1.2.3"));
        assert!(check_version(">=0.1.0", "0.1.0"));
        assert!(check_version("", "0.1.0"));
    }

    #[test]
    fn parses_package_name_and_version() {
        let p = write_temp(
            "pkg",
            "[package]\nname = \"graphics\"\nversion = \"0.2.1\"\nloft = \">=0.8\"\n",
        );
        let m = read_manifest(p.to_str().unwrap()).unwrap();
        assert_eq!(m.name.as_deref(), Some("graphics"));
        assert_eq!(m.version.as_deref(), Some("0.2.1"));
        assert_eq!(m.loft_version.as_deref(), Some(">=0.8"));
        assert_eq!(m.repository, None);
    }

    /// `[package] repository` parses (drives `loft package`'s monorepo URL/tag).
    #[test]
    fn parses_package_repository() {
        let p = write_temp(
            "repo",
            "[package]\nname = \"crypto\"\nversion = \"0.2.1\"\nrepository = \"loft-libs-core\"\n",
        );
        let m = read_manifest(p.to_str().unwrap()).unwrap();
        assert_eq!(m.repository.as_deref(), Some("loft-libs-core"));
    }

    #[test]
    fn parses_package_description() {
        let p = write_temp(
            "desc",
            "[package]\nname = \"crypto\"\nversion = \"0.2.1\"\ndescription = \"SHA-256, HMAC, base64.\"\n",
        );
        let m = read_manifest(p.to_str().unwrap()).unwrap();
        assert_eq!(m.description.as_deref(), Some("SHA-256, HMAC, base64."));
    }

    // @PLN21 Phase 2 — `[native] runtime-libs` / `build-deps` parse into trimmed
    // lists (with and without spaces), and are empty when absent.
    #[test]
    fn parses_native_runtime_and_build_deps() {
        let p = write_temp(
            "ndeps",
            "[native]\ncrate = \"loft-graphics\"\nruntime-libs = \"libGL.so.1, libasound.so.2\"\nbuild-deps = \"libgl-dev,libasound2-dev\"\n",
        );
        let m = read_manifest(p.to_str().unwrap()).unwrap();
        assert_eq!(m.runtime_libs, vec!["libGL.so.1", "libasound.so.2"]);
        assert_eq!(m.build_deps, vec!["libgl-dev", "libasound2-dev"]);

        let p2 = write_temp("nodeps", "[native]\ncrate = \"loft-random\"\n");
        let m2 = read_manifest(p2.to_str().unwrap()).unwrap();
        assert!(m2.runtime_libs.is_empty() && m2.build_deps.is_empty());
    }

    #[test]
    fn parses_dependencies() {
        let p = write_temp(
            "deps",
            "[dependencies]\nmath = \">=0.2\"\nutils = \"../utils\"\n",
        );
        let m = read_manifest(p.to_str().unwrap()).unwrap();
        assert_eq!(m.dependencies.len(), 2);
        assert_eq!(m.dependencies[0], ("math".to_string(), ">=0.2".to_string()));
        assert_eq!(
            m.dependencies[1],
            ("utils".to_string(), "../utils".to_string())
        );
    }

    #[test]
    fn parses_trigger_enabled() {
        let p = write_temp(
            "triggers",
            "[package]\nname = \"regex\"\n\n[triggers]\nenabled = true\n",
        );
        let m = read_manifest(p.to_str().unwrap()).unwrap();
        assert!(m.trigger_enabled, "[triggers] enabled = true should parse");
    }

    #[test]
    fn trigger_disabled_by_default() {
        let p = write_temp("notrig", "[package]\nname = \"plain\"\n");
        let m = read_manifest(p.to_str().unwrap()).unwrap();
        assert!(!m.trigger_enabled, "no [triggers] section -> disabled");
    }

    #[test]
    fn parses_compile_native() {
        let p = write_temp(
            "compile",
            "[package]\nname = \"mathlib\"\n\n[library]\ncompile = \"native\"\n",
        );
        let m = read_manifest(p.to_str().unwrap()).unwrap();
        assert_eq!(m.compile.as_deref(), Some("native"));
    }

    #[test]
    fn compile_absent_by_default() {
        let p = write_temp("nocompile", "[package]\nname = \"plain\"\n");
        let m = read_manifest(p.to_str().unwrap()).unwrap();
        assert_eq!(m.compile, None);
    }

    #[test]
    fn parses_native_section() {
        let p = write_temp(
            "native",
            r#"[package]
name = "graphics"
version = "0.1.0"

[native]
crate = "graphics_native"

[native.functions]
save_png = "graphics_native::save_png"
load_font = "graphics_native::load_font"

[native.wasm]
gl_create = "graphics_native::webgl::create_canvas"
"#,
        );
        let m = read_manifest(p.to_str().unwrap()).unwrap();
        assert_eq!(m.native_crate.as_deref(), Some("graphics_native"));
        assert_eq!(m.native_functions.len(), 2);
        assert_eq!(
            m.native_functions[0],
            (
                "save_png".to_string(),
                "graphics_native::save_png".to_string()
            )
        );
        assert_eq!(m.native_wasm.len(), 1);
        assert_eq!(
            m.native_wasm[0],
            (
                "gl_create".to_string(),
                "graphics_native::webgl::create_canvas".to_string()
            )
        );
    }

    // @PLN100 Slice 2 — the [build] section: default-targets array,
    // [build.target.<name>] keys, and the [build.target.<name>.requires] subtable
    // (out-of-order lines attach to the same target).
    #[test]
    fn parses_build_section() {
        let p = write_temp(
            "build",
            r#"[package]
name = "game"

[build]
default-targets = ["native", "html"]

[build.target.html.requires]
rust-targets = ["wasm32-unknown-unknown"]
tools = ["wasm-opt"]

[build.target.html]
shape = "html"
triple = "wasm32-unknown-unknown"
features = ["random", "png"]
"#,
        );
        let m = read_manifest(p.to_str().unwrap()).unwrap();
        assert_eq!(m.build_default_targets, vec!["native", "html"]);
        assert_eq!(m.build_targets.len(), 1);
        let html = &m.build_targets[0];
        assert_eq!(html.name, "html");
        assert_eq!(html.shape.as_deref(), Some("html"));
        assert_eq!(html.triple.as_deref(), Some("wasm32-unknown-unknown"));
        assert_eq!(html.features, vec!["random", "png"]);
        assert_eq!(html.requires_rust_targets, vec!["wasm32-unknown-unknown"]);
        assert_eq!(html.requires_tools, vec!["wasm-opt"]);
    }

    #[test]
    fn build_section_absent_by_default() {
        let p = write_temp("nobuild", "[package]\nname = \"plain\"\n");
        let m = read_manifest(p.to_str().unwrap()).unwrap();
        assert!(m.build_default_targets.is_empty() && m.build_targets.is_empty());
    }

    #[test]
    fn parses_wasm_bridge_section() {
        // Use synthetic names rather than real loft-library symbols
        // (`n_load_png` etc.) so the extraction-hygiene gate doesn't
        // flag this test as a library-symbol mention in `src/**`.
        let p = write_temp(
            "wasm-bridge",
            r#"[package]
name = "demo"
version = "0.1.0"

[wasm.bridge]
crate = "demo-wasm"
host_js = "wasm/host.js"

[wasm.bridge.routes]
n_demo_fn_a = "demo_fn_a"
n_demo_fn_b = "demo_fn_b"
"#,
        );
        let m = read_manifest(p.to_str().unwrap()).unwrap();
        assert_eq!(m.wasm_bridge_crate.as_deref(), Some("demo-wasm"));
        assert_eq!(m.wasm_bridge_host_js.as_deref(), Some("wasm/host.js"));
        assert_eq!(m.wasm_bridge_routes.len(), 2);
        assert_eq!(
            m.wasm_bridge_routes[0],
            ("n_demo_fn_a".to_string(), "demo_fn_a".to_string())
        );
        assert_eq!(
            m.wasm_bridge_routes[1],
            ("n_demo_fn_b".to_string(), "demo_fn_b".to_string())
        );
    }

    #[test]
    fn version_too_high_fails() {
        assert!(!check_version(">=2.0", "1.9.9"));
        assert!(!check_version(">=1.1", "1.0.0"));
        assert!(!check_version(">=1.0.1", "1.0.0"));
    }

    #[test]
    fn extract_path_dep_recognizes_inline_table() {
        assert_eq!(
            extract_path_dep(r#"{ path = "../gridmesh" }"#),
            Some("../gridmesh")
        );
        assert_eq!(
            extract_path_dep(r#"{path="../foo/bar"}"#),
            Some("../foo/bar")
        );
        // Multi-field inline table.
        assert_eq!(
            extract_path_dep(r#"{ path = "../X", version = "0.1" }"#),
            Some("../X")
        );
        // version-only entry.
        assert_eq!(extract_path_dep(r#"{ version = "0.1" }"#), None);
        // Plain version string (no inline table).
        assert_eq!(extract_path_dep(">=0.2"), None);
        assert_eq!(extract_path_dep("0.1"), None);
    }
}
