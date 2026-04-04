//! Native function registry: Rust implementations of loft built-ins.
//! Naming: `n_<name>` for globals, `t_<LEN><Type>_<method>` for methods.
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(non_snake_case)]
use crate::database::Stores;
use crate::keys::{DbRef, Str};
use crate::logger::Severity;
#[cfg(any(feature = "random", all(feature = "wasm", not(feature = "random"))))]
use crate::ops;
use crate::parallel::{
    WorkerProgram, run_parallel_direct, run_parallel_int, run_parallel_raw, run_parallel_ref,
    run_parallel_text,
};
use crate::platform::sep;
use crate::state::{Call, State};
use crate::vector;
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
    ("t_4text_is_lowercase", t_4text_is_lowercase),
    ("t_9character_is_lowercase", t_9character_is_lowercase),
    ("t_4text_is_uppercase", t_4text_is_uppercase),
    ("t_9character_is_uppercase", t_9character_is_uppercase),
    ("t_4text_is_numeric", t_4text_is_numeric),
    ("t_9character_is_numeric", t_9character_is_numeric),
    ("t_4text_is_alphanumeric", t_4text_is_alphanumeric),
    ("t_9character_is_alphanumeric", t_9character_is_alphanumeric),
    ("t_4text_is_alphabetic", t_4text_is_alphabetic),
    ("t_9character_is_alphabetic", t_9character_is_alphabetic),
    ("t_4text_is_whitespace", t_4text_is_whitespace),
    ("t_4text_is_control", t_4text_is_control),
    ("n_arguments", n_arguments),
    ("n_directory", n_directory),
    ("n_user_directory", n_user_directory),
    ("n_program_directory", n_program_directory),
    ("n_get_store_lock", n_get_store_lock),
    ("n_set_store_lock", n_set_store_lock),
    ("n_parallel_for_int", n_parallel_for_int),
    ("n_parallel_for", n_parallel_for),
    ("n_parallel_for_light", n_parallel_for_light),
    ("n_parallel_get_int", n_parallel_get_int),
    ("n_parallel_get_long", n_parallel_get_long),
    ("n_parallel_get_float", n_parallel_get_float),
    ("n_parallel_get_bool", n_parallel_get_bool),
    ("n_rand", n_rand),
    ("n_rand_seed", n_rand_seed),
    ("n_rand_indices", n_rand_indices),
    ("n_load_png", n_load_png),
    ("n_now", n_now),
    ("n_ticks", n_ticks),
    ("n_stack_trace", n_stack_trace),
    ("n_path_sep", n_path_sep),
    ("i_parse_error_push", i_parse_error_push),
    ("i_parse_errors", i_parse_errors),
    ("n_http_do", n_http_do),
    ("n_http_body", n_http_body),
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

fn t_4text_is_lowercase(stores: &mut Stores, stack: &mut DbRef) {
    let v_self = *stores.get::<Str>(stack);
    let new_value = {
        let mut res = true;
        for c in v_self.str().chars() {
            if !c.is_lowercase() {
                res = false;
            }
        }
        res
    };
    stores.put(stack, new_value);
}

fn t_9character_is_lowercase(stores: &mut Stores, stack: &mut DbRef) {
    let v_self = *stores.get::<char>(stack);
    let new_value = { v_self.is_lowercase() };
    stores.put(stack, new_value);
}

fn t_4text_is_uppercase(stores: &mut Stores, stack: &mut DbRef) {
    let v_self = *stores.get::<Str>(stack);
    let new_value = {
        let mut res = true;
        for c in v_self.str().chars() {
            if !c.is_uppercase() {
                res = false;
            }
        }
        res
    };
    stores.put(stack, new_value);
}

fn t_9character_is_uppercase(stores: &mut Stores, stack: &mut DbRef) {
    let v_self = *stores.get::<char>(stack);
    let new_value = { v_self.is_uppercase() };
    stores.put(stack, new_value);
}

fn t_4text_is_numeric(stores: &mut Stores, stack: &mut DbRef) {
    let v_self = *stores.get::<Str>(stack);
    let new_value = {
        let mut res = true;
        for c in v_self.str().chars() {
            if !c.is_numeric() {
                res = false;
            }
        }
        res
    };
    stores.put(stack, new_value);
}

