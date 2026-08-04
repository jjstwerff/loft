//! Native function registry: Rust implementations of loft built-ins.
//! Naming: `n_<name>` for globals, `t_<LEN><Type>_<method>` for methods.
// @I73 — native function registry
#![allow(non_snake_case)]
use crate::database::Stores;
use crate::keys::{DbRef, Str};
use crate::logger::Severity;
use crate::parallel::{WorkerProgram, run_parallel_text};
use crate::platform::sep;
use crate::state::{Call, State};
use crate::vector;
use std::sync::Arc;
// #620 — `wasm32-wasip2` uses the real `SystemTime` clock too, so the import
// follows the same gate as the `n_now` host arm below.
#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
use std::time::SystemTime;

/// Plan-06 phase 4d.A — typed worker-input dispatch.  Replaces the
/// sentinel-encoded `primitive_first_arg_slot_size` channel with an
/// explicit enum so the dispatcher's pre-loop code reads as a plain
/// match, and so wide-inline first args (tuples, fn-refs, anything
/// 9..=64 bytes) are not silently misrouted as "DbRef-by-pointer"
/// when their vector storage is actually inline.
///
/// Cap at 64 bytes: anything wider falls back to `Ref` so the
/// per-row push doesn't blow the worker stack frame budget.  64 is
/// one cache line plus headroom for 4-element tuples of Long /
/// Float.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum InputKind {
    /// Worker takes a `DbRef` in slot 0 (struct-by-ref, vector,
    /// keyed collection, or any inline-typed first arg whose slot
    /// width exceeds the 64-byte inline cap).
    Ref,
    /// Worker takes a 16-byte `Str` in slot 0.  The G3 path
    /// (text-input dispatch) hooks here.
    Text,
    /// Worker takes `size` bytes inline in slot 0.  `size` is the
    /// stack-representation width from `variables::size` (which
    /// reports 20 for fn-ref, 16 for `(int, int)`, etc.) — not the
    /// 4-byte vector storage width.
    Primitive { size: u8 },
}

/// Maximum bytes accepted for `InputKind::Primitive`.  Anything
/// wider falls back to `InputKind::Ref` so the worker stack frame
/// stays bounded.
const INPUT_PRIMITIVE_MAX_BYTES: usize = 64;

/// P189d — element types of a tuple first-arg, or `None` if the
/// worker's first arg isn't a tuple.  Used by the wide-input path
/// to dispatch `read_tuple_at_wide` (per-element inflation,
/// notably 4 B heap text-pointer → 16 B stack `Str`) instead of
/// `read_primitive_at_wide` (flat memcpy).
///
/// Plan-06 phase 4d.A.2: also returns `Some(vec![first.typedef])`
/// for `Type::Function` first-args so the dispatcher routes through
/// `read_tuple_at_wide` which produces a clean failure mode (vs. the
/// hang from raw `read_primitive_at_wide` reading 20 bytes from a
/// 4-byte stride).
fn tuple_first_arg_types(def: &crate::data::Definition) -> Option<Vec<crate::data::Type>> {
    let first = def.attributes.first()?;
    match &first.typedef {
        crate::data::Type::Tuple(elems) => Some(elems.clone()),
        crate::data::Type::Function(_, _, _) => Some(vec![first.typedef.clone()]),
        _ => None,
    }
}

fn input_kind_for_first_arg(def: &crate::data::Definition) -> InputKind {
    use crate::data::Type;
    let Some(first) = def.attributes.first() else {
        // No args — treat as Ref (the dispatcher won't push anything
        // either way; this matches the legacy fall-through to the
        // DbRef-passing path).
        return InputKind::Ref;
    };
    match &first.typedef {
        Type::Text(_) => InputKind::Text,
        Type::Boolean | Type::Enum(_, false, _) => InputKind::Primitive { size: 1 },
        Type::Single | Type::Character => InputKind::Primitive { size: 4 },
        Type::Integer(_) | Type::Float => InputKind::Primitive { size: 8 },
        Type::Function(_, _, _) | Type::Tuple(_) => {
            let sz = crate::variables::size(&first.typedef, &crate::data::Context::Argument);
            if (sz as usize) <= INPUT_PRIMITIVE_MAX_BYTES && sz > 0 {
                InputKind::Primitive { size: sz as u8 }
            } else {
                InputKind::Ref
            }
        }
        _ => InputKind::Ref,
    }
}

pub const FUNCTIONS: &[(&str, Call)] = &[
    ("n_assert", n_assert),
    ("n_panic", n_panic),
    ("n_log_info", n_log_info),
    ("n_log_warn", n_log_warn),
    ("n_log_error", n_log_error),
    ("n_log_fatal", n_log_fatal),
    ("n_write_text_raw", n_write_text_raw),
    ("n_env_variables", n_env_variables),
    // @PLN10 Phase 2 — env_variable dest-passing (os_variable now owns its String).
    ("n_env_variable_dest", n_env_variable_dest),
    // @PLN24 arc G — the optional-C-library availability query.
    ("n_c_library_available", n_c_library_available),
    // host_input dest-passing — reads all program input as one text (stdin on
    // native/WASI; the JS host on --html).  Non-null ("" when empty).
    ("n_host_input_dest", n_host_input_dest),
    ("n_host_output", n_host_output),
    ("t_4text_byte_at", t_4text_byte_at),
    ("t_4text_starts_with", t_4text_starts_with),
    ("t_4text_ends_with", t_4text_ends_with),
    ("t_4text_trim", t_4text_trim),
    ("t_4text_trim_start", t_4text_trim_start),
    ("t_4text_trim_end", t_4text_trim_end),
    ("t_4text_find", t_4text_find),
    ("t_4text_rfind", t_4text_rfind),
    ("t_4text_contains", t_4text_contains),
    ("t_4text_replace_dest", t_4text_replace_dest),
    ("t_4text_to_lowercase_dest", t_4text_to_lowercase_dest),
    ("t_4text_to_uppercase_dest", t_4text_to_uppercase_dest),
    // text_from_bytes: owned-text producer (vector<u8> -> text), dest-passing.
    ("n_text_from_bytes_dest", n_text_from_bytes_dest),
    // @PLN10 — destination-passing variants for the always-non-null text
    // producers; key is the loft def name + `_dest` (the lookup in
    // `gen_text_dest_call` / `try_text_dest_pass`).  Added to
    // `is_text_dest_native` so the Build-2 chokepoint routes them.
    // @PLN10 N2b — sets the destination for the NEXT cdylib FFI text return
    // (emitted right before a dest-passed cdylib text call; see
    // `gen_cdylib_text_dest_call`).  Not a text producer — a setter.
    ("n_set_bridge_dest", n_set_bridge_dest),
    ("n_source_dir_dest", n_source_dir_dest),
    ("n_os_temp_dir_dest", n_os_temp_dir_dest),
    ("n_os_cache_dir_dest", n_os_cache_dir_dest),
    ("n_json_errors_dest", n_json_errors_dest),
    ("t_9JsonValue_kind_dest", n_kind_dest),
    ("t_9JsonValue_to_json_dest", n_to_json_dest),
    ("t_9JsonValue_to_json_pretty_dest", n_to_json_pretty_dest),
    // @PLN10 Phase 2 — as_text dest-passing (null carried as the "\0" sentinel).
    ("t_9JsonValue_as_text_dest", n_as_text_dest),
    // @PLN10 Phase 1 — always-non-null `#rust`-template producers.
    ("n_ymd_days_ago_dest", n_ymd_days_ago_dest),
    ("n_store_memory_dest", n_store_memory_dest),
    // @PLN10 Phase 1 batch 2 — always-non-null codegen_runtime producers.
    ("i_parse_errors_dest", i_parse_errors_dest),
    ("n_struct_to_json_dest", n_struct_to_json_dest),
    ("n_struct_to_json_pretty_dest", n_struct_to_json_pretty_dest),
    ("t_9character_is_lowercase", t_9character_is_lowercase),
    ("t_9character_is_uppercase", t_9character_is_uppercase),
    ("t_9character_is_numeric", t_9character_is_numeric),
    ("t_9character_is_alphanumeric", t_9character_is_alphanumeric),
    ("t_9character_is_alphabetic", t_9character_is_alphabetic),
    ("t_9character_is_whitespace", t_9character_is_whitespace),
    ("t_9character_is_control", t_9character_is_control),
    ("n_arguments", n_arguments),
    ("n_mtime", n_mtime),
    ("n_is_dir", n_is_dir),
    ("n_is_file", n_is_file),
    ("n_list_dir", n_list_dir),
    ("n_read_bytes", n_read_bytes),
    ("n_write_bytes", n_write_bytes),
    #[cfg(feature = "mmap")]
    ("n_store_durable_check", n_store_durable_check),
    #[cfg(feature = "mmap")]
    ("n_store_durable_seal", n_store_durable_seal),
    #[cfg(feature = "mmap")]
    ("n_store_persist_bind", n_store_persist_bind),
    ("n_store_load", n_store_load),
    ("n_store_verify", n_store_verify),
    ("n_store_reclaim", n_store_reclaim),
    #[cfg(paged_store)]
    ("n_store_load_key", n_store_load_key),
    #[cfg(paged_store)]
    ("n_store_load_key_text", n_store_load_key_text),
    #[cfg(paged_store)]
    ("n_store_load_keys", n_store_load_keys),
    #[cfg(paged_store)]
    ("n_store_load_range", n_store_load_range),
    // Whole-image URL loads, verified and trusted. BOTH are available on the browser
    // (`--html`) target, where the fetch is bridged to JS `fetch()` via the asyncify
    // host import (same synchronous loft API as native). The verified one used to be
    // `registry`-only because `verify_sha256` lived in that module; the check needs
    // nothing but `sha2` and now sits in `crate::integrity`, so the pair no longer
    // splits by target (loft#678).
    #[cfg(any(
        feature = "registry",
        all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm"))
    ))]
    ("n_store_load_url", n_store_load_url),
    #[cfg(any(
        feature = "registry",
        all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm"))
    ))]
    ("n_store_load_url_trusted", n_store_load_url_trusted),
    ("n_store_load_untrusted", n_store_load_untrusted),
    ("n_eprint", n_eprint),
    ("n_directory", n_directory),
    ("n_user_directory", n_user_directory),
    ("n_program_directory", n_program_directory),
    ("n_get_store_lock", n_get_store_lock),
    ("n_set_store_lock", n_set_store_lock),
    ("n_protect_store_frees", n_protect_store_frees),
    ("n_unprotect_store_frees", n_unprotect_store_frees),
    ("n_yield_frame", n_yield_frame),
    ("n_parallel_for", n_parallel_for),
    ("n_parallel_for_light", n_parallel_for_light),
    ("n_parallel_discard", n_parallel_discard),
    ("n_parallel_queue", n_parallel_queue),
    ("n_parallel_fold", n_parallel_fold),
    ("n_parallel_buf_get", n_parallel_buf_get),
    ("n_parallel_buf_drop", n_parallel_buf_drop),
    ("n_parallel_queue_text", n_parallel_queue_text),
    // @PLN10 Phase 1 — dest-passing variant (interp scratch retirement).
    ("n_parallel_buf_get_text_dest", n_parallel_buf_get_text_dest),
    ("n_parallel_buf_drop_text", n_parallel_buf_drop_text),
    ("n_parallel_queue_ref", n_parallel_queue_ref),
    ("n_parallel_buf_get_ref", n_parallel_buf_get_ref),
    ("n_parallel_buf_drop_ref", n_parallel_buf_drop_ref),
    ("n_parallel_queue_narrow", n_parallel_queue_narrow),
    ("n_parallel_buf_get_narrow", n_parallel_buf_get_narrow),
    ("n_parallel_buf_drop_narrow", n_parallel_buf_drop_narrow),
    ("n_parallel_buf_get_single", n_parallel_buf_get_single),
    ("n_parallel_buf_get_float", n_parallel_buf_get_float),
    ("n_parallel_queue_fn", n_parallel_queue_fn),
    ("n_parallel_buf_get_fn", n_parallel_buf_get_fn),
    ("n_parallel_buf_drop_fn", n_parallel_buf_drop_fn),
    ("n_now", n_now),
    ("n_ticks", n_ticks),
    // @PLN18 08-S2 — live dispatch flip (no-op under the interpreter).
    ("n_live_flip", crate::live_dispatch::n_live_flip_stack),
    // @PLN18 08-S4 — background rebuild (env-driven; real on any tier).
    #[cfg(not(target_arch = "wasm32"))]
    (
        "n_rebuild_start",
        crate::live_dispatch::n_rebuild_start_stack,
    ),
    #[cfg(not(target_arch = "wasm32"))]
    (
        "n_rebuild_status",
        crate::live_dispatch::n_rebuild_status_stack,
    ),
    #[cfg(not(target_arch = "wasm32"))]
    (
        "n_kernel_rebuild_artifact_dest",
        crate::live_dispatch::n_kernel_rebuild_artifact_dest,
    ),
    // @PLN18 08-S5 — the native build swap.
    #[cfg(not(target_arch = "wasm32"))]
    ("n_swap_world", crate::engine_host::n_swap_world),
    #[cfg(not(target_arch = "wasm32"))]
    ("n_swap_start", crate::engine_host::n_swap_start),
    #[cfg(not(target_arch = "wasm32"))]
    ("n_kernel_swap_step", crate::engine_host::n_kernel_swap_step),
    #[cfg(not(target_arch = "wasm32"))]
    ("n_swap_retired", crate::engine_host::n_swap_retired),
    // @PLN18 — engine-host kernel natives (mechanics only; lib/engine_host
    // declares them; native targets only — the kernel has no wasm story).
    #[cfg(not(target_arch = "wasm32"))]
    ("n_kernel_listen", crate::engine_host::n_kernel_listen),
    #[cfg(not(target_arch = "wasm32"))]
    ("n_kernel_pump", crate::engine_host::n_kernel_pump),
    #[cfg(not(target_arch = "wasm32"))]
    (
        "n_kernel_next_event",
        crate::engine_host::n_kernel_next_event,
    ),
    #[cfg(not(target_arch = "wasm32"))]
    ("n_kernel_event_cid", crate::engine_host::n_kernel_event_cid),
    #[cfg(not(target_arch = "wasm32"))]
    (
        "n_kernel_event_kind",
        crate::engine_host::n_kernel_event_kind,
    ),
    #[cfg(not(target_arch = "wasm32"))]
    (
        "n_kernel_event_payload_dest",
        crate::engine_host::n_kernel_event_payload_dest,
    ),
    #[cfg(not(target_arch = "wasm32"))]
    ("n_kernel_tick_due", crate::engine_host::n_kernel_tick_due),
    #[cfg(not(target_arch = "wasm32"))]
    ("n_kernel_send", crate::engine_host::n_kernel_send),
    #[cfg(not(target_arch = "wasm32"))]
    (
        "n_kernel_http_fetch",
        crate::engine_host::n_kernel_http_fetch,
    ),
    #[cfg(not(target_arch = "wasm32"))]
    (
        "n_kernel_event_status",
        crate::engine_host::n_kernel_event_status,
    ),
    #[cfg(not(target_arch = "wasm32"))]
    (
        "n_kernel_client_event_status",
        crate::engine_host::n_kernel_client_event_status,
    ),
    #[cfg(not(target_arch = "wasm32"))]
    ("n_kernel_broadcast", crate::engine_host::n_kernel_broadcast),
    #[cfg(not(target_arch = "wasm32"))]
    ("n_kernel_idle", crate::engine_host::n_kernel_idle),
    #[cfg(not(target_arch = "wasm32"))]
    ("n_kernel_clients", crate::engine_host::n_kernel_clients),
    // @PLN18 phase 05a — the state-sync UDP channel.  (No cookie native: the
    // handshake cookie rides the WS 101 response as an `X-Loft-UDP` header —
    // transport negotiation is kernel-internal, invisible to loft code.)
    #[cfg(not(target_arch = "wasm32"))]
    ("n_kernel_udp_bound", crate::engine_host::n_kernel_udp_bound),
    // Pure (no sockets): registered on EVERY target — the browser kernel
    // reads the same wire-schema table.
    (
        "n_kernel_sync_class",
        crate::engine_host::n_kernel_sync_class,
    ),
    (
        "n_kernel_sync_class_keyed",
        crate::engine_host::n_kernel_sync_class_keyed,
    ),
    #[cfg(not(target_arch = "wasm32"))]
    ("n_kernel_keyframe", crate::engine_host::n_kernel_keyframe),
    #[cfg(not(target_arch = "wasm32"))]
    ("n_kernel_sync_next", crate::engine_host::n_kernel_sync_next),
    #[cfg(not(target_arch = "wasm32"))]
    ("n_kernel_sync_cid", crate::engine_host::n_kernel_sync_cid),
    #[cfg(not(target_arch = "wasm32"))]
    ("n_kernel_sync_seq", crate::engine_host::n_kernel_sync_seq),
    #[cfg(not(target_arch = "wasm32"))]
    (
        "n_kernel_sync_payload_dest",
        crate::engine_host::n_kernel_sync_payload_dest,
    ),
    // @PLN18 — the connector role (the client-side kernel).
    #[cfg(not(target_arch = "wasm32"))]
    ("n_kernel_connect", crate::engine_host::n_kernel_connect),
    #[cfg(not(target_arch = "wasm32"))]
    ("n_kernel_local", crate::engine_host::n_kernel_local),
    #[cfg(not(target_arch = "wasm32"))]
    ("n_kernel_post", crate::engine_host::n_kernel_post),
    #[cfg(not(target_arch = "wasm32"))]
    ("n_kernel_alive", crate::engine_host::n_kernel_alive),
    #[cfg(not(target_arch = "wasm32"))]
    ("n_kernel_stop", crate::engine_host::n_kernel_stop),
    ("n_kernel_frame", crate::engine_host::n_kernel_frame),
    #[cfg(not(target_arch = "wasm32"))]
    (
        "n_kernel_client_pump",
        crate::engine_host::n_kernel_client_pump,
    ),
    #[cfg(not(target_arch = "wasm32"))]
    (
        "n_kernel_client_alive",
        crate::engine_host::n_kernel_client_alive,
    ),
    #[cfg(not(target_arch = "wasm32"))]
    (
        "n_kernel_client_stop",
        crate::engine_host::n_kernel_client_stop,
    ),
    #[cfg(not(target_arch = "wasm32"))]
    (
        "n_kernel_client_next_event",
        crate::engine_host::n_kernel_client_next_event,
    ),
    #[cfg(not(target_arch = "wasm32"))]
    (
        "n_kernel_client_event_kind",
        crate::engine_host::n_kernel_client_event_kind,
    ),
    #[cfg(not(target_arch = "wasm32"))]
    (
        "n_kernel_client_event_cid",
        crate::engine_host::n_kernel_client_event_cid,
    ),
    #[cfg(not(target_arch = "wasm32"))]
    (
        "n_kernel_client_event_payload_dest",
        crate::engine_host::n_kernel_client_event_payload_dest,
    ),
    #[cfg(not(target_arch = "wasm32"))]
    (
        "n_kernel_client_tick_due",
        crate::engine_host::n_kernel_client_tick_due,
    ),
    #[cfg(not(target_arch = "wasm32"))]
    (
        "n_kernel_client_idle",
        crate::engine_host::n_kernel_client_idle,
    ),
    #[cfg(not(target_arch = "wasm32"))]
    (
        "n_kernel_client_send",
        crate::engine_host::n_kernel_client_send,
    ),
    #[cfg(not(target_arch = "wasm32"))]
    (
        "n_kernel_client_sync_next",
        crate::engine_host::n_kernel_client_sync_next,
    ),
    #[cfg(not(target_arch = "wasm32"))]
    (
        "n_kernel_client_sync_seq",
        crate::engine_host::n_kernel_client_sync_seq,
    ),
    #[cfg(not(target_arch = "wasm32"))]
    (
        "n_kernel_client_sync_payload_dest",
        crate::engine_host::n_kernel_client_sync_payload_dest,
    ),
    #[cfg(not(target_arch = "wasm32"))]
    (
        "n_kernel_client_udp_bound",
        crate::engine_host::n_kernel_client_udp_bound,
    ),
    #[cfg(not(target_arch = "wasm32"))]
    (
        "n_kernel_client_frame",
        crate::engine_host::n_kernel_client_frame,
    ),
    #[cfg(not(target_arch = "wasm32"))]
    (
        "n_kernel_default_host_dest",
        crate::engine_host::n_kernel_default_host_dest,
    ),
    ("n_stack_trace", n_stack_trace),
    ("n_reflect_type", n_reflect_type),
    ("n_type_named", n_type_named),
    ("n_path_sep", n_path_sep),
    ("i_parse_error_push", i_parse_error_push),
    ("n_hash_sorted", n_hash_sorted),
    ("n_radix_sorted", n_radix_sorted),
    ("n_spatial_range", n_spatial_range),
    ("n_hash_unsorted", n_hash_unsorted),
    // Plan-12 phase 1a (2026-05-23) — crypto `n_*` symbols
    // (`n_sha256`, `n_hmac_sha256`, `n_hmac_sha256_raw`,
    // `n_base64_encode`, `n_base64_decode`, `n_base64url_encode`)
    // moved out to `lib/crypto/native/` cdylib.  Resolved at runtime
    // via `extensions::wire_native_fns` from the loaded `loft_crypto`
    // crate (lib/crypto/loft.toml declares `native = "loft_crypto"`).
    ("n_json_parse", n_json_parse),
    ("n_json_null", n_json_null),
    ("n_json_bool", n_json_bool),
    ("n_json_number", n_json_number),
    ("n_json_string", n_json_string),
    ("n_json_array", n_json_array),
    ("n_json_object", n_json_object),
    ("n_keys", n_keys),
    ("n_fields", n_fields),
    ("n_has_field", n_has_field),
    ("n_as_number", n_as_number),
    ("n_as_long", n_as_long),
    ("n_as_bool", n_as_bool),
    ("n_field", n_field),
    ("n_item", n_item),
    ("n_len", n_len),
    ("n_struct_from_jsonvalue", n_struct_from_jsonvalue),
    // B7 (2026-04-13): when called with method syntax (`v.len()`),
    // the dispatcher resolves to `t_9JsonValue_<method>`.  Register
    // these aliases pointing at the same Rust impls so the call goes
    // through `OpStaticCall` instead of falling back to the empty-body
    // bytecode stub (which, prior to the def_code fix, double-freed
    // the JsonValue store via incorrect frame-unwind on return).
    ("t_9JsonValue_as_number", n_as_number),
    ("t_9JsonValue_as_long", n_as_long),
    ("t_9JsonValue_as_bool", n_as_bool),
    ("t_9JsonValue_field", n_field),
    ("t_9JsonValue_item", n_item),
    ("t_9JsonValue_len", n_len),
    ("t_9JsonValue_keys", n_keys),
    ("t_9JsonValue_fields", n_fields),
    ("t_9JsonValue_has_field", n_has_field),
];

// ── lib/web natives — wasm-pack build only (TTT v3.5) ────────────────────
//
// Native loft loads these from `lib/web/native/`'s cdylib at runtime.
// The wasm-pack interpreter has no dlopen, so we register them
// statically here under `cfg(all(target_arch="wasm32",
// feature="wasm"))`.  Real impls route to JS via `crate::wasm::host_*`;
// stubs (panicking on call) cover the natives v3.5 doesn't need yet
// so the loft program at least PARSES without "native function not
// loaded" tripping during compilation.
//
// Adding a new lib/web native:
//   1. Declare a Rust impl below.  `wasm` arm calls `host_*`; non-wasm
//      arm panics or returns a sensible default.
//   2. Append `(name, impl)` to `WEB_FUNCTIONS_WASM`.
//   3. JS side: extend `doc/loft-rt.js::createHost` with the matching
//      `host_*` method.

#[cfg(all(target_arch = "wasm32", feature = "wasm"))]
pub const WEB_FUNCTIONS_WASM: &[(&str, Call)] = &[
    ("n_ws_connect", n_ws_connect),
    ("n_ws_client_send", n_ws_client_send),
    ("n_ws_client_send_binary", n_ws_client_send_binary),
    ("n_ws_client_recv", n_ws_client_recv),
    ("n_ws_client_message", n_ws_client_message),
    ("n_ws_client_opcode", n_ws_client_opcode),
    ("n_ws_client_close", n_ws_client_close),
    // Non-WS lib/web natives — stubbed so a `use web` program compiles.
    // Real impls land when a v3.5+ feature actually needs them.
    ("n_http_do", n_stub_panic),
    ("n_http_body", n_stub_empty_text),
    ("n_sleep_ms", n_stub_noop),
    ("n_pack_reset", n_pack_reset),
    ("n_pack_u8", n_pack_u8),
    ("n_pack_u16_le", n_pack_u16_le),
    ("n_pack_u32_le", n_pack_u32_le),
    ("n_pack_take", n_pack_take),
    ("n_byte_at", n_byte_at),
];

#[cfg(all(target_arch = "wasm32", feature = "wasm"))]
fn n_stub_panic(_stores: &mut Stores, _stack: &mut DbRef) {
    panic!("native function not implemented in the browser-WASM build (TTT v3.5 stubs only ws_*)");
}

