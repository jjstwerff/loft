//! The standard-library sources (`default/*.loft`) embedded into the binary via
//! @I86 — Startup cache & embedded stdlib
//! `include_str!` — one home for the stdlib source set.
//!
//! Two consumers read the SAME embedded source: the WASM runtime loads it into
//! the browser sandbox, and `loft search` extracts the stdlib's function-level
//! API from it at search time.  Embedding (not reading `default/` from disk) is
//! the design invariant for `loft search`'s stdlib feed — a consumer never
//! extracts from source it does not have, so the stdlib surface is always
//! present and always matches this binary.

/// Every `default/*.loft` file as `(virtual-path, source)`, in stdlib load order.
pub const STDLIB_SOURCES: &[(&str, &str)] = &[
    (
        "default/01_code.loft",
        include_str!("../default/01_code.loft"),
    ),
    (
        "default/02_files.loft",
        include_str!("../default/02_files.loft"),
    ),
    (
        "default/03_text.loft",
        include_str!("../default/03_text.loft"),
    ),
    (
        "default/04_stacktrace.loft",
        include_str!("../default/04_stacktrace.loft"),
    ),
    (
        "default/05_coroutine.loft",
        include_str!("../default/05_coroutine.loft"),
    ),
    (
        "default/06_json.loft",
        include_str!("../default/06_json.loft"),
    ),
    (
        "default/07_reflect.loft",
        include_str!("../default/07_reflect.loft"),
    ),
];
