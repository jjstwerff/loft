// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @PLN100 Slice 2 — declarative project build phase

//! The `loft build` driver: resolve a project's declared / default build
//! targets, check each target's toolchain `requires` (doctor-style), and drive
//! the per-target compile.
//!
//! Built-in targets `native` / `html` / `wasi` are **implicit** — a zero-config
//! project (no `[build]` section) still builds.  A `[build.target.<name>]` entry
//! *overlays* a built-in of the same name (changing its `features` / `requires`)
//! or declares a **new** named target that maps to one of the built-in compile
//! shapes.  See `doc/claude/PACKAGES.md § The build phase` and the @PLN100 plan.

use crate::manifest::Manifest;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The compile action a target maps to — named by the target's `shape`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// `--native --check`: full native codegen + rustc compile, no run.
    Native,
    /// `--html`: a self-contained browser page (wasm32-unknown-unknown).
    Html,
    /// `--native-wasm`: a WASI `.wasm` (wasm32-wasip2).
    Wasi,
}

impl Shape {
    /// Parse a `shape = "..."` value (`native` | `html` | `wasi`).
    #[must_use]
    pub fn from_name(name: &str) -> Option<Shape> {
        match name {
            "native" => Some(Shape::Native),
            "html" => Some(Shape::Html),
            "wasi" => Some(Shape::Wasi),
            _ => None,
        }
    }

    /// The `loft` flags that drive this shape's compile.  `native` uses
    /// `--check` so it compiles + reports without RUNNING the program (a build,
    /// not an execution); `html` / `wasi` write their artifact file.
    #[must_use]
    pub fn compile_args(self) -> &'static [&'static str] {
        match self {
            Shape::Native => &["--native", "--check"],
            Shape::Html => &["--html"],
            Shape::Wasi => &["--native-wasm"],
        }
    }
}

/// A build target resolved from the built-in defaults overlaid with any matching
/// `[build.target.<name>]` manifest entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTarget {
    pub name: String,
    pub shape: Shape,
    pub triple: Option<String>,
    pub features: Vec<String>,
    /// rustup targets that must be installed (a HARD requirement — a missing one
    /// means the target cannot build).
    pub requires_rust_targets: Vec<String>,
    /// external tools that should be on `PATH` (a SOFT requirement — a missing
    /// one is a warning; the compile may still succeed with degraded output).
    pub requires_tools: Vec<String>,
}

/// The built-in target for `name`, or `None` if `name` is not a built-in.
fn builtin(name: &str) -> Option<ResolvedTarget> {
    let make =
        |shape, triple: Option<&str>, rust_targets: &[&str], tools: &[&str]| ResolvedTarget {
            name: name.to_string(),
            shape,
            triple: triple.map(str::to_string),
            features: Vec::new(),
            requires_rust_targets: rust_targets.iter().map(|s| (*s).to_string()).collect(),
            requires_tools: tools.iter().map(|s| (*s).to_string()).collect(),
        };
    match name {
        "native" => Some(make(Shape::Native, None, &[], &[])),
        "html" => Some(make(
            Shape::Html,
            Some("wasm32-unknown-unknown"),
            &["wasm32-unknown-unknown"],
            // wasm-opt/binaryen is only needed for asyncify (frame-yielding
            // programs); a compute-only page builds without it, so it is a SOFT
            // requirement (warn, don't block).
            &["wasm-opt"],
        )),
        "wasi" => Some(make(
            Shape::Wasi,
            Some("wasm32-wasip2"),
            &["wasm32-wasip2"],
            &[],
        )),
        _ => None,
    }
}

/// Resolve `name` to a build target: the built-in (if any) overlaid with the
/// matching `[build.target.<name>]` entry (each field the manifest declares
/// replaces the built-in default).
///
/// # Errors
/// Returns a message when `name` is neither a built-in nor a declared target, or
/// when a declared target's `shape` is not one of `native` / `html` / `wasi`.
pub fn resolve_target(name: &str, manifest: &Manifest) -> Result<ResolvedTarget, String> {
    let decl = manifest.build_targets.iter().find(|t| t.name == name);
    let base = builtin(name);
    let Some(decl) = decl else {
        // No manifest entry — the built-in, or an error naming the built-ins.
        return base.ok_or_else(|| {
            format!(
                "unknown build target `{name}` — built-ins are native, html, wasi; \
                 declare a custom one under [build.target.{name}]"
            )
        });
    };
    // Determine the shape: an explicit `shape =`, else the built-in's, else the
    // name itself if it names a shape, else `native`.
    let shape = match decl.shape.as_deref() {
        Some(s) => Shape::from_name(s).ok_or_else(|| {
            format!("build target `{name}`: unknown shape `{s}` (use native | html | wasi)")
        })?,
        None => base
            .as_ref()
            .map(|b| b.shape)
            .or_else(|| Shape::from_name(name))
            .unwrap_or(Shape::Native),
    };
    let mut t = base.unwrap_or_else(|| ResolvedTarget {
        name: name.to_string(),
        shape,
        triple: None,
        features: Vec::new(),
        requires_rust_targets: Vec::new(),
        requires_tools: Vec::new(),
    });
    t.shape = shape;
    if decl.triple.is_some() {
        t.triple.clone_from(&decl.triple);
    }
    if !decl.features.is_empty() {
        t.features.clone_from(&decl.features);
    }
    if !decl.requires_rust_targets.is_empty() {
        t.requires_rust_targets
            .clone_from(&decl.requires_rust_targets);
    }
    if !decl.requires_tools.is_empty() {
        t.requires_tools.clone_from(&decl.requires_tools);
    }
    Ok(t)
}