#[cfg(all(target_arch = "wasm32", feature = "wasm"))]
fn n_stub_empty_text(stores: &mut Stores, stack: &mut DbRef) {
    let s = crate::keys::Str::new("");
    stores.put(stack, s);
}

#[cfg(all(target_arch = "wasm32", feature = "wasm"))]
fn n_stub_noop(stores: &mut Stores, stack: &mut DbRef) {
    // Pop the i32 arg and discard (n_sleep_ms takes one integer).
    let _ = *stores.get::<i64>(stack);
}

// ── Real ws_* impls (route to JS host bridge via src/wasm.rs) ──────────

#[cfg(all(target_arch = "wasm32", feature = "wasm"))]
fn n_ws_connect(stores: &mut Stores, stack: &mut DbRef) {
    let url = *stores.get::<crate::keys::Str>(stack);
    let id = crate::wasm::host_ws_connect(url.str());
    stores.put(stack, i64::from(id));
}

#[cfg(all(target_arch = "wasm32", feature = "wasm"))]
fn n_ws_client_send(stores: &mut Stores, stack: &mut DbRef) {
    let msg = *stores.get::<crate::keys::Str>(stack);
    let id = *stores.get::<i64>(stack) as i32;
    let ok = crate::wasm::host_ws_send(id, msg.str(), false);
    stores.put(stack, ok);
}

#[cfg(all(target_arch = "wasm32", feature = "wasm"))]
fn n_ws_client_send_binary(stores: &mut Stores, stack: &mut DbRef) {
    let msg = *stores.get::<crate::keys::Str>(stack);
    let id = *stores.get::<i64>(stack) as i32;
    let ok = crate::wasm::host_ws_send(id, msg.str(), true);
    stores.put(stack, ok);
}

#[cfg(all(target_arch = "wasm32", feature = "wasm"))]
fn n_ws_client_recv(stores: &mut Stores, stack: &mut DbRef) {
    let id = *stores.get::<i64>(stack) as i32;
    let ok = crate::wasm::host_ws_recv(id);
    stores.put(stack, ok);
}

/// @PLN10 N2b (wasm tail) — put an owned-`String` text result, honouring the
/// cdylib bridge destination.  The web library's text producers (`pack_take`,
/// `ws_client_message`) carry `#native` symbols, so `is_cdylib_text_call` routes
/// them through `n_set_bridge_dest` on BOTH backends; in wasm they bind to these
/// DIRECT natives (`WEB_FUNCTIONS_WASM`) rather than the FFI bridge, so they must
/// honour `bridge_text_dest` exactly as `bridge_text_result` does — write into the
/// caller's work buffer + push nothing if set, else the legacy scratch `Str`.
#[cfg(all(target_arch = "wasm32", feature = "wasm"))]
fn put_owned_text_or_dest(stores: &mut Stores, stack: &mut DbRef, s: String) {
    if let Some(dest) = stores.bridge_text_dest.take() {
        if !s.is_empty() {
            stores
                .store_mut(&dest)
                .addr_mut::<String>(dest.rec, dest.pos)
                .push_str(&s);
        }
        return;
    }
    // @PLN10 D/G2 — see `extensions::bridge_text_result`: no dest ⇒ an uncovered
    // value position for this `#native` text producer.  Dead in the corpus;
    // degrade to an empty `Str` instead of `stores.scratch`, loud in dev.
    let _ = s;
    debug_assert!(
        false,
        "wasm #native text return reached without a dest (uncovered value \
         position) — @PLN10 N2b coverage gap"
    );
    stores.put(stack, crate::keys::Str::new(""));
}

#[cfg(all(target_arch = "wasm32", feature = "wasm"))]
fn n_ws_client_message(stores: &mut Stores, stack: &mut DbRef) {
    let msg = crate::wasm::host_ws_last_message();
    put_owned_text_or_dest(stores, stack, msg);
}

#[cfg(all(target_arch = "wasm32", feature = "wasm"))]
fn n_ws_client_opcode(stores: &mut Stores, stack: &mut DbRef) {
    let op = crate::wasm::host_ws_last_opcode();
    stores.put(stack, i64::from(op));
}

#[cfg(all(target_arch = "wasm32", feature = "wasm"))]
fn n_ws_client_close(stores: &mut Stores, stack: &mut DbRef) {
    let id = *stores.get::<i64>(stack) as i32;
    crate::wasm::host_ws_close(id);
}

// ── Pack helpers (per-thread byte buffer; pure Rust, no JS host) ───────

#[cfg(all(target_arch = "wasm32", feature = "wasm"))]
thread_local! {
    static PACK_BUF: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(all(target_arch = "wasm32", feature = "wasm"))]
fn n_pack_reset(_stores: &mut Stores, _stack: &mut DbRef) {
    PACK_BUF.with(|b| b.borrow_mut().clear());
}

#[cfg(all(target_arch = "wasm32", feature = "wasm"))]
fn n_pack_u8(_stores: &mut Stores, stack: &mut DbRef) {
    let b = *_stores.get::<i64>(stack) as i32;
    PACK_BUF.with(|buf| buf.borrow_mut().push((b & 0xff) as u8));
}

#[cfg(all(target_arch = "wasm32", feature = "wasm"))]
fn n_pack_u16_le(_stores: &mut Stores, stack: &mut DbRef) {
    let v = *_stores.get::<i64>(stack) as i32;
    PACK_BUF.with(|buf| {
        let mut buf = buf.borrow_mut();
        let v = (v & 0xffff) as u16;
        buf.extend_from_slice(&v.to_le_bytes());
    });
}

#[cfg(all(target_arch = "wasm32", feature = "wasm"))]
fn n_pack_u32_le(_stores: &mut Stores, stack: &mut DbRef) {
    let v = *_stores.get::<i64>(stack) as i32;
    PACK_BUF.with(|buf| {
        let mut buf = buf.borrow_mut();
        let v = v as u32;
        buf.extend_from_slice(&v.to_le_bytes());
    });
}

#[cfg(all(target_arch = "wasm32", feature = "wasm"))]
fn n_pack_take(stores: &mut Stores, stack: &mut DbRef) {
    let v = PACK_BUF.with(|buf| std::mem::take(&mut *buf.borrow_mut()));
    let s = unsafe { String::from_utf8_unchecked(v) };
    put_owned_text_or_dest(stores, stack, s);
}

#[cfg(all(target_arch = "wasm32", feature = "wasm"))]
fn n_byte_at(stores: &mut Stores, stack: &mut DbRef) {
    let t = *stores.get::<crate::keys::Str>(stack);
    let idx = *stores.get::<i64>(stack) as i32;
    let bytes = t.str().as_bytes();
    let result = if idx < 0 || (idx as usize) >= bytes.len() {
        -1i32
    } else {
        i32::from(bytes[idx as usize])
    };
    stores.put(stack, i64::from(result));
}

/// @PLN18 phase 07 — the BROWSER kernel: the connector role's natives over
/// the browser's own machinery, registered under the SAME symbols as the
/// native connector so `lib/engine_host` (and every script over it) is
/// shared verbatim — the script is the contract, the kernel is swappable.
#[cfg(all(target_arch = "wasm32", feature = "wasm"))]
pub const KERNEL_FUNCTIONS_WASM: &[(&str, Call)] = &[
    (
        "n_kernel_connect",
        crate::engine_host::browser::n_kernel_connect,
    ),
    (
        "n_kernel_local",
        crate::engine_host::browser::n_kernel_local,
    ),
    ("n_kernel_post", crate::engine_host::browser::n_kernel_post),
    (
        "n_kernel_frame",
        crate::engine_host::browser::n_kernel_frame,
    ),
    (
        "n_kernel_client_pump",
        crate::engine_host::browser::n_kernel_client_pump,
    ),
    (
        "n_kernel_client_alive",
        crate::engine_host::browser::n_kernel_client_alive,
    ),
    (
        "n_kernel_client_stop",
        crate::engine_host::browser::n_kernel_client_stop,
    ),
    (
        "n_kernel_client_next_event",
        crate::engine_host::browser::n_kernel_client_next_event,
    ),
    (
        "n_kernel_client_event_kind",
        crate::engine_host::browser::n_kernel_client_event_kind,
    ),
    (
        "n_kernel_client_event_cid",
        crate::engine_host::browser::n_kernel_client_event_cid,
    ),
    (
        "n_kernel_client_event_payload_dest",
        crate::engine_host::browser::n_kernel_client_event_payload_dest,
    ),
    (
        "n_kernel_client_tick_due",
        crate::engine_host::browser::n_kernel_client_tick_due,
    ),
    (
        "n_kernel_client_idle",
        crate::engine_host::browser::n_kernel_client_idle,
    ),
    (
        "n_kernel_client_send",
        crate::engine_host::browser::n_kernel_client_send,
    ),
    (
        "n_kernel_client_sync_next",
        crate::engine_host::browser::n_kernel_client_sync_next,
    ),
    (
        "n_kernel_client_sync_seq",
        crate::engine_host::browser::n_kernel_client_sync_seq,
    ),
    (
        "n_kernel_client_sync_payload_dest",
        crate::engine_host::browser::n_kernel_client_sync_payload_dest,
    ),
    (
        "n_kernel_client_udp_bound",
        crate::engine_host::browser::n_kernel_client_udp_bound,
    ),
    (
        "n_kernel_client_frame",
        crate::engine_host::browser::n_kernel_client_frame,
    ),
    (
        "n_kernel_default_host_dest",
        crate::engine_host::browser::n_kernel_default_host_dest,
    ),
    // @PLN18 08-S6 — the living-page swap (page-driven; see wasm.rs).
    ("n_swap_world", crate::engine_host::browser::n_swap_world),
    ("n_swap_start", crate::engine_host::browser::n_swap_start),
];

pub fn init(state: &mut State) {
    for (name, implement) in FUNCTIONS {
        state.static_fn(name, *implement);
    }
    // TTT v3.5 — register the lib/web natives in the wasm-pack
    // interpreter so `use web` programs run in the browser without a
    // cdylib (which the browser can't load).
    #[cfg(all(target_arch = "wasm32", feature = "wasm"))]
    for (name, implement) in WEB_FUNCTIONS_WASM {
        state.static_fn(name, *implement);
    }
    // @PLN18 phase 07 — the browser kernel (see KERNEL_FUNCTIONS_WASM).
    #[cfg(all(target_arch = "wasm32", feature = "wasm"))]
    for (name, implement) in KERNEL_FUNCTIONS_WASM {
        state.static_fn(name, *implement);
    }
}

fn n_assert(stores: &mut Stores, stack: &mut DbRef) {
    let v_line = *stores.get::<i64>(stack);
    let v_file = *stores.get::<Str>(stack);
    let v_message = *stores.get::<Str>(stack);
    let v_test = *stores.get::<bool>(stack);
    if stores.report_asserts {
        stores.assert_results.push((
            v_test,
            v_message.str().to_string(),
            v_file.str().to_string(),
            v_line as u32,
        ));
        if !v_test {
            stores.had_fatal = true;
        }
        return;
    }
    if v_test {
        return;
    }
    if let Some(ref logger) = stores.logger {
        let production = logger.lock().is_ok_and(|l| l.config.production);
        if production {
            // Plan-07 phase 4 — route through Logger::log_runtime_kind
            // so the captured-log shape matches the rest of the
            // production-mode runtime events (`[assertion_failed] …`).
            // Also produces the same severity (Error) per the C66
            // kind table.
            let kind = crate::runtime_error::RuntimeErrorKind::AssertionFailed {
                message: v_message.str().to_string(),
            };
            let pos = crate::lexer::Position {
                file: v_file.str().to_string(),
                line: v_line as u32,
                pos: 1,
            };
            if let Ok(mut lg) = logger.lock() {
                lg.log_runtime_kind(&kind, Some(&pos));
            }
            stores.had_fatal = true;
            return;
        }
    }
    // Plan-07 phase 4 — typed runtime error.  Replaces the legacy
    // Rust panic on failed assertion with a `RuntimeError` captured
    // on `Stores`; the dispatch loop in `state/mod.rs::execute_argv`
    // sees `runtime_error.is_some()` and halts gracefully, then
    // `main.rs` renders via the phase-2 pretty renderer.  Prior
    // production-mode log path above is unchanged — production
    // intentionally keeps logging-and-continuing semantics.
    stores.runtime_error = Some(Box::new(
        crate::runtime_error::RuntimeError::assertion_failed(
            v_message.str().to_string(),
            v_file.str().to_string(),
            v_line as u32,
        ),
    ));
    stores.had_fatal = true;
}

fn n_panic(stores: &mut Stores, stack: &mut DbRef) {
    let v_line = *stores.get::<i64>(stack);
    let v_file = *stores.get::<Str>(stack);
    let v_message = *stores.get::<Str>(stack);
    if let Some(ref logger) = stores.logger {
        let production = logger.lock().is_ok_and(|l| l.config.production);
        if production {
            // Plan-07 phase 4 — same routing as n_assert; ensures the
            // `[user_panic]` log entry matches the rest of the
            // production-mode runtime events.
            let kind = crate::runtime_error::RuntimeErrorKind::UserPanic {
                message: v_message.str().to_string(),
            };
            let pos = crate::lexer::Position {
                file: v_file.str().to_string(),
                line: v_line as u32,
                pos: 1,
            };
            if let Ok(mut lg) = logger.lock() {
                lg.log_runtime_kind(&kind, Some(&pos));
            }
            stores.had_fatal = true;
            return;
        }
    }
    // Plan-07 phase 4 — typed runtime error.  Same shape as n_assert
    // above; the loft `panic("msg")` builtin lands a `UserPanic`
    // variant.  See `RuntimeError::user_panic` for the constructor.
    stores.runtime_error = Some(Box::new(crate::runtime_error::RuntimeError::user_panic(
        v_message.str().to_string(),
        v_file.str().to_string(),
        v_line as u32,
    )));
    stores.had_fatal = true;
}

fn n_log_info(stores: &mut Stores, stack: &mut DbRef) {
    let v_line = *stores.get::<i64>(stack);
    let v_file = *stores.get::<Str>(stack);
    let v_message = *stores.get::<Str>(stack);
    if let Some(ref logger) = stores.logger
        && let Ok(mut lg) = logger.lock()
    {
        lg.log(Severity::Info, v_file.str(), v_line as u32, v_message.str());
    }
}

fn n_log_warn(stores: &mut Stores, stack: &mut DbRef) {
    let v_line = *stores.get::<i64>(stack);
    let v_file = *stores.get::<Str>(stack);
    let v_message = *stores.get::<Str>(stack);
    if let Some(ref logger) = stores.logger
        && let Ok(mut lg) = logger.lock()
    {
        lg.log(Severity::Warn, v_file.str(), v_line as u32, v_message.str());
    }
}

fn n_log_error(stores: &mut Stores, stack: &mut DbRef) {
    let v_line = *stores.get::<i64>(stack);
    let v_file = *stores.get::<Str>(stack);
    let v_message = *stores.get::<Str>(stack);
    if let Some(ref logger) = stores.logger
        && let Ok(mut lg) = logger.lock()
    {
        lg.log(
            Severity::Error,
            v_file.str(),
            v_line as u32,
            v_message.str(),
        );
    }
}

fn n_log_fatal(stores: &mut Stores, stack: &mut DbRef) {
    let v_line = *stores.get::<i64>(stack);
    let v_file = *stores.get::<Str>(stack);
    let v_message = *stores.get::<Str>(stack);
    if let Some(ref logger) = stores.logger
        && let Ok(mut lg) = logger.lock()
    {
        lg.log(
            Severity::Fatal,
            v_file.str(),
            v_line as u32,
            v_message.str(),
        );
    }
}

// Interpreter handler for `write_text_raw` — mirrors its `#rust` template.
// `write` (a loft fn) wraps the bool into a FileResult; this pushes the bool.
fn n_write_text_raw(stores: &mut Stores, stack: &mut DbRef) {
    let v_v = *stores.get::<Str>(stack);
    let v_file = *stores.get::<DbRef>(stack);
    let new_value = stores.write_file(&v_file, v_v.str());
    stores.put(stack, new_value);
}

fn n_env_variables(stores: &mut Stores, stack: &mut DbRef) {
    let new_value = { stores.os_variables() };
    stores.put(stack, new_value);
}

/// @PLN24 arc G — `c_library_available(soname)`.
///
/// The interpreter half of a query whose `--native` half is the `#rust` body on
/// the same declaration; both call `c_call::library_available`, so the two
/// backends cannot answer differently about the same library.
///
/// Without the `native-extensions` feature there is no `dlopen` to ask, and the
/// honest answer is false — a build that cannot load a C library certainly
/// cannot call into one.
fn n_c_library_available(stores: &mut Stores, stack: &mut DbRef) {
    let soname = *stores.get::<crate::keys::Str>(stack);
    #[cfg(feature = "native-extensions")]
    let ok = crate::c_call::library_available(soname.str());
    #[cfg(not(feature = "native-extensions"))]
    let ok = {
        let _ = soname;
        false
    };
    stores.put(stack, ok);
}

// @PLN10 Phase 2 — destination-passing variant of `n_env_variable`.
// Always-non-null (the env value, "" if unset).  Routed by `is_text_dest_native`.
fn n_env_variable_dest(stores: &mut Stores, stack: &mut DbRef) {
    let dest = *stores.get::<DbRef>(stack);
    let v_name = *stores.get::<Str>(stack);
    let value = stores.os_variable(v_name.str());
    stores
        .store_mut(&dest)
        .addr_mut::<String>(dest.rec, dest.pos)
        .push_str(&value);
}

// Reads all program input as one text: stdin on native/WASI, the JS host on
// --html (via the loft_io host-import branch in the #rust template — the
// interpreter never runs under --html, so this dest variant always reads
// stdin).  Non-null ("" when empty), so it writes straight into the caller's
// buffer.  Routed by `is_text_dest_native`.  Sibling of n_env_variable_dest.
fn n_host_output(stores: &mut Stores, stack: &mut DbRef) {
    let msg = stores.get::<Str>(stack).str().to_owned();
    stores.host_output_native(&msg);
}

fn n_host_input_dest(stores: &mut Stores, stack: &mut DbRef) {
    let dest = *stores.get::<DbRef>(stack);
    let value = stores.host_input_native();
    stores
        .store_mut(&dest)
        .addr_mut::<String>(dest.rec, dest.pos)
        .push_str(&value);
}

fn t_4text_byte_at(stores: &mut Stores, stack: &mut DbRef) {
    let v_i = *stores.get::<i64>(stack);
    let v_self = *stores.get::<Str>(stack);
    let new_value = Stores::text_byte_at_native(v_self.str(), v_i);
    stores.put(stack, new_value);
}

/// Destination-passing variant of `text_from_bytes` — the owned-text producer
/// path (`is_text_dest_native`).  Decodes the `vector<u8>` argument as UTF-8
/// and `push_str`s the result straight into the caller's return buffer,
/// avoiding the legacy scratch (`@PLN10`).  Invalid UTF-8 leaves the buffer
/// empty (the `String::from_utf8` fallback in `text_from_bytes_native`).
fn n_text_from_bytes_dest(stores: &mut Stores, stack: &mut DbRef) {
    let dest = *stores.get::<DbRef>(stack);
    let v_bytes = *stores.get::<DbRef>(stack);
    let new_value = stores.text_from_bytes_native(v_bytes);
    stores
        .store_mut(&dest)
        .addr_mut::<String>(dest.rec, dest.pos)
        .push_str(&new_value);
}

fn t_4text_starts_with(stores: &mut Stores, stack: &mut DbRef) {
    let v_value = *stores.get::<Str>(stack);
    let v_self = *stores.get::<Str>(stack);
    let new_value = { v_self.str().starts_with(v_value.str()) };
    stores.put(stack, new_value);
}

fn t_4text_ends_with(stores: &mut Stores, stack: &mut DbRef) {
    let v_value = *stores.get::<Str>(stack);
    let v_self = *stores.get::<Str>(stack);
    let new_value = { v_self.str().ends_with(v_value.str()) };
    stores.put(stack, new_value);
}

fn t_4text_trim(stores: &mut Stores, stack: &mut DbRef) {
    let v_both = *stores.get::<Str>(stack);
    let new_value = { v_both.str().trim() };
    stores.put(stack, new_value);
}

fn t_4text_trim_start(stores: &mut Stores, stack: &mut DbRef) {
    let v_self = *stores.get::<Str>(stack);
    let new_value = { v_self.str().trim_start() };
    stores.put(stack, new_value);
}

fn t_4text_trim_end(stores: &mut Stores, stack: &mut DbRef) {
    let v_self = *stores.get::<Str>(stack);
    let new_value = { v_self.str().trim_end() };
    stores.put(stack, new_value);
}

fn t_4text_find(stores: &mut Stores, stack: &mut DbRef) {
    let v_value = *stores.get::<Str>(stack);
    let v_self = *stores.get::<Str>(stack);
    let new_value: i64 = {
        if let Some(v) = v_self.str().find(v_value.str()) {
            v as i64
        } else {
            i64::MIN
        }
    };
    stores.put(stack, new_value);
}

fn t_4text_rfind(stores: &mut Stores, stack: &mut DbRef) {
    let v_value = *stores.get::<Str>(stack);
    let v_self = *stores.get::<Str>(stack);
    let new_value: i64 = {
        if let Some(v) = v_self.str().rfind(v_value.str()) {
            v as i64
        } else {
            i64::MIN
        }
    };
    stores.put(stack, new_value);
}

fn t_4text_contains(stores: &mut Stores, stack: &mut DbRef) {
    let v_value = *stores.get::<Str>(stack);
    let v_self = *stores.get::<Str>(stack);
    let new_value = { v_self.str().contains(v_value.str()) };
    stores.put(stack, new_value);
}

fn t_4text_replace_dest(stores: &mut Stores, stack: &mut DbRef) {
    let dest = *stores.get::<DbRef>(stack);
    let v_with = *stores.get::<Str>(stack);
    let v_value = *stores.get::<Str>(stack);
    let v_self = *stores.get::<Str>(stack);
    let new_value = v_self.str().replace(v_value.str(), v_with.str());
    stores
        .store_mut(&dest)
        .addr_mut::<String>(dest.rec, dest.pos)
        .push_str(&new_value);
}

fn t_4text_to_lowercase_dest(stores: &mut Stores, stack: &mut DbRef) {
    let dest = *stores.get::<DbRef>(stack);
    let v_self = *stores.get::<Str>(stack);
    let new_value = v_self.str().to_lowercase();
    stores
        .store_mut(&dest)
        .addr_mut::<String>(dest.rec, dest.pos)
        .push_str(&new_value);
}

fn t_4text_to_uppercase_dest(stores: &mut Stores, stack: &mut DbRef) {
    let dest = *stores.get::<DbRef>(stack);
    let v_self = *stores.get::<Str>(stack);
    let new_value = v_self.str().to_uppercase();
    stores
        .store_mut(&dest)
        .addr_mut::<String>(dest.rec, dest.pos)
        .push_str(&new_value);
}

fn t_9character_is_lowercase(stores: &mut Stores, stack: &mut DbRef) {
    let v_self = *stores.get::<char>(stack);
    stores.put(stack, v_self.is_lowercase());
}

fn t_9character_is_uppercase(stores: &mut Stores, stack: &mut DbRef) {
    let v_self = *stores.get::<char>(stack);
    stores.put(stack, v_self.is_uppercase());
}

fn t_9character_is_numeric(stores: &mut Stores, stack: &mut DbRef) {
    let v_self = *stores.get::<char>(stack);
    stores.put(stack, v_self.is_numeric());
}

fn t_9character_is_alphanumeric(stores: &mut Stores, stack: &mut DbRef) {
    let v_self = *stores.get::<char>(stack);
    stores.put(stack, v_self.is_alphanumeric());
}

fn t_9character_is_alphabetic(stores: &mut Stores, stack: &mut DbRef) {
    let v_self = *stores.get::<char>(stack);
    stores.put(stack, v_self.is_alphabetic());
}

fn t_9character_is_whitespace(stores: &mut Stores, stack: &mut DbRef) {
    let v_self = *stores.get::<char>(stack);
    stores.put(stack, v_self.is_whitespace());
}

fn t_9character_is_control(stores: &mut Stores, stack: &mut DbRef) {
    let v_self = *stores.get::<char>(stack);
    stores.put(stack, v_self.is_control());
}

fn n_arguments(stores: &mut Stores, stack: &mut DbRef) {
    let new_value = { stores.os_arguments() };
    stores.put(stack, new_value);
}

// @PLN10 Phase 1 — destination-passing variant of `n_ymd_days_ago`.
// Always-non-null (a date string), so the result writes straight into the
// caller's buffer instead of `stores.scratch`.  Routed by `is_text_dest_native`.
fn n_ymd_days_ago_dest(stores: &mut Stores, stack: &mut DbRef) {
    let dest = *stores.get::<DbRef>(stack);
    let v_days = *stores.get::<i64>(stack);
    let s = Stores::ymd_days_ago_native(v_days);
    stores
        .store_mut(&dest)
        .addr_mut::<String>(dest.rec, dest.pos)
        .push_str(&s);
}

// @PLN10 Phase 1 — destination-passing variant of `n_store_memory`.
// Always-non-null (a report string).  Routed by `is_text_dest_native`.
fn n_store_memory_dest(stores: &mut Stores, stack: &mut DbRef) {
    let dest = *stores.get::<DbRef>(stack);
    let report = stores.memory_report();
    stores
        .store_mut(&dest)
        .addr_mut::<String>(dest.rec, dest.pos)
        .push_str(&report);
}

