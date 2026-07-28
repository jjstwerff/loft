// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I75 — Diagnostics collector

use std::fmt::{Arguments, Debug, Display, Formatter};

#[derive(PartialOrd, Ord, PartialEq, Eq, Debug, Clone, Copy)]
pub enum Level {
    Debug,
    /// Advice — the code is CORRECT as written; this reports a deprecation, a cost,
    /// or a preferred spelling.  Deliberately below `Warning` in the ordering, and
    /// deliberately WITHOUT a deny switch.
    ///
    /// The split exists because one tier made the compatibility doctrine
    /// self-contradictory.  `revalidate-libs.yml` states that a new deprecation must
    /// not fail an already-shipped library, yet a library's own CI runs
    /// `LOFT_DENY_WARNINGS=1`, which fails on any warning — so `not null`, a
    /// deliberate no-op kept parseable so unrepublished libraries keep loading, made
    /// those libraries unable to pass their own CI without editing untouched code.
    ///
    /// The rule for choosing: **a diagnostic gates CI if and only if ignoring it can
    /// produce a wrong result.** Lost writes, byte/char index confusion and
    /// null-into-non-null gate; deprecations and perf notes advise.  Never add a
    /// `LOFT_DENY_ADVICE` — the moment advice can gate, cosmetics block a release and
    /// the split has bought nothing.
    Advice,
    Warning,
    Error,
    Fatal,
}

/// One diagnostic message with optional source location.
#[derive(Debug, Clone)]
pub struct DiagEntry {
    pub level: Level,
    pub message: String,
    pub file: String,
    pub line: u32,
    pub col: u32,
    /// Stable identity of the diagnostic — a kebab-case kind slug, e.g.
    /// `text-parse-may-fail`.  @PLN102 arc-E E1: the diagnostic's `code`
    /// (not its `message` prose) is the contractual, frozen-at-1.0 handle —
    /// prose stays freely improvable; a tool keys on the code.  `None` for
    /// sites not yet assigned a code (their prose is still the only handle
    /// until one is — assignment is additive, never a breaking change).
    pub code: Option<&'static str>,
    /// A machine-readable REPLACEMENT for the offending token when the
    /// diagnostic already knows one (a "did you mean 'X'?" — the same `X` a tool
    /// applies as a quick-fix).  The prose still carries it for humans; this is
    /// its structured form, so `codeAction` doesn't parse the message.  Set via
    /// [`Diagnostics::suggest_last`] right after the diagnostic is emitted.
    pub suggestion: Option<String>,
}

impl DiagEntry {
    /// Format as a single-line string: `Level: message at file:line:col`
    #[must_use]
    pub fn to_string_compact(&self) -> String {
        // @PLN102 arc-E E1 — `[code]` after the level names the precise,
        // frozen-identity diagnostic (rustc's `error[E0308]` shape); absent
        // when the site has no code yet.
        let tag = self.code.map_or(String::new(), |c| format!("[{c}]"));
        if self.file.is_empty() {
            format!("{:?}{tag}: {}", self.level, self.message)
        } else {
            format!(
                "{:?}{tag}: {} at {}:{}:{}",
                self.level, self.message, self.file, self.line, self.col
            )
        }
    }
}

pub struct Diagnostics {
    entries: Vec<DiagEntry>,
    level: Level,
}

impl Default for Diagnostics {
    fn default() -> Self {
        Self::new()
    }
}

impl Debug for Diagnostics {
    fn fmt(&self, fmt: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        let lines: Vec<String> = self
            .entries
            .iter()
            .map(DiagEntry::to_string_compact)
            .collect();
        fmt.write_str(&format!("{lines:?}"))
    }
}

impl Display for Diagnostics {
    fn fmt(&self, fmt: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        for (i, entry) in self.entries.iter().enumerate() {
            if i > 0 {
                fmt.write_str("\n")?;
            }
            fmt.write_str(&entry.to_string_compact())?;
        }
        Ok(())
    }
}

impl Diagnostics {
    /// T0.2 — rewrite line numbers for `file` through a desugar line map, so a
    /// beginner script's diagnostics are reported in the USER's coordinates
    /// rather than the generated source's.
    ///
    /// `map[i]` is the original line that generated line `i + 1` came from (see
    /// `script::script_desugar_mapped`).  A line past the map is left alone —
    /// a wrong-but-unchanged number beats an out-of-range one, and the snippet
    /// lookup then simply finds nothing rather than the wrong text.
    pub fn remap_lines(&mut self, file: &str, map: &[u32]) {
        for e in &mut self.entries {
            if e.file != file {
                continue;
            }
            if let Some(&orig) = map.get(e.line.saturating_sub(1) as usize) {
                e.line = orig;
            }
        }
    }

    #[must_use]
    pub fn new() -> Diagnostics {
        Diagnostics {
            entries: Vec::new(),
            level: Level::Debug,
        }
    }

    pub fn add(&mut self, level: Level, message: &str) {
        self.entries.push(DiagEntry {
            level,
            message: message.to_string(),
            file: String::new(),
            line: 0,
            col: 0,
            code: None,
            suggestion: None,
        });
        if level > self.level {
            self.level = level;
        }
    }

    pub fn add_at(&mut self, level: Level, message: &str, file: &str, line: u32, col: u32) {
        self.add_at_coded(level, None, message, file, line, col);
    }

