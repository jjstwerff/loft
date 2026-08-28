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

/// Whether applying a [`Fix`] needs a human to affirm something the compiler cannot know.
///
/// @PLN131 — the tiers gate **who may affirm the condition**, not whether a fix is
/// clickable. A conditional fix is still one click for a veteran: *"`src` is used again at
/// line 12 — if you do not need that, this becomes a move"* is something they judge
/// instantly about their own code, and clicking asserts it. What is forbidden is applying
/// one with nobody reading the condition.
///
/// |  | interactive (one click) | unattended (batch, CI) |
/// |---|---|---|
/// | `Mechanical` | yes | yes |
/// | `Conditional` | yes — the click IS the affirmation | **never** |
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixKind {
    /// The rewrite's meaning is determined by the code alone. Safe unattended.
    Mechanical,
    /// Correct only if `condition` holds, which only the author can decide.
    Conditional,
}

/// A rewrite the compiler can PLACE: replace `len` bytes at `line`:`col` with `text`.
///
/// @PLN131 steps 3–4 — an edit without a span is not applicable, only readable. The
/// diagnostic's own position cannot stand in for one: by detection time the lexer has often
/// drifted past the statement terminator (the same drift `diagnostic_at!` exists for), so a
/// cast reported at column 33 ends at column 31. A site that cannot state where its rewrite
/// goes must leave `edit` as `None` — a fix may only spell an edit it can also place, and
/// "drop the `#superseded` attribute" is the standing example of one that knows the rewrite
/// and not the span.
///
/// `len == 0` is an INSERTION at `col` (how `as τ` becomes `as τ?`); the columns are
/// 1-based, matching [`DiagEntry`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub line: u32,
    pub col: u32,
    pub len: u32,
    pub text: String,
}

/// What to write instead — the deliverable half of a diagnostic (@PLN131).
///
/// A diagnostic says what is wrong; this says what to write instead; the linked feature
/// says why. Three homes, no repetition — a fix that re-explains the problem is duplication
/// the reader pays for every time, and one that explains the concept inline has taken the
/// documentation's job.
///
/// `concept` is a **handle, not an explanation**: `move` is the searchable noun that opens
/// the door. `concept_ref` names the catalogue entry behind it, so the door leads somewhere
/// real — a door onto nothing is worse than no door.
#[derive(Debug, Clone)]
pub struct Fix {
    pub kind: FixKind,
    /// The imperative, standing alone: "build it in place", "drop the later use of `src`".
    pub title: String,
    /// What the author affirms by applying it. Required for `Conditional` (a conditional
    /// fix with no condition is malformed); `None` for `Mechanical`.
    pub condition: Option<String>,
    /// The concrete rewrite, when the compiler can spell AND place one. `None` when the fix
    /// is a deletion the author must place, when the shape admits no mechanical rewrite (the
    /// append shape has no "build it in place"), or when the span is unknown — see [`Edit`].
    /// Assuming every diagnostic offers one is how a suggestions feature ships a fix that
    /// does not exist.
    pub edit: Option<Edit>,
    /// The capability this uses — the searchable noun.
    pub concept: &'static str,
    /// The catalogue entry `concept` opens onto, e.g. `@F106`.
    pub concept_ref: &'static str,
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
    /// @PLN131 — what to write instead. Ranked most-teaching first: between "build the
    /// value in place" and "drop the later use", the first introduces an idiom reusable
    /// everywhere and the second is a local deletion, so rank on what a fix opens up rather
    /// than on how short it is. Empty when the compiler knows of no sound rewrite — which
    /// is the honest answer, and better than one whose condition the author can see is
    /// false. Shown by `--explain`; the LSP renders the same rows as code actions.
    pub fixes: Vec<Fix>,
}

impl DiagEntry {
    /// Encode the DISPLAY fields as one escaped line, for the whole-program cache manifest.
    ///
    /// A warm bundle load skips the parser, and diagnostics are a parser product — so
    /// without carrying them a cached run is SILENT where the cold run warned, and the same
    /// program run twice reports differently.  That is not a caching detail: a `warning`
    /// gates a library's CI (`LOFT_DENY_WARNINGS`), so the verdict would depend on whether
    /// anyone had run the build before.
    ///
    /// The STRUCTURED fields travel, not the rendered text, so a warm run re-renders through
    /// the same path a cold one does — `LOFT_ERRORS=pretty|compact`, colour and the
    /// warnings-off filter are all read at replay time and keep working.
    ///
    /// `fixes` is deliberately NOT carried: it feeds `loft fix`, which is its own subcommand
    /// and re-parses, and `--explain` forces a cold parse for the same reason.  Everything a
    /// normal run PRINTS is here.
    #[must_use]
    pub fn encode_for_cache(&self) -> String {
        fn esc(s: &str) -> String {
            s.replace('\\', "\\\\")
                .replace('\n', "\\n")
                .replace('\t', "\\t")
        }
        let lvl = match self.level {
            Level::Debug => "D",
            Level::Advice => "A",
            Level::Warning => "W",
            Level::Error => "E",
            Level::Fatal => "F",
        };
        format!(
            "{lvl}\t{}\t{}\t{}\t{}\t{}\t{}",
            self.line,
            self.col,
            esc(self.code.unwrap_or("")),
            esc(&self.file),
            esc(self.suggestion.as_deref().unwrap_or("")),
            esc(&self.message),
        )
    }