fn n_mtime(stores: &mut Stores, stack: &mut DbRef) {
    let v_path = *stores.get::<Str>(stack);
    let result = Stores::os_mtime_native(v_path.str());
    stores.put(stack, result);
}

/// Interpreter handler for `is_dir` — mirrors the `#rust` template in
/// `default/02_files.loft`.  Resolves the path against the program anchor,
/// then stats it.
fn n_is_dir(stores: &mut Stores, stack: &mut DbRef) {
    let v_path = *stores.get::<Str>(stack);
    let new_value = crate::codegen_runtime::fs_is_dir(&stores.resolve_path(v_path.str()));
    stores.put(stack, new_value);
}

/// Interpreter handler for `is_file` — mirrors the `#rust` template in
/// `default/02_files.loft`.
fn n_is_file(stores: &mut Stores, stack: &mut DbRef) {
    let v_path = *stores.get::<Str>(stack);
    let new_value = crate::codegen_runtime::fs_is_file(&stores.resolve_path(v_path.str()));
    stores.put(stack, new_value);
}

/// Interpreter handler for `list_dir` — mirrors the `#rust` template in
/// `default/02_files.loft`.  `fs_list_dir` re-homes the path internally and
/// returns the `vector<text>` of sorted entry names.
fn n_list_dir(stores: &mut Stores, stack: &mut DbRef) {
    let v_path = *stores.get::<Str>(stack);
    let new_value = stores.fs_list_dir(v_path.str());
    stores.put(stack, new_value);
}

/// Interpreter handler for `read_bytes` — mirrors the `#rust` template in
/// `default/02_files.loft`.  Returns the file contents as a `vector<u8>`.
fn n_read_bytes(stores: &mut Stores, stack: &mut DbRef) {
    let v_path = *stores.get::<Str>(stack);
    let new_value = stores.fs_read_bytes(v_path.str());
    stores.put(stack, new_value);
}

/// Interpreter handler for `write_bytes` — mirrors the `#rust` template in
/// `default/02_files.loft`.  Arguments are popped in REVERSE declaration
/// order (last-pushed first), so the `vector<u8>` payload comes off the stack
/// before the path — matching the generated handler convention
/// (`move_file` pops `to` then `from`; `get_dir` pops `result` then `path`).
fn n_write_bytes(stores: &mut Stores, stack: &mut DbRef) {
    let v_bytes = *stores.get::<DbRef>(stack);
    let v_path = *stores.get::<Str>(stack);
    let new_value = stores.fs_write_bytes(v_path.str(), v_bytes);
    stores.put(stack, new_value);
}

/// @PLAN38 phase 01b — interpreter handler for `store_durable_check`.
/// Mirrors the `#rust` template in `default/02_files.loft`.
#[cfg(feature = "mmap")]
fn n_store_durable_check(stores: &mut Stores, stack: &mut DbRef) {
    let v_path = *stores.get::<Str>(stack);
    let result = crate::store::Store::durable_check(std::path::Path::new(v_path.str()));
    stores.put(stack, result);
}

/// @PLAN38 phase 01b — interpreter handler for `store_durable_seal`.
/// Mirrors the `#rust` template in `default/02_files.loft`.
#[cfg(feature = "mmap")]
fn n_store_durable_seal(stores: &mut Stores, stack: &mut DbRef) {
    let v_path = *stores.get::<Str>(stack);
    let result = crate::store::Store::durable_seal(std::path::Path::new(v_path.str()));
    stores.put(stack, result);
}

/// @PLAN38 — interpreter handler for `store_persist_bind`.  Pops a
/// path (text) + a reference (DbRef) and re-roots the slot containing
/// the reference at the file path via mmap.  Returns `true` on success.
/// See `Stores::bind_path` for the full semantics (fresh-file vs.
/// existing-file modes) and `default/02_files.loft` for the loft
/// surface.
#[cfg(feature = "mmap")]
fn n_store_persist_bind(stores: &mut Stores, stack: &mut DbRef) {
    let v_path = *stores.get::<Str>(stack);
    let v_ref = *stores.get::<DbRef>(stack);
    let ok = stores.bind_path(v_ref.store_nr, std::path::Path::new(v_path.str()));
    stores.put(stack, ok);
}

/// Interpreter handler for `store_verify` — structural integrity check of a
/// store-rooted collection's heap graph (every pointer targets a live record).
/// @PLN97. Ungated — a general integrity tool.
fn n_store_verify(stores: &mut Stores, stack: &mut DbRef) {
    let v_ref = *stores.get::<DbRef>(stack);
    let ok = stores.verify_graph_ok(&v_ref);
    stores.put(stack, ok);
}

/// Interpreter handler for `store_reclaim` — hand the store's free tail back
/// and answer with the bytes it gave.  @PLN123 A3; mirrors the `#rust` template
/// in `default/02_files.loft`.
fn n_store_reclaim(stores: &mut Stores, stack: &mut DbRef) {
    let v_ref = *stores.get::<DbRef>(stack);
    let bytes = stores.reclaim_store(v_ref.store_nr);
    stores.put(stack, bytes);
}

/// Interpreter handler for `store_load` — HEAP-load a persisted store image
/// into the referenced collection's slot (portable, non-durable).  UNGATED:
/// works without the `mmap` feature (the piece wasm lacked — a heap copy, no
/// live file handle).  See `Stores::load_path` + `default/02_files.loft`.
/// @PLN97 arc G Phase 1.
fn n_store_load(stores: &mut Stores, stack: &mut DbRef) {
    let v_path = *stores.get::<Str>(stack);
    let v_ref = *stores.get::<DbRef>(stack);
    let ok = stores.load_path(v_ref.store_nr, std::path::Path::new(v_path.str()));
    stores.put(stack, ok);
}

/// Interpreter handler for `store_load_key` — load ONE integer-keyed entry from
/// a persisted hash image, fetching only the pages the lookup touches.  Args
/// pop in reverse: key, path, local.  @PLN97 arc G Phase 3a.
#[cfg(paged_store)]
fn n_store_load_key(stores: &mut Stores, stack: &mut DbRef) {
    let v_key = *stores.get::<i64>(stack);
    let v_path = *stores.get::<Str>(stack);
    let v_ref = *stores.get::<DbRef>(stack);
    let ok = stores.load_key(&v_ref, v_path.str(), v_key);
    stores.put(stack, ok);
}

/// Interpreter handler for `store_load_key_text` — load ONE text-keyed entry.
/// Args pop in reverse: key, path, local.  @PLN97 arc G Phase 3b.6.
#[cfg(paged_store)]
fn n_store_load_key_text(stores: &mut Stores, stack: &mut DbRef) {
    let v_key = *stores.get::<Str>(stack);
    let v_path = *stores.get::<Str>(stack);
    let v_ref = *stores.get::<DbRef>(stack);
    let ok = stores.load_key_text(&v_ref, v_path.str(), v_key.str());
    stores.put(stack, ok);
}

/// Interpreter handler for `store_load_range` — load the entries with integer
/// key in [lo, hi] from a persisted SORTED collection; returns the count.  Args
/// pop in reverse: hi, lo, path, local.  @PLN97 arc G Phase 4.
#[cfg(paged_store)]
fn n_store_load_range(stores: &mut Stores, stack: &mut DbRef) {
    let v_hi = *stores.get::<i64>(stack);
    let v_lo = *stores.get::<i64>(stack);
    let v_path = *stores.get::<Str>(stack);
    let v_ref = *stores.get::<DbRef>(stack);
    let n = stores.load_range(&v_ref, v_path.str(), v_lo, v_hi);
    stores.put(stack, n);
}

/// Interpreter handler for `store_load_keys` — load the given integer keys'
/// entries from a persisted hash image, fetching only the pages the lookups
/// touch; returns the count found.  Args pop in reverse: keys, path, local.
/// @PLN97 arc G Phase 3a.
#[cfg(paged_store)]
fn n_store_load_keys(stores: &mut Stores, stack: &mut DbRef) {
    let v_keys = *stores.get::<DbRef>(stack);
    let v_path = *stores.get::<Str>(stack);
    let v_ref = *stores.get::<DbRef>(stack);
    let n = stores.load_keys_vec(&v_ref, v_path.str(), &v_keys);
    stores.put(stack, n);
}

/// Interpreter handler for `store_load_url` — fetch a persisted store IMAGE over
/// HTTP(S) (or `file://`) from a TRUSTED source, verify its SHA-256 against the
/// caller-pinned digest, and (only on a match) HEAP-load it into the collection's
/// slot.  A fetch error or hash mismatch refuses (returns false, adopts nothing).
/// Args pop in reverse: sha256, url, local.  @PLN97 arc G Phase 0.
#[cfg(any(
    feature = "registry",
    all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm"))
))]
fn n_store_load_url(stores: &mut Stores, stack: &mut DbRef) {
    let v_sha = *stores.get::<Str>(stack);
    let v_url = *stores.get::<Str>(stack);
    let v_ref = *stores.get::<DbRef>(stack);
    let ok = stores.load_url_verified(v_ref.store_nr, v_url.str(), v_sha.str());
    stores.put(stack, ok);
}

/// Interpreter handler for `store_load_url_trusted` — fetch a whole store IMAGE
/// over HTTP(S)/`file://` from a TRUSTED source and HEAP-load it (no SHA check;
/// still structurally validated).  Args pop in reverse: url, local.
/// @PLN97 arc G Phase 0.  Available on native (`registry`) AND the browser
/// (`--html`) target — the fetch is bridged to JS `fetch()` via the asyncify host
/// import, so the loft API is identical (`Stores::load_url` / `net::fetch_bytes`).
#[cfg(any(
    feature = "registry",
    all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm"))
))]
fn n_store_load_url_trusted(stores: &mut Stores, stack: &mut DbRef) {
    let v_url = *stores.get::<Str>(stack);
    let v_ref = *stores.get::<DbRef>(stack);
    let ok = stores.load_url(v_ref.store_nr, v_url.str());
    stores.put(stack, ok);
}

/// Interpreter handler for `store_load_untrusted` — HEAP-load a store IMAGE from
/// a local file that may be UNTRUSTED, structurally validated before adoption
/// (rejects a crafted / corrupt image instead of hanging or over-reading).  Args
/// pop in reverse: path, local.  @PLN97 arc G Phase 2.
fn n_store_load_untrusted(stores: &mut Stores, stack: &mut DbRef) {
    let v_path = *stores.get::<Str>(stack);
    let v_ref = *stores.get::<DbRef>(stack);
    let ok = stores.load_path_untrusted(v_ref.store_nr, std::path::Path::new(v_path.str()));
    stores.put(stack, ok);
}

/// Write `text` to stderr — companion to `print()` / `println()`
/// which both go to stdout.  Use to separate machine-readable
/// output (JSON, structured data) on stdout from human-readable
/// status / summary / progress lines.  Driver: @PLAN37 phase 07's
/// `scan.loft` cutover writes `index/tags.json` to stdout, summary
/// stats to stderr — `make index > index/tags.json` then puts JSON
/// in the file while still showing the summary on screen.
fn n_eprint(stores: &mut Stores, stack: &mut DbRef) {
    let v = *stores.get::<Str>(stack);
    eprint!("{}", v.str());
}

fn n_directory(stores: &mut Stores, stack: &mut DbRef) {
    let v_v = *stores.get::<DbRef>(stack);
    let v_v = stores.store_mut(&v_v).addr_mut::<String>(v_v.rec, v_v.pos);
    let new_value = { Stores::os_directory(v_v) };
    stores.put(stack, new_value);
}

fn n_user_directory(stores: &mut Stores, stack: &mut DbRef) {
    let v_v = *stores.get::<DbRef>(stack);
    let v_v = stores.store_mut(&v_v).addr_mut::<String>(v_v.rec, v_v.pos);
    let new_value = { Stores::os_home(v_v) };
    stores.put(stack, new_value);
}

fn n_program_directory(stores: &mut Stores, stack: &mut DbRef) {
    let v_v = *stores.get::<DbRef>(stack);
    let v_v = stores.store_mut(&v_v).addr_mut::<String>(v_v.rec, v_v.pos);
    let new_value = { Stores::os_executable(v_v) };
    stores.put(stack, new_value);
}

// @PLN10 — destination-passing variant: write straight into the caller's
// buffer instead of `stores.scratch`.  Routed by `is_text_dest_native`.
fn n_source_dir_dest(stores: &mut Stores, stack: &mut DbRef) {
    let dest = *stores.get::<DbRef>(stack);
    let v = stores.source_dir.clone();
    stores
        .store_mut(&dest)
        .addr_mut::<String>(dest.rec, dest.pos)
        .push_str(&v);
}

// #635 — interpreter backends for the private `os_temp_dir()` / `os_cache_dir()`
// natives the `temp_dir()` / `cache_dir()` wrappers call. Text dest-passing
// (routed by `is_text_dest_native`), always non-null ("" only on a filesystem-less
// target; the loft wrapper maps "" to null).
fn n_os_temp_dir_dest(stores: &mut Stores, stack: &mut DbRef) {
    let dest = *stores.get::<DbRef>(stack);
    let v = Stores::os_temp_dir_native();
    stores
        .store_mut(&dest)
        .addr_mut::<String>(dest.rec, dest.pos)
        .push_str(&v);
}

fn n_os_cache_dir_dest(stores: &mut Stores, stack: &mut DbRef) {
    let dest = *stores.get::<DbRef>(stack);
    let v = Stores::os_cache_dir_native();
    stores
        .store_mut(&dest)
        .addr_mut::<String>(dest.rec, dest.pos)
        .push_str(&v);
}

/// @PLN10 N2b — set the destination record for the NEXT cdylib FFI text return.
/// `gen_cdylib_text_dest_call` emits an `OpStaticCall` to this immediately before
/// the cdylib's own `OpStaticCall`; it pops the work-buffer `DbRef` off the stack
/// and stashes it on `stores` so the bridge text path (`bridge_push_str` /
/// `push_loft_str`) writes the foreign `LoftStr` into that record instead of the
/// never-cleared `stores.scratch`.  Pushes nothing.
fn n_set_bridge_dest(stores: &mut Stores, stack: &mut DbRef) {
    let dest = *stores.get::<DbRef>(stack);
    stores.bridge_text_dest = Some(dest);
}

/// Read the lock state of the store that owns the record pointed to by `r`.
fn n_get_store_lock(stores: &mut Stores, stack: &mut DbRef) {
    let r = *stores.get::<DbRef>(stack);
    let locked = stores.is_store_locked(&r);
    stores.put(stack, locked);
}

/// Lock (or unlock) the store that owns the record pointed to by `r`.
/// From loft, only `d#lock = true` is accepted by the parser; `false` is only
/// reachable here if the variable is not marked `const`.
fn n_set_store_lock(stores: &mut Stores, stack: &mut DbRef) {
    let locked = *stores.get::<bool>(stack);
    let r = *stores.get::<DbRef>(stack);
    if locked {
        stores.lock_store(&r);
    } else {
        stores.unlock_store(&r);
    }
}

/// @P290 — soft "don't free" marker for the fn-call deep-copy bracket.
/// Sets `free_protected = true` so `OpCopyRecord`'s `0x8000` source-free
/// can't free a caller's arg if the callee returned a borrowed view —
/// writes / claims from the callee stay legal (unlike `lock_store`,
/// which is the hard user-facing `d#lock` tripwire that blocks writes).
fn n_protect_store_frees(stores: &mut Stores, stack: &mut DbRef) {
    let r = *stores.get::<DbRef>(stack);
    if r.rec != 0 && (r.store_nr as usize) < stores.allocations.len() {
        let origin = format!("call_bracket(store_nr={}, rec={})", r.store_nr, r.rec);
        stores.allocations[r.store_nr as usize].set_free_protected(origin);
    }
}

/// @P290 — clear the soft free-protection set by `n_protect_store_frees`.
fn n_unprotect_store_frees(stores: &mut Stores, stack: &mut DbRef) {
    let r = *stores.get::<DbRef>(stack);
    if r.rec != 0 && (r.store_nr as usize) < stores.allocations.len() {
        stores.allocations[r.store_nr as usize].clear_free_protected();
    }
}

/// Yield control back to the host frame loop.  Sets `frame_yield = true`,
/// which causes `State::execute_argv` to return after the current opcode
/// completes; the host then drives the next frame (browser raf in WASM,
/// `state.resume()` loop in native CLI / tests, etc.).  Mirrors the
/// `gl_swap_buffers` mechanism without requiring a graphics backend, so
/// frame-driven loft programs are testable in pure interpreter mode.
fn n_yield_frame(stores: &mut Stores, _stack: &mut DbRef) {
    stores.frame_yield = true;
}

// ── Parallel threading functions (feature = "threading") ──────────────

/// Plan-06 ARC.md A4 (closed 2026-05-07) — the legacy materialised
/// `parallel_for` path is unreachable.  After A3.6, every primitive
/// return type — including Single (4-byte f32) and Float (8-byte
/// f64) — routes through the Queue family (`n_parallel_queue` /
/// `_narrow` / `_text` / `_ref` / `_fn`).  The materialised path
/// (`Stitch::Concat`) had only one consumer in the parser (the
/// `actual_par_d_nr` resolution in `build_parallel_for_ir`) which is
/// now dead.  The function entry is retained in the FUNCTIONS table
/// for stack-trace symbol resolution but its body panics if invoked.
fn n_parallel_for(_stores: &mut Stores, _stack: &mut DbRef) {
    unreachable!(
        "n_parallel_for: ARC.md A4 retired the materialised path; \
         all par() routes through the Queue family (n_parallel_queue / \
         _narrow / _text / _ref / _fn).  If you reached this, the \
         parser failed to route the worker's return type to a Queue \
         variant — extend `narrow_route_for` / the route_*_queue \
         gates in src/parser/collections.rs."
    );
}

/// Plan-06 ARC.md A4 (closed 2026-05-07) — companion to
/// `n_parallel_for`.  Was the per-worker-pool variant; same fate
/// after A3.6 routes Single/Float through Queue.
fn n_parallel_for_light(_stores: &mut Stores, _stack: &mut DbRef) {
    unreachable!(
        "n_parallel_for_light: ARC.md A4 retired the light Concat \
         path.  See `n_parallel_for` panic message for the diagnosis \
         hint."
    );
}

/// Plan-06 PRIORITY.md spine step 3 — `Stitch::Discard` runtime entry.
/// Pops the same arg layout as `n_parallel_for` (input, element_size,
/// return_size, threads, func, extras..., n_extra), dispatches the
/// worker fn per row via `run_parallel_discard` (step 2), drops every
/// result.  No result vector allocated; nothing pushed back onto the
/// stack — void return.
///
/// Used when a fused for-par body never references the worker result
/// (`for x in input par(_=fn(x), N) { }` or
/// `for x in input par(r=fn(x), N) { /* no use of r */ }`).  Phase
/// 10 (drop materialised vector) extends this contract: any par call
/// whose result is consumed single-pass without random access lowers
/// here.
fn n_parallel_discard(stores: &mut Stores, stack: &mut DbRef) {
    // Same stack layout / pop order as n_parallel_for.
    let n_extra = *stores.get::<i64>(stack) as usize;
    let mut extra_args: Vec<u64> = Vec::with_capacity(n_extra);
    for _ in 0..n_extra {
        extra_args.push(*stores.get::<i64>(stack) as u64);
    }
    extra_args.reverse();

    let v_func = *stores.get::<i64>(stack) as i32;
    let v_threads = *stores.get::<i64>(stack) as i32;
    let v_return_size = *stores.get::<i64>(stack) as i32;
    let v_element_size = *stores.get::<i64>(stack) as i32;
    let v_input = *stores.get::<DbRef>(stack);

    let (fn_pos, program) = {
        let ctx = stores
            .parallel_ctx
            .as_ref()
            .expect("parallel_discard called outside State::execute()");
        let data = unsafe { &*ctx.data };
        assert!(
            v_func >= 0,
            "parallel_discard: invalid function reference {v_func}"
        );
        let d_nr = v_func as u32;
        let fn_pos = data.def(d_nr).code_position;
        let bytecode = unsafe { Arc::clone(&*ctx.bytecode) };
        let library = unsafe { Arc::clone(&*ctx.library) };
        (
            fn_pos,
            WorkerProgram {
                bytecode,
                library,
                stack_trace_lib_nr: ctx.stack_trace_lib_nr,
                data_ptr: ctx.data,
                fn_positions: Arc::new(data.definitions.iter().map(|d| d.code_position).collect()),
                line_numbers: Arc::new(std::collections::BTreeMap::new()),
            },
        )
    };

    let element_size = v_element_size as u32;
    let n_threads = (v_threads as usize).max(1);
    // return_size is needed by execute_at_raw to drain the worker's
    // return value off the stack.  Discard accepts any return shape;
    // clamp to the same 1..=8 backstop the light path uses.
    let return_size = {
        let ctx = stores
            .parallel_ctx
            .as_ref()
            .expect("parallel_discard: missing context");
        let data = unsafe { &*ctx.data };
        let def = data.def(v_func as u32);
        let derived = u32::from(crate::variables::size(
            &def.returned,
            &crate::data::Context::Argument,
        ));
        if (1..=8).contains(&derived) {
            derived
        } else {
            (v_return_size.clamp(1, 8)) as u32
        }
    };

    crate::parallel::run_parallel_discard(
        stores,
        program,
        fn_pos,
        &v_input,
        element_size,
        n_threads,
        &extra_args,
        return_size,
    );
    // Void return — nothing to push.  Note: post-2c integers are 8B,
    // so callers expecting a return on the stack would be wrong to
    // invoke this; the codegen path that emits the call sites this
    // fn knows it returns void and does not pop a result.
}

/// Plan-06 ARC.md A8.b — discriminator for the shared queue
/// dispatcher.  Each variant matches one of the 5 `n_parallel_queue*`
/// native fns; the per-stitch behaviour lives in
/// `parallel_queue_dispatch`'s match arms (dispatcher choice +
/// post-processing + per-type buffer-stack push).
#[derive(Copy, Clone)]
enum QueueStitch {
    Int,
    Text,
    Ref,
    Narrow,
    Fn,
}

