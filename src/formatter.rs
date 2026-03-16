// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Standalone token-stream formatter for `.loft` source files.
//!
//! Works without touching the parser or lexer: it scans the raw source text
//! into a small token representation, then emits canonical output.
//!
//! Public API:
//! - [`format_source`] — format a string, return formatted string
//! - [`check_source`]  — return `true` iff source is already formatted

// ─── Token types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Word(String),    // identifier or keyword
    Int(String),     // integer literal (raw text)
    Flt(String),     // float literal (raw text)
    Str(String),     // string literal including quotes
    Chr(String),     // character literal including quotes
    Sym(String),     // punctuation / operator
    Comment(String), // // … (without leading //)
    Newline,         // a single line boundary
    Blank,           // an extra blank line (>1 consecutive newlines)
}

// ─── Scanner ──────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_lines)]
fn scan(source: &str) -> Vec<Tok> {
    let chars: Vec<char> = source.chars().collect();
    let n = chars.len();
    let mut i = 0;
    let mut toks: Vec<Tok> = Vec::new();
    let mut newlines_pending: u32 = 0;

    macro_rules! flush_nl {
        () => {
            if newlines_pending == 1 {
                toks.push(Tok::Newline);
            } else if newlines_pending > 1 {
                toks.push(Tok::Newline);
                toks.push(Tok::Blank);
            }
            newlines_pending = 0;
        };
    }

    while i < n {
        let c = chars[i];
        match c {
            '\n' => {
                newlines_pending += 1;
                i += 1;
            }
            '\r' | ' ' | '\t' => {
                i += 1;
            }
            '/' if i + 1 < n && chars[i + 1] == '/' => {
                flush_nl!();
                i += 2;
                let start = i;
                while i < n && chars[i] != '\n' {
                    i += 1;
                }
                let text = chars[start..i]
                    .iter()
                    .collect::<String>()
                    .trim_end()
                    .to_string();
                toks.push(Tok::Comment(text));
            }
            '"' => {
                flush_nl!();
                let start = i;
                i += 1;
                while i < n {
                    if chars[i] == '\\' {
                        i += 2;
                    } else if chars[i] == '"' {
                        i += 1;
                        break;
                    } else {
                        i += 1;
                    }
                }
                toks.push(Tok::Str(chars[start..i].iter().collect()));
            }
            '\'' => {
                flush_nl!();
                let start = i;
                i += 1;
                while i < n {
                    if chars[i] == '\\' {
                        i += 2;
                    } else if chars[i] == '\'' {
                        i += 1;
                        break;
                    } else {
                        i += 1;
                    }
                }
                toks.push(Tok::Chr(chars[start..i].iter().collect()));
            }
            '0'..='9' => {
                flush_nl!();
                let start = i;
                let mut is_float = false;
                if c == '0' && i + 1 < n && (chars[i + 1] == 'x' || chars[i + 1] == 'X') {
                    i += 2;
                    while i < n && (chars[i].is_ascii_hexdigit() || chars[i] == '_') {
                        i += 1;
                    }
                } else if c == '0' && i + 1 < n && (chars[i + 1] == 'b' || chars[i + 1] == 'B') {
                    i += 2;
                    while i < n && (chars[i] == '0' || chars[i] == '1' || chars[i] == '_') {
                        i += 1;
                    }
                } else {
                    while i < n && (chars[i].is_ascii_digit() || chars[i] == '_') {
                        i += 1;
                    }
                    if i < n && chars[i] == '.' && i + 1 < n && chars[i + 1].is_ascii_digit() {
                        is_float = true;
                        i += 1;
                        while i < n && (chars[i].is_ascii_digit() || chars[i] == '_') {
                            i += 1;
                        }
                    }
                    if i < n && (chars[i] == 'e' || chars[i] == 'E') {
                        is_float = true;
                        i += 1;
                        if i < n && (chars[i] == '+' || chars[i] == '-') {
                            i += 1;
                        }
                        while i < n && chars[i].is_ascii_digit() {
                            i += 1;
                        }
                    }
                }
                if i < n && (chars[i] == 'l' || chars[i] == 'L') {
                    i += 1;
                } else if i < n && chars[i] == 'f' {
                    is_float = true;
                    i += 1;
                }
                let raw: String = chars[start..i].iter().collect();
                if is_float {
                    toks.push(Tok::Flt(raw));
                } else {
                    toks.push(Tok::Int(raw));
                }
            }
            _ if c.is_alphabetic() || c == '_' => {
                flush_nl!();
                let start = i;
                while i < n && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                toks.push(Tok::Word(chars[start..i].iter().collect()));
            }
            _ => {
                flush_nl!();
                // Check 3-char symbols first, then 2-char, then 1-char.
                let three: String = chars[i..std::cmp::min(i + 3, n)].iter().collect();
                let two: String = chars[i..std::cmp::min(i + 2, n)].iter().collect();
                if three == "..=" {
                    toks.push(Tok::Sym(three));
                    i += 3;
                } else {
                    match two.as_str() {
                        "::" | "->" | "=>" | "==" | "!=" | "<=" | ">=" | "&&" | "||" | "<<"
                        | ">>" | "+=" | "-=" | "*=" | "/=" | "%=" | "??" | ".." => {
                            toks.push(Tok::Sym(two));
                            i += 2;
                        }
                        _ => {
                            toks.push(Tok::Sym(c.to_string()));
                            i += 1;
                        }
                    }
                }
            }
        }
    }
    if newlines_pending > 0 {
        toks.push(Tok::Newline);
    }
    toks
}