    /// Rebuild an entry from [`encode_for_cache`].  `None` on any malformed line, which the
    /// caller treats as a cache MISS rather than as an absent diagnostic — a bundle that
    /// cannot reproduce what the parse said must not be served.
    #[must_use]
    pub fn decode_from_cache(line: &str) -> Option<Self> {
        fn unesc(s: &str) -> String {
            let mut out = String::with_capacity(s.len());
            let mut it = s.chars();
            while let Some(c) = it.next() {
                if c != '\\' {
                    out.push(c);
                    continue;
                }
                match it.next() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some(other) => out.push(other),
                    None => {}
                }
            }
            out
        }
        let mut f = line.split('\t');
        let level = match f.next()? {
            "D" => Level::Debug,
            "A" => Level::Advice,
            "W" => Level::Warning,
            "E" => Level::Error,
            "F" => Level::Fatal,
            _ => return None,
        };
        let line_no = f.next()?.parse::<u32>().ok()?;
        let col = f.next()?.parse::<u32>().ok()?;
        let code_s = unesc(f.next()?);
        let file = unesc(f.next()?);
        let sugg = unesc(f.next()?);
        let message = unesc(f.next()?);
        Some(Self {
            level,
            message,
            file,
            line: line_no,
            col,
            // `code` is `&'static str` because every producing site is a literal.  A decoded
            // one has to become static somehow; leaking is bounded and correct here — the
            // codes are a small frozen set and a warm load decodes each at most once per
            // process, so this cannot grow with runtime.
            code: (!code_s.is_empty()).then(|| &*Box::leak(code_s.into_boxed_str())),
            suggestion: (!sugg.is_empty()).then_some(sugg),
            fixes: Vec::new(),
        })
    }

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

/// The level a [`to_string_compact`](DiagEntry::to_string_compact) line reports — tolerating
/// the `[code]` tag, which is the whole reason this exists.
///
/// Seven places classified diagnostics by writing `line.starts_with("Advice:")` themselves,
/// and a coded diagnostic renders `Advice[superseded-call]:` — matching none of them. The
/// effect was not a mislabel but a REVERSAL: each of those sites treats "not a warning" as
/// "an error", so giving a diagnostic its stable identity turned it into a build failure in
/// the test runner, the wrap harness and both fuzz oracles at once. @PLN131 asks for that
/// identity 35 more times, so the classifier lives next to the renderer that produces the
/// string, where the two cannot drift.
#[must_use]
pub fn compact_level(line: &str) -> Option<Level> {
    for (name, level) in [
        ("Fatal", Level::Fatal),
        ("Error", Level::Error),
        ("Warning", Level::Warning),
        ("Advice", Level::Advice),
        ("Debug", Level::Debug),
    ] {
        if let Some(rest) = line.strip_prefix(name)
            && (rest.starts_with(':') || (rest.starts_with('[') && rest.contains("]:")))
        {
            return Some(level);
        }
    }
    None
}

