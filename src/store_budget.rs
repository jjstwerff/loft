// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! A memory ceiling for the store heap, and the report that says what filled it.
//!
//! A corrupted length or count does not always end in a bad dereference. Often it
//! ends in an allocation: the same fault that SIGSEGVs on one run drives a store to
//! grow without bound on the next. A time bound cannot catch that — a runaway store
//! reaches tens of gigabytes in seconds — and the kernel's OOM killer is worse than
//! useless as a diagnostic, because it reports only that a process died and is free
//! to kill a bystander instead of the culprit.
//!
//! So the ceiling lives here, inside loft, where the thing being allocated still has
//! a name. When a growth would cross it the run stops at that growth and says which
//! type was growing, how big it had already become, where the program was, and how
//! the rest of the heap was distributed. That last part is what turns "out of memory"
//! into a lead: one type holding 4 GiB in ONE store is a runaway length, and the same
//! 4 GiB spread over a million stores is a leak.
//!
//! **Off unless asked.** loft is unbounded by default and a real program may want the
//! whole machine, so a plain run is never capped. Test runs are, because a test that
//! wants tens of gigabytes is a bug either way, and because taking the developer's
//! machine down with it costs far more than the run.
//!
//! # The invariant
//!
//! [`total`] is the sum of `size * 8` over every store that OWNS its buffer. Stores
//! that borrow another's buffer, and file-backed stores whose bytes live in a
//! mapping, own no heap bytes and are not counted. The invariant is re-asserted at
//! every site in `store.rs` that allocates, reallocates or frees such a buffer —
//! `new`, `open`, `resize_store`, `shrink_to`, `clone_locked`, `snapshot_copy` and
//! `Drop`. A site that forgets makes the counter drift, which shows up as a ceiling
//! that trips early rather than as silent damage.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// Live bytes across every store that owns its buffer.
static TOTAL: AtomicU64 = AtomicU64::new(0);

/// The ceiling in bytes; `0` means no ceiling, which is the default.
static LIMIT: AtomicU64 = AtomicU64::new(0);

/// The default ceiling for a test run, used when nothing else says otherwise.
/// Generous for anything a test legitimately does, and small enough that a runaway
/// stops while the machine is still comfortable.
pub const DEFAULT_TEST_LIMIT: u64 = 2 << 30; // 2 GiB

/// Live bytes and store count per store type, maintained only while a ceiling is
/// set — with no ceiling there is no report to build, so there is nothing to pay for.
static BY_TYPE: Mutex<BTreeMap<u16, (u64, u32)>> = Mutex::new(BTreeMap::new());

/// Type id → name, so the report can say `Layer` rather than `kt=112`.
/// Filled as stores are created; a type nothing ever allocated needs no name.
static NAMES: Mutex<BTreeMap<u16, String>> = Mutex::new(BTreeMap::new());

/// Set the ceiling in bytes. `0` removes it.
pub fn set_limit(bytes: u64) {
    LIMIT.store(bytes, Ordering::Relaxed);
}

/// The ceiling in bytes, or `0` when there is none.
#[must_use]
pub fn limit() -> u64 {
    LIMIT.load(Ordering::Relaxed)
}

/// Live store bytes right now.
#[must_use]
pub fn total() -> u64 {
    TOTAL.load(Ordering::Relaxed)
}

/// Parse a ceiling written the way a person writes one: `2G`, `512M`, `1048576`.
/// A bare number is bytes; `K`/`M`/`G` (upper or lower, optional `B`) scale it.
/// `0` means no ceiling. Returns `None` when the text is not a size at all.
#[must_use]
pub fn parse_limit(text: &str) -> Option<u64> {
    let t = text.trim();
    let t = t.strip_suffix(['b', 'B']).unwrap_or(t);
    let (digits, scale) = match t.chars().last()? {
        'k' | 'K' => (&t[..t.len() - 1], 1u64 << 10),
        'm' | 'M' => (&t[..t.len() - 1], 1u64 << 20),
        'g' | 'G' => (&t[..t.len() - 1], 1u64 << 30),
        _ => (t, 1),
    };
    digits.trim().parse::<u64>().ok()?.checked_mul(scale)
}

