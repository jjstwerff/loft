// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Bytecode cache — `.loftc` file format for skipping recompilation.
//!
//! See `doc/claude/CONST_STORE.md` § Bytecode Cache for the design.

#![allow(clippy::cast_possible_truncation)]

use crate::keys::DbRef;
use crate::sha256;
use std::io::{Read, Write};

/// Magic bytes at the start of every `.loftc` file.
const MAGIC: &[u8; 4] = b"LFC1";

/// Loft version baked into the cache key so a different binary invalidates the cache.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Git commit hash (or timestamp) from `build.rs`.  Ensures that a rebuild of
/// the interpreter (e.g. a parser fix without a version bump) invalidates all
/// cached `.loftc` files.
const BUILD_ID: &str = env!("LOFT_BUILD_ID");

/// Compute the cache key: SHA-256(version + build_id + source contents).
/// `sources` is a list of (filename, content) pairs.
#[must_use] 
pub fn cache_key(sources: &[(&str, &str)]) -> [u8; 32] {
    let mut buf = Vec::new();
    buf.extend_from_slice(VERSION.as_bytes());
    buf.push(0);
    buf.extend_from_slice(BUILD_ID.as_bytes());
    buf.push(0);
    for (name, content) in sources {
        buf.extend_from_slice(name.as_bytes());
        buf.push(0);
        buf.extend_from_slice(content.as_bytes());
        buf.push(0);
    }
    sha256::sha256(&buf)
}

/// Cached compilation output — everything needed to skip `byte_code()`.
pub struct CacheData {
    pub bytecode: Vec<u8>,
    pub const_store_buf: Vec<u8>,
    pub vector_stores: Vec<VectorStoreEntry>,
    pub const_refs: Vec<ConstRefEntry>,
    pub functions: Vec<FunctionEntry>,
}

/// A pre-built vector constant store.
pub struct VectorStoreEntry {
    pub store_nr: u16,
    pub data: Vec<u8>,
}

/// A const_ref mapping: definition number → DbRef.
pub struct ConstRefEntry {
    pub d_nr: u32,
    pub store_nr: u16,
    pub rec: u32,
    pub pos: u32,
}

/// Per-function bytecode position.
pub struct FunctionEntry {
    pub d_nr: u32,
    pub code_position: u32,
    pub code_length: u32,
}

/// Write a cache file.
/// # Errors
/// Returns `Err` if the file cannot be created or written.
pub fn write_cache(path: &str, key: &[u8; 32], data: &CacheData) -> std::io::Result<()> {
    let mut f = std::fs::File::create(path)?;
    f.write_all(MAGIC)?;
    f.write_all(key)?;

    // Header counts
    write_u32(&mut f, data.bytecode.len() as u32)?;
    write_u32(&mut f, (data.const_store_buf.len() / 8) as u32)?;
    write_u32(&mut f, data.vector_stores.len() as u32)?;
    write_u32(&mut f, data.functions.len() as u32)?;

    // Bytecode
    f.write_all(&data.bytecode)?;

    // CONST_STORE raw buffer
    f.write_all(&data.const_store_buf)?;

    // Vector stores
    for vs in &data.vector_stores {
        write_u16(&mut f, vs.store_nr)?;
        write_u32(&mut f, (vs.data.len() / 8) as u32)?;
        f.write_all(&vs.data)?;
    }

    // Const refs
    write_u32(&mut f, data.const_refs.len() as u32)?;
    for cr in &data.const_refs {
        write_u32(&mut f, cr.d_nr)?;
        write_u16(&mut f, cr.store_nr)?;
        write_u32(&mut f, cr.rec)?;
        write_u32(&mut f, cr.pos)?;
    }

    // Functions
    for func in &data.functions {
        write_u32(&mut f, func.d_nr)?;
        write_u32(&mut f, func.code_position)?;
        write_u32(&mut f, func.code_length)?;
    }

    Ok(())
}