/// The targets `loft build` makes with no explicit names: `default-targets` from
/// the manifest, or `["native"]` so a zero-config project still builds.
#[must_use]
pub fn default_targets(manifest: &Manifest) -> Vec<String> {
    if manifest.build_default_targets.is_empty() {
        vec!["native".to_string()]
    } else {
        manifest.build_default_targets.clone()
    }
}

/// The unmet requirements of a target: missing rustup targets (HARD) and missing
/// tools (SOFT).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Missing {
    pub rust_targets: Vec<String>,
    pub tools: Vec<String>,
}

/// Which of `t`'s requirements are unmet.  `installed_rust_targets` is `None`
/// when the rustup target list could not be queried — then the rustup-target
/// gate is skipped (can't verify → don't block; the compile reports the real
/// error).  `tool_present` answers whether a tool is on `PATH`.
#[must_use]
pub fn missing_requires(
    t: &ResolvedTarget,
    installed_rust_targets: Option<&[String]>,
    tool_present: &dyn Fn(&str) -> bool,
) -> Missing {
    let rust_targets = match installed_rust_targets {
        Some(installed) => t
            .requires_rust_targets
            .iter()
            .filter(|rt| !installed.iter().any(|i| i == *rt))
            .cloned()
            .collect(),
        None => Vec::new(),
    };
    Missing {
        rust_targets,
        tools: t
            .requires_tools
            .iter()
            .filter(|tool| !tool_present(tool))
            .cloned()
            .collect(),
    }
}

/// The rustup targets installed on this host, or `None` if `rustup` could not be
/// run (so the caller skips the rustup-target gate rather than blocking).
#[must_use]
pub fn installed_rust_targets() -> Option<Vec<String>> {
    let out = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    Some(
        text.lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect(),
    )
}

/// Whether `tool` resolves to an executable on `PATH` (portable `which`).
#[must_use]
pub fn tool_on_path(tool: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| {
        if dir.join(tool).is_file() {
            return true;
        }
        cfg!(windows)
            && (dir.join(format!("{tool}.exe")).is_file()
                || dir.join(format!("{tool}.cmd")).is_file())
    })
}

/// A short one-line description of a target for progress output.
fn describe(t: &ResolvedTarget) -> String {
    match &t.triple {
        Some(triple) => format!("{:?} · {triple}", t.shape),
        None => format!("{:?}", t.shape),
    }
}

