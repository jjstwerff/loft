<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 04 — Java via jdtls

**Status:** Open (depends on phase 02; independent of phase 03)

## Goal

Light up `.java` files with the same hover / jump-to-def /
references UX rust-analyzer provides for `.rs` and loft-lsp
provides for `.loft`.  Closes the user's "potentially Java"
ask from the original 2026-05-13 framing.

Java is the language with the deepest tool ecosystem; supporting
it shows that the bridge architecture isn't loft-specific or
rust-specific — it's a real multi-language harness.  This is
the colleague-credibility phase: a Java engineer trying the
viewer immediately sees their language work.

## What ships

A jdtls adapter modelled on the rust-analyzer one, with
Java-specific complexity isolated:

```rust
// tools/loft-lsp-bridge/src/servers/jdtls.rs (NEW)
pub struct JdtlsServer {
    proc: Child,
    sender: lsp_server::Connection,
    receiver: ...
    workspace_root: PathBuf,
    initialised: bool,
    open_documents: HashMap<Url, OpenDocument>,
}
```

### Java-specific challenges

1. **JDT-LS launch is heavy**: classpath construction (~30
   JARs), workspace data dir, JVM args, equinox launcher.
   Encapsulated in `find_jdtls()` + `jdtls_launch_args()`.
2. **JDK detection**: `JAVA_HOME` env var, `/usr/lib/jvm/`
   on Linux, `/Library/Java/JavaVirtualMachines/` on macOS.
   Fall back with a clear error: "JDK not found.  Set
   `JAVA_HOME` or install OpenJDK 17+."
3. **Workspace data persistence**: jdtls writes its workspace
   index to a per-workspace data dir.  Bridge picks
   `~/.cache/loft-lsp-bridge/jdtls-workspaces/<sha256(root)>/`
   so the index survives bridge restarts (cooperates with
   phase 02's warm pool).
4. **Slower cold start**: jdtls indexes 5-30 s for typical
   projects.  Same banner UX as rust-analyzer cold start.

### Discovery

```rust
pub fn find_jdtls() -> anyhow::Result<JdtlsLayout> {
    // 1. $LOFT_JDTLS_HOME — explicit
    // 2. ~/bin/jdtls/ (manual install)
    // 3. /opt/jdtls/, /usr/share/jdtls/
    // 4. eclipse.jdt.ls release tarball auto-download (stretch — phase 04 doc)
    // Returns: { plugin_jar, config_dir, workspace_data_dir }
}
```

Auto-download (stretch): bridge fetches the latest release
tarball from `https://download.eclipse.org/jdtls/snapshots/`
on first launch if not found.  Behind a `--auto-download-jdtls`
flag — DEFAULT off; never silently downloads software.

### Acceptance

1. `make view` opened against a directory containing `.java`
   files; navigate to any `.java` file; hover over a method →
   tooltip with the Javadoc.
2. `Ctrl+Click` jumps to definition.  References sidebar
   shows callers across the project.
3. UI shape is identical to the rust + loft cases; same
   tooltip styling, same keybinds.
4. Cold start ≤ 30 s on a 100k-line Java project; warm start
   (phase 02 warm-pool hit) ≤ 2 s.
5. CI: `tests/lsp_bridge_jdtls.rs` integration test runs
   against a small fixture project (~10 classes).

### Documentation

DEBUG.md gains a § "Java code intelligence in the viewer"
with:
- JDK / jdtls install instructions per OS.
- `LOFT_JDTLS_HOME` env var reference.
- Workspace data dir layout.
- Troubleshooting: jdtls not found, workspace corrupted,
  classpath issues.

## Risks

| Risk | Mitigation |
|---|---|
| jdtls launch arguments vary by version | Detect version from `version.txt` in the install dir; pin command-line shape per major version. |
| JDK detection fragile across distros | Document the ones we test (OpenJDK 17, 21, GraalVM, Temurin); explicit `JAVA_HOME` always wins. |
| Workspace data dir grows unbounded | Per-workspace TTL (30 days idle); document the eviction. |
| Auto-download is a security concern | Keep auto-download behind a flag; default install is documentation-only.  Verify SHA256 of the downloaded tarball if auto-download ships. |
| Maven / Gradle multi-module projects need build-system integration | Phase 04 supports single-module projects only; multi-module is a follow-up.  Document the limitation. |

## Critical files

| Path | Action |
|---|---|
| `tools/loft-lsp-bridge/src/servers/jdtls.rs` | NEW |
| `tools/loft-lsp-bridge/src/servers/discover.rs` | EXTEND — `find_jdtls()` + `find_jdk()` |
| `tools/loft-lsp-bridge/src/translator.rs` | EXTEND — `JdtlsTranslator` (mostly identity; JDT URIs are spec-compliant) |
| `tools/loft-lsp-bridge/src/routing.rs` | EXTEND — `.java` → `Language::Java` |
| `tests/fixtures/lsp/java_project/` | NEW — minimal Maven-less Java project for tests |
| `tests/lsp_bridge_jdtls.rs` | NEW — integration test |
| `doc/claude/DEBUG.md` | EXTEND — § "Java code intelligence in the viewer" |

## Cross-references

- [Phase 01 — rust-analyzer](01-rust-analyzer.md) — same
  adapter shape.
- [Phase 02 — bridge intelligence](02-bridge-intelligence.md)
  — all infrastructure inherited.
- [Eclipse JDT-LS docs](https://github.com/eclipse-jdtls/eclipse.jdt.ls)
  — launch args, capability list, workspace model.