/// Plan-06 ARC.md A8.b — shared queue dispatcher.  Replaces the
/// boilerplate that was duplicated across `n_parallel_queue`,
/// `_text`, `_ref`, `_narrow`, and `_fn`.  Pops the common 7 args
/// from the runtime stack (n_extra, extras, fn, threads,
/// return_size, element_size, input), builds a `WorkerProgram` +
/// per-stitch context fetches (n_hidden_text, n_hidden_dests,
/// ret_type, data_ptr, primitive_input_size, tuple_input_types,
/// worker_return_size) in ONE parallel_ctx borrow scope, then
/// dispatches via match on `stitch`.
///
/// The 5 native fns each become a 1-line wrapper calling this with
/// the matching `QueueStitch` value.  Net ~150-200 LOC saved.
///
/// ARC.md A8 (collapse the 5 `run_parallel_*` dispatchers in
/// `src/parallel.rs` under one trait) was deferred — those
/// dispatchers diverge structurally.  A8.b targets a different
/// layer (interp-bridge native fns) where the divergence IS
/// boilerplate.
#[allow(clippy::too_many_lines)]
fn parallel_queue_dispatch(stores: &mut Stores, stack: &mut DbRef, stitch: QueueStitch) {
    let fn_label = match stitch {
        QueueStitch::Int => "parallel_queue",
        QueueStitch::Text => "parallel_queue_text",
        QueueStitch::Ref => "parallel_queue_ref",
        QueueStitch::Narrow => "parallel_queue_narrow",
        QueueStitch::Fn => "parallel_queue_fn",
    };

    // Pop common args (same layout as legacy n_parallel_for).
    let n_extra = *stores.get::<i64>(stack) as usize;
    let mut extra_args: Vec<u64> = Vec::with_capacity(n_extra);
    for _ in 0..n_extra {
        extra_args.push(*stores.get::<i64>(stack) as u64);
    }
    extra_args.reverse();

    let v_func = *stores.get::<i64>(stack) as i32;
    let v_threads = *stores.get::<i64>(stack) as i32;
    let v_return_size = *stores.get::<i64>(stack) as i32;
    let v_element_size = *stores.get::<i64>(stack) as i32;
    let v_input = *stores.get::<DbRef>(stack);

    // Build (fn_pos, program) + per-stitch context fetches in one
    // parallel_ctx borrow scope.  Snapshot raw `data_ptr` so the
    // Ref stitch's downstream call (which needs `&mut Stores`) can
    // dereference after we give up the borrow.  The ParallelCtx
    // outlives this fn's stack frame (set by State::execute before
    // any par fn dispatches).
    let (
        fn_pos,
        program,
        n_hidden_text,
        n_hidden_dests,
        ret_type,
        data_ptr,
        primitive_input_size,
        tuple_input_types,
        worker_return_size,
    ) = {
        let ctx = stores
            .parallel_ctx
            .as_ref()
            .unwrap_or_else(|| panic!("{fn_label} called outside State::execute()"));
        let data = unsafe { &*ctx.data };
        assert!(
            v_func >= 0,
            "{fn_label}: invalid function reference {v_func}"
        );
        let d_nr = v_func as u32;
        let def = data.def(d_nr);
        let fn_pos = def.code_position;
        let bytecode = unsafe { Arc::clone(&*ctx.bytecode) };
        let library = unsafe { Arc::clone(&*ctx.library) };
        let program = WorkerProgram {
            bytecode,
            library,
            stack_trace_lib_nr: ctx.stack_trace_lib_nr,
            data_ptr: ctx.data,
            fn_positions: Arc::new(data.definitions.iter().map(|d| d.code_position).collect()),
            line_numbers: Arc::new(std::collections::BTreeMap::new()),
        };
        // Per-stitch extras (computed unconditionally; cost is a
        // few attribute reads, not heap allocations).
        // @PLAN59: classify worker attrs by TYPE, not name prefix.  The
        // old prefix heuristic ('__'-named ⇒ text buffer, user-named
        // hidden ⇒ heap dest) misclassified wrapper-promoted dests (named
        // `__ref_1`, Reference/Vector-typed) as text buffers — a live
        // frame-underflow panic for par workers calling wrapper-promoted
        // fns ('No elements left on the stack 8 < 12').
        let n_hidden_text = def
            .attributes
            .iter()
            .filter(|a| crate::native_lib::is_text_work_buffer(&a.typedef))
            .count();
        let n_hidden_dests = def
            .attributes
            .iter()
            .filter(|a| {
                a.hidden
                    && matches!(
                        a.typedef,
                        crate::data::Type::Reference(_, _)
                            | crate::data::Type::Vector(_, _)
                            | crate::data::Type::Enum(_, true, _)
                    )
            })
            .count();
        let ret_type = def.returned.clone();
        let primitive_input_size = match input_kind_for_first_arg(def) {
            InputKind::Ref => 0u32,
            InputKind::Text => u32::MAX,
            InputKind::Primitive { size } => u32::from(size),
        };
        let tuple_input_types = tuple_first_arg_types(def);
        let worker_return_size = u32::from(crate::variables::size(
            &def.returned,
            &crate::data::Context::Argument,
        ));
        (
            fn_pos,
            program,
            n_hidden_text,
            n_hidden_dests,
            ret_type,
            ctx.data,
            primitive_input_size,
            tuple_input_types,
            worker_return_size,
        )
    };

    let element_size = v_element_size as u32;
    let n_threads = (v_threads as usize).max(1);

    // Per-stitch dispatcher call + buffer-stack push.
    let n_rows: i64 = match stitch {
        QueueStitch::Int => {
            // Plan-06 spine step 8b' — derive return_size from the
            // worker fn's def so primitive / text / tuple-input
            // workers dispatch through the right input arm.
            let return_size = if (1..=8).contains(&worker_return_size) {
                worker_return_size
            } else {
                (v_return_size.clamp(1, 8)) as u32
            };
            let buf = crate::parallel::run_parallel_queue(
                stores,
                program,
                fn_pos,
                &v_input,
                element_size,
                n_threads,
                &extra_args,
                return_size,
                primitive_input_size,
                tuple_input_types,
            );
            let n = buf.len() as i64;
            stores.par_buffer_stack.push(buf);
            n
        }
        QueueStitch::Text => {
            let _ = v_return_size; // text mode ignores caller-supplied size
            let n_rows = vector::length_vector(&v_input, &stores.allocations) as usize;
            let buf = run_parallel_text(
                stores,
                program,
                fn_pos,
                &v_input,
                element_size,
                n_threads,
                &extra_args,
                n_rows,
                n_hidden_text,
                primitive_input_size,
            );
            let n = buf.len() as i64;
            stores.par_text_buffer_stack.push(buf);
            n
        }
        QueueStitch::Ref => {
            let _ = v_return_size; // ref mode uses ret_type directly
            let n_rows = crate::vector::length_vector(&v_input, &stores.allocations) as usize;
            // SAFETY: data_ptr was snapshotted from parallel_ctx
            // above; the ParallelCtx outlives this stack frame.
            let data: &crate::data::Data = unsafe { &*data_ptr };
            let (refs, adopted) = crate::parallel::run_parallel_queue_ref(
                stores,
                program,
                fn_pos,
                &v_input,
                element_size,
                n_threads,
                &extra_args,
                n_rows,
                &ret_type,
                data,
                n_hidden_dests,
                primitive_input_size,
            );
            let n = refs.len() as i64;
            stores.par_ref_buffer_stack.push((refs, adopted));
            n
        }
        QueueStitch::Narrow => {
            // Plan-06 ARC.md A3 / A3.5 — workers run with the
            // SHAPE-NATURAL return size; the pack loop below
            // truncates each u64 row to the buffer stride.
            let stride = match v_return_size {
                1 | 2 | 4 => v_return_size as u8,
                other => {
                    panic!("parallel_queue_narrow: invalid return_size {other} (expected 1/2/4)")
                }
            };
            let buf64 = crate::parallel::run_parallel_queue(
                stores,
                program,
                fn_pos,
                &v_input,
                element_size,
                n_threads,
                &extra_args,
                worker_return_size,
                primitive_input_size,
                tuple_input_types,
            );
            let mut bytes = Vec::with_capacity(buf64.len() * stride as usize);
            for u in &buf64 {
                let row_bytes = u.to_le_bytes();
                bytes.extend_from_slice(&row_bytes[..stride as usize]);
            }
            let n = buf64.len() as i64;
            stores.par_narrow_buffer_stack.push((bytes, stride));
            n
        }
        QueueStitch::Fn => {
            let _ = v_return_size; // always 20 for fn-ref
            let buf = crate::parallel::run_parallel_queue_fn(
                stores,
                &program,
                fn_pos,
                &v_input,
                element_size,
                n_threads,
                &extra_args,
            );
            let n = (buf.len() / 20) as i64;
            stores.par_fn_buffer_stack.push(buf);
            n
        }
    };

    stores.put(stack, n_rows);
}

/// Plan-06 PRIORITY.md spine step 8a — `Stitch::Queue` runtime entry.
/// Pops the same arg layout as `n_parallel_for` (input, element_size,
/// return_size, threads, func, extras..., n_extra), dispatches the
/// worker fn per row via `run_parallel_queue` (step 4), and pushes
/// the resulting `Vec<u64>` onto `stores.par_buffer_stack`.  The
/// row count is pushed onto the operand stack so the caller can use
/// it as a loop bound.
///
/// Step 8b is the first parser-side consumer; until then the only
/// caller is the Rust unit test in `tests/threading.rs`.
///
/// Plan-06 ARC.md A8.b: body now delegates to
/// `parallel_queue_dispatch` — the shared scaffolding that was
/// duplicated across all 5 `n_parallel_queue*` fns.
fn n_parallel_queue(stores: &mut Stores, stack: &mut DbRef) {
    parallel_queue_dispatch(stores, stack, QueueStitch::Int);
}

/// Plan-06 ARC.md A5 — `Stitch::Reduce` runtime entry.
///
/// V1 stack layout (LIFO pop): `n_extra` (i64), N extras (i64), threads
/// (i32), fold_fn_d_nr (i32), init (i64), input (DbRef).
///
/// `fold_fn` must be declared as `fn fold(acc: integer, row: integer) ->
/// integer`; the runtime invokes it once per row to update each worker's
/// accumulator, then combines per-worker partials with the same fn.
/// Restricts to `vector<integer>` input — see ARC.md A5 for the future
/// extension story.
fn n_parallel_fold(stores: &mut Stores, stack: &mut DbRef) {
    let n_extra = *stores.get::<i64>(stack) as usize;
    let mut extra_args: Vec<u64> = Vec::with_capacity(n_extra);
    for _ in 0..n_extra {
        extra_args.push(*stores.get::<i64>(stack) as u64);
    }
    extra_args.reverse();

    let v_threads = *stores.get::<i64>(stack) as i32;
    let v_func = *stores.get::<i64>(stack) as i32;
    let v_init = *stores.get::<i64>(stack);
    let v_input = *stores.get::<DbRef>(stack);

    let (fn_pos, program, element_size) = {
        let ctx = stores
            .parallel_ctx
            .as_ref()
            .expect("parallel_fold called outside State::execute()");
        let data = unsafe { &*ctx.data };
        assert!(
            v_func >= 0,
            "parallel_fold: invalid function reference {v_func}"
        );
        let d_nr = v_func as u32;
        let fn_pos = data.def(d_nr).code_position;
        let bytecode = unsafe { Arc::clone(&*ctx.bytecode) };
        let library = unsafe { Arc::clone(&*ctx.library) };
        // V1: input is vector<integer>; element_size = 8.  Future
        // versions might derive this from the worker's row arg type.
        let elem_size = 8u32;
        (
            fn_pos,
            WorkerProgram {
                bytecode,
                library,
                stack_trace_lib_nr: ctx.stack_trace_lib_nr,
                data_ptr: ctx.data,
                fn_positions: Arc::new(data.definitions.iter().map(|d| d.code_position).collect()),
                line_numbers: Arc::new(std::collections::BTreeMap::new()),
            },
            elem_size,
        )
    };

    let n_threads = (v_threads as usize).max(1);
    let result = crate::parallel::run_parallel_fold(
        stores,
        program,
        fn_pos,
        &v_input,
        element_size,
        v_init,
        n_threads,
        &extra_args,
    );
    stores.put(stack, result);
}

/// Plan-06 spine step 8a — read one element from the active par buffer.
///
/// Pops `idx` (i64), reads `stores.par_buffer_stack.last()[idx]`, and
/// pushes the i64 value onto the operand stack.  The caller is
/// responsible for masking to the worker's actual return width when
/// less than 8 bytes — for narrow primitives the high bits are zero
/// (workers store via `to_le_bytes` into a u64 slot).
///
/// Panics if `par_buffer_stack` is empty (no active queue) or if
/// `idx` is out of range — both indicate a parser-side bug.
fn n_parallel_buf_get(stores: &mut Stores, stack: &mut DbRef) {
    let idx = *stores.get::<i64>(stack);
    let buf = stores
        .par_buffer_stack
        .last()
        .expect("parallel_buf_get: par_buffer_stack is empty");
    let val = buf[idx as usize] as i64;
    stores.put(stack, val);
}

/// Plan-06 spine step 8a — pop the active par buffer.  Called after
/// the body loop completes.  Panics if the stack is already empty
/// (parser-side bug).  Void return.
fn n_parallel_buf_drop(stores: &mut Stores, _stack: &mut DbRef) {
    stores
        .par_buffer_stack
        .pop()
        .expect("parallel_buf_drop: par_buffer_stack is already empty");
}

/// Plan-06 spine step 8c — text-return Queue runtime entry.  Same
/// arg layout as `n_parallel_queue`, but workers return owned
/// `String`s collected via `run_parallel_text` (which routes through
/// the per-worker output Store slot machinery — same path text
/// returns always use inside loft).  The resulting `Vec<String>` is
/// pushed onto `stores.par_text_buffer_stack`; the row count is
/// pushed onto the operand stack so the caller can use it as a
/// loop bound.
///
/// Step 8c's parser rewrite extends the gate to text returns so
/// fused for-par over text-returning workers no longer allocates a
/// heap text-vector.  Reads use `n_parallel_buf_get_text` (clones
/// into scratch following the standard text-return convention).
fn n_parallel_queue_text(stores: &mut Stores, stack: &mut DbRef) {
    parallel_queue_dispatch(stores, stack, QueueStitch::Text);
}

/// @PLN10 Phase 1 — destination-passing variant of `n_parallel_buf_get_text`.
/// Always-non-null (clones an owned `String` from the par text buffer).
/// Routed by `is_text_dest_native`.
fn n_parallel_buf_get_text_dest(stores: &mut Stores, stack: &mut DbRef) {
    let dest = *stores.get::<DbRef>(stack);
    let idx = *stores.get::<i64>(stack);
    let s_owned = {
        let buf = stores
            .par_text_buffer_stack
            .last()
            .expect("parallel_buf_get_text: par_text_buffer_stack is empty");
        buf[idx as usize].clone()
    };
    stores
        .store_mut(&dest)
        .addr_mut::<String>(dest.rec, dest.pos)
        .push_str(&s_owned);
}

/// Plan-06 spine step 8c — pop the active par text buffer.  Called
/// after the body loop completes.  Panics if the stack is already
/// empty (parser-side bug).  Void return.
fn n_parallel_buf_drop_text(stores: &mut Stores, _stack: &mut DbRef) {
    stores
        .par_text_buffer_stack
        .pop()
        .expect("parallel_buf_drop_text: par_text_buffer_stack is already empty");
}

/// Plan-06 spine step 8d.1 — reference-return Queue runtime entry.
/// Same arg layout as `n_parallel_queue` but workers return a
/// `DbRef` into their own output stores.  `run_parallel_queue_ref`
/// (8d.0) adopts each worker's output stores into the parent's
/// allocations table and rebases the per-row `DbRef`s into the
/// parent's namespace via `Stores::adopt_worker_excess` +
/// `rebase_walk_record`.  The resulting `(refs, adopted_stores)`
/// pair is pushed onto `stores.par_ref_buffer_stack`; the row count
/// is pushed onto the operand stack so the caller can use it as a
/// loop bound.
///
/// Step 8d.2 wires this through the parser; until then this is
/// exercised only by Rust unit tests in `tests/threading.rs`.
fn n_parallel_queue_ref(stores: &mut Stores, stack: &mut DbRef) {
    parallel_queue_dispatch(stores, stack, QueueStitch::Ref);
}

/// Plan-06 spine step 8d.1 — read one rebased `DbRef` from the
/// active par-ref buffer.  Pops `idx` (i64), reads
/// `par_ref_buffer_stack.last().0[idx]`, pushes the `DbRef` (12
/// bytes) onto the operand stack.
///
/// Panics if `par_ref_buffer_stack` is empty (no active queue) or
/// `idx` is out of range — both indicate a parser-side bug.
fn n_parallel_buf_get_ref(stores: &mut Stores, stack: &mut DbRef) {
    let idx = *stores.get::<i64>(stack);
    let r = {
        let buf = stores
            .par_ref_buffer_stack
            .last()
            .expect("parallel_buf_get_ref: par_ref_buffer_stack is empty");
        buf.0[idx as usize]
    };
    stores.put(stack, r);
}

/// Plan-06 spine step 8d.1 — pop the active par-ref buffer and free
/// every adopted store.  Called after the body loop completes.
/// Panics if the stack is already empty (parser-side bug).
///
/// Each adopted store_nr is freed via `Stores::free_named` with a
/// synthetic `DbRef` (`store_nr, rec=0, pos=0`) so the standard
/// alloc-free instrumentation logs the release.
fn n_parallel_buf_drop_ref(stores: &mut Stores, _stack: &mut DbRef) {
    let (_refs, adopted) = stores
        .par_ref_buffer_stack
        .pop()
        .expect("parallel_buf_drop_ref: par_ref_buffer_stack is already empty");
    for store_nr in adopted {
        let synthetic = DbRef {
            store_nr,
            rec: 0,
            pos: 0,
        };
        stores.free_named(&synthetic, "par_buf_drop_ref");
    }
}

/// Plan-06 ARC.md A3 — narrow-primitive Queue runtime entry.  Same
/// arg layout as `n_parallel_queue`; workers return a 1, 2, or 4-byte
/// value.  Internally reuses `run_parallel_queue`'s `Vec<u64>`
/// scratch buffer, then packs each row to its declared `return_size`
/// stride and pushes `(bytes, stride)` onto
/// `par_narrow_buffer_stack`.  The packed buffer saves memory on
/// large narrow-return workloads (~7× for 1-byte returns).
///
/// Reads use `n_parallel_buf_get_narrow(idx, return_size, signed)`.
fn n_parallel_queue_narrow(stores: &mut Stores, stack: &mut DbRef) {
    parallel_queue_dispatch(stores, stack, QueueStitch::Narrow);
}

/// Plan-06 ARC.md A3 — read one narrow row from the active narrow
/// buffer.  Pops `(idx, return_size, signed)` (i64s; `signed` is
/// 0/1).  Reads `return_size` little-endian bytes at offset
/// `idx * return_size` from `par_narrow_buffer_stack.last().0`,
/// sign-extending if `signed != 0`, and pushes the result as i64.
///
/// `return_size` is checked against the stored stride — a mismatch
/// is a parser-side bug.
fn n_parallel_buf_get_narrow(stores: &mut Stores, stack: &mut DbRef) {
    let signed = *stores.get::<i64>(stack) != 0;
    let return_size = *stores.get::<i64>(stack) as usize;
    let idx = *stores.get::<i64>(stack) as usize;

    let val: i64 = {
        let entry = stores
            .par_narrow_buffer_stack
            .last()
            .expect("parallel_buf_get_narrow: par_narrow_buffer_stack is empty");
        let stride = entry.1 as usize;
        debug_assert_eq!(
            stride, return_size,
            "parallel_buf_get_narrow: stride/return_size mismatch (stride={stride}, arg={return_size})"
        );
        let off = idx * stride;
        let bytes = &entry.0;
        match (stride, signed) {
            (1, false) => i64::from(bytes[off]),
            (1, true) => i64::from(bytes[off] as i8),
            (2, false) => i64::from(u16::from_le_bytes([bytes[off], bytes[off + 1]])),
            (2, true) => i64::from(i16::from_le_bytes([bytes[off], bytes[off + 1]])),
            (4, false) => i64::from(u32::from_le_bytes([
                bytes[off],
                bytes[off + 1],
                bytes[off + 2],
                bytes[off + 3],
            ])),
            (4, true) => i64::from(i32::from_le_bytes([
                bytes[off],
                bytes[off + 1],
                bytes[off + 2],
                bytes[off + 3],
            ])),
            _ => panic!("parallel_buf_get_narrow: invalid stride {stride}"),
        }
    };
    stores.put(stack, val);
}

/// Plan-06 ARC.md A3 — pop the active narrow-queue buffer.  Called
/// after the body loop completes.  Panics if the stack is already
/// empty (parser-side bug).  Void return.
fn n_parallel_buf_drop_narrow(stores: &mut Stores, _stack: &mut DbRef) {
    stores
        .par_narrow_buffer_stack
        .pop()
        .expect("parallel_buf_drop_narrow: par_narrow_buffer_stack is already empty");
}

/// Plan-06 ARC.md A3.6 — typed reader for `single` (f32) returns
/// from `n_parallel_queue_narrow` (stride 4).  Reads the same
/// per-row bytes as `n_parallel_buf_get_narrow` but interprets them
/// as a 4-byte f32 bit pattern via `f32::from_bits` instead of
/// returning an i64 that the caller would then have to bit-cast.
///
/// Justified by symmetry with `Store::set_single` /
/// `Store::get_single` in `src/store.rs:1406-1421`: those write /
/// read f32 as a typed-pointer memcpy at a slot.  Worker functions
/// returning `single` populate their return slot via the same
/// typed pointer write, so the slot bytes ARE the f32 bit pattern.
/// `execute_at_raw` reads those bytes as u64 (low 4 = f32 bits);
/// `n_parallel_queue_narrow` truncates to stride 4 — preserving the
/// f32 bit pattern.  This getter recovers the typed value with no
/// intermediate IR Op required.
///
/// Pushes the value as f32 onto the operand stack — `Type::Single`
/// has slot size 4 (per `variables::size`), matching `f32`'s width.
/// Mirrors how `OpConvSingleFromInt` returns f32 (`src/ops.rs:502`).
fn n_parallel_buf_get_single(stores: &mut Stores, stack: &mut DbRef) {
    let idx = *stores.get::<i64>(stack) as usize;
    let val: f32 = {
        let entry = stores
            .par_narrow_buffer_stack
            .last()
            .expect("parallel_buf_get_single: par_narrow_buffer_stack is empty");
        let stride = entry.1 as usize;
        debug_assert_eq!(
            stride, 4,
            "parallel_buf_get_single: stride must be 4 (got {stride})"
        );
        let off = idx * stride;
        let bytes = &entry.0;
        let raw = u32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
        f32::from_bits(raw)
    };
    stores.put(stack, val);
}

/// Plan-06 ARC.md A3.6 — typed reader for `float` (f64) returns
/// from `n_parallel_queue` (stride 8 / Vec<u64> rows).  Reads the
/// same per-row u64 as `n_parallel_buf_get` but interprets the bits
/// as f64 via `f64::from_bits` instead of returning a raw i64.
///
/// Same justification as `n_parallel_buf_get_single`: worker
/// functions returning `float` populate their return slot via a
/// typed memcpy (`Store::set_float`); `execute_at_raw` reads the
/// slot as u64 — the bytes ARE the f64 bit pattern.  This getter
/// recovers the typed value with no intermediate IR Op.
fn n_parallel_buf_get_float(stores: &mut Stores, stack: &mut DbRef) {
    let idx = *stores.get::<i64>(stack) as usize;
    let row = stores
        .par_buffer_stack
        .last()
        .expect("parallel_buf_get_float: par_buffer_stack is empty")[idx];
    let val = f64::from_bits(row);
    stores.put(stack, val);
}

/// Plan-06 ARC.md A6.b — fn-ref-return Queue runtime entry.  Same
/// arg layout as `n_parallel_queue` but workers return 20-byte
/// fn-ref blobs (8B i64 d_nr + 12B closure DbRef per Rust's
/// reordered layout).  `run_parallel_queue_fn` writes each row's
/// 20 bytes directly into a packed `Vec<u8>` via
/// `State::execute_at_raw_to`, bypassing both the truncating light
/// path (L1) and the body's `get_field`-as-i32 misread (L2).
///
/// The packed buffer is pushed onto `par_fn_buffer_stack`; readers
/// pull 20 bytes per row via `n_parallel_buf_get_fn`.  Reads are
/// inserted at the top of each fused-for-par iteration as
/// `Set(b_var, Call(buf_get_fn, [idx]))` so the body's
/// `CallRef(b_var, ...)` reads `b_var`'s 20-byte slot directly.
fn n_parallel_queue_fn(stores: &mut Stores, stack: &mut DbRef) {
    parallel_queue_dispatch(stores, stack, QueueStitch::Fn);
}

/// Plan-06 ARC.md A6.b — read one 20-byte fn-ref blob from the
/// active fn-buffer.  Pops `idx` (i64), reads 20 bytes at offset
/// `idx * 20` from `par_fn_buffer_stack.last()`, and pushes them
/// onto the operand stack as a fn-ref blob (matches the on-stack
/// representation `OpVarFnRef` and `OpPutFnRef` operate on:
/// `[u8; 20]`).
///
/// Declared as `-> integer` in stdlib (since loft's type system
/// can't express a generic-fn-typed return), but the parser
/// substitutes the call site's actual fn-ref type so codegen
/// allocates a 20-byte slot for `b_var`.  The 20-byte stack push
/// happens here at runtime regardless of the declared return type.
///
/// Panics if `par_fn_buffer_stack` is empty (no active queue) or
/// if `idx` is out of range — both indicate a parser-side bug.
fn n_parallel_buf_get_fn(stores: &mut Stores, stack: &mut DbRef) {
    let idx = *stores.get::<i64>(stack);
    let bytes_20: [u8; 20] = {
        let buf = stores
            .par_fn_buffer_stack
            .last()
            .expect("parallel_buf_get_fn: par_fn_buffer_stack is empty");
        let off = (idx as usize) * 20;
        let mut arr = [0u8; 20];
        arr.copy_from_slice(&buf[off..off + 20]);
        arr
    };
    stores.put(stack, bytes_20);
}

/// Plan-06 ARC.md A6.b — pop the active fn-queue buffer.  Called
/// after the body loop completes.  Panics if the stack is already
/// empty (parser-side bug).  Void return.
fn n_parallel_buf_drop_fn(stores: &mut Stores, _stack: &mut DbRef) {
    stores
        .par_fn_buffer_stack
        .pop()
        .expect("parallel_buf_drop_fn: par_fn_buffer_stack is already empty");
}

/// Parse a `LOFT_FAKE_*` env var into an `i64`.  Empty / unset / unparseable
/// values return `None`, letting the caller fall through to the real clock.
/// Used to freeze `ticks()` and `now()` for deterministic snapshot tests —
/// see `doc/claude/TESTING.md` § "Deterministic snapshots".
///
/// Only compiled on targets with `std::env` — every one except the browser
/// (`wasm32-unknown-unknown`); `wasm32-wasip2` has it (#620).
#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
fn fake_clock_env(var: &str) -> Option<i64> {
    std::env::var(var).ok()?.parse::<i64>().ok()
}

/// Return milliseconds since the Unix epoch (1970-01-01T00:00:00 UTC).
/// Returns `i64::MIN` (null) if the system clock reports a time before the epoch.
/// Honours `LOFT_FAKE_NOW_MS` when set (deterministic snapshot tests).
/// #620 — `wasm32-wasip2` (`--native-wasm`) reaches this arm too: WASI exposes a
/// real clock through `wasi:clocks`, which `std`'s `SystemTime` already uses.
#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
fn n_now(stores: &mut Stores, stack: &mut DbRef) {
    if let Some(fake) = fake_clock_env("LOFT_FAKE_NOW_MS") {
        stores.put(stack, fake);
        return;
    }
    let millis = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(i64::MIN, |d| d.as_millis() as i64);
    stores.put(stack, millis);
}