/// Drive `loft build`: resolve `requested` (or the default targets when empty),
/// doctor-check each target's `requires`, and compile it by re-invoking this
/// loft binary with the shape's flags on `entry` (from `project_dir`).  Returns
/// `true` only if every target built.
#[must_use]
pub fn run(requested: &[String], entry: &str, manifest: &Manifest, project_dir: &Path) -> bool {
    let names: Vec<String> = if requested.is_empty() {
        default_targets(manifest)
    } else {
        requested.to_vec()
    };
    let loft_exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("loft"));
    let installed = installed_rust_targets();
    let mut all_ok = true;
    eprintln!("loft build: {} → target(s): {}", entry, names.join(", "));
    for name in &names {
        let target = match resolve_target(name, manifest) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("loft build: {e}");
                all_ok = false;
                continue;
            }
        };
        let missing = missing_requires(&target, installed.as_deref(), &|t| tool_on_path(t));
        if !missing.rust_targets.is_empty() {
            eprintln!(
                "loft build: target `{name}` cannot build — missing rustup target(s): {}.",
                missing.rust_targets.join(", ")
            );
            for rt in &missing.rust_targets {
                eprintln!("             install:  rustup target add {rt}");
            }
            all_ok = false;
            continue;
        }
        if !missing.tools.is_empty() {
            eprintln!(
                "loft build: target `{name}` — optional tool(s) not on PATH: {} \
                 (building anyway; some output may be degraded).",
                missing.tools.join(", ")
            );
        }
        eprintln!("loft build: building `{name}` ({}) …", describe(&target));
        let status = Command::new(&loft_exe)
            .args(target.shape.compile_args())
            .arg(entry)
            .current_dir(project_dir)
            .status();
        match status {
            Ok(s) if s.success() => eprintln!("loft build: `{name}` ✓"),
            Ok(_) => {
                eprintln!("loft build: target `{name}` FAILED (see errors above).");
                all_ok = false;
            }
            Err(e) => {
                eprintln!("loft build: could not launch the compiler for `{name}`: {e}");
                all_ok = false;
            }
        }
    }
    all_ok
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{BuildTarget, Manifest};

    #[test]
    fn builtins_resolve_with_known_shapes_and_requires() {
        let m = Manifest::default();
        let html = resolve_target("html", &m).unwrap();
        assert_eq!(html.shape, Shape::Html);
        assert_eq!(html.triple.as_deref(), Some("wasm32-unknown-unknown"));
        assert_eq!(html.requires_rust_targets, vec!["wasm32-unknown-unknown"]);
        assert_eq!(html.requires_tools, vec!["wasm-opt"]);

        let wasi = resolve_target("wasi", &m).unwrap();
        assert_eq!(wasi.shape, Shape::Wasi);
        assert_eq!(wasi.requires_rust_targets, vec!["wasm32-wasip2"]);

        let native = resolve_target("native", &m).unwrap();
        assert_eq!(native.shape, Shape::Native);
        assert!(native.requires_rust_targets.is_empty());
    }

    #[test]
    fn unknown_target_is_an_error() {
        let m = Manifest::default();
        assert!(resolve_target("nope", &m).is_err());
    }

    #[test]
    fn manifest_entry_overlays_the_builtin() {
        let mut m = Manifest::default();
        m.build_targets.push(BuildTarget {
            name: "html".to_string(),
            shape: None,
            triple: None,
            features: vec!["random".to_string(), "png".to_string()],
            requires_rust_targets: Vec::new(),
            requires_tools: vec!["wasm-opt".to_string(), "wasm-strip".to_string()],
        });
        let html = resolve_target("html", &m).unwrap();
        // shape/triple keep the built-in; features + tools are replaced.
        assert_eq!(html.shape, Shape::Html);
        assert_eq!(html.triple.as_deref(), Some("wasm32-unknown-unknown"));
        assert_eq!(html.features, vec!["random", "png"]);
        assert_eq!(html.requires_tools, vec!["wasm-opt", "wasm-strip"]);
    }

    #[test]
    fn custom_target_needs_a_shape() {
        let mut m = Manifest::default();
        m.build_targets.push(BuildTarget {
            name: "mobile".to_string(),
            shape: Some("html".to_string()),
            triple: Some("wasm32-unknown-unknown".to_string()),
            ..Default::default()
        });
        let t = resolve_target("mobile", &m).unwrap();
        assert_eq!(t.shape, Shape::Html);

        m.build_targets.push(BuildTarget {
            name: "weird".to_string(),
            shape: Some("banana".to_string()),
            ..Default::default()
        });
        assert!(resolve_target("weird", &m).is_err());
    }

    #[test]
    fn default_targets_falls_back_to_native() {
        let m = Manifest::default();
        assert_eq!(default_targets(&m), vec!["native"]);
        let m2 = Manifest {
            build_default_targets: vec!["native".to_string(), "html".to_string()],
            ..Default::default()
        };
        assert_eq!(default_targets(&m2), vec!["native", "html"]);
    }

    #[test]
    fn missing_requires_splits_hard_and_soft() {
        let html = resolve_target("html", &Manifest::default()).unwrap();
        // rustup target absent, wasm-opt absent → both reported.
        let installed: Vec<String> = vec![];
        let miss = missing_requires(&html, Some(&installed), &|_| false);
        assert_eq!(miss.rust_targets, vec!["wasm32-unknown-unknown"]);
        assert_eq!(miss.tools, vec!["wasm-opt"]);

        // rustup target present, wasm-opt present → nothing missing.
        let installed = vec!["wasm32-unknown-unknown".to_string()];
        let miss = missing_requires(&html, Some(&installed), &|_| true);
        assert!(miss.rust_targets.is_empty() && miss.tools.is_empty());

        // Can't query rustup → rustup-target gate skipped.
        let miss = missing_requires(&html, None, &|_| true);
        assert!(miss.rust_targets.is_empty());
    }
}
