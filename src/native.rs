//! Native function registry: Rust implementations of loft built-ins.
//! Naming: `n_<name>` for globals, `t_<LEN><Type>_<method>` for methods.
#![allow(non_snake_case)]
use crate::database::Stores;
use crate::keys::{DbRef, Str};
use crate::logger::Severity;
#[cfg(feature = "threading")]
use crate::parallel::{
    WorkerProgram, run_parallel_direct, run_parallel_int, run_parallel_raw, run_parallel_ref,
    run_parallel_text,
};
use crate::platform::sep;
use crate::state::{Call, State};
#[cfg(feature = "threading")]
use crate::vector;
#[cfg(feature = "threading")]
use std::sync::Arc;
#[cfg(not(feature = "wasm"))]
use std::time::SystemTime;

pub const FUNCTIONS: &[(&str, Call)] = &[
    ("n_assert", n_assert),
    ("n_panic", n_panic),
    ("n_log_info", n_log_info),
    ("n_log_warn", n_log_warn),
    ("n_log_error", n_log_error),
    ("n_log_fatal", n_log_fatal),
    ("t_4File_write", t_4File_write),
    ("n_env_variables", n_env_variables),
    ("n_env_variable", n_env_variable),
    ("t_4text_starts_with", t_4text_starts_with),
    ("t_4text_ends_with", t_4text_ends_with),
    ("t_4text_trim", t_4text_trim),
    ("t_4text_trim_start", t_4text_trim_start),
    ("t_4text_trim_end", t_4text_trim_end),
    ("t_4text_find", t_4text_find),
    ("t_4text_rfind", t_4text_rfind),
    ("t_4text_contains", t_4text_contains),
    ("t_4text_replace", t_4text_replace),
    ("t_4text_replace_dest", t_4text_replace_dest),
    ("t_4text_to_lowercase", t_4text_to_lowercase),
    ("t_4text_to_lowercase_dest", t_4text_to_lowercase_dest),
    ("t_4text_to_uppercase", t_4text_to_uppercase),
    ("t_4text_to_uppercase_dest", t_4text_to_uppercase_dest),
    ("t_9character_is_lowercase", t_9character_is_lowercase),
    ("t_9character_is_uppercase", t_9character_is_uppercase),
    ("t_9character_is_numeric", t_9character_is_numeric),
    ("t_9character_is_alphanumeric", t_9character_is_alphanumeric),
    ("t_9character_is_alphabetic", t_9character_is_alphabetic),
    ("t_9character_is_whitespace", t_9character_is_whitespace),
    ("t_9character_is_control", t_9character_is_control),
    ("n_arguments", n_arguments),
    ("n_directory", n_directory),
    ("n_user_directory", n_user_directory),
    ("n_program_directory", n_program_directory),
    ("n_source_dir", n_source_dir),
    ("n_get_store_lock", n_get_store_lock),
    ("n_set_store_lock", n_set_store_lock),
    #[cfg(feature = "threading")]
    ("n_parallel_for_int", n_parallel_for_int),
    #[cfg(feature = "threading")]
    ("n_parallel_for", n_parallel_for),
    #[cfg(feature = "threading")]
    ("n_parallel_for_light", n_parallel_for_light),
    #[cfg(feature = "threading")]
    ("n_parallel_get_int", n_parallel_get_int),
    #[cfg(feature = "threading")]
    ("n_parallel_get_long", n_parallel_get_long),
    #[cfg(feature = "threading")]
    ("n_parallel_get_float", n_parallel_get_float),
    #[cfg(feature = "threading")]
    ("n_parallel_get_bool", n_parallel_get_bool),
    ("n_now", n_now),
    ("n_ticks", n_ticks),
    ("n_stack_trace", n_stack_trace),
    ("n_path_sep", n_path_sep),
    ("i_parse_error_push", i_parse_error_push),
    ("i_parse_errors", i_parse_errors),
    ("n_hash_sorted", n_hash_sorted),
    ("n_sha256", n_sha256),
    ("n_hmac_sha256", n_hmac_sha256),
    ("n_base64_encode", n_base64_encode),
    ("n_base64_decode", n_base64_decode),
    ("n_base64url_encode", n_base64url_encode),
    ("n_hmac_sha256_raw", n_hmac_sha256_raw),
    ("n_json_parse", n_json_parse),
    ("n_json_errors", n_json_errors),
    ("n_json_null", n_json_null),
    ("n_json_bool", n_json_bool),
    ("n_json_number", n_json_number),
    ("n_json_string", n_json_string),
    ("n_json_array", n_json_array),
    ("n_json_object", n_json_object),
    ("n_kind", n_kind),
    ("n_keys", n_keys),
    ("n_fields", n_fields),
    ("n_has_field", n_has_field),
    ("n_to_json", n_to_json),
    ("n_to_json_pretty", n_to_json_pretty),
    ("n_as_text", n_as_text),
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
    ("t_9JsonValue_as_text", n_as_text),
    ("t_9JsonValue_as_number", n_as_number),
    ("t_9JsonValue_as_long", n_as_long),
    ("t_9JsonValue_as_bool", n_as_bool),
    ("t_9JsonValue_field", n_field),
    ("t_9JsonValue_item", n_item),
    ("t_9JsonValue_len", n_len),
    ("t_9JsonValue_kind", n_kind),
    ("t_9JsonValue_keys", n_keys),
    ("t_9JsonValue_fields", n_fields),
    ("t_9JsonValue_has_field", n_has_field),
    ("t_9JsonValue_to_json", n_to_json),
    ("t_9JsonValue_to_json_pretty", n_to_json_pretty),
];

pub fn init(state: &mut State) {
    for (name, implement) in FUNCTIONS {
        state.static_fn(name, *implement);
    }
}

fn n_assert(stores: &mut Stores, stack: &mut DbRef) {
    let v_line = *stores.get::<i32>(stack);
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
        let production = logger.lock().map(|l| l.config.production).unwrap_or(false);
        if production {
            if let Ok(mut lg) = logger.lock() {
                lg.log(
                    Severity::Error,
                    v_file.str(),
                    v_line as u32,
                    v_message.str(),
                );
            }
            stores.had_fatal = true;
            return;
        }
    }
    let msg = v_message.str();
    let file = v_file.str();
    panic!("{msg} ({file}:{v_line})");
}

fn n_panic(stores: &mut Stores, stack: &mut DbRef) {
    let v_line = *stores.get::<i32>(stack);
    let v_file = *stores.get::<Str>(stack);
    let v_message = *stores.get::<Str>(stack);
    if let Some(ref logger) = stores.logger {
        let production = logger.lock().map(|l| l.config.production).unwrap_or(false);
        if production {
            if let Ok(mut lg) = logger.lock() {
                lg.log(
                    Severity::Fatal,
                    v_file.str(),
                    v_line as u32,
                    v_message.str(),
                );
            }
            stores.had_fatal = true;
            return;
        }
    }
    let msg = v_message.str();
    let file = v_file.str();
    panic!("{msg} ({file}:{v_line})");
}

fn n_log_info(stores: &mut Stores, stack: &mut DbRef) {
    let v_line = *stores.get::<i32>(stack);
    let v_file = *stores.get::<Str>(stack);
    let v_message = *stores.get::<Str>(stack);
    if let Some(ref logger) = stores.logger
        && let Ok(mut lg) = logger.lock()
    {
        lg.log(Severity::Info, v_file.str(), v_line as u32, v_message.str());
    }
}