/// Try to read and validate a cache file. Returns None on any mismatch.
#[must_use] 
pub fn read_cache(path: &str, expected_key: &[u8; 32]) -> Option<CacheData> {
    let mut f = std::fs::File::open(path).ok()?;

    // Magic
    let mut magic = [0u8; 4];
    f.read_exact(&mut magic).ok()?;
    if &magic != MAGIC {
        return None;
    }

    // Hash
    let mut hash = [0u8; 32];
    f.read_exact(&mut hash).ok()?;
    if &hash != expected_key {
        return None;
    }

    // Header
    let bytecode_len = read_u32(&mut f)? as usize;
    let const_store_words = read_u32(&mut f)? as usize;
    let n_vector_stores = read_u32(&mut f)? as usize;
    let n_functions = read_u32(&mut f)? as usize;

    // Bytecode
    let mut bytecode = vec![0u8; bytecode_len];
    f.read_exact(&mut bytecode).ok()?;

    // CONST_STORE
    let const_store_bytes = const_store_words * 8;
    let mut const_store_buf = vec![0u8; const_store_bytes];
    f.read_exact(&mut const_store_buf).ok()?;

    // Vector stores
    let mut vector_stores = Vec::with_capacity(n_vector_stores);
    for _ in 0..n_vector_stores {
        let store_nr = read_u16(&mut f)?;
        let words = read_u32(&mut f)? as usize;
        let mut data = vec![0u8; words * 8];
        f.read_exact(&mut data).ok()?;
        vector_stores.push(VectorStoreEntry { store_nr, data });
    }

    // Const refs
    let n_const_refs = read_u32(&mut f)? as usize;
    let mut const_refs = Vec::with_capacity(n_const_refs);
    for _ in 0..n_const_refs {
        let d_nr = read_u32(&mut f)?;
        let store_nr = read_u16(&mut f)?;
        let rec = read_u32(&mut f)?;
        let pos = read_u32(&mut f)?;
        const_refs.push(ConstRefEntry {
            d_nr,
            store_nr,
            rec,
            pos,
        });
    }

    // Functions
    let mut functions = Vec::with_capacity(n_functions);
    for _ in 0..n_functions {
        let d_nr = read_u32(&mut f)?;
        let code_position = read_u32(&mut f)?;
        let code_length = read_u32(&mut f)?;
        functions.push(FunctionEntry {
            d_nr,
            code_position,
            code_length,
        });
    }

    Some(CacheData {
        bytecode,
        const_store_buf,
        vector_stores,
        const_refs,
        functions,
    })
}

/// Convert a `.loft` path to its cache path (`.loftc`).
#[must_use] 
pub fn cache_path(source_path: &str) -> String {
    let p = std::path::Path::new(source_path);
    if p.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("loft"))
    {
        format!("{source_path}c")
    } else {
        format!("{source_path}.loftc")
    }
}

/// Collect the CacheData from current State + Data after byte_code().
#[must_use] 
pub fn collect_cache_data(state: &crate::state::State, data: &crate::data::Data) -> CacheData {
    use crate::data::DefType;

    // Bytecode
    let bytecode = (*state.bytecode).clone();

    // CONST_STORE buffer (store 1)
    let cs = &state.database.allocations[crate::database::CONST_STORE as usize];
    let const_store_buf =
        unsafe { std::slice::from_raw_parts(cs.ptr, cs.capacity_words() as usize * 8).to_vec() };

    // Vector constant stores
    let mut vector_stores = Vec::new();
    let null_ref = DbRef {
        store_nr: u16::MAX,
        rec: 0,
        pos: 0,
    };
    for cr in &state.const_refs {
        if *cr == null_ref || cr.store_nr <= crate::database::CONST_STORE {
            continue;
        }
        let store = &state.database.allocations[cr.store_nr as usize];
        if store.free {
            continue;
        }
        let buf = unsafe {
            std::slice::from_raw_parts(store.ptr, store.capacity_words() as usize * 8).to_vec()
        };
        // Avoid duplicate entries for the same store_nr.
        if !vector_stores
            .iter()
            .any(|v: &VectorStoreEntry| v.store_nr == cr.store_nr)
        {
            vector_stores.push(VectorStoreEntry {
                store_nr: cr.store_nr,
                data: buf,
            });
        }
    }

    // Const refs
    let mut const_refs_out = Vec::new();
    for (i, cr) in state.const_refs.iter().enumerate() {
        if *cr != null_ref {
            const_refs_out.push(ConstRefEntry {
                d_nr: i as u32,
                store_nr: cr.store_nr,
                rec: cr.rec,
                pos: cr.pos,
            });
        }
    }

    // Functions
    let mut functions = Vec::new();
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        if def.def_type == DefType::Function && def.code_position > 0 {
            functions.push(FunctionEntry {
                d_nr,
                code_position: def.code_position,
                code_length: def.code_length,
            });
        }
    }

    CacheData {
        bytecode,
        const_store_buf,
        vector_stores,
        const_refs: const_refs_out,
        functions,
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn write_u32(w: &mut impl Write, v: u32) -> std::io::Result<()> {
    w.write_all(&v.to_le_bytes())
}

fn write_u16(w: &mut impl Write, v: u16) -> std::io::Result<()> {
    w.write_all(&v.to_le_bytes())
}

fn read_u32(r: &mut impl Read) -> Option<u32> {
    let mut buf = [0u8; 4];
    r.read_exact(&mut buf).ok()?;
    Some(u32::from_le_bytes(buf))
}

fn read_u16(r: &mut impl Read) -> Option<u16> {
    let mut buf = [0u8; 2];
    r.read_exact(&mut buf).ok()?;
    Some(u16::from_le_bytes(buf))
}
