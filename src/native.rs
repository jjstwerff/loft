//! Native function registry: every loft built-in that is implemented in Rust
//! rather than in `default/*.loft` is registered here via the `FUNCTIONS` table.
//!
//! ## Naming conventions for entries in `FUNCTIONS`
//!
//! * `n_<name>`                — global function (no receiver).
//!   Example: `n_assert`, `n_rand`, `n_env_variables`.
//! * `t_<LEN><Type>_<method>` — method on a built-in type, where `<LEN>` is the
//!   number of characters in `<Type>`.
//!   Example: `t_4text_starts_with` (type `text`, 4 chars), `t_9character_is_numeric` (type `character`, 9 chars).
//!
//! The `<LEN><Type>` prefix lets the runtime dispatch table look up methods by
//! type name without a hash map lookup.
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(non_snake_case)]
use crate::database::Stores;
use crate::keys::{DbRef, Str};
use crate::logger::Severity;
use crate::ops;
use crate::parallel::{WorkerProgram, run_parallel_int, run_parallel_raw};
use crate::state::{Call, State};
use crate::vector;
use std::sync::Arc;
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
    ("t_4text_to_lowercase", t_4text_to_lowercase),
    ("t_4text_to_uppercase", t_4text_to_uppercase),
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
    ("n_parallel_get_int", n_parallel_get_int),
    ("n_parallel_get_long", n_parallel_get_long),
    ("n_parallel_get_float", n_parallel_get_float),
    ("n_parallel_get_bool", n_parallel_get_bool),
    ("n_rand", n_rand),
    ("n_rand_seed", n_rand_seed),
    ("n_rand_indices", n_rand_indices),
    ("n_now", n_now),
    ("n_ticks", n_ticks),
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
    let s = stores
        .scratch
        .last()
        .map(|s| Str {
            ptr: s.as_ptr(),
            len: s.len() as u32,
        })
        .unwrap();
    stores.put(stack, s);
}

fn t_4text_to_lowercase(stores: &mut Stores, stack: &mut DbRef) {
    let v_self = *stores.get::<Str>(stack);
    let new_value = v_self.str().to_lowercase();
    stores.scratch.push(new_value);
    let s = stores
        .scratch
        .last()
        .map(|s| Str {
            ptr: s.as_ptr(),
            len: s.len() as u32,
        })
        .unwrap();
    stores.put(stack, s);
}

fn t_4text_to_uppercase(stores: &mut Stores, stack: &mut DbRef) {
    let v_self = *stores.get::<Str>(stack);
    let new_value = v_self.str().to_uppercase();
    stores.scratch.push(new_value);
    let s = stores
        .scratch
        .last()
        .map(|s| Str {
            ptr: s.as_ptr(),
            len: s.len() as u32,
        })
        .unwrap();
    stores.put(stack, s);
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

/// Compiler-checked parallel map: `parallel_for(fn worker, input_vec, threads)`.
// (n_parallel_for_int section ends here)
///
/// The parser rewrites the user-friendly call into this internal 5-arg signature:
/// ```loft
/// fn parallel_for(input: reference, element_size: integer, return_size: integer,
///                 threads: integer, func: integer) -> reference
/// ```
/// `func` is the definition number of the worker function (verified at compile time).
/// `return_size` is 1 (boolean), 4 (integer), or 8 (long/float).
/// Returns a `reference` pointing to a freshly allocated result vector.
fn n_parallel_for(stores: &mut Stores, stack: &mut DbRef) {
    // Pop in reverse declaration order.
    let v_func = *stores.get::<i32>(stack);
    let v_threads = *stores.get::<i32>(stack);
    let v_return_size = *stores.get::<i32>(stack);
    let v_element_size = *stores.get::<i32>(stack);
    let v_input = *stores.get::<DbRef>(stack);

    let (fn_pos, program) = {
        let ctx = stores
            .parallel_ctx
            .as_ref()
            .expect("parallel_for called outside State::execute()");
        let data = unsafe { &*ctx.data };
        assert!(
            v_func >= 0,
            "parallel_for: invalid function reference {v_func}"
        );
        let fn_pos = data.def(v_func as u32).code_position;
        let bytecode = unsafe { Arc::clone(&*ctx.bytecode) };
        let text_code = unsafe { Arc::clone(&*ctx.text_code) };
        let library = unsafe { Arc::clone(&*ctx.library) };
        (
            fn_pos,
            WorkerProgram {
                bytecode,
                text_code,
                library,
            },
        )
    };

    let element_size = v_element_size as u32;
    let return_size = v_return_size.clamp(1, 8) as u32;
    let n_threads = (v_threads as usize).max(1);
    let n = vector::length_vector(&v_input, &stores.allocations) as usize;

    let results = run_parallel_raw(
        stores,
        program,
        fn_pos,
        &v_input,
        element_size,
        return_size,
        n_threads,
    );

    // Build result vector in a fresh store.
    let result_db = stores.null();
    let bytes_per_element = return_size;
    let vec_words = ((n as u32) * bytes_per_element + 15) / 8;
    let vec_words = vec_words.max(1);
    let vec_cr = stores.claim(&result_db, vec_words);
    let vec_rec = vec_cr.rec;
    let header_cr = stores.claim(&result_db, 1);
    let header_rec = header_cr.rec;

    {
        let store = stores.store_mut(&result_db);
        store.set_int(vec_rec, 4, n as i32);
        let mut fld = 8u32;
        for &raw in &results {
            match bytes_per_element {
                8 => {
                    store.set_long(vec_rec, fld, raw as i64);
                }
                1 => {
                    store.set_byte(vec_rec, fld, 0, raw as i32);
                }
                _ => {
                    store.set_int(vec_rec, fld, raw as i32);
                }
            }
            fld += bytes_per_element;
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
    stores.put(stack, ops::rand_int(v_lo, v_hi));
}

/// Seed the thread-local PCG RNG so subsequent `rand()` calls are reproducible.
fn n_rand_seed(stores: &mut Stores, stack: &mut DbRef) {
    let v_seed = *stores.get::<i64>(stack);
    ops::rand_seed(v_seed);
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

    // Build the shuffled index list in a temporary Rust Vec.
    let mut indices: Vec<i32> = (0..n as i32).collect();
    ops::shuffle_ints(&mut indices);

    // Allocate a new store to hold the result vector.
    let result_db = stores.null();
    let vec_words = (n as u32 * 4 + 15) / 8;
    let vec_words = vec_words.max(1);
    let vec_cr = stores.claim(&result_db, vec_words);
    let vec_rec = vec_cr.rec;
    let header_cr = stores.claim(&result_db, 1);
    let header_rec = header_cr.rec;

    {
        let store = stores.store_mut(&result_db);
        store.set_int(vec_rec, 4, n as i32);
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

/// Return milliseconds since the Unix epoch (1970-01-01T00:00:00 UTC).
/// Returns `i64::MIN` (null) if the system clock reports a time before the epoch.
fn n_now(stores: &mut Stores, stack: &mut DbRef) {
    let millis = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(i64::MIN, |d| d.as_millis() as i64);
    stores.put(stack, millis);
}

/// Return microseconds elapsed since program start (monotonic clock).
/// Use for frame timing and benchmarks; unaffected by wall-clock adjustments.
fn n_ticks(stores: &mut Stores, stack: &mut DbRef) {
    let micros = stores.start_time.elapsed().as_micros() as i64;
    stores.put(stack, micros);
}