    /// Like [`add_at`], but carries a stable `code` (kebab-case kind slug).
    /// @PLN102 arc-E E1 — the code is the frozen identity; prose is free.
    pub fn add_at_coded(
        &mut self,
        level: Level,
        code: Option<&'static str>,
        message: &str,
        file: &str,
        line: u32,
        col: u32,
    ) {
        self.entries.push(DiagEntry {
            level,
            message: message.to_string(),
            file: file.to_string(),
            line,
            col,
            code,
            suggestion: None,
        });
        if level > self.level {
            self.level = level;
        }
    }

    /// Attach a machine-readable `suggestion` (a replacement token) to the
    /// most-recently-added diagnostic — call right after emitting a "did you
    /// mean 'X'?" so `codeAction` can offer the fix without parsing prose.
    pub fn suggest_last(&mut self, suggestion: &str) {
        if let Some(last) = self.entries.last_mut() {
            last.suggestion = Some(suggestion.to_string());
        }
    }

    pub fn fill(&mut self, other: &Diagnostics) {
        for e in &other.entries {
            self.entries.push(e.clone());
        }
        if other.level > self.level {
            self.level = other.level;
        }
    }

    /// Backward-compatible: return each entry as a formatted string.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        self.entries
            .iter()
            .map(DiagEntry::to_string_compact)
            .collect()
    }

    #[must_use]
    pub fn entries(&self) -> &[DiagEntry] {
        &self.entries
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub fn level(&self) -> Level {
        self.level
    }
}

#[must_use]
pub fn diagnostic_format(_level: Level, message: Arguments<'_>) -> String {
    format!("{message}")
}

#[macro_export]
macro_rules! diagnostic {
    // @PLN102 arc-E E1 — coded form: `diagnostic!(lexer, Level::Error, code =
    // "kebab-slug", "prose {x}")`.  The code is the frozen identity; prose is
    // freely improvable.  This arm must precede the uncoded one (tried in order).
    ($lexer:expr, $level:expr, code = $code:expr, $($arg:tt)+) => (
        $lexer.diagnostic_coded($level.clone(), $code, &diagnostic_format($level, format_args!($($arg)+)))
    );
    ($lexer:expr, $level:expr, $($arg:tt)+) => (
        $lexer.diagnostic($level.clone(), &diagnostic_format($level, format_args!($($arg)+)))
    )
}

#[macro_export]
macro_rules! specific {
    ($lexer:expr, $result:expr, $level:expr, $($arg:tt)+) => (
        $lexer.specific($result, $level.clone(), &diagnostic_format($level, format_args!($($arg)+)))
    )
}

/// Emit a diagnostic at an explicit `Position` instead of the lexer's
/// current cursor.  Expressions, call arguments, and type names are fully
/// parsed before they are type-checked, so by detection time the cursor has
/// drifted to the statement terminator — capture the offending node's start
/// position at parse time and point the caret there.
#[macro_export]
macro_rules! diagnostic_at {
    ($lexer:expr, $pos:expr, $level:expr, $($arg:tt)+) => (
        $lexer.pos_diagnostic($level.clone(), $pos, &diagnostic_format($level, format_args!($($arg)+)))
    )
}

/// Levenshtein edit distance between two strings.
#[must_use]
pub fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        curr[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            curr[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(curr[j] + 1);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

/// Find the closest match to `name` among `candidates` (Levenshtein
/// distance ≤ 2).
#[must_use]
pub fn suggest_similar<'a>(name: &str, candidates: &[&'a str]) -> Option<&'a str> {
    candidates
        .iter()
        .copied()
        .filter(|c| {
            let d = levenshtein(name, c);
            d > 0 && d <= 2
        })
        .min_by_key(|c| levenshtein(name, c))
}

/// Plan-07 phase 5: suggestion with a short-name guard.
/// Names of 1–3 chars never suggest — too short to typo-match without
/// noise (generic placeholders `T` / `K` / `V`, coin-flip pairs like
/// `id` / `ok`).  4+ chars get the full Levenshtein-2 ceiling so common
/// single-char typos AND transpositions (`naem`→`name`, `Bleu`→`Blue`,
/// `reuslt`→`result`) are caught — the same distance the uncapped
/// variable-suggestion path uses.  Empty `name` never suggests.
///
/// Distance bounds by name length:
/// - 1–3 chars → 0 (no suggestion).
/// - 4+ chars → 2 (single-char typo or transposition).
///
/// The earlier `min(2, name_chars / 4)` cap only reached distance 2 at
/// 8+ chars, so it silently dropped every 4–7-char transposition — the
/// single most common real typo.  `parser/objects.rs::known_var_or_type`
/// had already worked around this by using the uncapped `suggest_similar`
/// for variables; this brings the field / type / function sites in line.
#[must_use]
pub fn suggest_similar_capped<'a>(name: &str, candidates: &[&'a str]) -> Option<&'a str> {
    if name.chars().count() <= 3 {
        return None;
    }
    let max_dist = 2;
    candidates
        .iter()
        .copied()
        .filter(|c| {
            let d = levenshtein(name, c);
            d > 0 && d <= max_dist
        })
        .min_by_key(|c| levenshtein(name, c))
}