fn n_log_warn(stores: &mut Stores, stack: &mut DbRef) {
    let v_line = *stores.get::<i32>(stack);
    let v_file = *stores.get::<Str>(stack);
    let v_message = *stores.get::<Str>(stack);
    if let Some(ref logger) = stores.logger
        && let Ok(mut lg) = logger.lock()
    {
        lg.log(Severity::Warn, v_file.str(), v_line as u32, v_message.str());
    }
}

fn n_log_error(stores: &mut Stores, stack: &mut DbRef) {
    let v_line = *stores.get::<i32>(stack);
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
    let v_line = *stores.get::<i32>(stack);
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

fn t_4File_write(stores: &mut Stores, stack: &mut DbRef) {
    let v_v = *stores.get::<Str>(stack);
    let v_self = *stores.get::<DbRef>(stack);
    stores.write_file(&v_self, v_v.str());
}

fn n_env_variables(stores: &mut Stores, stack: &mut DbRef) {
    let new_value = { stores.os_variables() };
    stores.put(stack, new_value);
}

fn n_env_variable(stores: &mut Stores, stack: &mut DbRef) {
    let v_name = *stores.get::<Str>(stack);
    let new_value = { Stores::os_variable(v_name.str()) };
    stores.put(stack, new_value);
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
    let new_value = {
        if let Some(v) = v_self.str().find(v_value.str()) {
            v as i32
        } else {
            i32::MIN
        }
    };
    stores.put(stack, new_value);
}

fn t_4text_rfind(stores: &mut Stores, stack: &mut DbRef) {
    let v_value = *stores.get::<Str>(stack);
    let v_self = *stores.get::<Str>(stack);
    let new_value = {
        if let Some(v) = v_self.str().rfind(v_value.str()) {
            v as i32
        } else {
            i32::MIN
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

fn t_4text_replace(stores: &mut Stores, stack: &mut DbRef) {
    let v_with = *stores.get::<Str>(stack);
    let v_value = *stores.get::<Str>(stack);
    let v_self = *stores.get::<Str>(stack);
    let new_value = v_self.str().replace(v_value.str(), v_with.str());
    stores.scratch.push(new_value);
    let s = Str::new(stores.scratch.last().unwrap());
    stores.put(stack, s);
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

fn t_4text_to_lowercase(stores: &mut Stores, stack: &mut DbRef) {
    let v_self = *stores.get::<Str>(stack);
    let new_value = v_self.str().to_lowercase();
    stores.scratch.push(new_value);
    let s = Str::new(stores.scratch.last().unwrap());
    stores.put(stack, s);
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

fn t_4text_to_uppercase(stores: &mut Stores, stack: &mut DbRef) {
    let v_self = *stores.get::<Str>(stack);
    let new_value = v_self.str().to_uppercase();
    stores.scratch.push(new_value);
    let s = Str::new(stores.scratch.last().unwrap());
    stores.put(stack, s);
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

/// Return the directory of the main source file being executed.
fn n_source_dir(stores: &mut Stores, stack: &mut DbRef) {
    stores.scratch.clear();
    stores.scratch.push(stores.source_dir.clone());
    stores.put(stack, Str::new(&stores.scratch[0]));
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

// ── Parallel threading functions (feature = "threading") ──────────────

/// Dispatch a compiled loft function over every row of an input vector,
/// running `threads` OS threads in parallel.
/// The worker function must have the signature `fn f(row: &T) -> integer`.
/// Returns a `reference` pointing to a freshly allocated vector of integers,
/// one per input row in the original order.
#[cfg(feature = "threading")]
fn n_parallel_for_int(stores: &mut Stores, stack: &mut DbRef) {
    // Pop arguments (last-pushed first).
    let v_threads = *stores.get::<i32>(stack);
    let v_element_size = *stores.get::<i32>(stack);
    let v_input = *stores.get::<DbRef>(stack);
    let v_func = *stores.get::<Str>(stack);

    // Resolve function name → bytecode position via ParallelCtx.
    let (fn_pos, program) = {
        let ctx = stores
            .parallel_ctx
            .as_ref()
            .expect("parallel_for_int called outside State::execute()");
        // SAFETY: pointers are valid for the duration of State::execute().
        let data = unsafe { &*ctx.data };
        let func_name = v_func.str();
        let d_nr = data.def_nr(&format!("n_{func_name}"));
        assert_ne!(
            d_nr,
            u32::MAX,
            "parallel_for_int: unknown function '{func_name}'"
        );
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
                // n_parallel_for path does not propagate line_numbers; workers
                // get function name + file but report line 0.  Fixing this
                // requires threading line_numbers through ParallelCtx.
                line_numbers: Arc::new(std::collections::BTreeMap::new()),
            },
        )
    };

    let element_size = v_element_size as u32;
    let n_threads = (v_threads as usize).max(1);

    let results = run_parallel_int(stores, program, fn_pos, &v_input, element_size, n_threads);
    let n = results.len();

    // Build an integer-vector result in a fresh store.
    let result_db = stores.null(); // allocates an empty store
    // Vector data record: fld=4 count, fld=8+ elements (4 bytes each).
    // Record size in 8-byte units: ceil((8 + n*4) / 8).
    let vec_words = ((n as u32) * 4 + 15) / 8;
    let vec_words = vec_words.max(1);
    let vec_cr = stores.claim(&result_db, vec_words);
    let vec_rec = vec_cr.rec;
    // Header record: fld=4 holds the pointer to vec_rec.
    let header_cr = stores.claim(&result_db, 1);
    let header_rec = header_cr.rec;

    {
        let store = stores.store_mut(&result_db);
        store.set_int(vec_rec, 4, n as i32);
        for (i, &val) in results.iter().enumerate() {
            store.set_int(vec_rec, 8 + i as u32 * 4, val);
        }
        store.set_int(header_rec, 4, vec_rec as i32);
    }

    let result_ref = DbRef {
        store_nr: result_db.store_nr,
        rec: header_rec,
        pos: 4,
    };
    stores.put(stack, result_ref);
}

#[cfg(feature = "threading")]
/// Internal `parallel_for` dispatch: pop args from stack, spawn workers, collect results.
/// `return_size`: 0=text, 1=bool, 4=int, 8=long/float.
fn n_parallel_for(stores: &mut Stores, stack: &mut DbRef) {
    // Stack layout (push order from codegen):
    //   vec(12B), elem_size(4B), return_size(4B), threads(4B), func(4B),
    //   extra1(4B), ..., extraN(4B), n_extra(4B)
    // Pop order (LIFO): n_extra, extraN, ..., extra1, func, threads, return_size, elem_size, vec

    let n_extra = *stores.get::<i32>(stack) as usize;
    let mut extra_args: Vec<u64> = Vec::with_capacity(n_extra);
    for _ in 0..n_extra {
        extra_args.push(*stores.get::<i32>(stack) as u64);
    }
    extra_args.reverse(); // restore push order (first extra = first worker param)

    let v_func = *stores.get::<i32>(stack);
    let v_threads = *stores.get::<i32>(stack);
    let v_return_size = *stores.get::<i32>(stack);
    let v_element_size = *stores.get::<i32>(stack);
    let v_input = *stores.get::<DbRef>(stack);

    let (fn_pos, program, n_hidden_text) = {
        let ctx = stores
            .parallel_ctx
            .as_ref()
            .expect("parallel_for called outside State::execute()");
        let data = unsafe { &*ctx.data };
        assert!(
            v_func >= 0,
            "parallel_for: invalid function reference {v_func}"
        );
        let d_nr = v_func as u32;
        let fn_pos = data.def(d_nr).code_position;
        // Count hidden __work_N text params for text-returning workers.
        let n_hidden = data
            .def(d_nr)
            .attributes
            .iter()
            .filter(|a| a.name.starts_with("__"))
            .count();
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
                // n_parallel_for path does not propagate line_numbers; workers
                // get function name + file but report line 0.  Fixing this
                // requires threading line_numbers through ParallelCtx.
                line_numbers: Arc::new(std::collections::BTreeMap::new()),
            },
            n_hidden,
        )
    };

    let element_size = v_element_size as u32;
    let n_threads = (v_threads as usize).max(1);
    let n = vector::length_vector(&v_input, &stores.allocations) as usize;
    // return_size == 0 signals text mode; -1 signals reference (struct) mode.
    let is_text = v_return_size == 0;
    let is_ref = v_return_size == -1;
    // For ref mode, compute the actual inline struct size from known_type.
    // This is used for result vector allocation.
    let return_size = if is_text {
        4u32
    } else if is_ref {
        // Will be set below after known_type is resolved.
        0u32 // placeholder
    } else {
        v_return_size.clamp(1, 8) as u32
    };

    // For reference returns, look up the struct's known_type for deep-copy
    // and compute the inline struct size for the result vector.
    let (known_type, return_size) = if is_ref {
        let ctx = stores
            .parallel_ctx
            .as_ref()
            .expect("parallel_for: missing context");
        let data = unsafe { &*ctx.data };
        let def = data.def(v_func as u32);
        let kt = match &def.returned {
            crate::data::Type::Reference(d_nr, _) => data.def(*d_nr).known_type,
            _ => u16::MAX,
        };
        let sz = u32::from(stores.size(kt));
        (kt, sz)
    } else {
        (u16::MAX, return_size)
    };

    let result_ref = parallel_execute_and_collect(
        stores,
        program,
        fn_pos,
        &v_input,
        element_size,
        return_size,
        is_text,
        is_ref,
        known_type,
        n_threads,
        &extra_args,
        n,
        n_hidden_text,
    );
    stores.put(stack, result_ref);
}

#[cfg(feature = "threading")]
/// Lightweight variant — borrows stores read-only instead of deep-copying.
/// Same stack layout as `n_parallel_for` plus an extra `pool_m` argument.
fn n_parallel_for_light(stores: &mut Stores, stack: &mut DbRef) {
    // Same stack layout as n_parallel_for: n_extra on top, then declared params.
    // Pop order (LIFO): n_extra, extras..., func, threads, return_size, elem_size, input.
    let n_extra = *stores.get::<i32>(stack) as usize;
    let mut extra_args: Vec<u64> = Vec::with_capacity(n_extra);
    for _ in 0..n_extra {
        extra_args.push(*stores.get::<i32>(stack) as u64);
    }
    extra_args.reverse();

    let v_func = *stores.get::<i32>(stack);
    let v_threads = *stores.get::<i32>(stack);
    let v_return_size = *stores.get::<i32>(stack);
    let v_element_size = *stores.get::<i32>(stack);
    let v_input = *stores.get::<DbRef>(stack);

    let (fn_pos, program) = {
        let ctx = stores
            .parallel_ctx
            .as_ref()
            .expect("parallel_for_light called outside State::execute()");
        let data = unsafe { &*ctx.data };
        assert!(
            v_func >= 0,
            "parallel_for_light: invalid function reference {v_func}"
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
                // n_parallel_for path does not propagate line_numbers; workers
                // get function name + file but report line 0.  Fixing this
                // requires threading line_numbers through ParallelCtx.
                line_numbers: Arc::new(std::collections::BTreeMap::new()),
            },
        )
    };

    let element_size = v_element_size as u32;
    let return_size = v_return_size.clamp(1, 8) as u32;
    let n_threads = (v_threads as usize).max(1);
    let n = crate::vector::length_vector(&v_input, &stores.allocations) as usize;
    let pool_m: usize = 2;

    // Allocate result vector using the same helper as n_parallel_for.
    let result_ref = parallel_light_execute_and_collect(
        stores,
        program,
        fn_pos,
        &v_input,
        element_size,
        return_size,
        n_threads,
        &extra_args,
        n,
        pool_m,
    );
    stores.put(stack, result_ref);
}

#[cfg(feature = "threading")]
/// Allocate result vector, create pool, dispatch light workers, collect.
#[allow(clippy::too_many_arguments)]
fn parallel_light_execute_and_collect(
    stores: &mut Stores,
    program: WorkerProgram,
    fn_pos: u32,
    input: &DbRef,
    element_size: u32,
    return_size: u32,
    n_threads: usize,
    extra_args: &[u64],
    n: usize,
    pool_m: usize,
) -> DbRef {
    let result_db = stores.null();
    let vec_words = ((n as u32) * return_size + 15) / 8;
    let vec_cr = stores.claim(&result_db, vec_words.max(1));
    let vec_rec = vec_cr.rec;
    let header_cr = stores.claim(&result_db, 1);
    let header_rec = header_cr.rec;
    stores.store_mut(&result_db).set_int(vec_rec, 4, n as i32);
    stores
        .store_mut(&result_db)
        .set_int(header_rec, 4, vec_rec as i32);
    let out_ptr = stores.store_mut(&result_db).buffer(vec_rec).as_mut_ptr();

    let mut pool = crate::parallel::WorkerPool::new(n_threads, pool_m, 256);
    crate::parallel::run_parallel_light(
        stores,
        program,
        fn_pos,
        input,
        element_size,
        return_size,
        n_threads,
        extra_args,
        out_ptr,
        n,
        &mut pool,
    );
    // Return with pos=4 (not pos=8 from claim) — parallel_get_int reads at v_ref.pos.
    DbRef {
        store_nr: result_db.store_nr,
        rec: header_rec,
        pos: 4,
    }
}

#[cfg(feature = "threading")]
/// Allocate a result vector, dispatch workers, and collect results.
#[allow(clippy::too_many_arguments)]
fn parallel_execute_and_collect(
    stores: &mut Stores,
    program: WorkerProgram,
    fn_pos: u32,
    input: &DbRef,
    element_size: u32,
    return_size: u32,
    is_text: bool,
    is_ref: bool,
    known_type: u16,
    n_threads: usize,
    extra_args: &[u64],
    n: usize,
    n_hidden_text: usize,
) -> DbRef {
    let result_db = stores.null();
    let vec_words = ((n as u32) * return_size + 15) / 8;
    let vec_cr = stores.claim(&result_db, vec_words.max(1));
    let vec_rec = vec_cr.rec;
    let header_cr = stores.claim(&result_db, 1);
    let header_rec = header_cr.rec;
    stores.store_mut(&result_db).set_int(vec_rec, 4, n as i32);
    stores
        .store_mut(&result_db)
        .set_int(header_rec, 4, vec_rec as i32);

    if is_ref {
        let batches = run_parallel_ref(
            stores,
            program,
            fn_pos,
            input,
            element_size,
            n_threads,
            extra_args,
            n,
        );
        // Deep-copy each worker-created struct directly into the result vector
        // at the inline element position.  The struct bytes live at
        // vec_rec offset `8 + i * struct_size`; OpGetVector will return a DbRef
        // pointing there so field access works without extra indirection.
        let struct_size = u32::from(stores.size(known_type));
        for (batch, mut worker_stores) in batches {
            for (i, src_ref) in batch {
                let dest = DbRef {
                    store_nr: result_db.store_nr,
                    rec: vec_rec,
                    pos: 8 + (i as u32) * struct_size,
                };
                stores.copy_from_worker(&src_ref, &dest, &mut worker_stores, known_type);
            }
        }
    } else if is_text {
        let strings = run_parallel_text(
            stores,
            program,
            fn_pos,
            input,
            element_size,
            n_threads,
            extra_args,
            n,
            n_hidden_text,
        );
        let store = stores.store_mut(&result_db);
        for (i, s) in strings.iter().enumerate() {
            let s_pos = store.set_str(s);
            store.set_int(vec_rec, 8 + i as u32 * 4, s_pos as i32);
        }
    } else if return_size >= 4 {
        let out_ptr = stores.store_mut(&result_db).buffer(vec_rec).as_mut_ptr();
        run_parallel_direct(
            stores,
            program,
            fn_pos,
            input,
            element_size,
            return_size,
            n_threads,
            extra_args,
            out_ptr,
            n,
        );
    } else {
        let results = run_parallel_raw(
            stores,
            program,
            fn_pos,
            input,
            element_size,
            return_size,
            n_threads,
            extra_args,
        );
        let store = stores.store_mut(&result_db);
        let mut fld = 8u32;
        for &raw in &results {
            store.set_byte(vec_rec, fld, 0, raw as i32);
            fld += 1;
        }
    }
    DbRef {
        store_nr: result_db.store_nr,
        rec: header_rec,
        pos: 4,
    }
}

#[cfg(feature = "threading")]
fn n_parallel_get_int(stores: &mut Stores, stack: &mut DbRef) {
    let v_idx = *stores.get::<i32>(stack);
    let v_ref = *stores.get::<DbRef>(stack);
    let store = stores.store(&v_ref);
    let vec_rec = store.get_int(v_ref.rec, v_ref.pos) as u32;
    let val = store.get_int(vec_rec, 8 + v_idx as u32 * 4);
    stores.put(stack, val);
}

#[cfg(feature = "threading")]
fn n_parallel_get_long(stores: &mut Stores, stack: &mut DbRef) {
    let v_idx = *stores.get::<i32>(stack);
    let v_ref = *stores.get::<DbRef>(stack);
    let store = stores.store(&v_ref);
    let vec_rec = store.get_int(v_ref.rec, v_ref.pos) as u32;
    let val = store.get_long(vec_rec, 8 + v_idx as u32 * 8);
    stores.put(stack, val);
}

#[cfg(feature = "threading")]
fn n_parallel_get_float(stores: &mut Stores, stack: &mut DbRef) {
    let v_idx = *stores.get::<i32>(stack);
    let v_ref = *stores.get::<DbRef>(stack);
    let store = stores.store(&v_ref);
    let vec_rec = store.get_int(v_ref.rec, v_ref.pos) as u32;
    let bits = store.get_long(vec_rec, 8 + v_idx as u32 * 8);
    stores.put(stack, f64::from_bits(bits as u64));
}

#[cfg(feature = "threading")]
fn n_parallel_get_bool(stores: &mut Stores, stack: &mut DbRef) {
    let v_idx = *stores.get::<i32>(stack);
    let v_ref = *stores.get::<DbRef>(stack);
    let store = stores.store(&v_ref);
    let vec_rec = store.get_int(v_ref.rec, v_ref.pos) as u32;
    let val = store.get_byte(vec_rec, 8 + v_idx as u32, 0);
    stores.put(stack, val != 0);
}

/// Return milliseconds since the Unix epoch (1970-01-01T00:00:00 UTC).
/// Returns `i64::MIN` (null) if the system clock reports a time before the epoch.
#[cfg(not(feature = "wasm"))]
fn n_now(stores: &mut Stores, stack: &mut DbRef) {
    let millis = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(i64::MIN, |d| d.as_millis() as i64);
    stores.put(stack, millis);
}

#[cfg(feature = "wasm")]
fn n_now(stores: &mut Stores, stack: &mut DbRef) {
    stores.put(stack, crate::wasm::host_time_now());
}

/// Return microseconds elapsed since program start (monotonic clock).
/// Use for frame timing and benchmarks; unaffected by wall-clock adjustments.
#[cfg(not(target_arch = "wasm32"))]
fn n_ticks(stores: &mut Stores, stack: &mut DbRef) {
    let micros = stores.start_time.elapsed().as_micros() as i64;
    stores.put(stack, micros);
}

#[cfg(all(target_arch = "wasm32", feature = "wasm"))]
fn n_ticks(stores: &mut Stores, stack: &mut DbRef) {
    let now_ms = crate::wasm::host_time_ticks();
    let elapsed_micros = (now_ms - stores.start_time_ms) * 1000;
    stores.put(stack, elapsed_micros);
}

#[cfg(all(target_arch = "wasm32", not(feature = "wasm")))]
fn n_ticks(stores: &mut Stores, stack: &mut DbRef) {
    // P137: no host time bridge on the --html build; return 0.
    stores.put(stack, 0i64);
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
    stores.store_mut(&vec).set_int(vec.rec, vec.pos, 0);

    for (frame_idx, (fn_name, file, line)) in snapshot.iter().enumerate() {
        let elm = crate::vector::vector_append(&vec, sf_size, &mut stores.allocations);
        let fn_str = stores.store_mut(&vec).set_str(fn_name.as_str());
        stores
            .store_mut(&vec)
            .set_int(elm.rec, elm.pos + function_pos, fn_str as i32);
        let file_str = stores.store_mut(&vec).set_str(file.as_str());
        stores
            .store_mut(&vec)
            .set_int(elm.rec, elm.pos + file_pos, file_str as i32);
        stores
            .store_mut(&vec)
            .set_int(elm.rec, elm.pos + line_pos, *line as i32);
        // Explicitly zero arguments and variables so that reused (non-zeroed) store
        // blocks don't leave garbage data that looks like a valid first_block_rec.
        stores
            .store_mut(&vec)
            .set_int(elm.rec, elm.pos + arguments_pos, 0);
        stores
            .store_mut(&vec)
            .set_int(elm.rec, elm.pos + vars_field_pos, 0);

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
        .set_int(inner_rec, 4, frame_vars.len() as i32);
    // Link from the StackFrame.variables field to this inner record.
    stores
        .store_mut(sf_vec)
        .set_int(parent_rec, vars_field_abs, inner_rec as i32);

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
        stores.store_mut(sf_vec).set_int(
            inner_rec,
            inline_pos + u32::from(name_pos),
            name_str as i32,
        );
        // Write type_name
        let type_str = stores.store_mut(sf_vec).set_str(&vs.type_name);
        stores.store_mut(sf_vec).set_int(
            inner_rec,
            inline_pos + u32::from(type_pos),
            type_str as i32,
        );
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
                store_mut.set_int(inner_rec, av_abs + u32::from(int_n_pos), *n);
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
                store_mut.set_int(inner_rec, av_abs + u32::from(char_c_pos), *c as i32);
            }
            crate::database::VarValueSnapshot::Text(s) => {
                store_mut.set_byte(inner_rec, av_abs, 0, 8);
                let txt = store_mut.set_str(s);
                store_mut.set_int(inner_rec, av_abs + u32::from(text_t_pos), txt as i32);
            }
            crate::database::VarValueSnapshot::Ref { store, rec, pos } => {
                store_mut.set_byte(inner_rec, av_abs, 0, 9);
                store_mut.set_int(inner_rec, av_abs + u32::from(ref_store_pos), *store);
                store_mut.set_int(inner_rec, av_abs + u32::from(ref_rec_pos), *rec);
                store_mut.set_int(inner_rec, av_abs + u32::from(ref_pos_pos), *pos);
            }
            crate::database::VarValueSnapshot::Other(desc) => {
                store_mut.set_byte(inner_rec, av_abs, 0, 11);
                let txt = store_mut.set_str(desc);
                store_mut.set_int(inner_rec, av_abs + u32::from(other_desc_pos), txt as i32);
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

fn i_parse_errors(stores: &mut Stores, stack: &mut DbRef) {
    let msg = stores.last_parse_errors.join("\n");
    stores.last_parse_errors.clear();
    stores.scratch.clear();
    stores.scratch.push(msg);
    stores.put(stack, Str::new(&stores.scratch[0]));
}

// HTTP client glue removed — n_http_do and n_http_body are now auto-marshalled.
// The cdylib stores the response body in a thread-local, returned via LoftStr.

// ── Crypto built-ins (always available) ─────────────────────────────────

fn hex_encode(data: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(data.len() * 2);
    for b in data {
        let _ = write!(out, "{b:02x}");
    }
    out
}

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
    let v_tp = *stores.get::<i32>(stack);
    let v_h = *stores.get::<DbRef>(stack);
    let result = stores.build_hash_sorted_vec(&v_h, v_tp as u16);
    stores.put(stack, result);
}

fn n_sha256(stores: &mut Stores, stack: &mut DbRef) {
    let v = *stores.get::<Str>(stack);
    let hash = crate::sha256::sha256(v.str().as_bytes());
    stores.scratch.clear();
    stores.scratch.push(hex_encode(&hash));
    stores.put(stack, Str::new(&stores.scratch[0]));
}

fn n_hmac_sha256(stores: &mut Stores, stack: &mut DbRef) {
    let v_data = *stores.get::<Str>(stack);
    let v_key = *stores.get::<Str>(stack);
    let mac = crate::sha256::hmac_sha256(v_key.str().as_bytes(), v_data.str().as_bytes());
    stores.scratch.clear();
    stores.scratch.push(hex_encode(&mac));
    stores.put(stack, Str::new(&stores.scratch[0]));
}

fn n_hmac_sha256_raw(stores: &mut Stores, stack: &mut DbRef) {
    let v_data = *stores.get::<Str>(stack);
    let v_key = *stores.get::<Str>(stack);
    let mac = crate::sha256::hmac_sha256(v_key.str().as_bytes(), v_data.str().as_bytes());
    stores.scratch.clear();
    stores
        .scratch
        .push(std::str::from_utf8(&mac).unwrap_or("").to_string());
    stores.put(stack, Str::new(&stores.scratch[0]));
}

fn n_base64_encode(stores: &mut Stores, stack: &mut DbRef) {
    let v = *stores.get::<Str>(stack);
    stores.scratch.clear();
    stores
        .scratch
        .push(crate::base64::encode(v.str().as_bytes()));
    stores.put(stack, Str::new(&stores.scratch[0]));
}

fn n_base64_decode(stores: &mut Stores, stack: &mut DbRef) {
    let v = *stores.get::<Str>(stack);
    let decoded = crate::base64::decode(v.str());
    stores.scratch.clear();
    stores
        .scratch
        .push(String::from_utf8_lossy(&decoded).to_string());
    stores.put(stack, Str::new(&stores.scratch[0]));
}

fn n_base64url_encode(stores: &mut Stores, stack: &mut DbRef) {
    let v = *stores.get::<Str>(stack);
    stores.scratch.clear();
    stores
        .scratch
        .push(crate::base64::encode_url(v.str().as_bytes()));
    stores.put(stack, Str::new(&stores.scratch[0]));
}

// ── WebSocket + TCP + OpenGL + random glue removed ─────────────────────
// These functions are now auto-marshalled by extensions::wire_native_fns().
// See EXTERNAL_LIBS.md Phase 5 for design.

// ── P54: JsonValue native bindings (primitive-only, step 2) ─────────────
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

const JV_DISCR_NULL: i32 = 1;
const JV_DISCR_BOOL: i32 = 2;
const JV_DISCR_NUMBER: i32 = 3;
const JV_DISCR_STRING: i32 = 4;
const JV_DISCR_ARRAY: i32 = 5;
const JV_DISCR_OBJECT: i32 = 6;

/// Allocate a fresh `JsonValue` record in its own store and return
/// the DbRef.  Caller writes the discriminant byte at pos+0 and any
/// variant payload at pos + position(variant_tp, field_name).
fn jv_alloc(stores: &mut Stores) -> DbRef {
    let jv_tp = stores.name("JsonValue");
    let size_bytes = u32::from(stores.size(jv_tp));
    // database(n) → claim(n) which expects 8-byte words; round up
    // and add 1 word for the record header.
    let words = size_bytes.div_ceil(8) + 1;
    stores.database(words.max(2))
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
fn dbref_to_parsed(stores: &Stores, src: &DbRef) -> crate::json::Parsed {
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
        JV_DISCR_STRING => {
            let str_tp = stores.name("JString");
            let val_pos = u32::from(stores.position(str_tp, "value"));
            let s_rec = stores.store(src).get_int(src.rec, src.pos + val_pos) as u32;
            let s = stores.store(src).get_str(s_rec).to_owned();
            crate::json::Parsed::Str(s)
        }
        JV_DISCR_ARRAY => {
            let array_tp = stores.name("JArray");
            let items_pos = u32::from(stores.position(array_tp, "items")) + src.pos;
            let items_rec = stores.store(src).get_int(src.rec, items_pos);
            let mut children = Vec::new();
            if items_rec > 0 {
                let length = stores.store(src).get_int(items_rec as u32, 4);
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
            let fields_rec = stores.store(src).get_int(src.rec, fields_pos);
            let mut entries = Vec::new();
            if fields_rec > 0 {
                let length = stores.store(src).get_int(fields_rec as u32, 4);
                let jf_tp = stores.name("JsonField");
                let jf_size = u32::from(stores.size(jf_tp));
                let name_field_pos = u32::from(stores.position(jf_tp, "name"));
                let value_field_pos = u32::from(stores.position(jf_tp, "value"));
                for i in 0..length {
                    let elem_offset =
                        8u32 + u32::try_from(i).expect("non-negative length") * jf_size;
                    let name_rec = stores
                        .store(src)
                        .get_int(fields_rec as u32, elem_offset + name_field_pos)
                        as u32;
                    let name = stores.store(src).get_str(name_rec).to_owned();
                    let value_slot = DbRef {
                        store_nr: src.store_nr,
                        rec: fields_rec as u32,
                        pos: elem_offset + value_field_pos,
                    };
                    entries.push((name, dbref_to_parsed(stores, &value_slot)));
                }
            }
            crate::json::Parsed::Object(entries)
        }
        _ => crate::json::Parsed::Null,
    }
}

fn materialise_primitive_into(stores: &mut Stores, slot: &DbRef, child: &crate::json::Parsed) {
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
            sm.set_int(slot.rec, val_pos, s_rec as i32);
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
            sm.set_int(slot.rec, items_abs_pos, 0);
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
            sm.set_int(slot.rec, fields_abs_pos, 0);
            for (key, inner) in v {
                let elm =
                    crate::vector::vector_append(&fields_db, jf_size, &mut stores.allocations);
                let name_rec = stores.store_mut(&elm).set_str(key);
                stores
                    .store_mut(&elm)
                    .set_int(elm.rec, elm.pos + name_field_pos, name_rec as i32);
                let value_slot = DbRef {
                    store_nr: elm.store_nr,
                    rec: elm.rec,
                    pos: elm.pos + value_field_pos,
                };
                materialise_primitive_into(stores, &value_slot, inner);
                crate::vector::vector_finish(&fields_db, &mut stores.allocations);
            }
        }
    }
}

fn n_json_parse(stores: &mut Stores, stack: &mut DbRef) {
    let v_raw = *stores.get::<Str>(stack);
    let parsed = crate::json::parse(v_raw.str());
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
            store_mut.set_int(result.rec, value_pos, s_rec as i32);
            stores.last_json_errors.clear();
        }
        Ok(crate::json::Parsed::Array(v)) if v.is_empty() => {
            // Step 4 first slice (2026-04-14): empty arrays don't need
            // arena recursion — set the JArray discriminant and leave
            // the items field zero-initialised.  `n_len` on JArray
            // returns 0 today (every JArray is empty until the full
            // arena materialiser ships), so this reads as the empty
            // array callers expect.
            stores
                .store_mut(&result)
                .set_byte(result.rec, pos, 0, JV_DISCR_ARRAY);
            stores.last_json_errors.clear();
        }
        Ok(crate::json::Parsed::Object(v)) if v.is_empty() => {
            // Step 4 first slice (2026-04-14): empty objects mirror
            // empty arrays — discriminant only; no field-vector to
            // materialise; `n_len` returns 0 for every JObject today.
            stores
                .store_mut(&result)
                .set_byte(result.rec, pos, 0, JV_DISCR_OBJECT);
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
            store_mut.set_int(result.rec, items_abs_pos, 0);
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
            store_mut.set_int(result.rec, fields_abs_pos, 0);
            for (key, child) in v {
                let elm =
                    crate::vector::vector_append(&fields_db, jf_size, &mut stores.allocations);
                // Write name: set_str claims a sub-record for the
                // key bytes; store its record-nr in the name field.
                let name_rec = stores.store_mut(&elm).set_str(key);
                stores
                    .store_mut(&elm)
                    .set_int(elm.rec, elm.pos + name_field_pos, name_rec as i32);
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
        Err(err) => {
            stores
                .store_mut(&result)
                .set_byte(result.rec, pos, 0, JV_DISCR_NULL);
            stores.last_json_errors.clear();
            stores
                .last_json_errors
                .push(crate::json::format_error(v_raw.str(), &err, 2, 1));
        }
    }
    stores.put(stack, result);
}

fn n_json_errors(stores: &mut Stores, stack: &mut DbRef) {
    let msg = stores.last_json_errors.join("|");
    stores.scratch.clear();
    stores.scratch.push(msg);
    stores.put(stack, Str::new(&stores.scratch[0]));
}

fn n_as_text(stores: &mut Stores, stack: &mut DbRef) {
    let v = *stores.get::<DbRef>(stack);
    let discr = stores.store(&v).get_byte(v.rec, v.pos, 0);
    if discr == JV_DISCR_STRING {
        let str_tp = stores.name("JString");
        let value_pos = u32::from(stores.position(str_tp, "value")) + v.pos;
        let s_rec = stores.store(&v).get_int(v.rec, value_pos) as u32;
        let s = stores.store(&v).get_str(s_rec).to_string();
        stores.scratch.clear();
        stores.scratch.push(s);
        stores.put(stack, Str::new(&stores.scratch[0]));
    } else {
        stores.put(stack, Str::new(crate::state::STRING_NULL));
    }
}

fn n_as_number(stores: &mut Stores, stack: &mut DbRef) {
    let v = *stores.get::<DbRef>(stack);
    let discr = stores.store(&v).get_byte(v.rec, v.pos, 0);
    if discr == JV_DISCR_NUMBER {
        let num_tp = stores.name("JNumber");
        let value_pos = u32::from(stores.position(num_tp, "value")) + v.pos;
        let n = stores.store(&v).get_float(v.rec, value_pos);
        stores.put(stack, n);
    } else {
        stores.put(stack, f64::NAN);
    }
}

fn n_as_long(stores: &mut Stores, stack: &mut DbRef) {
    let v = *stores.get::<DbRef>(stack);
    let discr = stores.store(&v).get_byte(v.rec, v.pos, 0);
    if discr == JV_DISCR_NUMBER {
        let num_tp = stores.name("JNumber");
        let value_pos = u32::from(stores.position(num_tp, "value")) + v.pos;
        let n = stores.store(&v).get_float(v.rec, value_pos);
        stores.put(stack, n.trunc() as i64);
    } else {
        stores.put(stack, i64::MIN);
    }
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
        let fallback = jv_alloc(stores);
        stores
            .store_mut(&fallback)
            .set_byte(fallback.rec, fallback.pos, 0, JV_DISCR_NULL);
        stores.put(stack, fallback);
        return;
    }
    let obj_tp = stores.name("JObject");
    let fields_pos = u32::from(stores.position(obj_tp, "fields")) + self_ref.pos;
    let fields_rec = stores.store(&self_ref).get_int(self_ref.rec, fields_pos);
    if fields_rec <= 0 {
        let fallback = jv_alloc(stores);
        stores
            .store_mut(&fallback)
            .set_byte(fallback.rec, fallback.pos, 0, JV_DISCR_NULL);
        stores.put(stack, fallback);
        return;
    }
    let length = stores.store(&self_ref).get_int(fields_rec as u32, 4);
    let jf_tp = stores.name("JsonField");
    let jf_size = u32::from(stores.size(jf_tp));
    let name_field_pos = u32::from(stores.position(jf_tp, "name"));
    let value_field_pos = u32::from(stores.position(jf_tp, "value"));
    let lookup = name.str().to_owned();
    for i in 0..length {
        let elm_offset = 8u32 + u32::try_from(i).expect("non-negative length") * jf_size;
        let name_rec = stores
            .store(&self_ref)
            .get_int(fields_rec as u32, elm_offset + name_field_pos) as u32;
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
    let fallback = jv_alloc(stores);
    stores
        .store_mut(&fallback)
        .set_byte(fallback.rec, fallback.pos, 0, JV_DISCR_NULL);
    stores.put(stack, fallback);
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
    let index = *stores.get::<i32>(stack);
    let self_ref = *stores.get::<DbRef>(stack);
    let discr = stores
        .store(&self_ref)
        .get_byte(self_ref.rec, self_ref.pos, 0);
    if discr != JV_DISCR_ARRAY || index < 0 {
        let fallback = jv_alloc(stores);
        stores
            .store_mut(&fallback)
            .set_byte(fallback.rec, fallback.pos, 0, JV_DISCR_NULL);
        stores.put(stack, fallback);
        return;
    }
    let array_tp = stores.name("JArray");
    let items_pos = u32::from(stores.position(array_tp, "items")) + self_ref.pos;
    let items_rec = stores.store(&self_ref).get_int(self_ref.rec, items_pos);
    if items_rec <= 0 {
        let fallback = jv_alloc(stores);
        stores
            .store_mut(&fallback)
            .set_byte(fallback.rec, fallback.pos, 0, JV_DISCR_NULL);
        stores.put(stack, fallback);
        return;
    }
    let length = stores.store(&self_ref).get_int(items_rec as u32, 4);
    if index >= length {
        let fallback = jv_alloc(stores);
        stores
            .store_mut(&fallback)
            .set_byte(fallback.rec, fallback.pos, 0, JV_DISCR_NULL);
        stores.put(stack, fallback);
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
    let len = match discr {
        JV_DISCR_ARRAY => {
            let array_tp = stores.name("JArray");
            let items_pos = u32::from(stores.position(array_tp, "items")) + v.pos;
            let items_rec = stores.store(&v).get_int(v.rec, items_pos);
            if items_rec <= 0 {
                0
            } else {
                stores.store(&v).get_int(items_rec as u32, 4)
            }
        }
        JV_DISCR_OBJECT => {
            let obj_tp = stores.name("JObject");
            let fields_pos = u32::from(stores.position(obj_tp, "fields")) + v.pos;
            let fields_rec = stores.store(&v).get_int(v.rec, fields_pos);
            if fields_rec <= 0 {
                0
            } else {
                stores.store(&v).get_int(fields_rec as u32, 4)
            }
        }
        _ => i32::MIN,
    };
    stores.put(stack, len);
}

// ─────────────── P54 step 5 — single-walker `Struct.parse(JsonValue)` ──────
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
    let struct_kt_arg = *stores.get::<i32>(stack);
    let src = *stores.get::<DbRef>(stack);
    let struct_kt = struct_kt_arg as u16;
    // `stores.size` returns the struct's size in bytes; `database`
    // wants words (8 bytes each).  Round up + min 2 words so the
    // handle's i32 fields at offset 8 always have valid backing
    // (the boolean allocator-corruption fix from stash@{0}).
    let bytes = u32::from(stores.size(struct_kt));
    let words = bytes.div_ceil(8);
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
fn populate_struct_from_jsonvalue(stores: &mut Stores, dest: &DbRef, struct_kt: u16, src: &DbRef) {
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
                let s_rec = stores.store(&sub).get_int(sub.rec, value_pos) as u32;
                let text_val = stores.store(&sub).get_str(s_rec).to_owned();
                let new_s_rec = stores.store_mut(dest).set_str(&text_val);
                stores
                    .store_mut(dest)
                    .set_int(dest.rec, dest_field_pos, new_s_rec as i32);
            } else {
                stores.store_mut(dest).set_int(dest.rec, dest_field_pos, 0);
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
                Parts::EnumValue(_, _) | Parts::Enum(_) => {
                    // Mixed struct-enum field — only `JsonValue`
                    // passthrough is supported today.  Skip the copy
                    // when the source is absent (sub is a dummy
                    // pointing at the dest, copy would garble the
                    // dest's own bytes).
                    if sub_jv.is_some() {
                        let inner_name = stores.types[content_kt as usize].name.clone();
                        if inner_name == "JsonValue" {
                            let jv_size = u32::from(stores.size(content_kt));
                            copy_bytes(stores, &sub, dest, dest_field_pos, jv_size);
                        }
                    }
                    // Other struct-enum types: leave at default.
                }
                Parts::Vector(elem_kt) => {
                    // Vector field: handle is a 4-byte rec-nr at
                    // `dest_field_pos`.  Iterate JArray items and
                    // append per element via the existing
                    // `vector_append` machinery.  Absent source →
                    // skip (handle stays at zero = empty vector).
                    if sub_jv.is_some() {
                        let dest_handle = DbRef {
                            store_nr: dest.store_nr,
                            rec: dest.rec,
                            pos: dest_field_pos,
                        };
                        populate_vector_from_jarray(stores, &dest_handle, elem_kt, &sub);
                    }
                }
                _ => {
                    // Other field types (Hash, Sorted, Index, Spacial,
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
    let fields_rec = stores.store(src).get_int(src.rec, fields_pos);
    if fields_rec <= 0 {
        return None;
    }
    let length = stores.store(src).get_int(fields_rec as u32, 4);
    let jf_tp = stores.name("JsonField");
    let jf_size = u32::from(stores.size(jf_tp));
    let name_field_pos = u32::from(stores.position(jf_tp, "name"));
    let value_field_pos = u32::from(stores.position(jf_tp, "value"));
    for i in 0..length {
        let elm_off = 8u32 + u32::try_from(i).expect("non-negative") * jf_size;
        let name_rec = stores
            .store(src)
            .get_int(fields_rec as u32, elm_off + name_field_pos) as u32;
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

fn unwrap_long(
    stores: &mut Stores,
    sub: &DbRef,
    item_discr: i32,
    struct_name: &str,
    field_name: &str,
) -> i64 {
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
) -> i32 {
    push_kind_mismatch(stores, item_discr, JV_DISCR_NUMBER, struct_name, field_name);
    if item_discr != JV_DISCR_NUMBER {
        return i32::MIN;
    }
    let num_tp = stores.name("JNumber");
    let value_pos = u32::from(stores.position(num_tp, "value")) + sub.pos;
    let f = stores.store(sub).get_float(sub.rec, value_pos);
    if !f.is_finite() {
        return i32::MIN;
    }
    let as_i64 = f as i64;
    if (i64::from(i32::MIN)..=i64::from(i32::MAX)).contains(&as_i64) {
        as_i64 as i32
    } else {
        i32::MIN
    }
}

fn unwrap_float(
    stores: &mut Stores,
    sub: &DbRef,
    item_discr: i32,
    struct_name: &str,
    field_name: &str,
) -> f64 {
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
    let s_rec = stores.store(sub).get_int(sub.rec, value_pos) as u32;
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
    let items_rec = stores.store(src_arr).get_int(src_arr.rec, items_pos);
    if items_rec <= 0 {
        return;
    }
    let length = stores.store(src_arr).get_int(items_rec as u32, 4);
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
                .set_int(elm.rec, elm.pos, new_s_rec as i32);
        } else if matches!(elem_parts, Parts::Struct(_)) {
            // Struct element — recurse into the walker writing into
            // the freshly-appended embedded element slot.
            populate_struct_from_jsonvalue(stores, &elm, elem_kt, &item);
        }
        crate::vector::vector_finish(dest_handle, &mut stores.allocations);
    }
}

/// Q4 primitive constructor — allocate a JsonValue set to the `JNull`
/// variant and return a DbRef to it.  No arena needed (JNull has no
/// payload), so this can ship ahead of P54 step 4's container
/// materialisation.  Useful for test fixtures that want to construct
/// a known-null JsonValue without going through `json_parse("null")`.
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
    store_mut.set_int(result.rec, value_pos, s_rec as i32);
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
        let input_inner_rec = stores.store(&items).get_int(items.rec, items.pos) as u32;
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
        let input_inner_rec = stores.store(&fields).get_int(fields.rec, fields.pos) as u32;
        let jf_tp = stores.name("JsonField");
        let jf_size = u32::from(stores.size(jf_tp));
        let name_field_pos = u32::from(stores.position(jf_tp, "name"));
        let value_field_pos = u32::from(stores.position(jf_tp, "value"));
        let mut entries: Vec<(String, crate::json::Parsed)> = Vec::with_capacity(length as usize);
        for i in 0..length {
            let elem_offset = 8u32 + i * jf_size;
            let name_rec = stores
                .store(&fields)
                .get_int(input_inner_rec, elem_offset + name_field_pos)
                as u32;
            let name = stores.store(&fields).get_str(name_rec).to_owned();
            let value_slot = DbRef {
                store_nr: fields.store_nr,
                rec: input_inner_rec,
                pos: elem_offset + value_field_pos,
            };
            entries.push((name, dbref_to_parsed(stores, &value_slot)));
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
    let fields_rec = stores.store(&v).get_int(v.rec, fields_pos);
    if fields_rec <= 0 {
        stores.put(stack, false);
        return;
    }
    let length = stores.store(&v).get_int(fields_rec as u32, 4);
    let jf_tp = stores.name("JsonField");
    let jf_size = u32::from(stores.size(jf_tp));
    let name_field_pos = u32::from(stores.position(jf_tp, "name"));
    let lookup = name.str().to_owned();
    for i in 0..length {
        let elm_offset = 8u32 + u32::try_from(i).expect("non-negative length") * jf_size;
        let name_rec = stores
            .store(&v)
            .get_int(fields_rec as u32, elm_offset + name_field_pos) as u32;
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
    stores.store_mut(&vec).set_int(vec.rec, vec.pos, 0);
    if discr != JV_DISCR_OBJECT {
        stores.put(stack, vec);
        return;
    }
    let obj_tp = stores.name("JObject");
    let fields_pos = u32::from(stores.position(obj_tp, "fields")) + v.pos;
    let fields_rec = stores.store(&v).get_int(v.rec, fields_pos);
    if fields_rec <= 0 {
        stores.put(stack, vec);
        return;
    }
    let length = stores.store(&v).get_int(fields_rec as u32, 4);
    let jf_tp = stores.name("JsonField");
    let jf_size = u32::from(stores.size(jf_tp));
    let name_field_pos = u32::from(stores.position(jf_tp, "name"));
    for i in 0..length {
        let elm_offset = 8u32 + u32::try_from(i).expect("non-negative length") * jf_size;
        let name_rec_in_jobject = stores
            .store(&v)
            .get_int(fields_rec as u32, elm_offset + name_field_pos)
            as u32;
        let name_str = stores.store(&v).get_str(name_rec_in_jobject).to_owned();
        let elm = crate::vector::vector_append(&vec, text_size, &mut stores.allocations);
        let new_name_rec = stores.store_mut(&elm).set_str(&name_str);
        stores
            .store_mut(&elm)
            .set_int(elm.rec, elm.pos, new_name_rec as i32);
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
    stores.store_mut(&vec).set_int(vec.rec, vec.pos, 0);
    if discr != JV_DISCR_OBJECT {
        stores.put(stack, vec);
        return;
    }
    let obj_tp = stores.name("JObject");
    let fields_pos = u32::from(stores.position(obj_tp, "fields")) + v.pos;
    let fields_rec = stores.store(&v).get_int(v.rec, fields_pos);
    if fields_rec <= 0 {
        stores.put(stack, vec);
        return;
    }
    let length = stores.store(&v).get_int(fields_rec as u32, 4);
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
            .get_int(fields_rec as u32, elm_offset + name_field_pos) as u32;
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
            .set_int(elm.rec, elm.pos + name_field_pos, new_name_rec as i32);
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

/// Q2 — `kind(self: JsonValue) -> text` reads the discriminant byte
/// at offset 0 of the JsonValue record and returns the variant name
/// as text.  Cheap: one memory read + a literal map, no arena walk.
/// Useful for free-form introspection (logs, conditional branches)
/// without committing to a particular variant via pattern match.
///
/// Unknown / uninitialised bytes map to `"JUnknown"` rather than
/// panicking, so the function is safe to call on any DbRef that
/// parses as a JsonValue — defensive posture for the period when
/// step 4's arena may write intermediate states.
fn n_kind(stores: &mut Stores, stack: &mut DbRef) {
    let v = *stores.get::<DbRef>(stack);
    let discr = stores.store(&v).get_byte(v.rec, v.pos, 0);
    let name = match discr {
        JV_DISCR_NULL => "JNull",
        JV_DISCR_BOOL => "JBool",
        JV_DISCR_NUMBER => "JNumber",
        JV_DISCR_STRING => "JString",
        JV_DISCR_ARRAY => "JArray",
        JV_DISCR_OBJECT => "JObject",
        _ => "JUnknown",
    };
    stores.scratch.clear();
    stores.scratch.push(name.to_string());
    stores.put(stack, Str::new(&stores.scratch[0]));
}

/// Render a JsonValue to RFC 8259 JSON text.  The `pretty` flag
/// controls indent emission in container arms: when `true`,
/// non-empty `JArray` / `JObject` emit `\n` + 2-space indent per
/// element/field and dedent the closing bracket to the parent's
/// depth.  Empty containers stay `[]` / `{}` regardless.
/// Primitives are byte-identical in both modes.
fn json_to_text(stores: &Stores, v: &DbRef, pretty: bool) -> String {
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
        JV_DISCR_STRING => {
            let str_tp = stores.name("JString");
            let value_pos = u32::from(stores.position(str_tp, "value")) + v.pos;
            let s_rec = stores.store(v).get_int(v.rec, value_pos) as u32;
            let raw = stores.store(v).get_str(s_rec).to_string();
            let mut out = String::with_capacity(raw.len() + 2);
            write_json_string(&mut out, &raw);
            out
        }
        JV_DISCR_ARRAY => {
            let array_tp = stores.name("JArray");
            let items_pos = u32::from(stores.position(array_tp, "items")) + v.pos;
            let items_rec = stores.store(v).get_int(v.rec, items_pos);
            if items_rec <= 0 {
                return "[]".to_string();
            }
            let length = stores.store(v).get_int(items_rec as u32, 4);
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
            let fields_rec = stores.store(v).get_int(v.rec, fields_pos);
            if fields_rec <= 0 {
                return "{}".to_string();
            }
            let length = stores.store(v).get_int(fields_rec as u32, 4);
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
                    .get_int(fields_rec as u32, elm_offset + name_field_pos)
                    as u32;
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

/// Q3 primitive-slice — `to_json(self: JsonValue) -> text`
/// serialises a JsonValue to canonical RFC 8259 JSON text.  Covers
/// the four primitive variants today; JArray / JObject return a
/// `"<pending step 4>"` placeholder rather than panicking, so
/// callers can already use `to_json(v)` on mixed trees without
/// crashing — the stub visibly marks the frontier.
///
/// Strings escape `"`, `\\`, and the control bytes `<0x20`; UTF-8
/// bytes pass through verbatim (RFC 8259 allows both; shortest
/// wins).  Numbers use Rust's default `f64::Display`, which
/// already emits shortest-round-trip.  Booleans render as `true`
/// / `false`; null renders as `null`.
fn n_to_json(stores: &mut Stores, stack: &mut DbRef) {
    let v = *stores.get::<DbRef>(stack);
    let out = json_to_text(stores, &v, false);
    stores.scratch.clear();
    stores.scratch.push(out);
    stores.put(stack, Str::new(&stores.scratch[0]));
}

/// Q3 primitive-slice — `to_json_pretty(self: JsonValue) -> text`
/// mirrors `to_json` today because primitive variants carry no
/// nested structure — canonical and pretty output are
/// byte-identical.  Retained as a separate entry point so the
/// surface is forward-compatible: once P54 step 4 arena-
/// materialises `JArray` / `JObject`, this path will branch into
/// 2-space indent + one-element-per-line layout at the same site.
fn n_to_json_pretty(stores: &mut Stores, stack: &mut DbRef) {
    let v = *stores.get::<DbRef>(stack);
    let out = json_to_text(stores, &v, true);
    stores.scratch.clear();
    stores.scratch.push(out);
    stores.put(stack, Str::new(&stores.scratch[0]));
}