#[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), feature = "wasm"))]
fn n_now(stores: &mut Stores, stack: &mut DbRef) {
    stores.put(stack, crate::wasm::host_time_now());
}

/// #620 — the `--html` build (`wasm32-unknown-unknown`, no `wasm` feature) has
/// no `std` clock, so this reads the host's `Date.now()` through the `loft_io`
/// import bridge.  It used to return a hardcoded 0, which is the whole second
/// half of #620: every `now()` read the same instant and every duration
/// measured 0ms with no error anywhere.
#[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))]
fn n_now(stores: &mut Stores, stack: &mut DbRef) {
    stores.put(stack, crate::loft_host_time_now_ms() as i64);
}

/// Return microseconds elapsed since program start (monotonic clock).
/// Use for frame timing and benchmarks; unaffected by wall-clock adjustments.
/// Honours `LOFT_FAKE_TICKS_US` when set (deterministic snapshot tests).
/// #620 — see `n_now`: `wasm32-wasip2` has a real monotonic clock behind
/// `std::time::Instant`, so it shares the host path.
#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
fn n_ticks(stores: &mut Stores, stack: &mut DbRef) {
    if let Some(fake) = fake_clock_env("LOFT_FAKE_TICKS_US") {
        stores.put(stack, fake);
        return;
    }
    let micros = stores.start_time.elapsed().as_micros() as i64;
    stores.put(stack, micros);
}

#[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), feature = "wasm"))]
fn n_ticks(stores: &mut Stores, stack: &mut DbRef) {
    let now_ms = crate::wasm::host_time_ticks();
    let elapsed_micros = (now_ms - stores.start_time_ms) * 1000;
    stores.put(stack, elapsed_micros);
}

/// #620 — browser arm; `wasm32-wasip2` uses the real clock above.  Reads the
/// host's `performance.now()`, which is already monotonic and page-relative
/// (exactly `ticks()`'s contract), so it needs no start-time subtraction.
#[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))]
fn n_ticks(stores: &mut Stores, stack: &mut DbRef) {
    stores.put(stack, crate::loft_host_time_ticks_us() as i64);
}

/// TR1.3: Build `vector<StackFrame>` from the call-stack snapshot in Stores.
/// The snapshot is populated by `State::static_call` before this runs.
fn n_stack_trace(stores: &mut Stores, stack: &mut DbRef) {
    let snapshot = std::mem::take(&mut stores.call_stack_snapshot);
    let vars_snapshot = std::mem::take(&mut stores.variables_snapshot);
    let sf_elm = stores.name("StackFrame");
    let sf_size = u32::from(stores.size(sf_elm));
    let var_elm = stores.name("VarInfo");
    let var_size = u32::from(stores.size(var_elm));
    // look up every field position from the schema instead of hard-coding
    // byte offsets.  If a future edit to `default/04_stacktrace.loft` reorders
    // fields, renames them, or changes their type sizes, the lookups update
    // automatically — no silent garbage at runtime.  A missing field name
    // panics with a clear message in both debug and release.
    let lookup = |field: &str| {
        let p = stores.position(sf_elm, field);
        assert_ne!(
            p,
            u16::MAX,
            "StackFrame schema is missing field '{field}' — \
             default/04_stacktrace.loft has drifted from src/native.rs::n_stack_trace"
        );
        u32::from(p)
    };
    let function_pos = lookup("function");
    let file_pos = lookup("file");
    let line_pos = lookup("line");
    let arguments_pos = lookup("arguments");
    let vars_field_pos = lookup("variables");
    let vec = stores.database(sf_size);
    stores.store_mut(&vec).set_u32_raw(vec.rec, vec.pos, 0);

    for (frame_idx, (fn_name, file, line)) in snapshot.iter().enumerate() {
        let elm = crate::vector::vector_append(&vec, sf_size, &mut stores.allocations);
        let fn_str = stores.store_mut(&vec).set_str(fn_name.as_str());
        stores
            .store_mut(&vec)
            .set_u32_raw(elm.rec, elm.pos + function_pos, fn_str);
        let file_str = stores.store_mut(&vec).set_str(file.as_str());
        stores
            .store_mut(&vec)
            .set_u32_raw(elm.rec, elm.pos + file_pos, file_str);
        stores
            .store_mut(&vec)
            .set_int(elm.rec, elm.pos + line_pos, i64::from(*line));
        // Explicitly zero arguments and variables so that reused (non-zeroed) store
        // blocks don't leave garbage data that looks like a valid first_block_rec.
        stores
            .store_mut(&vec)
            .set_u32_raw(elm.rec, elm.pos + arguments_pos, 0);
        stores
            .store_mut(&vec)
            .set_u32_raw(elm.rec, elm.pos + vars_field_pos, 0);

        // TR1.4: build vector<VarInfo> for this frame from the snapshot.
        if let Some(frame_vars) = vars_snapshot.get(frame_idx) {
            populate_frame_variables(
                stores,
                &vec,
                elm.rec,
                elm.pos + vars_field_pos,
                var_size,
                frame_vars,
            );
        }
        crate::vector::vector_finish(&vec, &mut stores.allocations);
    }
    stores.put(stack, vec);
}

/// @PLN127 arc B: build a `TypeInfo` for one type id.
///
/// The source is @PLN105's [`crate::database::LayoutDesc`], not the
/// type table directly. That descriptor is pinned byte-for-byte against the
/// @PLN97 layout dump and is what the browser bridge already reads, so
/// reflection cannot become a second, drifting description of the same layout —
/// which is the whole reason it exists rather than a fresh walk of `Parts`.
///
/// Field positions come from the schema (`stores.position`) exactly as
/// `n_stack_trace` does: a rename in `default/07_reflect.loft` then panics with a
/// clear message instead of writing silent garbage at byte 65535.
fn n_reflect_type(stores: &mut Stores, stack: &mut DbRef) {
    let kt = *stores.get::<i64>(stack) as u16;
    let result = reflect_type_into(stores, kt);
    stores.put(stack, result);
}

/// @PLN127 arc C: `type_named(name)` — reflection with no value in hand.
///
/// `Stores::name` is a total lookup that answers `u16::MAX` for a name this
/// program has no type for, so a typo reads back as ABSENT rather than minting a
/// type. `--native` replays the type registrations in `init()`, names included,
/// which is why a runtime name works there too — the question the plan expected
/// to be load-bearing.
fn n_type_named(stores: &mut Stores, stack: &mut DbRef) {
    let raw = *stores.get::<Str>(stack);
    let result = type_named_in(stores, raw.str());
    stores.put(stack, result);
}

/// Shared by both backends: the named type's shape, or a null `DbRef`.
pub fn type_named_in(stores: &mut Stores, name: &str) -> DbRef {
    let kt = stores.name(name);
    if kt == u16::MAX {
        return DbRef::NULL;
    }
    reflect_type_into(stores, kt)
}

/// The `TypeKind` discriminant for a descriptor node — 1-indexed, matching the
/// variant order in `default/07_reflect.loft`.
fn reflect_kind(node: Option<&crate::database::LayoutNode>) -> i32 {
    use crate::database::{BaseKind, LayoutNode};
    match node {
        Some(LayoutNode::Base(b)) => match b {
            BaseKind::Integer => 1,
            BaseKind::Long => 2,
            BaseKind::Single => 3,
            BaseKind::Float => 4,
            BaseKind::Boolean => 5,
            BaseKind::Text => 6,
            BaseKind::Character => 7,
        },
        // A narrow scalar is an INTEGER that happens to be stored small. The
        // declared width shows up in `size`, so reporting a separate kind would
        // make a caller match twice on one fact.
        Some(
            LayoutNode::Byte { .. }
            | LayoutNode::Short { .. }
            | LayoutNode::Int { .. }
            | LayoutNode::ShortRaw { .. },
        ) => 1,
        Some(LayoutNode::Record(_)) => 8,
        Some(LayoutNode::Choices(_)) => 9,
        Some(LayoutNode::EnumValue(_, _)) => 10,
        Some(LayoutNode::Vector(_) | LayoutNode::Array(_) | LayoutNode::FlatArray { .. }) => 11,
        Some(LayoutNode::Iterated(_)) => 12,
        Some(LayoutNode::Ref | LayoutNode::ChildRec(_)) => 13,
        // A kind this loft version has no name for is reported as such, never
        // guessed at: a wrong kind is worse than an unknown one, because a
        // caller can branch on unknown and cannot detect a lie.
        _ => 14,
    }
}

/// Fill a `TypeInfo` record for `kt` and return it.
#[allow(clippy::too_many_lines)] // one schema-driven writer; splitting it hides the field map
/// # Panics
/// When `default/07_reflect.loft` has drifted from the field names read here —
/// deliberately loud, because the alternative is a silent write to byte 65535.
pub fn reflect_type_into(stores: &mut Stores, kt: u16) -> DbRef {
    use crate::database::LayoutNode;
    let desc = stores.layout_descriptor(&[kt]);
    let node = desc.nodes.get(&kt).cloned();
    let type_name = desc
        .names
        .get(&kt)
        .cloned()
        .unwrap_or_else(|| format!("#{kt}"));
    let size = desc.sizes.get(&kt).copied().unwrap_or(0);

    let ti_tp = stores.name("TypeInfo");
    let fi_tp = stores.name("FieldInfo");
    let vi_tp = stores.name("VariantInfo");
    let lookup = |stores: &Stores, tp: u16, ty: &str, field: &str| {
        let p = stores.position(tp, field);
        assert_ne!(
            p,
            u16::MAX,
            "{ty} schema is missing field '{field}' — \
             default/07_reflect.loft has drifted from src/native.rs::reflect_type_into"
        );
        u32::from(p)
    };
    let ti_name = lookup(stores, ti_tp, "TypeInfo", "name");
    let ti_kind = lookup(stores, ti_tp, "TypeInfo", "kind");
    let ti_size = lookup(stores, ti_tp, "TypeInfo", "size");
    let ti_fields = lookup(stores, ti_tp, "TypeInfo", "fields");
    let ti_variants = lookup(stores, ti_tp, "TypeInfo", "variants");
    let ti_element = lookup(stores, ti_tp, "TypeInfo", "element");
    let fi_name = lookup(stores, fi_tp, "FieldInfo", "name");
    let fi_type = lookup(stores, fi_tp, "FieldInfo", "type_name");
    let fi_pos = lookup(stores, fi_tp, "FieldInfo", "position");
    let fi_kind = lookup(stores, fi_tp, "FieldInfo", "kind");
    let vi_name = lookup(stores, vi_tp, "VariantInfo", "name");
    let vi_tag = lookup(stores, vi_tp, "VariantInfo", "tag");

    let ti_bytes = u32::from(stores.size(ti_tp));
    let out = stores.database(ti_bytes.div_ceil(8) + 1);
    // Stamp the record's own type id. Without it the record is typeless, which
    // reads as `kt=65535 ?` in a leak report and leaves anything that walks the
    // value by schema with nothing to walk.
    stores
        .store_mut(&out)
        .set_u32_raw(out.rec, 4, u32::from(ti_tp));
    let name_str = stores.store_mut(&out).set_str(&type_name);
    stores
        .store_mut(&out)
        .set_u32_raw(out.rec, out.pos + ti_name, name_str);
    stores
        .store_mut(&out)
        .set_byte(out.rec, out.pos + ti_kind, 0, reflect_kind(node.as_ref()));
    stores
        .store_mut(&out)
        .set_int(out.rec, out.pos + ti_size, i64::from(size));
    // Zero the two vector headers: a reused store block would otherwise leave
    // bytes that read as a valid first record.
    stores
        .store_mut(&out)
        .set_u32_raw(out.rec, out.pos + ti_fields, 0);
    stores
        .store_mut(&out)
        .set_u32_raw(out.rec, out.pos + ti_variants, 0);

    // The element type, for the two kinds that hold one.
    let elem = match &node {
        Some(LayoutNode::Vector(e) | LayoutNode::Array(e) | LayoutNode::ChildRec(e)) => Some(*e),
        Some(LayoutNode::FlatArray { elem }) => Some(*elem),
        Some(LayoutNode::Iterated(it)) => Some(it.elem()),
        _ => None,
    };
    let elem_name = elem.map_or_else(String::new, |e| {
        desc.names
            .get(&e)
            .cloned()
            .unwrap_or_else(|| format!("#{e}"))
    });
    let elem_str = stores.store_mut(&out).set_str(&elem_name);
    stores
        .store_mut(&out)
        .set_u32_raw(out.rec, out.pos + ti_element, elem_str);

    // Fields — a record and a struct-enum variant both have them, and they are
    // the same list, so they are written by one path.
    if let Some(LayoutNode::Record(fields) | LayoutNode::EnumValue(_, fields)) = &node {
        let fi_bytes = u32::from(stores.size(fi_tp));
        let words = ((fields.len() as u32) * fi_bytes + 15) / 8 + 1;
        let vec_rec = stores.store_mut(&out).claim(words.max(1));
        stores
            .store_mut(&out)
            .set_u32_raw(vec_rec, 4, fields.len() as u32);
        stores
            .store_mut(&out)
            .set_u32_raw(out.rec, out.pos + ti_fields, vec_rec);
        for (i, f) in fields.iter().enumerate() {
            let at = 8 + (i as u32) * fi_bytes;
            let fld = stores.store_mut(&out).set_str(&f.name);
            stores
                .store_mut(&out)
                .set_u32_raw(vec_rec, at + fi_name, fld);
            let f_ty = desc
                .names
                .get(&f.content)
                .cloned()
                .unwrap_or_else(|| format!("#{}", f.content));
            let f_ty_str = stores.store_mut(&out).set_str(&f_ty);
            stores
                .store_mut(&out)
                .set_u32_raw(vec_rec, at + fi_type, f_ty_str);
            stores
                .store_mut(&out)
                .set_int(vec_rec, at + fi_pos, i64::from(f.position));
            let k = reflect_kind(desc.nodes.get(&f.content));
            stores.store_mut(&out).set_byte(vec_rec, at + fi_kind, 0, k);
        }
    }

    // Variants — an enum only.
    if let Some(LayoutNode::Choices(vals)) = &node {
        let vi_bytes = u32::from(stores.size(vi_tp));
        let words = ((vals.len() as u32) * vi_bytes + 15) / 8 + 1;
        let vec_rec = stores.store_mut(&out).claim(words.max(1));
        stores
            .store_mut(&out)
            .set_u32_raw(vec_rec, 4, vals.len() as u32);
        stores
            .store_mut(&out)
            .set_u32_raw(out.rec, out.pos + ti_variants, vec_rec);
        for (i, (_tp, name)) in vals.iter().enumerate() {
            let at = 8 + (i as u32) * vi_bytes;
            let n = stores.store_mut(&out).set_str(name);
            stores.store_mut(&out).set_u32_raw(vec_rec, at + vi_name, n);
            // 1-indexed: the store spells "absent" as 0, so a real variant is
            // never 0 and a caller can test for it.
            stores
                .store_mut(&out)
                .set_int(vec_rec, at + vi_tag, i as i64 + 1);
        }
    }
    out
}

/// TR1.4: append a `vector<VarInfo>` to the StackFrame at the given offset.
/// Each VarInfo gets `name`, `type_name` (text fields) and `value` (ArgValue
/// struct-enum) populated from the runtime snapshot captured by static_call.
///
/// ArgValue is a loft struct-enum.  The discriminant is a 1-indexed byte at
/// offset 0 (0 = null, 1 = first variant, ...).  Variant data lives at
/// offsets resolved via `stores.position(<variant_type>, <field>)`.
#[allow(clippy::similar_names)]
fn populate_frame_variables(
    stores: &mut Stores,
    sf_vec: &DbRef,
    parent_rec: u32,
    vars_field_abs: u32,
    var_elm_size: u32,
    frame_vars: &[crate::database::VarSnapshot],
) {
    if frame_vars.is_empty() {
        return;
    }
    let var_elm = stores.name("VarInfo");
    // Allocate the inner vector record for this frame's variables.
    let vec_words = ((frame_vars.len() as u32) * var_elm_size + 15) / 8 + 1;
    let inner_rec = stores.store_mut(sf_vec).claim(vec_words.max(1));
    // Header: count
    stores
        .store_mut(sf_vec)
        .set_u32_raw(inner_rec, 4, frame_vars.len() as u32);
    // Link from the StackFrame.variables field to this inner record.
    stores
        .store_mut(sf_vec)
        .set_u32_raw(parent_rec, vars_field_abs, inner_rec);

    // schema-driven field position lookup.  A typo or rename in
    // default/04_stacktrace.loft surfaces as a clear panic instead of a
    // silent write to byte 65535.
    let lookup = |tp: u16, ty_name: &str, field: &str| {
        let p = stores.position(tp, field);
        assert_ne!(
            p,
            u16::MAX,
            "{ty_name} schema is missing field '{field}' — \
             default/04_stacktrace.loft has drifted from src/native.rs"
        );
        p
    };
    let name_pos = lookup(var_elm, "VarInfo", "name");
    let type_pos = lookup(var_elm, "VarInfo", "type_name");
    let val_pos = lookup(var_elm, "VarInfo", "value");

    // ArgValue variant types (resolve once).
    let bool_tp = stores.name("BoolVal");
    let int_tp = stores.name("IntVal");
    let long_tp = stores.name("LongVal");
    let float_tp = stores.name("FloatVal");
    let single_tp = stores.name("SingleVal");
    let char_tp = stores.name("CharVal");
    let text_tp = stores.name("TextVal");
    let ref_tp = stores.name("RefVal");
    let other_tp = stores.name("OtherVal");

    let bool_b_pos = lookup(bool_tp, "BoolVal", "b");
    let int_n_pos = lookup(int_tp, "IntVal", "n");
    let long_n_pos = lookup(long_tp, "LongVal", "n");
    let float_f_pos = lookup(float_tp, "FloatVal", "f");
    let single_f_pos = lookup(single_tp, "SingleVal", "f");
    let char_c_pos = lookup(char_tp, "CharVal", "c");
    let text_t_pos = lookup(text_tp, "TextVal", "t");
    let ref_store_pos = lookup(ref_tp, "RefVal", "store");
    let ref_rec_pos = lookup(ref_tp, "RefVal", "rec");
    let ref_pos_pos = lookup(ref_tp, "RefVal", "pos");
    let other_desc_pos = lookup(other_tp, "OtherVal", "description");

    for (i, vs) in frame_vars.iter().enumerate() {
        let inline_pos = 8 + (i as u32) * var_elm_size;
        // Write name
        let name_str = stores.store_mut(sf_vec).set_str(&vs.name);
        stores
            .store_mut(sf_vec)
            .set_u32_raw(inner_rec, inline_pos + u32::from(name_pos), name_str);
        // Write type_name
        let type_str = stores.store_mut(sf_vec).set_str(&vs.type_name);
        stores
            .store_mut(sf_vec)
            .set_u32_raw(inner_rec, inline_pos + u32::from(type_pos), type_str);
        // Write ArgValue: discriminant byte at av_abs (1-indexed),
        // variant data at av_abs + position(variant_tp, field_name).
        let av_abs = inline_pos + u32::from(val_pos);
        let store_mut = stores.store_mut(sf_vec);
        match &vs.value {
            crate::database::VarValueSnapshot::Null => {
                store_mut.set_byte(inner_rec, av_abs, 0, 1);
            }
            crate::database::VarValueSnapshot::Bool(b) => {
                store_mut.set_byte(inner_rec, av_abs, 0, 2);
                store_mut.set_byte(inner_rec, av_abs + u32::from(bool_b_pos), 0, i32::from(*b));
            }
            crate::database::VarValueSnapshot::Int(n) => {
                store_mut.set_byte(inner_rec, av_abs, 0, 3);
                store_mut.set_int(inner_rec, av_abs + u32::from(int_n_pos), i64::from(*n));
            }
            crate::database::VarValueSnapshot::Long(n) => {
                store_mut.set_byte(inner_rec, av_abs, 0, 4);
                store_mut.set_long(inner_rec, av_abs + u32::from(long_n_pos), *n);
            }
            crate::database::VarValueSnapshot::Float(f) => {
                store_mut.set_byte(inner_rec, av_abs, 0, 5);
                store_mut.set_float(inner_rec, av_abs + u32::from(float_f_pos), *f);
            }
            crate::database::VarValueSnapshot::Single(f) => {
                store_mut.set_byte(inner_rec, av_abs, 0, 6);
                store_mut.set_single(inner_rec, av_abs + u32::from(single_f_pos), *f);
            }
            crate::database::VarValueSnapshot::Char(c) => {
                store_mut.set_byte(inner_rec, av_abs, 0, 7);
                store_mut.set_u32_raw(inner_rec, av_abs + u32::from(char_c_pos), *c as u32);
            }
            crate::database::VarValueSnapshot::Text(s) => {
                store_mut.set_byte(inner_rec, av_abs, 0, 8);
                let txt = store_mut.set_str(s);
                store_mut.set_u32_raw(inner_rec, av_abs + u32::from(text_t_pos), txt);
            }
            crate::database::VarValueSnapshot::Ref { store, rec, pos } => {
                store_mut.set_byte(inner_rec, av_abs, 0, 9);
                store_mut.set_u32_raw(inner_rec, av_abs + u32::from(ref_store_pos), *store as u32);
                store_mut.set_u32_raw(inner_rec, av_abs + u32::from(ref_rec_pos), *rec as u32);
                store_mut.set_u32_raw(inner_rec, av_abs + u32::from(ref_pos_pos), *pos as u32);
            }
            crate::database::VarValueSnapshot::Other(desc) => {
                store_mut.set_byte(inner_rec, av_abs, 0, 11);
                let txt = store_mut.set_str(desc);
                store_mut.set_u32_raw(inner_rec, av_abs + u32::from(other_desc_pos), txt);
            }
        }
    }
}

/// Return the platform path separator as a loft `character`.
/// `'\\'` on Windows filesystems, `'/'` everywhere else.
fn n_path_sep(stores: &mut Stores, stack: &mut DbRef) {
    stores.put(stack, sep());
}

/// Return the error text from the last `Type.parse()` call.
/// Empty string means the parse succeeded.
fn i_parse_error_push(stores: &mut Stores, stack: &mut DbRef) {
    let msg = *stores.get::<Str>(stack);
    stores.last_parse_errors.push(msg.str().to_owned());
}

// @PLN10 Phase 1 — destination-passing variant of `i_parse_errors`.
// Always-non-null (the joined error text, possibly empty); no stack args.
// Routed by `is_text_dest_native`.
fn i_parse_errors_dest(stores: &mut Stores, stack: &mut DbRef) {
    let dest = *stores.get::<DbRef>(stack);
    let msg = stores.last_parse_errors.join("\n");
    stores.last_parse_errors.clear();
    stores
        .store_mut(&dest)
        .addr_mut::<String>(dest.rec, dest.pos)
        .push_str(&msg);
}

// HTTP client glue removed — n_http_do and n_http_body are now auto-marshalled.
// The cdylib stores the response body in a thread-local, returned via LoftStr.

// ── Crypto built-ins moved to lib/crypto/native (plan-12 phase 1a) ──────

/// C60 Step 3a-part2: iterate a hash in ascending key order.
/// Wraps `Stores::build_hash_sorted_vec` (src/database/allocation.rs).
///
/// Call shape in loft:
///
/// ```loft
/// pub fn hash_sorted(h: reference, tp: integer) -> reference;
/// ```
///
/// Returns a fresh `vector<reference<T>>` whose elements are refs
/// into the hash's original store, one per live record, sorted
/// ascending by the hash's key field(s).  Callers pass the hash's
/// type id (`tp`) explicitly — the parser-desugared `for e in h`
/// path emits it as a compile-time constant; direct callers must
/// use `sizeof(hash<T[…]>)`-style type introspection to obtain it.
fn n_hash_sorted(stores: &mut Stores, stack: &mut DbRef) {
    let v_tp = *stores.get::<i64>(stack) as u16;
    let v_h = *stores.get::<DbRef>(stack);
    let result = stores.build_hash_sorted_vec(&v_h, v_tp);
    stores.put(stack, result);
}

/// Raw bucket-walk sibling of `n_hash_sorted` for `for e in h par(...)` —
/// skips the key sort because the parallel queue has no use for hash order.
fn n_hash_unsorted(stores: &mut Stores, stack: &mut DbRef) {
    let v_tp = *stores.get::<i64>(stack) as u16;
    let v_h = *stores.get::<DbRef>(stack);
    let result = stores.build_hash_unsorted_vec(&v_h, v_tp);
    stores.put(stack, result);
}

/// @PLN48 — iterate a `spatial`/`radix` collection in natural key order.  Wraps
/// `Stores::build_radix_sorted_vec`; no sort — the tree walk is already ordered.
fn n_radix_sorted(stores: &mut Stores, stack: &mut DbRef) {
    let v_tp = *stores.get::<i64>(stack) as u16;
    let v_r = *stores.get::<DbRef>(stack);
    let result = stores.build_radix_sorted_vec(&v_r, v_tp);
    stores.put(stack, result);
}