/// The same line with its `[code]` tag removed, for comparing against prose written before
/// the code existed (`@EXPECT_WARNING` text, goldens). Returns `line` unchanged when there
/// is no tag.
#[must_use]
pub fn strip_compact_code(line: &str) -> String {
    for name in ["Fatal", "Error", "Warning", "Advice", "Debug"] {
        if let Some(rest) = line.strip_prefix(name)
            && rest.starts_with('[')
            && let Some(close) = rest.find("]:")
        {
            return format!("{name}{}", &rest[close + 1..]);
        }
    }
    line.to_string()
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
            fixes: Vec::new(),
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
            fixes: Vec::new(),
        });
        if level > self.level {
            self.level = level;
        }
    }

    /// Re-add an entry decoded from the whole-program cache, preserving the level
    /// bookkeeping `add_at_coded` does — so a warm run's exit code and errors-only
    /// filtering behave exactly as the cold run's did.
    ///
    /// Separate from `add_at_coded` because this is a REPLAY, not a new finding: the
    /// entry is already complete (suggestion included) and must not be re-derived.
    pub fn restore_from_cache(&mut self, entry: DiagEntry) {
        if entry.level > self.level {
            self.level = entry.level;
        }
        self.entries.push(entry);
    }

    /// Attach a machine-readable `suggestion` (a replacement token) to the
    /// most-recently-added diagnostic — call right after emitting a "did you
    /// mean 'X'?" so `codeAction` can offer the fix without parsing prose.
    pub fn suggest_last(&mut self, suggestion: &str) {
        if let Some(last) = self.entries.last_mut() {
            last.suggestion = Some(suggestion.to_string());
        }
    }

    /// @PLN131 — attach "what to write instead" to the most-recently-added diagnostic.
    ///
    /// A `Conditional` fix carrying no condition is dropped rather than shown: the
    /// condition is the thing a clicking author affirms, so one that cannot state it is
    /// malformed, and showing it would let a click affirm nothing.
    pub fn fix_last(&mut self, fix: Fix) {
        if fix.kind == FixKind::Conditional && fix.condition.is_none() {
            debug_assert!(false, "a conditional fix must state its condition");
            return;
        }
        if let Some(last) = self.entries.last_mut() {
            last.fixes.push(fix);
        }
    }

    /// The index of the entry [`Self::fix_last`] would attach to, for a fix whose EDIT is
    /// only spellable later (loft#1003).
    ///
    /// `redundant-coalesce` is the shape: the notice fires when the `??` is recognised,
    /// but the span to delete runs to the end of a default that has not been parsed yet.
    /// Holding the index lets the edit be attached once the end is known, without
    /// reordering the diagnostic — `fix_last` cannot, because whatever the default's own
    /// parse reported would be the last entry by then.
    #[must_use]
    pub fn last_index(&self) -> Option<usize> {
        self.entries.len().checked_sub(1)
    }

    /// Give the fix at `(entry, fix_at)` the edit its emit site could not yet spell.
    ///
    /// A no-op when the index no longer names that fix — a diagnostic dropped between the
    /// notice and the edit costs the edit, never a wrong one written at a stale span.
    pub fn set_fix_edit(&mut self, entry: usize, fix_at: usize, edit: Edit) {
        if let Some(f) = self
            .entries
            .get_mut(entry)
            .and_then(|e| e.fixes.get_mut(fix_at))
        {
            f.edit = Some(edit);
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

/// Whether the parser is on its FIRST pass, mirrored out of `Parser::first_pass` so a
/// diagnostic can know which pass emitted it without threading the parser through.
pub static IN_FIRST_PASS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Keep [`IN_FIRST_PASS`] in step with `Parser::first_pass`.  Called beside every write to
/// that field; a missed one under-reports, which is the safe direction for an audit.
pub fn set_first_pass(v: bool) {
    IN_FIRST_PASS.store(v, std::sync::atomic::Ordering::Relaxed);
}

/// `LOFT_AUDIT_PASS1=1` — report which diagnostic SITES can fire while the parser is still
/// on pass 1.
///
/// Not a user diagnostic: the audience is this repo. It answers one question, about the
/// compiler rather than the program — *which refusals are reachable before types are
/// resolved?* A refusal phrased as a type REQUIREMENT that fires on pass 1 may be refusing
/// an UNRESOLVED type as a WRONG one, which makes declaration order decide whether a
/// program compiles. Five sites of that class have been found and fixed (`call_op`,
/// `parse_match`'s `!valid_enum` exit, both text-index bounds, and a spatial slice's
/// limit); the fourth pair retro-broke a published library, and the fifth was found by
/// enumerating refusals rather than by writing probes.
///
/// **Reading the output.** A site that PRINTS is reachable on pass 1 — that is a fact, and
/// it is what this instrument is for. Silence is NOT the converse: it means only that no
/// program in the run reached that site on pass 1, which is indistinguishable from "never
/// reached at all". Pair a silent site with a probe that reaches its diagnostic on pass 2
/// before recording it as gated.
///
/// Firing on pass 1 is not by itself a defect — a name collision is correctly reported
/// there, and a genuinely wrong type (`s[true]`) is refused there too, since the deferrals
/// cover only `unknown`. Measured over the 811-script corpus, 34 sites fire and none was a
/// new defect: it is a candidate list to read, not a verdict.
///
/// The reporting asymmetry is deliberate. [`set_first_pass`] is called beside every write
/// to `Parser::first_pass`, so a write this misses can only make it report FEWER sites,
/// never a phantom — which is what makes a printed site safe to act on and silence merely
/// inferred.
#[must_use]
pub fn audit_pass1_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOFT_AUDIT_PASS1").is_some())
}

/// Record a diagnostic emission site when the pass-1 audit is armed.  Cheap and inert
/// otherwise: one cached bool, then one relaxed atomic load.
pub fn audit_site(loc: &'static std::panic::Location<'static>) {
    if !audit_pass1_enabled() || !IN_FIRST_PASS.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    eprintln!("[pass1-site] {}:{}", loc.file(), loc.line());
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
    // Coded form, mirroring `diagnostic!`.  This arm must precede the uncoded one.
    ($lexer:expr, $pos:expr, $level:expr, code = $code:expr, $($arg:tt)+) => (
        $lexer.pos_diagnostic_coded($level.clone(), $pos, $code, &diagnostic_format($level, format_args!($($arg)+)))
    );
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