/// Read the ceiling from `LOFT_MEMORY_LIMIT`, falling back to `default`.
/// An unparseable value is reported and the default is kept — a typo in a limit
/// must not silently remove the limit.
pub fn apply_env_limit(default: u64) {
    let Ok(v) = std::env::var("LOFT_MEMORY_LIMIT") else {
        set_limit(default);
        return;
    };
    if let Some(bytes) = parse_limit(&v) {
        set_limit(bytes);
    } else {
        eprintln!(
            "loft: LOFT_MEMORY_LIMIT='{v}' is not a size (try 2G, 512M or 0) — \
             keeping the default {}",
            human(default)
        );
        set_limit(default);
    }
}

/// Record what a store type is called, so the report can name it.
pub fn note_type_name(kt: u16, name: &str) {
    if LIMIT.load(Ordering::Relaxed) == 0 || name.is_empty() {
        return;
    }
    if let Ok(mut names) = NAMES.lock() {
        names.entry(kt).or_insert_with(|| name.to_string());
    }
}

/// A store of type `kt` took `bytes` more heap.
pub(crate) fn add(kt: u16, bytes: usize) {
    TOTAL.fetch_add(bytes as u64, Ordering::Relaxed);
    if LIMIT.load(Ordering::Relaxed) == 0 {
        return;
    }
    if let Ok(mut by) = BY_TYPE.lock() {
        let e = by.entry(kt).or_insert((0, 0));
        e.0 += bytes as u64;
        e.1 += 1;
    }
}

/// A store of type `kt` gave `bytes` of heap back.
pub(crate) fn release(kt: u16, bytes: usize) {
    TOTAL.fetch_sub(bytes as u64, Ordering::Relaxed);
    if LIMIT.load(Ordering::Relaxed) == 0 {
        return;
    }
    if let Ok(mut by) = BY_TYPE.lock()
        && let Some(e) = by.get_mut(&kt)
    {
        e.0 = e.0.saturating_sub(bytes as u64);
        e.1 = e.1.saturating_sub(1);
    }
}

/// A store of type `kt`, created at bytecode position `born_at`, is about to grow
/// from `old` to `new` bytes.
///
/// # Panics
/// When the growth would cross the ceiling.  The panic carries the full report and
/// fires BEFORE the reallocation, so the store is left exactly as it was and the
/// message describes the growth that was refused rather than one already made.
pub(crate) fn grow(kt: u16, old: usize, new: usize, born_at: u32) {
    let added = new.saturating_sub(old) as u64;
    let cap = LIMIT.load(Ordering::Relaxed);
    assert!(
        cap == 0 || TOTAL.load(Ordering::Relaxed) + added <= cap,
        "{}",
        refusal(kt, old, new, cap, born_at)
    );
    TOTAL.fetch_add(added, Ordering::Relaxed);
    if cap == 0 {
        return;
    }
    if let Ok(mut by) = BY_TYPE.lock() {
        by.entry(kt).or_insert((0, 0)).0 += added;
    }
}

/// A store of type `kt` shrank from `old` to `new` bytes.
pub(crate) fn shrink(kt: u16, old: usize, new: usize) {
    let freed = old.saturating_sub(new) as u64;
    TOTAL.fetch_sub(freed, Ordering::Relaxed);
    if LIMIT.load(Ordering::Relaxed) == 0 {
        return;
    }
    if let Ok(mut by) = BY_TYPE.lock()
        && let Some(e) = by.get_mut(&kt)
    {
        e.0 = e.0.saturating_sub(freed);
    }
}