/// @PLN48 S3 — a spatial range slice.  Args pop in reverse declaration order.
fn n_spatial_range(stores: &mut Stores, stack: &mut DbRef) {
    let limit = *stores.get::<i64>(stack);
    let tz = *stores.get::<i64>(stack);
    let ty = *stores.get::<i64>(stack);
    let tx = *stores.get::<i64>(stack);
    let has_till = *stores.get::<i64>(stack);
    let fz = *stores.get::<i64>(stack);
    let fy = *stores.get::<i64>(stack);
    let fx = *stores.get::<i64>(stack);
    let tp = *stores.get::<i64>(stack) as u16;
    let coll = *stores.get::<DbRef>(stack);
    let result = stores.build_radix_range_vec(&coll, tp, fx, fy, fz, has_till, tx, ty, tz, limit);
    stores.put(stack, result);
}

// Plan-12 phase 1a (2026-05-23) — crypto `n_*` impls moved to
// `lib/crypto/native/src/lib.rs` (cdylib).  See the registry section
// above for the routing; the loft binary loads them via
// `extensions::wire_native_fns` at runtime when a program does
// `use crypto`.

// ── WebSocket + TCP + OpenGL + random glue removed ─────────────────────
// These functions are now auto-marshalled by extensions::wire_native_fns().
// See EXTERNAL_LIBS.md Phase 5 for design.

// ── JsonValue native bindings ──────────────────────────────────────────
//
// `default/06_json.loft` declares the JsonValue struct-enum.  Variant
// discriminants are 1-indexed in declaration order:
//   1 = JNull, 2 = JBool, 3 = JNumber, 4 = JString,
//   5 = JArray, 6 = JObject (5/6 not yet implemented; return JNull).
//
// Allocation pattern (matches `populate_frame_variables` at line 1017):
//   stores.database(words) creates a fresh store + claims a record;
//   the returned DbRef has rec=<claimed>, pos=8 (struct body start).
//   When the loft variable holding the DbRef goes out of scope,
//   OpFreeRef on it frees the entire store — single ownership, no
//   ref-count puzzles.

// JsonValue variant discriminants — exposed `pub(crate)` so the
// `--native` runtime wrappers in `src/codegen_runtime.rs` can match
// against the same byte values the interp uses.
pub(crate) const JV_DISCR_NULL: i32 = 1;
pub(crate) const JV_DISCR_BOOL: i32 = 2;
pub(crate) const JV_DISCR_NUMBER: i32 = 3;
pub(crate) const JV_DISCR_STRING: i32 = 4;
pub(crate) const JV_DISCR_ARRAY: i32 = 5;
pub(crate) const JV_DISCR_OBJECT: i32 = 6;
// @PLN109 — integer-shaped JSON numbers preserve their exact i64 (H5).  The
// `JInteger` variant is declared LAST in `JsonValue` (06_json.loft) so the
// existing discriminants 1–6 stay stable; this must match its position.
pub(crate) const JV_DISCR_INT: i32 = 7;

/// Allocate a fresh `JsonValue` record in its own store and return
/// the DbRef.  Caller writes the discriminant byte at pos+0 and any
/// variant payload at pos + position(variant_tp, field_name).
pub(crate) fn jv_alloc(stores: &mut Stores) -> DbRef {
    let jv_tp = stores.name("JsonValue");
    let size_bytes = u32::from(stores.size(jv_tp));
    // database(n) → claim(n) which expects 8-byte words; round up
    // and add 1 word for the record header.
    let words = size_bytes.div_ceil(8) + 1;
    stores.database(words.max(2))
}

/// Shared `JsonValue::JNull` sentinel used by `n_field` / `n_item` fallback
/// paths — those natives have `dep=[0]` (borrow from self), so any freshly
/// allocated record is a leak.  Instead, they all return this single record.
/// Lazily allocated on first call; its store is `lock()`ed so future writes
/// panic (guaranteeing the sentinel stays JNull) and `check_store_leaks`
/// ignores it for the process lifetime.
/// `pub(crate)` so the `--native` JSON runtime wrappers in
/// `src/codegen_runtime.rs` can return the same sentinel.
pub(crate) fn jv_null_sentinel(stores: &mut Stores) -> DbRef {
    if let Some(r) = stores.jnull_sentinel {
        return r;
    }
    let r = jv_alloc(stores);
    stores
        .store_mut(&r)
        .set_byte(r.rec, r.pos, 0, JV_DISCR_NULL);
    if (r.store_nr as usize) < stores.allocations.len() {
        stores.allocations[r.store_nr as usize]
            .lock_with_origin("native.rs::ensure_jnull_sentinel");
    }
    stores.jnull_sentinel = Some(r);
    r
}

// (Note: the `materialise_primitive_into` rustdoc lives directly
// above its `fn` declaration further down — the helper between
// here and there is `dbref_to_parsed`, which has its own rustdoc.)

/// Walk a JsonValue tree (rooted at `src`) and materialise it as
/// a `crate::json::Parsed` value tree.  Symmetric inverse of
/// `materialise_primitive_into` — together they let
/// `n_json_array` / `n_json_object` accept caller-built trees
/// (in some other store) and reconstruct them in the new arena.
///
/// Read-only access to `stores`; safe to interleave with the
/// read-paths used by `n_to_json` etc.  Recurses through
/// containers; allocates `Vec` / `String` for the Parsed
/// representation but never touches DbRef ownership.
pub(crate) fn dbref_to_parsed(stores: &Stores, src: &DbRef) -> crate::json::Parsed {
    let discr = stores.store(src).get_byte(src.rec, src.pos, 0);
    match discr {
        JV_DISCR_NULL => crate::json::Parsed::Null,
        JV_DISCR_BOOL => {
            let bool_tp = stores.name("JBool");
            let val_pos = u32::from(stores.position(bool_tp, "value"));
            let b = stores.store(src).get_byte(src.rec, src.pos + val_pos, 0) != 0;
            crate::json::Parsed::Bool(b)
        }
        JV_DISCR_NUMBER => {
            let num_tp = stores.name("JNumber");
            let val_pos = u32::from(stores.position(num_tp, "value"));
            let n = stores.store(src).get_float(src.rec, src.pos + val_pos);
            crate::json::Parsed::Number(n)
        }
        JV_DISCR_INT => {
            let int_tp = stores.name("JInteger");
            let val_pos = u32::from(stores.position(int_tp, "value"));
            let n = stores.store(src).get_int(src.rec, src.pos + val_pos);
            crate::json::Parsed::Int(n)
        }
        JV_DISCR_STRING => {
            let str_tp = stores.name("JString");
            let val_pos = u32::from(stores.position(str_tp, "value"));
            let s_rec = stores.store(src).get_u32_raw(src.rec, src.pos + val_pos);
            let s = stores.store(src).get_str(s_rec).to_owned();
            crate::json::Parsed::Str(s)
        }
        JV_DISCR_ARRAY => {
            let array_tp = stores.name("JArray");
            let items_pos = u32::from(stores.position(array_tp, "items")) + src.pos;
            let items_rec = stores.store(src).get_i32_raw(src.rec, items_pos);
            let mut children = Vec::new();
            if items_rec > 0 {
                let length = i64::from(stores.store(src).get_u32_raw(items_rec as u32, 4));
                let jv_tp = stores.name("JsonValue");
                let jv_size = u32::from(stores.size(jv_tp));
                for i in 0..length {
                    let elem_offset =
                        8u32 + u32::try_from(i).expect("non-negative length") * jv_size;
                    let src_elm = DbRef {
                        store_nr: src.store_nr,
                        rec: items_rec as u32,
                        pos: elem_offset,
                    };
                    children.push(dbref_to_parsed(stores, &src_elm));
                }
            }
            crate::json::Parsed::Array(children)
        }
        JV_DISCR_OBJECT => {
            let obj_tp = stores.name("JObject");
            let fields_pos = u32::from(stores.position(obj_tp, "fields")) + src.pos;
            let fields_rec = stores.store(src).get_i32_raw(src.rec, fields_pos);
            let mut entries = Vec::new();
            if fields_rec > 0 {
                let length = i64::from(stores.store(src).get_u32_raw(fields_rec as u32, 4));
                let jf_tp = stores.name("JsonField");
                let jf_size = u32::from(stores.size(jf_tp));
                let name_field_pos = u32::from(stores.position(jf_tp, "name"));
                let value_field_pos = u32::from(stores.position(jf_tp, "value"));
                for i in 0..length {
                    let elem_offset =
                        8u32 + u32::try_from(i).expect("non-negative length") * jf_size;
                    let name_rec = stores
                        .store(src)
                        .get_u32_raw(fields_rec as u32, elem_offset + name_field_pos);
                    let name = stores.store(src).get_str(name_rec).to_owned();
                    let value_slot = DbRef {
                        store_nr: src.store_nr,
                        rec: fields_rec as u32,
                        pos: elem_offset + value_field_pos,
                    };
                    entries.push((name, 0usize, dbref_to_parsed(stores, &value_slot)));
                }
            }
            crate::json::Parsed::Object(entries)
        }
        _ => crate::json::Parsed::Null,
    }
}

pub(crate) fn materialise_primitive_into(
    stores: &mut Stores,
    slot: &DbRef,
    child: &crate::json::Parsed,
) {
    match child {
        crate::json::Parsed::Null => {
            stores
                .store_mut(slot)
                .set_byte(slot.rec, slot.pos, 0, JV_DISCR_NULL);
        }
        crate::json::Parsed::Bool(b) => {
            let bool_tp = stores.name("JBool");
            let val_pos = u32::from(stores.position(bool_tp, "value")) + slot.pos;
            let sm = stores.store_mut(slot);
            sm.set_byte(slot.rec, slot.pos, 0, JV_DISCR_BOOL);
            sm.set_byte(slot.rec, val_pos, 0, i32::from(*b));
        }
        crate::json::Parsed::Number(n) => {
            let num_tp = stores.name("JNumber");
            let val_pos = u32::from(stores.position(num_tp, "value")) + slot.pos;
            let sm = stores.store_mut(slot);
            sm.set_byte(slot.rec, slot.pos, 0, JV_DISCR_NUMBER);
            sm.set_float(slot.rec, val_pos, *n);
        }
        // @PLN109 — an integer-shaped number materialises as `JInteger`, holding
        // the exact i64 (H5); `as_long`/`as_integer` read it without f64 rounding.
        crate::json::Parsed::Int(n) => {
            let int_tp = stores.name("JInteger");
            let val_pos = u32::from(stores.position(int_tp, "value")) + slot.pos;
            let sm = stores.store_mut(slot);
            sm.set_byte(slot.rec, slot.pos, 0, JV_DISCR_INT);
            sm.set_int(slot.rec, val_pos, *n);
        }
        // Both `Str` and `Ident` materialise the same way — a
        // `JString` JsonValue.  `Ident` only arises under
        // `Dialect::Lenient`, which `n_json_parse` does not use
        // today; handling it here keeps the dispatcher exhaustive
        // without panicking if a future caller passes lenient
        // output through.
        crate::json::Parsed::Str(s) | crate::json::Parsed::Ident(s) => {
            let str_tp = stores.name("JString");
            let val_pos = u32::from(stores.position(str_tp, "value")) + slot.pos;
            let s_rec = stores.store_mut(slot).set_str(s);
            let sm = stores.store_mut(slot);
            sm.set_byte(slot.rec, slot.pos, 0, JV_DISCR_STRING);
            sm.set_u32_raw(slot.rec, val_pos, s_rec);
        }
        crate::json::Parsed::Array(v) => {
            // Step 4 fourth slice (2026-04-14) — recurse into nested
            // arrays.  The items vector lives in the slot's own
            // store (arena-in-store), so the whole sub-tree frees
            // with the root.
            let array_tp = stores.name("JArray");
            let items_field_pos = u32::from(stores.position(array_tp, "items"));
            let items_abs_pos = slot.pos + items_field_pos;
            let items_db = DbRef {
                store_nr: slot.store_nr,
                rec: slot.rec,
                pos: items_abs_pos,
            };
            let jv_tp = stores.name("JsonValue");
            let jv_size = u32::from(stores.size(jv_tp));
            let sm = stores.store_mut(slot);
            sm.set_byte(slot.rec, slot.pos, 0, JV_DISCR_ARRAY);
            sm.set_u32_raw(slot.rec, items_abs_pos, 0);
            for inner in v {
                let elm = crate::vector::vector_append(&items_db, jv_size, &mut stores.allocations);
                materialise_primitive_into(stores, &elm, inner);
                crate::vector::vector_finish(&items_db, &mut stores.allocations);
            }
        }
        crate::json::Parsed::Object(v) => {
            // Step 4 fourth slice — recurse into nested objects.
            // Mirrors the top-level object branch in n_json_parse.
            let obj_tp = stores.name("JObject");
            let fields_field_pos = u32::from(stores.position(obj_tp, "fields"));
            let fields_abs_pos = slot.pos + fields_field_pos;
            let fields_db = DbRef {
                store_nr: slot.store_nr,
                rec: slot.rec,
                pos: fields_abs_pos,
            };
            let jf_tp = stores.name("JsonField");
            let jf_size = u32::from(stores.size(jf_tp));
            let name_field_pos = u32::from(stores.position(jf_tp, "name"));
            let value_field_pos = u32::from(stores.position(jf_tp, "value"));
            let sm = stores.store_mut(slot);
            sm.set_byte(slot.rec, slot.pos, 0, JV_DISCR_OBJECT);
            sm.set_u32_raw(slot.rec, fields_abs_pos, 0);
            for (key, _key_at, inner) in v {
                let elm =
                    crate::vector::vector_append(&fields_db, jf_size, &mut stores.allocations);
                let name_rec = stores.store_mut(&elm).set_str(key);
                stores
                    .store_mut(&elm)
                    .set_u32_raw(elm.rec, elm.pos + name_field_pos, name_rec);
                let value_slot = DbRef {
                    store_nr: elm.store_nr,
                    rec: elm.rec,
                    pos: elm.pos + value_field_pos,
                };
                materialise_primitive_into(stores, &value_slot, inner);
                crate::vector::vector_finish(&fields_db, &mut stores.allocations);
            }
        }
        // `Constructor` (`Tag{…}`) is Lenient-only; the Strict `json_parse`
        // path never produces it.  Materialise as the equivalent single-field
        // `JObject` `{tag: body}` so the dispatcher stays exhaustive and a future
        // lenient caller degrades sensibly rather than panicking.
        crate::json::Parsed::Constructor(tag, tag_at, body) => {
            let synthetic =
                crate::json::Parsed::Object(vec![(tag.clone(), *tag_at, (**body).clone())]);
            materialise_primitive_into(stores, slot, &synthetic);
        }
    }
}

fn n_json_parse(stores: &mut Stores, stack: &mut DbRef) {
    let v_raw = *stores.get::<Str>(stack);
    let result = json_parse_into_stores(stores, v_raw.str());
    stores.put(stack, result);
}

/// P268: shared materialisation helper extracted from the interp
/// `n_json_parse` body so the native `--native` runtime stub
/// (`codegen_runtime::n_json_parse`) can call the same logic without
/// going through the bytecode VM's `(stores, stack)` calling
/// convention.  Returns the allocated `JsonValue` `DbRef`; updates
/// `stores.last_json_errors` exactly as the interp path does.
pub fn json_parse_into_stores(stores: &mut Stores, raw: &str) -> DbRef {
    let parsed = crate::json::parse(raw);
    let result = jv_alloc(stores);
    let pos = result.pos;
    match parsed {
        Ok(crate::json::Parsed::Null) => {
            stores
                .store_mut(&result)
                .set_byte(result.rec, pos, 0, JV_DISCR_NULL);
            stores.last_json_errors.clear();
        }
        Ok(crate::json::Parsed::Bool(b)) => {
            let bool_tp = stores.name("JBool");
            let value_pos = u32::from(stores.position(bool_tp, "value")) + pos;
            let store_mut = stores.store_mut(&result);
            store_mut.set_byte(result.rec, pos, 0, JV_DISCR_BOOL);
            store_mut.set_byte(result.rec, value_pos, 0, i32::from(b));
            stores.last_json_errors.clear();
        }
        Ok(crate::json::Parsed::Number(n)) => {
            let num_tp = stores.name("JNumber");
            let value_pos = u32::from(stores.position(num_tp, "value")) + pos;
            let store_mut = stores.store_mut(&result);
            store_mut.set_byte(result.rec, pos, 0, JV_DISCR_NUMBER);
            store_mut.set_float(result.rec, value_pos, n);
            stores.last_json_errors.clear();
        }
        // @PLN109 — an integer-shaped top-level number → `JInteger` (exact i64).
        Ok(crate::json::Parsed::Int(n)) => {
            let int_tp = stores.name("JInteger");
            let value_pos = u32::from(stores.position(int_tp, "value")) + pos;
            let store_mut = stores.store_mut(&result);
            store_mut.set_byte(result.rec, pos, 0, JV_DISCR_INT);
            store_mut.set_int(result.rec, value_pos, n);
            stores.last_json_errors.clear();
        }
        // `Ident` is only emitted under `Dialect::Lenient`; the
        // call above uses `parse` (Strict) so this arm is
        // structurally unreachable today but kept for exhaustive
        // coverage, rendering `Ident(x)` as the same JString as a
        // quoted `"x"` would.
        Ok(crate::json::Parsed::Str(s) | crate::json::Parsed::Ident(s)) => {
            let str_tp = stores.name("JString");
            let value_pos = u32::from(stores.position(str_tp, "value")) + pos;
            let s_rec = stores.store_mut(&result).set_str(&s);
            let store_mut = stores.store_mut(&result);
            store_mut.set_byte(result.rec, pos, 0, JV_DISCR_STRING);
            store_mut.set_u32_raw(result.rec, value_pos, s_rec);
            stores.last_json_errors.clear();
        }
        Ok(crate::json::Parsed::Array(v)) if v.is_empty() => {
            // Empty array: set the JArray discriminant AND explicitly zero
            // the items-vector handle.  @P357 — `jv_alloc` claims a RECYCLED
            // store record whose bytes are stale, so the items handle is NOT
            // zero-initialised by allocation (the original comment here wrongly
            // assumed it was).  Left stale, `item(i)`/`len` read a phantom
            // non-empty vector (e.g. `json_parse("[]").item(0)` returning a
            // garbage object and `len` reporting 8 after earlier parses
            // populated then freed that block — surfaced in the training port's
            // store engine on a real export with an empty `ghost_sessions.json`).
            // The non-empty arm below already zeros this handle (so
            // `vector_append` claims a fresh record); the empty arm must too.
            let array_tp = stores.name("JArray");
            let items_abs_pos = pos + u32::from(stores.position(array_tp, "items"));
            let store_mut = stores.store_mut(&result);
            store_mut.set_byte(result.rec, pos, 0, JV_DISCR_ARRAY);
            store_mut.set_u32_raw(result.rec, items_abs_pos, 0);
            stores.last_json_errors.clear();
        }
        Ok(crate::json::Parsed::Object(v)) if v.is_empty() => {
            // Empty object: same @P357 fix as the empty-array arm — zero the
            // stale fields-vector handle, not just the discriminant.
            let obj_tp = stores.name("JObject");
            let fields_abs_pos = pos + u32::from(stores.position(obj_tp, "fields"));
            let store_mut = stores.store_mut(&result);
            store_mut.set_byte(result.rec, pos, 0, JV_DISCR_OBJECT);
            store_mut.set_u32_raw(result.rec, fields_abs_pos, 0);
            stores.last_json_errors.clear();
        }
        Ok(crate::json::Parsed::Array(ref v)) => {
            // Step 4 second + fourth slices (2026-04-14): non-empty
            // arrays.  Elements are materialised via `vector_append`
            // into a sub-record inside the root JsonValue's store
            // (arena-in-store).  Nested containers recurse via
            // `materialise_primitive_into` (which despite the name
            // also handles Array / Object now).
            let array_tp = stores.name("JArray");
            let items_field_pos = u32::from(stores.position(array_tp, "items"));
            let items_abs_pos = pos + items_field_pos;
            let items_db = DbRef {
                store_nr: result.store_nr,
                rec: result.rec,
                pos: items_abs_pos,
            };
            let jv_tp = stores.name("JsonValue");
            let jv_size = u32::from(stores.size(jv_tp));
            let store_mut = stores.store_mut(&result);
            store_mut.set_byte(result.rec, pos, 0, JV_DISCR_ARRAY);
            // Zero the items-vector handle (record #) so vector_append
            // claims a fresh vector record on the first iteration.
            store_mut.set_u32_raw(result.rec, items_abs_pos, 0);
            for child in v {
                let elm = crate::vector::vector_append(&items_db, jv_size, &mut stores.allocations);
                materialise_primitive_into(stores, &elm, child);
                crate::vector::vector_finish(&items_db, &mut stores.allocations);
            }
            stores.last_json_errors.clear();
        }
        Ok(crate::json::Parsed::Object(ref v)) => {
            // Step 4 third + fourth slices (2026-04-14): non-empty
            // objects.  Each (name, value) pair becomes a
            // `JsonField` element in the fields vector, stored in
            // the root's arena.  Nested containers in values
            // recurse via `materialise_primitive_into`.
            let obj_tp = stores.name("JObject");
            let fields_field_pos = u32::from(stores.position(obj_tp, "fields"));
            let fields_abs_pos = pos + fields_field_pos;
            let fields_db = DbRef {
                store_nr: result.store_nr,
                rec: result.rec,
                pos: fields_abs_pos,
            };
            let jf_tp = stores.name("JsonField");
            let jf_size = u32::from(stores.size(jf_tp));
            let name_field_pos = u32::from(stores.position(jf_tp, "name"));
            let value_field_pos = u32::from(stores.position(jf_tp, "value"));
            let store_mut = stores.store_mut(&result);
            store_mut.set_byte(result.rec, pos, 0, JV_DISCR_OBJECT);
            store_mut.set_u32_raw(result.rec, fields_abs_pos, 0);
            for (key, _key_at, child) in v {
                let elm =
                    crate::vector::vector_append(&fields_db, jf_size, &mut stores.allocations);
                // Write name: set_str claims a sub-record for the
                // key bytes; store its record-nr in the name field.
                let name_rec = stores.store_mut(&elm).set_str(key);
                stores
                    .store_mut(&elm)
                    .set_u32_raw(elm.rec, elm.pos + name_field_pos, name_rec);
                // Write value: inline JsonValue at the value-field
                // offset within the JsonField slot.
                let value_slot = DbRef {
                    store_nr: elm.store_nr,
                    rec: elm.rec,
                    pos: elm.pos + value_field_pos,
                };
                materialise_primitive_into(stores, &value_slot, child);
                crate::vector::vector_finish(&fields_db, &mut stores.allocations);
            }
            stores.last_json_errors.clear();
        }
        // `Constructor` (`Tag{…}`) is Lenient-only; this Strict path never emits
        // it.  Materialise via the shared helper (which renders it as the
        // equivalent single-field object) so the match stays exhaustive.
        Ok(c @ crate::json::Parsed::Constructor(..)) => {
            materialise_primitive_into(stores, &result, &c);
            stores.last_json_errors.clear();
        }
        Err(err) => {
            stores
                .store_mut(&result)
                .set_byte(result.rec, pos, 0, JV_DISCR_NULL);
            stores.last_json_errors.clear();
            stores
                .last_json_errors
                .push(crate::json::format_error(raw, &err, 2, 1));
        }
    }
    result
}

// @PLN10 — destination-passing variant of `n_json_errors`.
fn n_json_errors_dest(stores: &mut Stores, stack: &mut DbRef) {
    let dest = *stores.get::<DbRef>(stack);
    let msg = stores.last_json_errors.join("|");
    stores
        .store_mut(&dest)
        .addr_mut::<String>(dest.rec, dest.pos)
        .push_str(&msg);
}

// @PLN10 Phase 2 — destination-passing variant of `n_as_text`.  text-null is
// CONTENT-based (`conv_bool_from_text`: content == "\0"), so the dest carries
// null by holding the `STRING_NULL` ("\0") bytes — `?? ` / `!` / format all read
// it identically to the old sentinel (probed on both backends).  So `as_text`
// dest-passes like any other producer; no "null-aware primitive" is needed.
// Bonus: per-call dests retire the @P354 sibling-aliasing scratch hazard.
fn n_as_text_dest(stores: &mut Stores, stack: &mut DbRef) {
    let dest = *stores.get::<DbRef>(stack);
    let v = *stores.get::<DbRef>(stack);
    let discr = stores.store(&v).get_byte(v.rec, v.pos, 0);
    let out: String = if discr == JV_DISCR_STRING {
        let str_tp = stores.name("JString");
        let value_pos = u32::from(stores.position(str_tp, "value")) + v.pos;
        let s_rec = stores.store(&v).get_u32_raw(v.rec, value_pos);
        stores.store(&v).get_str(s_rec).to_string()
    } else {
        crate::state::STRING_NULL.to_string()
    };
    stores
        .store_mut(&dest)
        .addr_mut::<String>(dest.rec, dest.pos)
        .push_str(&out);
}

