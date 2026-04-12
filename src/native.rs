//! Native function registry: Rust implementations of loft built-ins.
//! Naming: `n_<name>` for globals, `t_<LEN><Type>_<method>` for methods.
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_truncation)]
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
    ("n_sha256", n_sha256),
    ("n_hmac_sha256", n_hmac_sha256),
    ("n_base64_encode", n_base64_encode),
    ("n_base64_decode", n_base64_decode),
    ("n_base64url_encode", n_base64url_encode),
    ("n_hmac_sha256_raw", n_hmac_sha256_raw),
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
                fn_positions: Arc::new(
                    data.definitions.iter().map(|d| d.code_position).collect(),
                ),
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
                fn_positions: Arc::new(
                    data.definitions.iter().map(|d| d.code_position).collect(),
                ),
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
/// A14.7: lightweight variant — borrows stores read-only instead of deep-copying.
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
                fn_positions: Arc::new(
                    data.definitions.iter().map(|d| d.code_position).collect(),
                ),
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
/// A14.7: allocate result vector, create pool, dispatch light workers, collect.
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
#[cfg(not(feature = "wasm"))]
fn n_ticks(stores: &mut Stores, stack: &mut DbRef) {
    let micros = stores.start_time.elapsed().as_micros() as i64;
    stores.put(stack, micros);
}

#[cfg(feature = "wasm")]
fn n_ticks(stores: &mut Stores, stack: &mut DbRef) {
    let now_ms = crate::wasm::host_time_ticks();
    let elapsed_micros = (now_ms - stores.start_time_ms) * 1000;
    stores.put(stack, elapsed_micros);
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
    // P89: look up every field position from the schema instead of hard-coding
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

    // P89: schema-driven field position lookup.  A typo or rename in
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