/// `4.9 GiB`, `180.0 MiB`, `512 B` — a size a person can compare at a glance.
#[must_use]
pub fn human(bytes: u64) -> String {
    const UNITS: [(u64, &str); 3] = [(1 << 30, "GiB"), (1 << 20, "MiB"), (1 << 10, "KiB")];
    for (scale, name) in UNITS {
        if bytes >= scale {
            #[allow(clippy::cast_precision_loss)] // display only
            return format!("{:.1} {name}", bytes as f64 / scale as f64);
        }
    }
    format!("{bytes} B")
}

fn name_of(kt: u16) -> String {
    NAMES
        .lock()
        .ok()
        .and_then(|n| n.get(&kt).cloned())
        .unwrap_or_else(|| "?".to_string())
}

/// The heap by type, biggest first — the part that separates a runaway length from
/// a leak.  At most `top` rows; the rest are summarised.
#[must_use]
pub fn breakdown(top: usize) -> String {
    let Ok(by) = BY_TYPE.lock() else {
        return String::new();
    };
    let mut rows: Vec<(u16, u64, u32)> = by.iter().map(|(&k, &(b, n))| (k, b, n)).collect();
    drop(by);
    rows.sort_by_key(|&(_, b, _)| std::cmp::Reverse(b));
    let mut out = String::new();
    for &(kt, bytes, stores) in rows.iter().take(top) {
        let plural = if stores == 1 { "store" } else { "stores" };
        let _ = writeln!(
            out,
            "    {:<24} kt={:<6} {:>10}  in {stores} {plural}",
            name_of(kt),
            kt,
            human(bytes)
        );
    }
    if rows.len() > top {
        let rest: u64 = rows[top..].iter().map(|&(_, b, _)| b).sum();
        let _ = writeln!(
            out,
            "    … and {} more types holding {}",
            rows.len() - top,
            human(rest)
        );
    }
    out
}

/// The message a refused growth carries.
fn refusal(kt: u16, old: usize, new: usize, cap: u64, born_at: u32) -> String {
    let mut m = format!(
        "loft: store memory limit reached — {} in use, limit {}\n\n  \
         the growth that crossed it\n    a store of type `{}` (kt={kt}) growing {} → {}\n",
        human(total()),
        human(cap),
        name_of(kt),
        human(old as u64),
        human(new as u64),
    );
    // The bytecode position the store was allocated at, and deliberately NOT a
    // `file:line` derived from it.  The interpreter's span table records CALL SITES
    // only, so resolving an arbitrary pc through it returns the nearest span BELOW —
    // which is routinely in an unrelated function, and a diagnostic that sends the
    // reader to the wrong file costs more than one that stays quiet.  `LOFT_STORES=summary`
    // prints the same pc with a line number, resolved against the denser per-run
    // table this module cannot reach.
    if born_at != 0 {
        let _ = writeln!(m, "    the store was allocated at pc={born_at}");
    }
    let rows = breakdown(8);
    if !rows.is_empty() {
        m.push_str("\n  where the memory is\n");
        m.push_str(&rows);
        m.push_str(
            "\n  One type holding nearly all of it in ONE store is a runaway length;\n  \
             the same total spread over very many stores is a leak.\n",
        );
    }
    m.push_str("\n  Raise or remove the limit with LOFT_MEMORY_LIMIT=<size|0>, e.g. 8G.\n");
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sizes_a_person_would_write() {
        assert_eq!(parse_limit("1024"), Some(1024));
        assert_eq!(parse_limit("2G"), Some(2 << 30));
        assert_eq!(parse_limit("512M"), Some(512 << 20));
        assert_eq!(parse_limit("4k"), Some(4 << 10));
        assert_eq!(parse_limit("8GB"), Some(8 << 30));
        assert_eq!(parse_limit("0"), Some(0));
        assert_eq!(parse_limit("plenty"), None);
    }

    #[test]
    fn sizes_read_at_a_glance() {
        assert_eq!(human(512), "512 B");
        assert_eq!(human(2 << 30), "2.0 GiB");
        assert_eq!(human(180 << 20), "180.0 MiB");
    }
}