fn n_as_number(stores: &mut Stores, stack: &mut DbRef) {
    let v = *stores.get::<DbRef>(stack);
    let discr = stores.store(&v).get_byte(v.rec, v.pos, 0);
    // @PLN109 — a JInteger widens to f64; a JNumber reads as-is; else NaN.
    #[allow(clippy::cast_precision_loss)]
    let n = if discr == JV_DISCR_NUMBER {
        let num_tp = stores.name("JNumber");
        let value_pos = u32::from(stores.position(num_tp, "value")) + v.pos;
        stores.store(&v).get_float(v.rec, value_pos)
    } else if discr == JV_DISCR_INT {
        let int_tp = stores.name("JInteger");
        let value_pos = u32::from(stores.position(int_tp, "value")) + v.pos;
        stores.store(&v).get_int(v.rec, value_pos) as f64
    } else {
        f64::NAN
    };
    stores.put(stack, n);
}

fn n_as_long(stores: &mut Stores, stack: &mut DbRef) {
    let v = *stores.get::<DbRef>(stack);
    let discr = stores.store(&v).get_byte(v.rec, v.pos, 0);
    // @PLN109 — a JInteger reads its EXACT i64 (H5); a JNumber truncates; else MIN.
    let n = if discr == JV_DISCR_INT {
        let int_tp = stores.name("JInteger");
        let value_pos = u32::from(stores.position(int_tp, "value")) + v.pos;
        stores.store(&v).get_int(v.rec, value_pos)
    } else if discr == JV_DISCR_NUMBER {
        let num_tp = stores.name("JNumber");
        let value_pos = u32::from(stores.position(num_tp, "value")) + v.pos;
        stores.store(&v).get_float(v.rec, value_pos).trunc() as i64
    } else {
        i64::MIN
    };
    stores.put(stack, n);
}

fn n_as_bool(stores: &mut Stores, stack: &mut DbRef) {
    let v = *stores.get::<DbRef>(stack);
    let discr = stores.store(&v).get_byte(v.rec, v.pos, 0);
    if discr == JV_DISCR_BOOL {
        let bool_tp = stores.name("JBool");
        let value_pos = u32::from(stores.position(bool_tp, "value")) + v.pos;
        let b = stores.store(&v).get_byte(v.rec, value_pos, 0) != 0;
        stores.put(stack, b);
    } else {
        stores.put(stack, false);
    }
}

/// JObject indexer.  Dispatches on the discriminant: for a real
/// JObject, linear-scans the arena `fields` vector by name and
/// returns a borrowed DbRef into the matching value slot.  For
/// any other variant or a missing key, returns a fresh `JNull`
/// so chained access stays safe (every intermediate failure
/// produces `JNull`, never a trap).
fn n_field(stores: &mut Stores, stack: &mut DbRef) {
    let name = *stores.get::<Str>(stack);
    let self_ref = *stores.get::<DbRef>(stack);
    let discr = stores
        .store(&self_ref)
        .get_byte(self_ref.rec, self_ref.pos, 0);
    if discr != JV_DISCR_OBJECT {
        let r = jv_null_sentinel(stores);
        stores.put(stack, r);
        return;
    }
    let obj_tp = stores.name("JObject");
    let fields_pos = u32::from(stores.position(obj_tp, "fields")) + self_ref.pos;
    let fields_rec = stores
        .store(&self_ref)
        .get_i32_raw(self_ref.rec, fields_pos);
    if fields_rec <= 0 {
        let r = jv_null_sentinel(stores);
        stores.put(stack, r);
        return;
    }
    let length = i64::from(stores.store(&self_ref).get_u32_raw(fields_rec as u32, 4));
    let jf_tp = stores.name("JsonField");
    let jf_size = u32::from(stores.size(jf_tp));
    let name_field_pos = u32::from(stores.position(jf_tp, "name"));
    let value_field_pos = u32::from(stores.position(jf_tp, "value"));
    let lookup = name.str().to_owned();
    for i in 0..length {
        let elm_offset = 8u32 + u32::try_from(i).expect("non-negative length") * jf_size;
        let name_rec = stores
            .store(&self_ref)
            .get_u32_raw(fields_rec as u32, elm_offset + name_field_pos);
        let stored_name = stores.store(&self_ref).get_str(name_rec).to_owned();
        if stored_name == lookup {
            let value_ref = DbRef {
                store_nr: self_ref.store_nr,
                rec: fields_rec as u32,
                pos: elm_offset + value_field_pos,
            };
            stores.put(stack, value_ref);
            return;
        }
    }
    let r = jv_null_sentinel(stores);
    stores.put(stack, r);
}

/// JArray indexer.  Step 4 second slice: for a real JArray with
/// primitive elements, reads the arena sub-record at
/// `8 + index * sizeof(JsonValue)` and returns a DbRef to the
/// element.  Out-of-range indices, non-JArray receivers, and
/// empty arrays return a fresh `JNull`.
///
/// The returned DbRef points INTO the parent's store (not a
/// fresh one) — it's a borrowed view that lives as long as the
/// parent's store does.  Matches the file-pattern arena contract.
fn n_item(stores: &mut Stores, stack: &mut DbRef) {
    let index = *stores.get::<i64>(stack) as i32;
    let self_ref = *stores.get::<DbRef>(stack);
    let discr = stores
        .store(&self_ref)
        .get_byte(self_ref.rec, self_ref.pos, 0);
    if discr != JV_DISCR_ARRAY || index < 0 {
        let r = jv_null_sentinel(stores);
        stores.put(stack, r);
        return;
    }
    let array_tp = stores.name("JArray");
    let items_pos = u32::from(stores.position(array_tp, "items")) + self_ref.pos;
    let items_rec = stores.store(&self_ref).get_i32_raw(self_ref.rec, items_pos);
    if items_rec <= 0 {
        let r = jv_null_sentinel(stores);
        stores.put(stack, r);
        return;
    }
    let length = stores.store(&self_ref).get_u32_raw(items_rec as u32, 4) as i32;
    if index >= length {
        let r = jv_null_sentinel(stores);
        stores.put(stack, r);
        return;
    }
    let jv_tp = stores.name("JsonValue");
    let jv_size = u32::from(stores.size(jv_tp));
    let elm_offset =
        8u32 + u32::try_from(index).expect("non-negative index checked above") * jv_size;
    let elm_ref = DbRef {
        store_nr: self_ref.store_nr,
        rec: items_rec as u32,
        pos: elm_offset,
    };
    stores.put(stack, elm_ref);
}

/// JArray / JObject length.  Primitive variants return the integer
/// null sentinel (`i32::MIN`) — "no length defined".  Both
/// container variants read the arena sub-vector's length word at
/// offset 4 of the vector record; empty containers (no record
/// allocated) return 0.
fn n_len(stores: &mut Stores, stack: &mut DbRef) {
    let v = *stores.get::<DbRef>(stack);
    let discr = stores.store(&v).get_byte(v.rec, v.pos, 0);
    let len: i64 = match discr {
        JV_DISCR_ARRAY => {
            let array_tp = stores.name("JArray");
            let items_pos = u32::from(stores.position(array_tp, "items")) + v.pos;
            let items_rec = stores.store(&v).get_i32_raw(v.rec, items_pos);
            if items_rec <= 0 {
                0
            } else {
                i64::from(stores.store(&v).get_u32_raw(items_rec as u32, 4))
            }
        }
        JV_DISCR_OBJECT => {
            let obj_tp = stores.name("JObject");
            let fields_pos = u32::from(stores.position(obj_tp, "fields")) + v.pos;
            let fields_rec = stores.store(&v).get_i32_raw(v.rec, fields_pos);
            if fields_rec <= 0 {
                0
            } else {
                i64::from(stores.store(&v).get_u32_raw(fields_rec as u32, 4))
            }
        }
        _ => i64::MIN,
    };
    stores.put(stack, len);
}

// ─────────────── single-walker `Struct.parse(JsonValue)` ──────────────────
//
// `n_struct_from_jsonvalue` is the single source of truth for unwrapping
// a `JsonValue` into a struct.  The compile-time `parse_type_parse`
// emits exactly one call to this function regardless of struct shape.
// The walker uses `stores.types[struct_kt].parts` to enumerate fields
// at runtime and dispatches on each field's declared type:
//
//   primitive (text/long/integer/float/boolean) → unwrap with Q1 schema-
//                                                 side type-mismatch check
//   `Type::Reference(struct_d, _)`              → recurse on the embedded
//                                                 sub-struct (no
//                                                 separate alloc — the
//                                                 nested struct's bytes
//                                                 live inline at the
//                                                 field's position)
//   `Type::Enum(jv_d, true, _)` (JsonValue)     → byte-copy the field's
//                                                 JsonValue bytes
//   `Type::Vector(inner, _)`                    → iterate the JArray and
//                                                 append per element via
//                                                 `vector_append`,
//                                                 recursing into the
//                                                 walker for struct
//                                                 elements

/// Walker entry point — pops `(src: DbRef, struct_kt: i32)` from the
/// stack, allocates a struct of `struct_kt`, populates its fields from
/// `src`, and pushes the result DbRef.  The compile-time codegen calls
/// this for every `Struct.parse(JsonValue)` invocation.
fn n_struct_from_jsonvalue(stores: &mut Stores, stack: &mut DbRef) {
    let struct_kt_arg = *stores.get::<i64>(stack) as i32;
    let src = *stores.get::<DbRef>(stack);
    let struct_kt = struct_kt_arg as u16;
    // `stores.size` returns the struct's size in bytes; `database`
    // wants words (8 bytes each).  Round up + 1 word for the record
    // header (matches `jv_alloc`); without the `+1`, struct fields at
    // offset >= (words*8 - 8) would land past the record's tail —
    // `populate_struct_from_jsonvalue` panicked at the second field
    // of `WithPayload { name: text, info: JsonValue }` because name
    // ended up at struct-relative offset 16 in a record whose data
    // area was only 16 bytes.  See `p54_struct_parse_captures_jsonvalue_field_verbatim`.
    let bytes = u32::from(stores.size(struct_kt));
    let words = bytes.div_ceil(8) + 1;
    let result = stores.database(words.max(2));
    populate_struct_from_jsonvalue(stores, &result, struct_kt, &src);
    stores.put(stack, result);
}

/// Internal helper: populate the struct at `dest` (already allocated,
/// `dest.pos = 8`) from the JsonValue at `src`.  Walks every declared
/// field via `Stores::types[struct_kt].parts`, looks up each field by
/// name in `src` (which must be a `JObject` for any field lookup to
/// succeed — wrong-kind sources leave every field at zero-init), and
/// dispatches on the field's declared type.
pub(crate) fn populate_struct_from_jsonvalue(
    stores: &mut Stores,
    dest: &DbRef,
    struct_kt: u16,
    src: &DbRef,
) {
    use crate::database::Parts;
    // Cache the well-known type known_types so per-field dispatch is an
    // integer compare, not a name compare.
    let kt_long = stores.name("long");
    let kt_int = stores.name("integer");
    let kt_float = stores.name("float");
    let kt_bool = stores.name("boolean");
    let kt_text = stores.name("text");
    // Parts::Struct(_) iteration: clone the field list because we need
    // a long-lived borrow on `stores` for the writes below.
    let fields = match &stores.types[struct_kt as usize].parts {
        Parts::Struct(f) => f.clone(),
        _ => return,
    };
    let struct_name = stores.types[struct_kt as usize].name.clone();
    for field in &fields {
        let content_kt = field.content;
        let dest_field_pos = dest.pos + u32::from(field.position);
        // Find the JSON sub-value by name.  Absent → synthesise a
        // JNull discriminant so the unwrap functions write each
        // field's null sentinel (matches the legacy
        // `Type.parse(text)` behaviour where missing fields land
        // as null, not zero-init bytes).
        let sub_jv = lookup_jobject_field(stores, src, &field.name);
        let item_discr = match &sub_jv {
            Some(s) => stores.store(s).get_byte(s.rec, s.pos, 0),
            None => JV_DISCR_NULL,
        };
        // Dummy ref for absent fields — the unwrap functions
        // short-circuit on JNull/wrong-kind and never read from sub
        // unless the discriminant matches.
        let sub = sub_jv.unwrap_or(*dest);
        // Dispatch on the field's declared content type.  For
        // primitive types we cache-compare via known_type.  For
        // nested struct, vector, and JsonValue passthrough we look
        // at the content type's `Parts` variant.
        if content_kt == kt_long {
            let value = unwrap_long(stores, &sub, item_discr, &struct_name, &field.name);
            stores
                .store_mut(dest)
                .set_long(dest.rec, dest_field_pos, value);
        } else if content_kt == kt_int {
            let value = unwrap_int(stores, &sub, item_discr, &struct_name, &field.name);
            stores
                .store_mut(dest)
                .set_int(dest.rec, dest_field_pos, value);
        } else if content_kt == kt_float {
            let value = unwrap_float(stores, &sub, item_discr, &struct_name, &field.name);
            stores
                .store_mut(dest)
                .set_float(dest.rec, dest_field_pos, value);
        } else if content_kt == kt_bool {
            let value = unwrap_bool(stores, &sub, item_discr, &struct_name, &field.name);
            stores
                .store_mut(dest)
                .set_byte(dest.rec, dest_field_pos, 0, value);
        } else if content_kt == kt_text {
            // Text null sentinel is a 0 str_rec (read-back via
            // `get_str(0)` returns `STRING_NULL = "\0"` which loft
            // treats as null).  When the source is absent or the
            // wrong kind, write 0 directly instead of allocating an
            // empty string — empty `""` is a real (non-null) text
            // and would break the legacy `!field` null check.
            push_kind_mismatch(
                stores,
                item_discr,
                JV_DISCR_STRING,
                &struct_name,
                &field.name,
            );
            if item_discr == JV_DISCR_STRING {
                let str_tp = stores.name("JString");
                let value_pos = u32::from(stores.position(str_tp, "value")) + sub.pos;
                let s_rec = stores.store(&sub).get_u32_raw(sub.rec, value_pos);
                let text_val = stores.store(&sub).get_str(s_rec).to_owned();
                let new_s_rec = stores.store_mut(dest).set_str(&text_val);
                stores
                    .store_mut(dest)
                    .set_u32_raw(dest.rec, dest_field_pos, new_s_rec);
            } else {
                stores
                    .store_mut(dest)
                    .set_u32_raw(dest.rec, dest_field_pos, 0);
            }
        } else {
            // Look at the field type's Parts to decide what to do.
            match stores.types[content_kt as usize].parts.clone() {
                Parts::Struct(_) => {
                    // Nested struct: the sub-struct's bytes live inline
                    // at the field's position.  Recurse into the walker
                    // with a DbRef pointing at the embedded slot.  A
                    // wrong-kind / absent source still gets recursed —
                    // the inner walker's `lookup_jobject_field` will
                    // return None for every field and the inner
                    // primitives all land at their null sentinels via
                    // the same JNull-synthesis path used here.
                    let nested_dest = DbRef {
                        store_nr: dest.store_nr,
                        rec: dest.rec,
                        pos: dest_field_pos,
                    };
                    populate_struct_from_jsonvalue(stores, &nested_dest, content_kt, &sub);
                }
                Parts::EnumValue(_, _) | Parts::Enum(_)
                    // Mixed struct-enum field — only `JsonValue`
                    // passthrough is supported today.  Skip the copy
                    // when the source is absent (sub is a dummy
                    // pointing at the dest, copy would garble the
                    // dest's own bytes).
                    if sub_jv.is_some() => {
                        let inner_name = stores.types[content_kt as usize].name.clone();
                        if inner_name == "JsonValue" {
                            let jv_size = u32::from(stores.size(content_kt));
                            copy_bytes(stores, &sub, dest, dest_field_pos, jv_size);
                        }
                    }
                    // Other struct-enum types: leave at default.
                Parts::Vector(elem_kt)
                    // Vector field: handle is a 4-byte rec-nr at
                    // `dest_field_pos`.  Iterate JArray items and
                    // append per element via the existing
                    // `vector_append` machinery.  Absent source →
                    // skip (handle stays at zero = empty vector).
                    if sub_jv.is_some() => {
                        let dest_handle = DbRef {
                            store_nr: dest.store_nr,
                            rec: dest.rec,
                            pos: dest_field_pos,
                        };
                        populate_vector_from_jarray(stores, &dest_handle, elem_kt, &sub);
                    }
                _ => {
                    // Other field types (Hash, Sorted, Index, Radix,
                    // Array, Base, Byte, Short) are not yet handled.
                    // Leave at zero-init default.
                }
            }
        }
    }
}

/// Find a field by name in a JObject's fields vector.  Returns a DbRef
/// pointing at the field's value slot (suitable for further dispatch)
/// or None if the source isn't a JObject or the name isn't present.
fn lookup_jobject_field(stores: &Stores, src: &DbRef, name: &str) -> Option<DbRef> {
    let src_discr = stores.store(src).get_byte(src.rec, src.pos, 0);
    if src_discr != JV_DISCR_OBJECT {
        return None;
    }
    let obj_tp = stores.name("JObject");
    let fields_pos = u32::from(stores.position(obj_tp, "fields")) + src.pos;
    let fields_rec = stores.store(src).get_i32_raw(src.rec, fields_pos);
    if fields_rec <= 0 {
        return None;
    }
    let length = i64::from(stores.store(src).get_u32_raw(fields_rec as u32, 4));
    let jf_tp = stores.name("JsonField");
    let jf_size = u32::from(stores.size(jf_tp));
    let name_field_pos = u32::from(stores.position(jf_tp, "name"));
    let value_field_pos = u32::from(stores.position(jf_tp, "value"));
    for i in 0..length {
        let elm_off = 8u32 + u32::try_from(i).expect("non-negative") * jf_size;
        let name_rec = stores
            .store(src)
            .get_u32_raw(fields_rec as u32, elm_off + name_field_pos);
        if stores.store(src).get_str(name_rec) == name {
            return Some(DbRef {
                store_nr: src.store_nr,
                rec: fields_rec as u32,
                pos: elm_off + value_field_pos,
            });
        }
    }
    None
}

/// Q1 schema-side: push a path-qualified diagnostic when a field's
/// JsonValue has the wrong discriminant.  Absent fields (JNull) pass
/// silently — only a non-null wrong kind triggers the diagnostic.
fn push_kind_mismatch(
    stores: &mut Stores,
    actual_discr: i32,
    expected_discr: i32,
    struct_name: &str,
    field_name: &str,
) {
    if actual_discr == JV_DISCR_NULL || actual_discr == expected_discr {
        return;
    }
    let actual_name = match actual_discr {
        JV_DISCR_NULL => "JNull",
        JV_DISCR_BOOL => "JBool",
        JV_DISCR_NUMBER => "JNumber",
        JV_DISCR_STRING => "JString",
        JV_DISCR_ARRAY => "JArray",
        JV_DISCR_OBJECT => "JObject",
        JV_DISCR_INT => "JInteger",
        _ => "JUnknown",
    };
    let expected_name = match expected_discr {
        JV_DISCR_BOOL => "JBool",
        JV_DISCR_NUMBER => "JNumber",
        JV_DISCR_STRING => "JString",
        JV_DISCR_ARRAY => "JArray",
        JV_DISCR_OBJECT => "JObject",
        _ => "?",
    };
    stores.last_json_errors.push(format!(
        "{struct_name}.{field_name}: expected {expected_name}, got {actual_name}"
    ));
}

/// @PLN109 — read the exact i64 out of a `JInteger` store value, or `None` if
/// `item_discr` is not `JV_DISCR_INT`.  Integer-shaped JSON numbers materialise
/// as `JInteger` (H5), so `unwrap_long`/`unwrap_int` read them without f64
/// rounding; `unwrap_float` widens them.
fn jinteger_value(stores: &Stores, sub: &DbRef, item_discr: i32) -> Option<i64> {
    if item_discr != JV_DISCR_INT {
        return None;
    }
    let int_tp = stores.name("JInteger");
    let value_pos = u32::from(stores.position(int_tp, "value")) + sub.pos;
    Some(stores.store(sub).get_int(sub.rec, value_pos))
}

fn unwrap_long(
    stores: &mut Stores,
    sub: &DbRef,
    item_discr: i32,
    struct_name: &str,
    field_name: &str,
) -> i64 {
    if let Some(n) = jinteger_value(stores, sub, item_discr) {
        return n;
    }
    push_kind_mismatch(stores, item_discr, JV_DISCR_NUMBER, struct_name, field_name);
    if item_discr != JV_DISCR_NUMBER {
        return i64::MIN;
    }
    let num_tp = stores.name("JNumber");
    let value_pos = u32::from(stores.position(num_tp, "value")) + sub.pos;
    let f = stores.store(sub).get_float(sub.rec, value_pos);
    if f.is_finite() { f as i64 } else { i64::MIN }
}

fn unwrap_int(
    stores: &mut Stores,
    sub: &DbRef,
    item_discr: i32,
    struct_name: &str,
    field_name: &str,
) -> i64 {
    if let Some(n) = jinteger_value(stores, sub, item_discr) {
        return n;
    }
    push_kind_mismatch(stores, item_discr, JV_DISCR_NUMBER, struct_name, field_name);
    if item_discr != JV_DISCR_NUMBER {
        return i64::MIN;
    }
    let num_tp = stores.name("JNumber");
    let value_pos = u32::from(stores.position(num_tp, "value")) + sub.pos;
    let f = stores.store(sub).get_float(sub.rec, value_pos);
    if !f.is_finite() {
        return i64::MIN;
    }
    f as i64
}

fn unwrap_float(
    stores: &mut Stores,
    sub: &DbRef,
    item_discr: i32,
    struct_name: &str,
    field_name: &str,
) -> f64 {
    // A JInteger fed to a float field widens to f64 (no mismatch).
    #[allow(clippy::cast_precision_loss)]
    if let Some(n) = jinteger_value(stores, sub, item_discr) {
        return n as f64;
    }
    push_kind_mismatch(stores, item_discr, JV_DISCR_NUMBER, struct_name, field_name);
    if item_discr != JV_DISCR_NUMBER {
        return f64::NAN;
    }
    let num_tp = stores.name("JNumber");
    let value_pos = u32::from(stores.position(num_tp, "value")) + sub.pos;
    stores.store(sub).get_float(sub.rec, value_pos)
}

fn unwrap_bool(
    stores: &mut Stores,
    sub: &DbRef,
    item_discr: i32,
    struct_name: &str,
    field_name: &str,
) -> i32 {
    push_kind_mismatch(stores, item_discr, JV_DISCR_BOOL, struct_name, field_name);
    if item_discr != JV_DISCR_BOOL {
        return 0;
    }
    let bool_tp = stores.name("JBool");
    let value_pos = u32::from(stores.position(bool_tp, "value")) + sub.pos;
    stores.store(sub).get_byte(sub.rec, value_pos, 0)
}

fn unwrap_text(
    stores: &mut Stores,
    sub: &DbRef,
    item_discr: i32,
    struct_name: &str,
    field_name: &str,
) -> String {
    push_kind_mismatch(stores, item_discr, JV_DISCR_STRING, struct_name, field_name);
    if item_discr != JV_DISCR_STRING {
        return String::new();
    }
    let str_tp = stores.name("JString");
    let value_pos = u32::from(stores.position(str_tp, "value")) + sub.pos;
    let s_rec = stores.store(sub).get_u32_raw(sub.rec, value_pos);
    stores.store(sub).get_str(s_rec).to_owned()
}

/// Byte-copy `n_bytes` from `src` to `(dest.rec, dest_pos)` — used for
/// the JsonValue-passthrough field case.  The runtime equivalent of the
/// compile-time `OpCopyRecord` op for an inline struct-enum field.
fn copy_bytes(stores: &mut Stores, src: &DbRef, dest: &DbRef, dest_pos: u32, n_bytes: u32) {
    // Snapshot the bytes first because writing to dest may borrow
    // stores mutably and invalidate the source pointer.
    let mut buf: Vec<u8> = Vec::with_capacity(n_bytes as usize);
    for i in 0..n_bytes {
        buf.push(*stores.store(src).addr::<u8>(src.rec, src.pos + i));
    }
    let dest_store = stores.store_mut(dest);
    for (i, byte) in buf.iter().enumerate() {
        *dest_store.addr_mut::<u8>(dest.rec, dest_pos + i as u32) = *byte;
    }
}