// ─── Formatter ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Ctx {
    Block,     // inside { } of fn/if/for/loop body
    StructDef, // inside struct/enum declaration body
    ArgList,   // inside ( ) of call or fn declaration
    ArrayLit,  // inside [ ]
    StructLit, // inside { } of struct constructor
}

/// These operators get a space before and after.
const BINARY_OPS: &[&str] = &[
    "+", "-", "*", "/", "%", "==", "!=", "<", ">", "<=", ">=", "&&", "||", "&", "|", "^", "<<",
    ">>", "??",
];

const ASSIGN_OPS: &[&str] = &["=", "+=", "-=", "*=", "/=", "%="];

/// Keywords that introduce a block and should be followed by a space.
const BLOCK_KWS: &[&str] = &["if", "else", "for", "while", "loop", "match", "fn"];

/// Keywords that get a trailing space in general.
const KW_SPACE: &[&str] = &[
    "fn", "pub", "struct", "enum", "type", "use", "if", "else", "for", "in", "return", "break",
    "continue", "loop", "as", "and", "or", "not", "let", "while", "match",
];

#[allow(clippy::struct_excessive_bools)]
struct Fmt {
    depth: usize,
    ctx: Vec<Ctx>,
    out: String,
    /// Last non-whitespace token text we emitted.
    prev: String,
    at_line_start: bool,
    /// Set after block-opening keywords (`fn`, `if`, `else`, `for`, etc.) or `->`.
    /// Cleared when the opening `{` is consumed.  Causes next `{` to open a Block.
    next_brace_is_block: bool,
    /// Set when we are inside a `struct Name` or `enum Name` header, waiting for `{`.
    after_struct_hdr: bool,
    /// True when the last closed top-level item was `}` (signals: insert blank before next item).
    last_top_closed: bool,
}

impl Fmt {
    fn new() -> Self {
        Fmt {
            depth: 0,
            ctx: Vec::new(),
            out: String::new(),
            prev: String::new(),
            at_line_start: true,
            next_brace_is_block: false,
            after_struct_hdr: false,
            last_top_closed: false,
        }
    }

    fn indent(&self) -> String {
        "  ".repeat(self.depth)
    }

    fn top_ctx(&self) -> Option<&Ctx> {
        self.ctx.last()
    }

    fn emit_nl(&mut self) {
        while self.out.ends_with(' ') {
            self.out.pop();
        }
        self.out.push('\n');
        self.at_line_start = true;
    }

    fn ensure_nl(&mut self) {
        if !self.at_line_start {
            self.emit_nl();
        }
    }

    fn emit_indent_if_needed(&mut self) {
        if self.at_line_start {
            let s = self.indent();
            self.out.push_str(&s);
            // at_line_start stays true if indent is empty (depth 0)
            if !s.is_empty() {
                self.at_line_start = false;
            }
        }
    }