fn t_9character_is_numeric(stores: &mut Stores, stack: &mut DbRef) {
    let v_self = *stores.get::<char>(stack);
    let new_value = { v_self.is_numeric() };
    stores.put(stack, new_value);
}

fn t_4text_is_alphanumeric(stores: &mut Stores, stack: &mut DbRef) {
    let v_self = *stores.get::<Str>(stack);
    let new_value = {
        let mut res = true;
        for c in v_self.str().chars() {
            if !c.is_alphanumeric() {
                res = false;
            }
        }
        res
    };
    stores.put(stack, new_value);
}

fn t_9character_is_alphanumeric(stores: &mut Stores, stack: &mut DbRef) {
    let v_self = *stores.get::<char>(stack);
    let new_value = { v_self.is_alphanumeric() };
    stores.put(stack, new_value);
}

fn t_4text_is_alphabetic(stores: &mut Stores, stack: &mut DbRef) {
    let v_self = *stores.get::<Str>(stack);
    let new_value = {
        let mut res = true;
        for c in v_self.str().chars() {
            if !c.is_alphabetic() {
                res = false;
            }
        }
        res
    };
    stores.put(stack, new_value);
}

fn t_9character_is_alphabetic(stores: &mut Stores, stack: &mut DbRef) {
    let v_self = *stores.get::<char>(stack);
    let new_value = { v_self.is_alphabetic() };
    stores.put(stack, new_value);
}

fn t_4text_is_whitespace(stores: &mut Stores, stack: &mut DbRef) {
    let v_self = *stores.get::<Str>(stack);
    let new_value = {
        let mut res = true;
        for c in v_self.str().chars() {
            if !c.is_whitespace() {
                res = false;
            }
        }
        res
    };
    stores.put(stack, new_value);
}