/// Populate a `vector<T>` field embedded in a struct from a JArray.
/// The dest handle is at `dest_handle` (a 4-byte rec-nr slot inside
/// the parent struct).  Iterates the JArray's items and for each one
/// appends to the vector via `vector_append`, dispatching on the
/// element type's `Parts`.
fn populate_vector_from_jarray(
    stores: &mut Stores,
    dest_handle: &DbRef,
    elem_kt: u16,
    src_arr: &DbRef,
) {
    use crate::database::Parts;
    let arr_discr = stores.store(src_arr).get_byte(src_arr.rec, src_arr.pos, 0);
    if arr_discr != JV_DISCR_ARRAY {
        return;
    }
    let array_tp = stores.name("JArray");
    let items_pos = u32::from(stores.position(array_tp, "items")) + src_arr.pos;
    let items_rec = stores.store(src_arr).get_i32_raw(src_arr.rec, items_pos);
    if items_rec <= 0 {
        return;
    }
    let length = i64::from(stores.store(src_arr).get_u32_raw(items_rec as u32, 4));
    let jv_tp = stores.name("JsonValue");
    let jv_size = u32::from(stores.size(jv_tp));
    let elem_size = u32::from(stores.size(elem_kt));
    let kt_long = stores.name("long");
    let kt_int = stores.name("integer");
    let kt_float = stores.name("float");
    let kt_bool = stores.name("boolean");
    let kt_text = stores.name("text");
    let elem_parts = stores.types[elem_kt as usize].parts.clone();
    let elem_name = stores.types[elem_kt as usize].name.clone();
    for i in 0..length {
        let elm_offset = 8u32 + u32::try_from(i).expect("non-negative") * jv_size;
        let item = DbRef {
            store_nr: src_arr.store_nr,
            rec: items_rec as u32,
            pos: elm_offset,
        };
        let item_discr = stores
            .store(src_arr)
            .get_byte(items_rec as u32, elm_offset, 0);
        let elm = crate::vector::vector_append(dest_handle, elem_size, &mut stores.allocations);
        if elem_kt == kt_long {
            let v = unwrap_long(stores, &item, item_discr, "vector", &elem_name);
            stores.store_mut(&elm).set_long(elm.rec, elm.pos, v);
        } else if elem_kt == kt_int {
            let v = unwrap_int(stores, &item, item_discr, "vector", &elem_name);
            stores.store_mut(&elm).set_int(elm.rec, elm.pos, v);
        } else if elem_kt == kt_float {
            let v = unwrap_float(stores, &item, item_discr, "vector", &elem_name);
            stores.store_mut(&elm).set_float(elm.rec, elm.pos, v);
        } else if elem_kt == kt_bool {
            let v = unwrap_bool(stores, &item, item_discr, "vector", &elem_name);
            stores.store_mut(&elm).set_byte(elm.rec, elm.pos, 0, v);
        } else if elem_kt == kt_text {
            let s = unwrap_text(stores, &item, item_discr, "vector", &elem_name);
            let new_s_rec = stores.store_mut(&elm).set_str(&s);
            stores
                .store_mut(&elm)
                .set_u32_raw(elm.rec, elm.pos, new_s_rec);
        } else if matches!(elem_parts, Parts::Struct(_)) {
            // Struct element — recurse into the walker writing into
            // the freshly-appended embedded element slot.
            populate_struct_from_jsonvalue(stores, &elm, elem_kt, &item);
        }
        crate::vector::vector_finish(dest_handle, &mut stores.allocations);
    }
}

// ───────────────────────────────────────────────────────────────────────
// Q3 second half — `T.to_json()` / `T.to_json_pretty()` for any user
// struct.  Mirror of `n_struct_from_jsonvalue` (P54 step 5) in the
// serialise direction.  The compile-time fallback in
// `src/parser/fields.rs::field()` lowers `instance.to_json()` to a
// single call to `n_struct_to_json(instance, struct_kt)` regardless
// of struct shape; the actual rendering is delegated to the
// existing `Stores::show_json` (in `src/database/format.rs`), which
// already walks every declared field via `Parts::Struct` /
// `Parts::Vector` / etc. and produces canonical JSON when its
// `json: true` flag is set.

// @PLN10 Phase 1 — destination-passing variants of `n_struct_to_json` /
// `n_struct_to_json_pretty`.  Always-non-null (canonical JSON text).
// Routed by `is_text_dest_native`.
fn n_struct_to_json_dest(stores: &mut Stores, stack: &mut DbRef) {
    struct_to_json_dispatch_dest(stores, stack, false);
}

fn n_struct_to_json_pretty_dest(stores: &mut Stores, stack: &mut DbRef) {
    struct_to_json_dispatch_dest(stores, stack, true);
}

fn struct_to_json_dispatch_dest(stores: &mut Stores, stack: &mut DbRef, pretty: bool) {
    let dest = *stores.get::<DbRef>(stack);
    let struct_kt_arg = *stores.get::<i64>(stack) as i32;
    let src = *stores.get::<DbRef>(stack);
    let struct_kt = struct_kt_arg as u16;
    let mut out = String::new();
    stores.show_json(&mut out, &src, struct_kt, pretty);
    stores
        .store_mut(&dest)
        .addr_mut::<String>(dest.rec, dest.pos)
        .push_str(&out);
}

/// Allocate a JsonValue set to the `JNull` variant and return a
/// DbRef to it.  No arena needed (JNull has no payload).  Useful
/// for test fixtures that want to construct a known-null JsonValue
/// without going through `json_parse("null")`.
fn n_json_null(stores: &mut Stores, stack: &mut DbRef) {
    let result = jv_alloc(stores);
    stores
        .store_mut(&result)
        .set_byte(result.rec, result.pos, 0, JV_DISCR_NULL);
    stores.last_json_errors.clear();
    stores.put(stack, result);
}

/// Q4 primitive constructor — allocate a JsonValue set to the
/// `JBool` variant with the supplied boolean payload.
fn n_json_bool(stores: &mut Stores, stack: &mut DbRef) {
    let v = *stores.get::<bool>(stack);
    let result = jv_alloc(stores);
    let pos = result.pos;
    let bool_tp = stores.name("JBool");
    let value_pos = u32::from(stores.position(bool_tp, "value")) + pos;
    let store_mut = stores.store_mut(&result);
    store_mut.set_byte(result.rec, pos, 0, JV_DISCR_BOOL);
    store_mut.set_byte(result.rec, value_pos, 0, i32::from(v));
    stores.last_json_errors.clear();
    stores.put(stack, result);
}

/// Q4 primitive constructor — allocate a JsonValue set to the
/// `JNumber` variant with the supplied float payload.  Rejects
/// non-finite inputs (NaN / ±Inf) by storing `JNull` + appending a
/// diagnostic to `json_errors()`, matching the spec'd
/// `to_json_pretty` behaviour for non-finite floats.
fn n_json_number(stores: &mut Stores, stack: &mut DbRef) {
    let n = *stores.get::<f64>(stack);
    let result = jv_alloc(stores);
    let pos = result.pos;
    if n.is_finite() {
        let num_tp = stores.name("JNumber");
        let value_pos = u32::from(stores.position(num_tp, "value")) + pos;
        let store_mut = stores.store_mut(&result);
        store_mut.set_byte(result.rec, pos, 0, JV_DISCR_NUMBER);
        store_mut.set_float(result.rec, value_pos, n);
        stores.last_json_errors.clear();
    } else {
        stores
            .store_mut(&result)
            .set_byte(result.rec, pos, 0, JV_DISCR_NULL);
        stores.last_json_errors.clear();
        stores
            .last_json_errors
            .push(format!("json_number: non-finite value {n} stored as JNull"));
    }
    stores.put(stack, result);
}

/// Q4 primitive constructor — allocate a JsonValue set to the
/// `JString` variant with the supplied text payload.  The string
/// is copied into the JsonValue's own store (same pattern as
/// `n_json_parse` primitives), so the returned DbRef owns the
/// text independently of the input's lifetime.
fn n_json_string(stores: &mut Stores, stack: &mut DbRef) {
    let v = *stores.get::<Str>(stack);
    let s_owned = v.str().to_owned();
    let result = jv_alloc(stores);
    let pos = result.pos;
    let str_tp = stores.name("JString");
    let value_pos = u32::from(stores.position(str_tp, "value")) + pos;
    let s_rec = stores.store_mut(&result).set_str(&s_owned);
    let store_mut = stores.store_mut(&result);
    store_mut.set_byte(result.rec, pos, 0, JV_DISCR_STRING);
    store_mut.set_u32_raw(result.rec, value_pos, s_rec);
    stores.last_json_errors.clear();
    stores.put(stack, result);
}

/// Q4 container constructor — `json_array(items: vector<JsonValue>)`
/// builds a `JArray` JsonValue carrying a deep-copy of the input
/// vector's elements in the new arena.  Each input element is
/// converted to `Parsed` via `dbref_to_parsed` (recursive read of
/// the source tree) and then written into the result arena via
/// the same `materialise_primitive_into` path `n_json_parse`
/// uses.  Result arena is independent of the caller's input; the
/// returned tree frees as one unit when the root DbRef leaves
/// scope.  Empty input still produces an empty JArray.
fn n_json_array(stores: &mut Stores, stack: &mut DbRef) {
    let items = *stores.get::<DbRef>(stack);
    let length = crate::vector::length_vector(&items, &stores.allocations);
    let result = jv_alloc(stores);
    if length == 0 {
        stores
            .store_mut(&result)
            .set_byte(result.rec, result.pos, 0, JV_DISCR_ARRAY);
        stores.last_json_errors.clear();
    } else {
        // Read the input vector's inner record and walk each slot
        // into a Parsed snapshot.  Done in two passes — read the
        // source under `&Stores`, then write into the dest under
        // `&mut Stores` — so the borrow checker stays happy.
        let input_inner_rec = stores.store(&items).get_u32_raw(items.rec, items.pos);
        let jv_tp = stores.name("JsonValue");
        let jv_size = u32::from(stores.size(jv_tp));
        let mut children = Vec::with_capacity(length as usize);
        for i in 0..length {
            let elem_offset = 8u32 + i * jv_size;
            let src_elm = DbRef {
                store_nr: items.store_nr,
                rec: input_inner_rec,
                pos: elem_offset,
            };
            children.push(dbref_to_parsed(stores, &src_elm));
        }
        materialise_primitive_into(stores, &result, &crate::json::Parsed::Array(children));
        stores.last_json_errors.clear();
    }
    stores.put(stack, result);
}

/// Q4 container constructor — `json_object(fields: vector<JsonField>)`
/// mirrors `json_array`: deep-copies each (name, value) pair from
/// the input arena into the new arena via `dbref_to_parsed` →
/// `materialise_primitive_into`.  Empty input still produces an
/// empty JObject.
fn n_json_object(stores: &mut Stores, stack: &mut DbRef) {
    let fields = *stores.get::<DbRef>(stack);
    let length = crate::vector::length_vector(&fields, &stores.allocations);
    let result = jv_alloc(stores);
    if length == 0 {
        stores
            .store_mut(&result)
            .set_byte(result.rec, result.pos, 0, JV_DISCR_OBJECT);
        stores.last_json_errors.clear();
    } else {
        let input_inner_rec = stores.store(&fields).get_u32_raw(fields.rec, fields.pos);
        let jf_tp = stores.name("JsonField");
        let jf_size = u32::from(stores.size(jf_tp));
        let name_field_pos = u32::from(stores.position(jf_tp, "name"));
        let value_field_pos = u32::from(stores.position(jf_tp, "value"));
        let mut entries: Vec<(String, usize, crate::json::Parsed)> =
            Vec::with_capacity(length as usize);
        for i in 0..length {
            let elem_offset = 8u32 + i * jf_size;
            let name_rec = stores
                .store(&fields)
                .get_u32_raw(input_inner_rec, elem_offset + name_field_pos);
            let name = stores.store(&fields).get_str(name_rec).to_owned();
            let value_slot = DbRef {
                store_nr: fields.store_nr,
                rec: input_inner_rec,
                pos: elem_offset + value_field_pos,
            };
            entries.push((name, 0usize, dbref_to_parsed(stores, &value_slot)));
        }
        materialise_primitive_into(stores, &result, &crate::json::Parsed::Object(entries));
        stores.last_json_errors.clear();
    }
    stores.put(stack, result);
}

/// Q2 — `has_field(self: JsonValue, name: text) -> boolean` checks
/// whether a JObject contains a key.  Primitive variants always
/// return false — they have no notion of fields — so users can
/// safely call `v.has_field("name")` on any JsonValue without
/// first pattern-matching the variant.
///
/// For a real JObject, walks the arena `fields` vector and
/// returns `true` iff the name matches an entry.  Distinguishes
/// "absent" from "present-but-null" — a field whose value is
/// `JNull` still returns `true`.
fn n_has_field(stores: &mut Stores, stack: &mut DbRef) {
    let name = *stores.get::<Str>(stack);
    let v = *stores.get::<DbRef>(stack);
    let discr = stores.store(&v).get_byte(v.rec, v.pos, 0);
    if discr != JV_DISCR_OBJECT {
        stores.put(stack, false);
        return;
    }
    let obj_tp = stores.name("JObject");
    let fields_pos = u32::from(stores.position(obj_tp, "fields")) + v.pos;
    let fields_rec = stores.store(&v).get_i32_raw(v.rec, fields_pos);
    if fields_rec <= 0 {
        stores.put(stack, false);
        return;
    }
    let length = i64::from(stores.store(&v).get_u32_raw(fields_rec as u32, 4));
    let jf_tp = stores.name("JsonField");
    let jf_size = u32::from(stores.size(jf_tp));
    let name_field_pos = u32::from(stores.position(jf_tp, "name"));
    let lookup = name.str().to_owned();
    for i in 0..length {
        let elm_offset = 8u32 + u32::try_from(i).expect("non-negative length") * jf_size;
        let name_rec = stores
            .store(&v)
            .get_u32_raw(fields_rec as u32, elm_offset + name_field_pos);
        let stored_name = stores.store(&v).get_str(name_rec).to_owned();
        if stored_name == lookup {
            stores.put(stack, true);
            return;
        }
    }
    stores.put(stack, false);
}

/// Q2 — `keys(self: JsonValue) -> vector<text>` returns the list
/// of declared field names of a `JObject` in insertion order.
/// Any other variant returns an empty vector — same forward-
/// compatible shape as `has_field` so callers can write
/// `for k in v.keys() { ... }` on any JsonValue without first
/// destructuring.
fn n_keys(stores: &mut Stores, stack: &mut DbRef) {
    let v = *stores.get::<DbRef>(stack);
    let discr = stores.store(&v).get_byte(v.rec, v.pos, 0);
    let text_tp = stores.name("text");
    let text_size = u32::from(stores.size(text_tp));
    // Allocate the vector handle in a fresh store; element size
    // matches `stores.size("text")` (4 bytes for the record-nr
    // pointing into the same store's string area).
    let vec = stores.database(text_size.max(1));
    stores.store_mut(&vec).set_u32_raw(vec.rec, vec.pos, 0);
    if discr != JV_DISCR_OBJECT {
        stores.put(stack, vec);
        return;
    }
    let obj_tp = stores.name("JObject");
    let fields_pos = u32::from(stores.position(obj_tp, "fields")) + v.pos;
    let fields_rec = stores.store(&v).get_i32_raw(v.rec, fields_pos);
    if fields_rec <= 0 {
        stores.put(stack, vec);
        return;
    }
    let length = i64::from(stores.store(&v).get_u32_raw(fields_rec as u32, 4));
    let jf_tp = stores.name("JsonField");
    let jf_size = u32::from(stores.size(jf_tp));
    let name_field_pos = u32::from(stores.position(jf_tp, "name"));
    for i in 0..length {
        let elm_offset = 8u32 + u32::try_from(i).expect("non-negative length") * jf_size;
        let name_rec_in_jobject = stores
            .store(&v)
            .get_u32_raw(fields_rec as u32, elm_offset + name_field_pos);
        let name_str = stores.store(&v).get_str(name_rec_in_jobject).to_owned();
        let elm = crate::vector::vector_append(&vec, text_size, &mut stores.allocations);
        let new_name_rec = stores.store_mut(&elm).set_str(&name_str);
        stores
            .store_mut(&elm)
            .set_u32_raw(elm.rec, elm.pos, new_name_rec);
        crate::vector::vector_finish(&vec, &mut stores.allocations);
    }
    stores.put(stack, vec);
}

/// Q2 — `fields(self: JsonValue) -> vector<JsonField>` returns
/// the (name, value) entries of a `JObject` in insertion order
/// so callers can `for entry in fields(v) { … entry.name …
/// entry.value … }`.  Any other variant returns an empty vector,
/// matching the `keys` / `has_field` forward-compat shape.
///
/// **JObject walk (2026-04-14):** for each JsonField, copies the
/// name into the result store and uses
/// `dbref_to_parsed` + `materialise_primitive_into` to fully
/// deep-copy the value (including nested containers) into the
/// result arena.  Each entry's value lives entirely in the
/// result store — caller's input arena can be freed
/// independently.
fn n_fields(stores: &mut Stores, stack: &mut DbRef) {
    let v = *stores.get::<DbRef>(stack);
    let discr = stores.store(&v).get_byte(v.rec, v.pos, 0);
    let jf_tp = stores.name("JsonField");
    let jf_size = u32::from(stores.size(jf_tp));
    let vec = stores.database(jf_size.max(1));
    stores.store_mut(&vec).set_u32_raw(vec.rec, vec.pos, 0);
    if discr != JV_DISCR_OBJECT {
        stores.put(stack, vec);
        return;
    }
    let obj_tp = stores.name("JObject");
    let fields_pos = u32::from(stores.position(obj_tp, "fields")) + v.pos;
    let fields_rec = stores.store(&v).get_i32_raw(v.rec, fields_pos);
    if fields_rec <= 0 {
        stores.put(stack, vec);
        return;
    }
    let length = i64::from(stores.store(&v).get_u32_raw(fields_rec as u32, 4));
    let name_field_pos = u32::from(stores.position(jf_tp, "name"));
    let value_field_pos = u32::from(stores.position(jf_tp, "value"));
    // Read each input field's name + value (recursive Parsed
    // snapshot) before writing — keeps the borrow checker happy
    // and lets `materialise_primitive_into` reuse its existing
    // recursion shape.
    let mut entries: Vec<(String, crate::json::Parsed)> = Vec::with_capacity(length as usize);
    for i in 0..length {
        let elm_offset = 8u32 + u32::try_from(i).expect("non-negative length") * jf_size;
        let name_rec = stores
            .store(&v)
            .get_u32_raw(fields_rec as u32, elm_offset + name_field_pos);
        let name = stores.store(&v).get_str(name_rec).to_owned();
        let value_slot = DbRef {
            store_nr: v.store_nr,
            rec: fields_rec as u32,
            pos: elm_offset + value_field_pos,
        };
        entries.push((name, dbref_to_parsed(stores, &value_slot)));
    }
    for (name, value) in entries {
        let elm = crate::vector::vector_append(&vec, jf_size, &mut stores.allocations);
        let new_name_rec = stores.store_mut(&elm).set_str(&name);
        stores
            .store_mut(&elm)
            .set_u32_raw(elm.rec, elm.pos + name_field_pos, new_name_rec);
        let value_slot = DbRef {
            store_nr: elm.store_nr,
            rec: elm.rec,
            pos: elm.pos + value_field_pos,
        };
        materialise_primitive_into(stores, &value_slot, &value);
        crate::vector::vector_finish(&vec, &mut stores.allocations);
    }
    stores.put(stack, vec);
}

// @PLN10 — destination-passing variant of `n_kind`.
fn n_kind_dest(stores: &mut Stores, stack: &mut DbRef) {
    let dest = *stores.get::<DbRef>(stack);
    let v = *stores.get::<DbRef>(stack);
    let discr = stores.store(&v).get_byte(v.rec, v.pos, 0);
    let name = match discr {
        JV_DISCR_NULL => "JNull",
        JV_DISCR_BOOL => "JBool",
        JV_DISCR_NUMBER => "JNumber",
        JV_DISCR_STRING => "JString",
        JV_DISCR_ARRAY => "JArray",
        JV_DISCR_OBJECT => "JObject",
        JV_DISCR_INT => "JInteger",
        _ => "JUnknown",
    };
    stores
        .store_mut(&dest)
        .addr_mut::<String>(dest.rec, dest.pos)
        .push_str(name);
}

/// Render a JsonValue to RFC 8259 JSON text.  The `pretty` flag
/// controls indent emission in container arms: when `true`,
/// non-empty `JArray` / `JObject` emit `\n` + 2-space indent per
/// element/field and dedent the closing bracket to the parent's
/// depth.  Empty containers stay `[]` / `{}` regardless.
/// Primitives are byte-identical in both modes.
pub(crate) fn json_to_text(stores: &Stores, v: &DbRef, pretty: bool) -> String {
    json_to_text_at(stores, v, pretty, 0)
}

fn write_indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn write_json_string(out: &mut String, raw: &str) {
    out.push('"');
    for ch in raw.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                use std::fmt::Write as _;
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn json_to_text_at(stores: &Stores, v: &DbRef, pretty: bool, depth: usize) -> String {
    let discr = stores.store(v).get_byte(v.rec, v.pos, 0);
    match discr {
        JV_DISCR_NULL => "null".to_string(),
        JV_DISCR_BOOL => {
            let bool_tp = stores.name("JBool");
            let value_pos = u32::from(stores.position(bool_tp, "value")) + v.pos;
            let b = stores.store(v).get_byte(v.rec, value_pos, 0);
            if b != 0 {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
        JV_DISCR_NUMBER => {
            let num_tp = stores.name("JNumber");
            let value_pos = u32::from(stores.position(num_tp, "value")) + v.pos;
            let n = stores.store(v).get_float(v.rec, value_pos);
            if n.is_finite() {
                format!("{n}")
            } else {
                "null".to_string()
            }
        }
        // @PLN109 — a JInteger serialises as its exact integer (no `.0`).
        JV_DISCR_INT => {
            let int_tp = stores.name("JInteger");
            let value_pos = u32::from(stores.position(int_tp, "value")) + v.pos;
            format!("{}", stores.store(v).get_int(v.rec, value_pos))
        }
        JV_DISCR_STRING => {
            let str_tp = stores.name("JString");
            let value_pos = u32::from(stores.position(str_tp, "value")) + v.pos;
            let s_rec = stores.store(v).get_u32_raw(v.rec, value_pos);
            let raw = stores.store(v).get_str(s_rec).to_string();
            let mut out = String::with_capacity(raw.len() + 2);
            write_json_string(&mut out, &raw);
            out
        }
        JV_DISCR_ARRAY => {
            let array_tp = stores.name("JArray");
            let items_pos = u32::from(stores.position(array_tp, "items")) + v.pos;
            let items_rec = stores.store(v).get_i32_raw(v.rec, items_pos);
            if items_rec <= 0 {
                return "[]".to_string();
            }
            let length = i64::from(stores.store(v).get_u32_raw(items_rec as u32, 4));
            if length <= 0 {
                return "[]".to_string();
            }
            let jv_tp = stores.name("JsonValue");
            let jv_size = u32::from(stores.size(jv_tp));
            let mut out = String::with_capacity(length as usize * 4 + 2);
            out.push('[');
            for i in 0..length {
                if i > 0 {
                    out.push(',');
                }
                if pretty {
                    out.push('\n');
                    write_indent(&mut out, depth + 1);
                }
                let elm_offset = 8u32 + u32::try_from(i).expect("non-negative length") * jv_size;
                let elm_ref = DbRef {
                    store_nr: v.store_nr,
                    rec: items_rec as u32,
                    pos: elm_offset,
                };
                out.push_str(&json_to_text_at(stores, &elm_ref, pretty, depth + 1));
            }
            if pretty {
                out.push('\n');
                write_indent(&mut out, depth);
            }
            out.push(']');
            out
        }
        JV_DISCR_OBJECT => {
            let obj_tp = stores.name("JObject");
            let fields_pos = u32::from(stores.position(obj_tp, "fields")) + v.pos;
            let fields_rec = stores.store(v).get_i32_raw(v.rec, fields_pos);
            if fields_rec <= 0 {
                return "{}".to_string();
            }
            let length = i64::from(stores.store(v).get_u32_raw(fields_rec as u32, 4));
            if length <= 0 {
                return "{}".to_string();
            }
            let jf_tp = stores.name("JsonField");
            let jf_size = u32::from(stores.size(jf_tp));
            let name_field_pos = u32::from(stores.position(jf_tp, "name"));
            let value_field_pos = u32::from(stores.position(jf_tp, "value"));
            let mut out = String::with_capacity(length as usize * 8 + 2);
            out.push('{');
            for i in 0..length {
                if i > 0 {
                    out.push(',');
                }
                if pretty {
                    out.push('\n');
                    write_indent(&mut out, depth + 1);
                }
                let elm_offset = 8u32 + u32::try_from(i).expect("non-negative length") * jf_size;
                let name_rec = stores
                    .store(v)
                    .get_u32_raw(fields_rec as u32, elm_offset + name_field_pos);
                let raw = stores.store(v).get_str(name_rec).to_string();
                write_json_string(&mut out, &raw);
                out.push(':');
                if pretty {
                    out.push(' ');
                }
                let value_ref = DbRef {
                    store_nr: v.store_nr,
                    rec: fields_rec as u32,
                    pos: elm_offset + value_field_pos,
                };
                out.push_str(&json_to_text_at(stores, &value_ref, pretty, depth + 1));
            }
            if pretty {
                out.push('\n');
                write_indent(&mut out, depth);
            }
            out.push('}');
            out
        }
        _ => "null".to_string(),
    }
}

// @PLN10 — destination-passing variant of `n_to_json`.
fn n_to_json_dest(stores: &mut Stores, stack: &mut DbRef) {
    let dest = *stores.get::<DbRef>(stack);
    let v = *stores.get::<DbRef>(stack);
    let out = json_to_text(stores, &v, false);
    stores
        .store_mut(&dest)
        .addr_mut::<String>(dest.rec, dest.pos)
        .push_str(&out);
}

// @PLN10 — destination-passing variant of `n_to_json_pretty`.
fn n_to_json_pretty_dest(stores: &mut Stores, stack: &mut DbRef) {
    let dest = *stores.get::<DbRef>(stack);
    let v = *stores.get::<DbRef>(stack);
    let out = json_to_text(stores, &v, true);
    stores
        .store_mut(&dest)
        .addr_mut::<String>(dest.rec, dest.pos)
        .push_str(&out);
}