    /// Decide if a space should be emitted before `tok` given `prev`.
    fn need_space(&self, tok: &str) -> bool {
        let p = self.prev.as_str();
        if self.at_line_start || p.is_empty() {
            return false;
        }
        // Never space after open bracket
        if p == "(" || p == "[" {
            return false;
        }
        // Never space before close bracket or comma or semicolon
        if tok == ")" || tok == "]" || tok == "," || tok == ";" {
            return false;
        }
        // `{` — space is handled at the call site (before `{` depends on ctx)
        if tok == "{" || tok == "}" {
            return false;
        }
        // :: and . and .. and ..= — no spaces either side
        if tok == "::"
            || p == "::"
            || tok == "."
            || p == "."
            || tok == ".."
            || p == ".."
            || tok == "..="
            || p == "..="
        {
            return false;
        }
        // After a unary operator sentinel, no space before operand
        if p == "unary" {
            return false;
        }
        // `:` type annotation — no space before
        if tok == ":" {
            return false;
        }
        // `(` — no space before (function call, control-flow)
        if tok == "(" {
            return false;
        }
        // `[` — no space before (index)
        if tok == "[" {
            return false;
        }
        // `!` unary — no space after; need space before only if prev was a word/closer
        // Space after `->` and `=>`
        if p == "->" || p == "=>" {
            return true;
        }
        // Space before `->`/`=>`
        if tok == "->" || tok == "=>" {
            return true;
        }
        // Space after binary/assign op (when next tok is not a closer handled above)
        if BINARY_OPS.contains(&p) || ASSIGN_OPS.contains(&p) {
            return true;
        }
        // Space before binary op.  Note: `-` and `+` are handled in handle_sym
        // (unary detection there sets prev="unary", skipping this path entirely).
        if BINARY_OPS.contains(&tok) {
            return true;
        }
        if ASSIGN_OPS.contains(&tok) {
            return true;
        }
        // Space after keywords in KW_SPACE
        if KW_SPACE.contains(&p) {
            return true;
        }
        // Space before keywords (when not at line start)
        if KW_SPACE.contains(&tok) {
            return true;
        }
        // Space after close bracket before word/literal
        if p == ")" || p == "]" {
            return tok
                .chars()
                .next()
                .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '"' || c == '\'');
        }
        // Two adjacent alphanumeric tokens always need a space (e.g. `boolean size(1)`).
        let p_ends_alnum = p
            .chars()
            .last()
            .is_some_and(|c| c.is_alphanumeric() || c == '_');
        let t_starts_alnum = tok
            .chars()
            .next()
            .is_some_and(|c| c.is_alphanumeric() || c == '_');
        if p_ends_alnum && t_starts_alnum {
            return true;
        }
        false
    }

    fn push(&mut self, tok: &str) {
        let sp = self.need_space(tok);
        self.emit_indent_if_needed();
        if sp && !self.at_line_start {
            self.out.push(' ');
        }
        self.out.push_str(tok);
        self.at_line_start = false;
        self.prev = tok.to_string();
    }

    fn process(&mut self, tokens: &[Tok]) {
        let mut i = 0;
        let mut trailing: Option<String> = None;

        while i < tokens.len() {
            match &tokens[i] {
                Tok::Newline => {
                    // If the next meaningful token is `else`, suppress the newline
                    // so that `} else {` stays on one line.
                    let next_word = tokens[(i + 1)..]
                        .iter()
                        .find(|t| !matches!(t, Tok::Newline | Tok::Blank));
                    if matches!(next_word, Some(Tok::Word(w)) if w == "else") {
                        // skip all consecutive newlines/blanks up to `else`
                        i += 1;
                        while i < tokens.len() && matches!(tokens[i], Tok::Newline | Tok::Blank) {
                            i += 1;
                        }
                    } else {
                        self.flush_trailing(&mut trailing);
                        i += 1;
                    }
                }
                Tok::Blank => {
                    // Blank lines are re-inserted by the formatter based on structure,
                    // not preserved from input. Just note that there was a blank.
                    i += 1;
                }
                Tok::Comment(text) => {
                    let text = text.clone();
                    if self.at_line_start {
                        // standalone comment line
                        self.emit_indent_if_needed();
                        self.out.push_str("//");
                        if !text.is_empty() {
                            self.out.push_str(&text);
                        }
                        self.at_line_start = false;
                        // don't update prev — comments don't affect spacing
                    } else {
                        // trailing comment — flush after code on this line
                        trailing = Some(text.clone());
                    }
                    i += 1;
                }
                Tok::Word(w) => {
                    let w = w.clone();
                    self.handle_word(&w, &mut trailing);
                    i += 1;
                }
                Tok::Sym(s) => {
                    let s = s.clone();
                    self.handle_sym(&s, tokens, &mut i, &mut trailing);
                }
                Tok::Int(s) | Tok::Flt(s) | Tok::Str(s) | Tok::Chr(s) => {
                    let s = s.clone();
                    self.push(&s);
                    i += 1;
                }
            }
        }
        self.flush_trailing(&mut trailing);
        if !self.out.ends_with('\n') {
            self.out.push('\n');
        }
    }

    fn flush_trailing(&mut self, tc: &mut Option<String>) {
        if let Some(comment) = tc.take() {
            while self.out.ends_with(' ') {
                self.out.pop();
            }
            self.out.push_str("  //");
            self.out.push_str(&comment);
        }
        self.ensure_nl();
    }

    fn handle_word(&mut self, w: &str, tc: &mut Option<String>) {
        // Insert blank line before top-level declarations (fn, struct, enum, type, use)
        // only when the previous top-level block was closed with `}`.
        if self.depth == 0
            && self.at_line_start
            && self.last_top_closed
            && matches!(w, "fn" | "pub" | "struct" | "enum" | "type" | "use")
        {
            // emit blank line (we are already at line start after previous `}`)
            self.out.push('\n');
            self.last_top_closed = false;
        }

        self.push(w);

        // Track state for `{` context detection
        if w == "struct" || w == "enum" {
            self.after_struct_hdr = true;
            self.next_brace_is_block = false;
        } else if BLOCK_KWS.contains(&w) {
            // fn, if, else, for, while, loop, match — next `{` is always a block body
            self.after_struct_hdr = false;
            self.next_brace_is_block = true;
        }
        // Other words (type names, identifiers) do not clear next_brace_is_block,
        // so `fn foo() -> integer {` still lands on Block.
        let _ = tc;
    }

    #[allow(clippy::too_many_lines)]
    fn handle_sym(&mut self, s: &str, tokens: &[Tok], i: &mut usize, tc: &mut Option<String>) {
        match s {
            "{" => self.open_brace(tokens, i, tc),
            "}" => {
                self.close_brace(tc);
                *i += 1;
            }
            "(" => {
                self.push("(");
                self.ctx.push(Ctx::ArgList);
                *i += 1;
            }
            ")" => {
                while self.out.ends_with(' ') {
                    self.out.pop();
                }
                self.ctx.pop();
                self.emit_indent_if_needed();
                self.out.push(')');
                self.at_line_start = false;
                self.prev = ")".to_string();
                *i += 1;
            }
            "[" => {
                self.push("[");
                self.ctx.push(Ctx::ArrayLit);
                *i += 1;
            }
            "]" => {
                while self.out.ends_with(' ') {
                    self.out.pop();
                }
                self.ctx.pop();
                self.emit_indent_if_needed();
                self.out.push(']');
                self.at_line_start = false;
                self.prev = "]".to_string();
                *i += 1;
            }
            ";" => {
                while self.out.ends_with(' ') {
                    self.out.pop();
                }
                self.out.push(';');
                self.at_line_start = false;
                self.prev = ";".to_string();
                self.flush_trailing(tc);
                *i += 1;
            }
            "," => {
                while self.out.ends_with(' ') {
                    self.out.pop();
                }
                // Skip trailing commas (next meaningful token is a closer)
                let next = tokens[(*i + 1)..]
                    .iter()
                    .find(|t| !matches!(t, Tok::Newline | Tok::Blank | Tok::Comment(_)));
                let trailing_comma = matches!(
                    next,
                    Some(Tok::Sym(cs)) if cs == ")" || cs == "]" || cs == "}"
                );
                if !trailing_comma {
                    self.out.push(',');
                    self.at_line_start = false;
                    self.prev = ",".to_string();
                    match self.top_ctx() {
                        Some(Ctx::StructDef) => {
                            self.flush_trailing(tc);
                        }
                        _ => {
                            self.out.push(' ');
                        }
                    }
                }
                *i += 1;
            }
            ":" => {
                while self.out.ends_with(' ') {
                    self.out.pop();
                }
                self.out.push(':');
                self.out.push(' ');
                self.at_line_start = false;
                self.prev = ":".to_string();
                *i += 1;
            }
            "::" => {
                while self.out.ends_with(' ') {
                    self.out.pop();
                }
                self.out.push_str("::");
                self.at_line_start = false;
                self.prev = "::".to_string();
                *i += 1;
            }
            "." | ".." | "..=" => {
                while self.out.ends_with(' ') {
                    self.out.pop();
                }
                self.out.push_str(s);
                self.at_line_start = false;
                self.prev = s.to_string();
                *i += 1;
            }
            "->" | "=>" => {
                self.push(s);
                self.next_brace_is_block = true;
                self.after_struct_hdr = false;
                *i += 1;
            }
            "-" | "+" => {
                // Detect unary position: prev was an open bracket, operator, or
                // nothing — in that case emit without space and set prev="unary"
                // so the operand won't pick up a spurious space.
                let unary = self.prev.is_empty()
                    || matches!(
                        self.prev.as_str(),
                        "(" | "[" | "," | ";" | ":" | "return" | "not" | "unary"
                    )
                    || BINARY_OPS.contains(&self.prev.as_str())
                    || ASSIGN_OPS.contains(&self.prev.as_str());
                if unary {
                    // Space before the unary op if context requires it (e.g. after `=`),
                    // but not if the output already ends with a space (e.g. after `,`).
                    let sp = self.need_space(s) && !self.out.ends_with(' ');
                    self.emit_indent_if_needed();
                    if sp && !self.at_line_start {
                        self.out.push(' ');
                    }
                    self.out.push_str(s);
                    self.at_line_start = false;
                    self.prev = "unary".to_string(); // sentinel: no space before operand
                } else {
                    self.push(s);
                }
                *i += 1;
            }
            _ => {
                self.push(s);
                *i += 1;
            }
        }
    }

    fn open_brace(&mut self, tokens: &[Tok], i: &mut usize, tc: &mut Option<String>) {
        let ctx = if self.after_struct_hdr {
            self.after_struct_hdr = false;
            self.next_brace_is_block = false;
            Ctx::StructDef
        } else if self.depth == 0 || self.next_brace_is_block {
            // Top-level or after a block-opening keyword / `->`: always a block body
            self.next_brace_is_block = false;
            Ctx::Block
        } else {
            // Inside an existing block without a block-opener keyword: struct literal
            self.next_brace_is_block = false;
            Ctx::StructLit
        };

        match ctx {
            Ctx::StructDef | Ctx::Block => {
                // Opening brace on same line, body on next line(s)
                while self.out.ends_with(' ') {
                    self.out.pop();
                }
                if !self.at_line_start {
                    self.out.push(' ');
                }
                self.out.push('{');
                self.at_line_start = false;
                self.prev = "{".to_string();
                self.ctx.push(ctx);
                self.depth += 1;
                self.flush_trailing(tc);
                // skip any newlines/blanks immediately after the `{` line
                *i += 1;
                while *i < tokens.len() && matches!(tokens[*i], Tok::Newline | Tok::Blank) {
                    *i += 1;
                }
            }
            Ctx::StructLit => {
                while self.out.ends_with(' ') {
                    self.out.pop();
                }
                self.out.push_str(" {");
                self.at_line_start = false;
                self.prev = "{".to_string();
                self.ctx.push(Ctx::StructLit);
                // struct literal stays inline
                *i += 1;
            }
            _ => unreachable!(),
        }
    }

    fn close_brace(&mut self, tc: &mut Option<String>) {
        let popped = self.ctx.pop();
        match popped {
            Some(Ctx::Block | Ctx::StructDef) => {
                self.depth -= 1;
                self.flush_trailing(tc);
                let ind = self.indent();
                self.out.push_str(&ind);
                self.out.push('}');
                self.at_line_start = false;
                self.prev = "}".to_string();
                if self.depth == 0 {
                    self.last_top_closed = true;
                }
            }
            Some(Ctx::StructLit) => {
                while self.out.ends_with(' ') {
                    self.out.pop();
                }
                self.out.push_str(" }");
                self.at_line_start = false;
                self.prev = "}".to_string();
            }
            _ => {
                // unmatched or unknown — emit anyway
                self.out.push('}');
                self.prev = "}".to_string();
            }
        }
    }
}

// ─── Public API ───────────────────────────────────────────────────────────────

/// Format loft source code and return the formatted string.
///
/// Always outputs LF (`\n`) line endings regardless of the input line ending
/// style.  CRLF inputs are normalised before scanning.
#[must_use]
pub fn format_source(source: &str) -> String {
    // Normalise CRLF → LF so the formatter output is always \n-only.
    let owned;
    let src: &str = if source.contains('\r') {
        owned = source.replace('\r', "");
        &owned
    } else {
        source
    };
    let tokens = scan(src);
    let mut fmt = Fmt::new();
    fmt.process(&tokens);
    fmt.out
}

/// Return `true` if the source is already in canonical format.
///
/// CRLF line endings are normalised before the comparison so that a file
/// checked out with Windows line endings is treated the same as one with LF.
#[must_use]
pub fn check_source(source: &str) -> bool {
    let owned;
    let src: &str = if source.contains('\r') {
        owned = source.replace('\r', "");
        &owned
    } else {
        source
    };
    format_source(src) == src
}