fn t_4text_is_control(stores: &mut Stores, stack: &mut DbRef) {
    let v_self = *stores.get::<Str>(stack);
    let new_value = {
        let mut res = true;
        for c in v_self.str().chars() {
            if !c.is_control() {
                res = false;
            }
        }
        res
    };
    stores.put(stack, new_value);
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

/// Dispatch a compiled loft function over every row of an input vector,
/// running `threads` OS threads in parallel.
///
/// Loft signature:
/// ```loft
/// fn parallel_for_int(func: text, input: reference,
///                     element_size: integer, threads: integer) -> reference;
/// ```
///
/// The worker function must have the signature `fn f(row: &T) -> integer`.
/// Returns a `reference` pointing to a freshly allocated vector of integers,
/// one per input row in the original order.
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
        let text_code = unsafe { Arc::clone(&*ctx.text_code) };
        let library = unsafe { Arc::clone(&*ctx.library) };
        (
            fn_pos,
            WorkerProgram {
                bytecode,
                text_code,
                library,
                stack_trace_lib_nr: ctx.stack_trace_lib_nr,
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
        let text_code = unsafe { Arc::clone(&*ctx.text_code) };
        let library = unsafe { Arc::clone(&*ctx.library) };
        (
            fn_pos,
            WorkerProgram {
                bytecode,
                text_code,
                library,
                stack_trace_lib_nr: ctx.stack_trace_lib_nr,
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
        let text_code = unsafe { Arc::clone(&*ctx.text_code) };
        let library = unsafe { Arc::clone(&*ctx.library) };
        (
            fn_pos,
            WorkerProgram {
                bytecode,
                text_code,
                library,
                stack_trace_lib_nr: ctx.stack_trace_lib_nr,
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

// ── parallel_get_* ────────────────────────────────────────────────────────────
//
// Read element `idx` from a parallel_for result reference.
// The result layout (from n_parallel_for):
//   header rec, pos=4  → i32 pointing to vec_rec
//   vec_rec, pos=8+i*S → element i  (S = 4 int / 8 long+float / 1 bool bytes)
//
// Emitted by the compiler for `for(a in src) |b = f(a)| * N { ... }`.

fn n_parallel_get_int(stores: &mut Stores, stack: &mut DbRef) {
    let v_idx = *stores.get::<i32>(stack);
    let v_ref = *stores.get::<DbRef>(stack);
    let store = stores.store(&v_ref);
    let vec_rec = store.get_int(v_ref.rec, v_ref.pos) as u32;
    let val = store.get_int(vec_rec, 8 + v_idx as u32 * 4);
    stores.put(stack, val);
}

fn n_parallel_get_long(stores: &mut Stores, stack: &mut DbRef) {
    let v_idx = *stores.get::<i32>(stack);
    let v_ref = *stores.get::<DbRef>(stack);
    let store = stores.store(&v_ref);
    let vec_rec = store.get_int(v_ref.rec, v_ref.pos) as u32;
    let val = store.get_long(vec_rec, 8 + v_idx as u32 * 8);
    stores.put(stack, val);
}

fn n_parallel_get_float(stores: &mut Stores, stack: &mut DbRef) {
    let v_idx = *stores.get::<i32>(stack);
    let v_ref = *stores.get::<DbRef>(stack);
    let store = stores.store(&v_ref);
    let vec_rec = store.get_int(v_ref.rec, v_ref.pos) as u32;
    let bits = store.get_long(vec_rec, 8 + v_idx as u32 * 8);
    stores.put(stack, f64::from_bits(bits as u64));
}

fn n_parallel_get_bool(stores: &mut Stores, stack: &mut DbRef) {
    let v_idx = *stores.get::<i32>(stack);
    let v_ref = *stores.get::<DbRef>(stack);
    let store = stores.store(&v_ref);
    let vec_rec = store.get_int(v_ref.rec, v_ref.pos) as u32;
    let val = store.get_byte(vec_rec, 8 + v_idx as u32, 0);
    stores.put(stack, val != 0);
}

/// Return a random integer in [lo, hi] (inclusive); null (`i32::MIN`) if lo > hi.
fn n_rand(stores: &mut Stores, stack: &mut DbRef) {
    let v_hi = *stores.get::<i32>(stack);
    let v_lo = *stores.get::<i32>(stack);
    let result = if let Some(f) = unsafe {
        crate::extensions::get_native_fn::<extern "C" fn(i32, i32) -> i32>("loft_rand_int")
    } {
        f(v_lo, v_hi)
    } else {
        #[cfg(feature = "random")]
        {
            ops::rand_int(v_lo, v_hi)
        }
        #[cfg(not(feature = "random"))]
        {
            i32::MIN
        }
    };
    stores.put(stack, result);
}

/// Seed the thread-local PCG RNG so subsequent `rand()` calls are reproducible.
fn n_rand_seed(stores: &mut Stores, stack: &mut DbRef) {
    let v_seed = *stores.get::<i64>(stack);
    if let Some(f) =
        unsafe { crate::extensions::get_native_fn::<extern "C" fn(i64)>("loft_rand_seed") }
    {
        f(v_seed);
    } else {
        #[cfg(feature = "random")]
        ops::rand_seed(v_seed);
    }
}

/// Return a vector of `n` integers `[0, 1, ..., n-1]` in a random order.
/// Returns an empty vector when `n <= 0` or `n` is null.
fn n_rand_indices(stores: &mut Stores, stack: &mut DbRef) {
    let v_n = *stores.get::<i32>(stack);
    let n = if v_n == i32::MIN || v_n <= 0 {
        0usize
    } else {
        v_n as usize
    };

    // Get shuffled indices — from cdylib or built-in fallback.
    let indices: Vec<i32> = if let Some(f) = unsafe {
        crate::extensions::get_native_fn::<unsafe extern "C" fn(i32, *mut *mut i32, *mut usize)>(
            "loft_rand_indices",
        )
    } {
        let mut ptr: *mut i32 = std::ptr::null_mut();
        let mut len: usize = 0;
        unsafe {
            f(
                v_n,
                std::ptr::addr_of_mut!(ptr),
                std::ptr::addr_of_mut!(len),
            );
        }
        if ptr.is_null() || len == 0 {
            Vec::new()
        } else {
            let v = unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec();
            if let Some(free_fn) = unsafe {
                crate::extensions::get_native_fn::<unsafe extern "C" fn(*mut i32, usize)>(
                    "loft_free_indices",
                )
            } {
                unsafe { free_fn(ptr, len) };
            }
            v
        }
    } else {
        #[cfg(feature = "random")]
        {
            let mut v: Vec<i32> = (0..n as i32).collect();
            ops::shuffle_ints(&mut v);
            v
        }
        #[cfg(not(feature = "random"))]
        {
            Vec::new()
        }
    };

    // Write into store.
    let result_db = stores.null();
    let vec_words = (n as u32 * 4 + 15) / 8;
    let vec_words = vec_words.max(1);
    let vec_cr = stores.claim(&result_db, vec_words);
    let vec_rec = vec_cr.rec;
    let header_cr = stores.claim(&result_db, 1);
    let header_rec = header_cr.rec;

    {
        let store = stores.store_mut(&result_db);
        store.set_int(vec_rec, 4, indices.len() as i32);
        for (i, &val) in indices.iter().enumerate() {
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

/// EXT.1: load a PNG image into an Image struct.
/// Calls the imaging package's cdylib decode function (resolved at load time),
/// then writes the raw pixel data into the store.
fn n_load_png(stores: &mut Stores, stack: &mut DbRef) {
    let v_image = *stores.get::<DbRef>(stack);
    let v_path = *stores.get::<Str>(stack);

    let decode_fn = unsafe {
        crate::extensions::get_native_fn::<
            unsafe extern "C" fn(
                *const u8,
                usize,
                *mut u32,
                *mut u32,
                *mut *mut u8,
                *mut usize,
            ) -> bool,
        >("loft_decode_png")
    };
    let result = if let Some(decode) = decode_fn {
        // Call cdylib's decode function via C-ABI
        let path = v_path.str();
        let mut width: u32 = 0;
        let mut height: u32 = 0;
        let mut pixels: *mut u8 = std::ptr::null_mut();
        let mut pixels_len: usize = 0;
        let ok = unsafe {
            decode(
                path.as_ptr(),
                path.len(),
                std::ptr::addr_of_mut!(width),
                std::ptr::addr_of_mut!(height),
                std::ptr::addr_of_mut!(pixels),
                std::ptr::addr_of_mut!(pixels_len),
            )
        };
        if ok && !pixels.is_null() {
            // Write decoded data into the Image struct in the store
            let pixel_data = unsafe { std::slice::from_raw_parts(pixels, pixels_len) };
            let wrote =
                write_image_to_store(stores, &v_image, width, height, pixel_data, v_path.str());
            // Free the cdylib-allocated buffer
            if let Some(free_fn) = unsafe {
                crate::extensions::get_native_fn::<unsafe extern "C" fn(*mut u8, usize)>(
                    "loft_free_pixels",
                )
            } {
                unsafe { free_fn(pixels, pixels_len) };
            }
            wrote
        } else {
            false
        }
    } else {
        // Fallback: use built-in png_store if available (png feature)
        #[cfg(feature = "png")]
        {
            stores.get_png(v_path.str(), &v_image)
        }
        #[cfg(not(feature = "png"))]
        {
            false
        }
    };
    stores.put(stack, result);
}

/// Write raw RGB pixel data into an Image struct in the store.
#[allow(clippy::cast_possible_wrap)]
fn write_image_to_store(
    stores: &mut Stores,
    image_ref: &DbRef,
    width: u32,
    height: u32,
    pixels: &[u8],
    file_path: &str,
) -> bool {
    let store = stores.store_mut(image_ref);
    // Set name field
    if let Some(name) = std::path::Path::new(file_path).file_name() {
        let name_pos = store.set_str(name.to_str().unwrap_or(""));
        store.set_int(image_ref.rec, image_ref.pos, name_pos as i32);
    }
    // Set width, height
    store.set_int(image_ref.rec, image_ref.pos + 4, width as i32);
    store.set_int(image_ref.rec, image_ref.pos + 8, height as i32);
    // Allocate vector for pixel data: 8-byte header + pixel bytes
    let pixel_count = pixels.len() / 3; // 3 bytes per Pixel (r, g, b)
    let img = store.claim((pixels.len() / 8) as u32 + 2);
    store.set_int(img, 4, pixel_count as i32);
    // Copy pixel data after the 8-byte vector header
    let buf = store.buffer(img);
    let header_bytes = 8;
    if buf.len() > header_bytes && buf.len() - header_bytes >= pixels.len() {
        buf[header_bytes..header_bytes + pixels.len()].copy_from_slice(pixels);
    }
    store.set_int(image_ref.rec, image_ref.pos + 12, img as i32);
    true
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
    let sf_elm = stores.name("StackFrame");
    let sf_size = u32::from(stores.size(sf_elm));
    // Fix #89: validate that hard-coded field offsets match the actual type layout.
    // Fix #89: validate that hard-coded field offsets match the actual type layout.
    debug_assert_eq!(
        stores.position(sf_elm, "function"),
        0,
        "StackFrame.function offset mismatch"
    );
    debug_assert_eq!(
        stores.position(sf_elm, "file"),
        4,
        "StackFrame.file offset mismatch"
    );
    debug_assert_eq!(
        stores.position(sf_elm, "line"),
        8,
        "StackFrame.line offset mismatch"
    );
    let vec = stores.database(sf_size);
    stores.store_mut(&vec).set_int(vec.rec, vec.pos, 0);

    for (fn_name, file, line) in &snapshot {
        let elm = crate::vector::vector_append(&vec, sf_size, &mut stores.allocations);
        let fn_str = stores.store_mut(&vec).set_str(fn_name.as_str());
        stores
            .store_mut(&vec)
            .set_int(elm.rec, elm.pos, fn_str as i32);
        let file_str = stores.store_mut(&vec).set_str(file.as_str());
        stores
            .store_mut(&vec)
            .set_int(elm.rec, elm.pos + 4, file_str as i32);
        stores
            .store_mut(&vec)
            .set_int(elm.rec, elm.pos + 8, *line as i32);
        // Explicitly zero arguments and variables so that reused (non-zeroed) store
        // blocks don't leave garbage data that looks like a valid first_block_rec.
        stores.store_mut(&vec).set_int(elm.rec, elm.pos + 12, 0);
        stores.store_mut(&vec).set_int(elm.rec, elm.pos + 16, 0);
        crate::vector::vector_finish(&vec, &mut stores.allocations);
    }
    stores.put(stack, vec);
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

// ── HTTP client glue (H4) ───────────────────────────────────────────────

/// Perform an HTTP request. Returns the status code as integer.
/// The response body is stored in `stores.scratch` for retrieval by `n_http_body`.
fn n_http_do(stores: &mut Stores, stack: &mut DbRef) {
    let v_headers = *stores.get::<Str>(stack);
    let v_body = *stores.get::<Str>(stack);
    let v_url = *stores.get::<Str>(stack);
    let v_method = *stores.get::<Str>(stack);

    #[allow(clippy::type_complexity)]
    let status = if let Some(req_fn) = unsafe {
        crate::extensions::get_native_fn::<
            unsafe extern "C" fn(
                *const u8,
                usize,
                *const u8,
                usize,
                *const u8,
                usize,
                *const u8,
                usize,
                *mut i32,
                *mut *mut u8,
                *mut usize,
            ),
        >("loft_http_request")
    } {
        let method = v_method.str();
        let url = v_url.str();
        let body = v_body.str();
        let headers = v_headers.str();
        let mut out_status: i32 = 0;
        let mut out_body: *mut u8 = std::ptr::null_mut();
        let mut out_body_len: usize = 0;
        let (bp, bl) = if body.is_empty() {
            (std::ptr::null(), 0)
        } else {
            (body.as_ptr(), body.len())
        };
        let (hp, hl) = if headers.is_empty() {
            (std::ptr::null(), 0)
        } else {
            (headers.as_ptr(), headers.len())
        };
        unsafe {
            req_fn(
                method.as_ptr(),
                method.len(),
                url.as_ptr(),
                url.len(),
                bp,
                bl,
                hp,
                hl,
                std::ptr::addr_of_mut!(out_status),
                std::ptr::addr_of_mut!(out_body),
                std::ptr::addr_of_mut!(out_body_len),
            );
        }
        // Store the response body in scratch for n_http_body to retrieve.
        stores.scratch.clear();
        if !out_body.is_null() && out_body_len > 0 {
            let s = unsafe {
                std::str::from_utf8_unchecked(std::slice::from_raw_parts(out_body, out_body_len))
            };
            stores.scratch.push(s.to_string());
            if let Some(free_fn) = unsafe {
                crate::extensions::get_native_fn::<unsafe extern "C" fn(*mut u8, usize)>(
                    "loft_free_string",
                )
            } {
                unsafe { free_fn(out_body, out_body_len) };
            }
        } else {
            stores.scratch.push(String::new());
        }
        out_status
    } else {
        i32::MIN // null — cdylib not loaded
    };
    stores.put(stack, status);
}

/// Retrieve the body from the last HTTP request (stored in scratch).
fn n_http_body(stores: &mut Stores, stack: &mut DbRef) {
    let body = if stores.scratch.is_empty() {
        ""
    } else {
        &stores.scratch[0]
    };
    stores.put(stack, Str::new(body));
}
