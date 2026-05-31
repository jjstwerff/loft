<!-- Copyright (c) 2026 Jurjen Stellingwerff -->
<!-- SPDX-License-Identifier: LGPL-3.0-or-later -->

# `tests/fixtures/mock-registry/` — offline registry fixture

@PLAN12 Phase 6.12 companion to `tests/fixtures/libs/`.

Hand-crafted minimal registry that lets the registry-resolution
code paths (`registry_index::parse_index`,
`registry_advisories::classify`, `loft audit`, `loft update`,
`loft bundle import`) be tested without network access or signature
verification (test fixture is intentionally unsigned).

## Usage in tests

```rust
let mock_url = format!(
    "file://{}/tests/fixtures/mock-registry/index.json",
    env!("CARGO_MANIFEST_DIR")
);
// SAFETY: tests are single-threaded for this env dance.
unsafe { std::env::set_var("LOFT_REGISTRY_URL", &mock_url) };
// ... call loft::install::load_index() etc.
```

Set `LOFT_OFFLINE=1` to avoid any network paths, or rely on the
`file://` URL handler in `registry_index::http_get_bytes`.

## Contents

- **`index.json`** — two test packages (`test_alpha`, `test_beta`)
  with non-overlapping yanked-vs-active versions; deps wired so
  the transitive-resolution code paths can be exercised.
- **`advisories.json`** — two test advisories (`security_critical`
  + `bug`) targeting `test_alpha 0.1.0` and `test_beta 0.1.0` so
  the severity classifier can be exercised.
- **`packages/`** — empty directory (tarballs intentionally
  absent; tests exercise the resolution + classifier code paths,
  not the download/extract end-to-end).  When a test needs real
  tarballs, write them into here as part of the test setup +
  teardown.

## Tarball sha256 placeholders

All `sha256` values are `0000...0000` (64 zero hex chars) since
no actual tarball exists.  Tests that exercise sha256 verification
must EITHER build a real tarball and patch the index OR mock the
hash check at the call site.

## Schema versions

Both files use `schema_version: 1`.  Bump alongside production
when the schema evolves.
