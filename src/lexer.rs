// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Change a text into symbols to use in the parser.
//! It is possible to link to the current position in the lexer (link) and return to it (revert)
//! when the parser has to try a certain path and might dismiss this later.

use crate::diagnostics::{Diagnostics, Level, diagnostic_format};
use std::cell::RefCell;
use std::collections::HashSet;
use std::fmt::{Debug, Display, Formatter};
use std::fs::File;
use std::io::{BufRead, BufReader, Result as IoResult};
use std::iter::Peekable;
use std::rc::Rc;
use std::vec::IntoIter;

#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    /// Expect code with spaces, line ends and remarks removed.
    Code,
    /// Expect formatting expressions, when encountering a closing bracket continue with a string.
    Formatting,
}

/// An item parsed by the lexer.
#[derive(Debug, Clone, PartialEq)]
pub enum LexItem {
    /// This routine cannot directly parse negative number, because - is reported as a token.
    /// Second token is if the number started with a 0. Only needed for string formatting.
    Integer(u32, bool),
    Long(u64),
    Float(f64),
    Single(f32),
    /// Can be both a keyword and one or more position tokens.
    Token(String),
    /// A still unknown identifier.
    Identifier(String),
    /// A constant string: was presented as "content" with possibly escaped tokens inside.
    CString(String),
    Character(u32),
    /// The end of the content is reached.
    None,
}

#[derive(Clone, PartialEq)]
pub struct Position {
    /// The file name where this construct is found.
    pub file: String,
    /// The line where this result was found.
    pub line: u32,
    /// The position on the line where this result was found.
    pub pos: u32,
}

impl Position {
    fn format(&self, fmt: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        fmt.write_str(&format!("{}:{}:{}", self.file, self.line, self.pos))
    }
}

impl Debug for Position {
    fn fmt(&self, fmt: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        self.format(fmt)
    }
}

impl Display for Position {
    fn fmt(&self, fmt: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        self.format(fmt)
    }
}

/// The lexer can be iterated to gain a string of results.
#[derive(Debug, Clone, PartialEq)]
pub struct LexResult {
    pub has: LexItem,
    pub position: Position,
}

impl LexResult {
    fn new(it: LexItem, position: Position) -> LexResult {
        LexResult { has: it, position }
    }
}

/// A lexer that can remember a state via a link and then optionally return to that state.
///
/// It defaults to reading all found data into Text elements but has a list of TOKENS and
/// KEYWORDS that are parsed when a line starts with a token.
pub struct Lexer {
    lines: Box<dyn Iterator<Item = IoResult<String>>>,
    iter: Peekable<IntoIter<char>>,
    peek: LexResult,
    /// Keep the scanned items in memory when a Link is created to return when reverted to this link.
    memory: Vec<LexResult>,
    /// Keep track of the number of currently in use links
    links: Rc<RefCell<u32>>,
    /// Keep track of where we are in the current memory structure
    link: usize,
    position: Position,
    tokens: HashSet<String>,
    keywords: HashSet<String>,
    /// Should we expect code with whitespaces here?
    mode: Mode,
    /// True while the lexer is inside a `{...}` format expression of a string literal.
    /// Allows `"` (and `\"`) to open a nested string literal instead of closing the outer one.
    in_format_expr: bool,
    /// True when the current format expression belongs to a backtick string.
    /// After `}` closes the expression, resume with `backtick_string_resume()`
    /// instead of `string()`.
    in_backtick: bool,
    diagnostics: Diagnostics,
}

impl Debug for Lexer {
    fn fmt(&self, fmt: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        fmt.write_str(&format!("{:?}", self.position))
    }
}

static LINE: String = String::new();

static TOKENS: &[&str] = &[
    ":", "::", ".", "..", ",", "{", "}", "(", ")", "[", "]", ";", "!", "!=", "+", "+=", "-", "-=",
    "*", "**", "*=", "/", "/=", "%", "%=", "=", "==", "<", "<=", ">", ">=", "&", "&&", "|", "||",
    "->", "=>", "^", "<<", ">>", "$", "//", "#", "?", "??", "@",
];

static KEYWORDS: &[&str] = &[
    "as",
    "if",
    "in",
    "else",
    "for",
    "while",
    "continue",
    "break",
    "return",
    "yield",
    "true",
    "false",
    "null",
    "struct",
    "fn",
    "type",
    "enum",
    "pub",
    "and",
    "or",
    "use",
    "match",
    "sizeof",
    "debug_assert",
    "assert",
    "panic",
    "interface",
];

#[derive(Debug)]
pub struct Link {
    links: Rc<RefCell<u32>>,
    pos: usize,
}

impl Drop for Link {
    fn drop(&mut self) {
        *self.links.borrow_mut() -= 1;
    }
}

fn hex_parse(val: &str) -> Option<u64> {
    let mut res: u64 = 0;
    for ch in val.chars() {
        if ch.is_ascii_digit() {
            res = res * 16 + ch as u64 - '0' as u64;
        } else if ch.is_ascii_hexdigit() {
            res = res * 16 + 10 + ch.to_ascii_lowercase() as u64 - 'a' as u64;
        } else {
            return None;
        }
    }
    Some(res)
}

fn bin_parse(val: &str) -> Option<u64> {
    let mut res: u64 = 0;
    for ch in val.chars() {
        if ('0'..='1').contains(&ch) {
            res = res * 2 + ch as u64 - '0' as u64;
        } else {
            return None;
        }
    }
    Some(res)
}

fn oct_parse(val: &str) -> Option<u64> {
    let mut res: u64 = 0;
    for ch in val.chars() {
        if ('0'..='7').contains(&ch) {
            res = res * 8 + ch as u64 - '0' as u64;
        } else {
            return None;
        }
    }
    Some(res)
}

impl Default for Lexer {
    fn default() -> Self {
        let mut result = Lexer {
            lines: Box::new(Vec::new().into_iter()),
            peek: LexResult {
                has: LexItem::None,
                position: Position {
                    file: String::new(),
                    line: 0,
                    pos: 0,
                },
            },
            position: Position {
                file: String::new(),
                line: 0,
                pos: 0,
            },
            memory: Vec::new(),
            link: 0,
            links: Rc::new(RefCell::new(0)),
            iter: LINE.chars().collect::<Vec<_>>().into_iter().peekable(),
            tokens: HashSet::new(),
            keywords: HashSet::new(),
            mode: Mode::Code,
            in_format_expr: false,
            in_backtick: false,
            diagnostics: Diagnostics::new(),
        };
        for s in TOKENS {
            result.tokens.insert(String::from(*s));
        }
        for s in KEYWORDS {
            result.keywords.insert(String::from(*s));
        }
        result
    }
}

impl Lexer {
    #[allow(unused)]
    fn new(lines: impl Iterator<Item = IoResult<String>> + 'static, filename: &str) -> Lexer {
        let mut result = Lexer {
            lines: Box::new(lines),
            peek: LexResult {
                has: LexItem::None,
                position: Position {
                    file: filename.to_string(),
                    line: 0,
                    pos: 0,
                },
            },
            position: Position {
                file: filename.to_string(),
                line: 0,
                pos: 0,
            },
            memory: Vec::new(),
            link: 0,
            links: Rc::new(RefCell::new(0)),
            iter: LINE.chars().collect::<Vec<_>>().into_iter().peekable(),
            tokens: HashSet::new(),
            keywords: HashSet::new(),
            mode: Mode::Code,
            in_format_expr: false,
            in_backtick: false,
            diagnostics: Diagnostics::new(),
        };
        for s in TOKENS {
            result.tokens.insert(String::from(*s));
        }
        for s in KEYWORDS {
            result.keywords.insert(String::from(*s));
        }
        result
    }

    pub fn to(&mut self, scope: (u32, u32)) {
        self.position.line = scope.0;
        self.position.pos = scope.1;
    }

    #[allow(clippy::too_many_lines)] // large lexer dispatch — splitting would obscure control flow
    fn next(&mut self) -> Option<LexResult> {
        if self.link < self.memory.len() {
            let n = self.memory[self.link].clone();
            self.link += 1;
            return Some(n);
        }
        if self.mode != Mode::Formatting {
            loop {
                if let Some(&c) = self.iter.peek() {
                    if c != ' ' && c != '\t' {
                        break;
                    }
                    self.next_char();
                } else if let Some(line_result) = self.lines.next() {
                    match line_result {
                        Ok(ln) => {
                            if self.position.line == 0 && ln.starts_with("#!/") {
                                continue;
                            }
                            self.iter = ln.chars().collect::<Vec<_>>().into_iter().peekable();
                            self.position.line += 1;
                            self.position.pos = 1;
                        }
                        Err(e) => {
                            self.position.line += 1;
                            self.err(
                                Level::Fatal,
                                &format!(
                                    "Cannot read line {} — is the file valid UTF-8? ({})",
                                    self.position.line, e
                                ),
                            );
                            break;
                        }
                    }
                } else {
                    break;
                }
            }
        }
        let pos = self.position.clone();
        if let Some(&c) = self.iter.peek() {
            Some(match c {
                '0'..='9' => self.number(),
                '"' => {
                    self.next_char();
                    if self.in_format_expr {
                        self.string_nested(false)
                    } else {
                        self.string()
                    }
                }
                '`' => {
                    self.next_char();
                    self.backtick_string()
                }
                '\'' => {
                    self.next_char();
                    self.char()
                }
                ' ' | '\t' => {
                    self.next_char();
                    LexResult::new(LexItem::Token(" ".to_string()), pos)
                }
                _ => {
                    let single = String::from(c);
                    if self.tokens.contains(&single) {
                        self.next_char();
                        if let Some(&d) = self.iter.peek() {
                            let double = format!("{c}{d}");
                            if self.tokens.contains(&double) {
                                self.next_char();
                                LexResult::new(LexItem::Token(double), pos)
                            } else if self.mode == Mode::Formatting && single == "}" {
                                self.in_format_expr = false;
                                if self.in_backtick {
                                    self.in_backtick = false;
                                    self.backtick_string_resume()
                                } else {
                                    self.string()
                                }
                            } else {
                                LexResult::new(LexItem::Token(single), pos)
                            }
                        } else {
                            LexResult::new(LexItem::Token(single), pos)
                        }
                    } else if c == '\\' && self.in_format_expr {
                        // `\"` inside a format expression opens a nested string literal.
                        self.next_char(); // consume '\'
                        if let Some(&nc) = self.iter.peek() {
                            if nc == '"' {
                                self.next_char(); // consume '"'
                                self.string_nested(true)
                            } else {
                                self.err(
                                    Level::Error,
                                    "Expected '\"' after '\\' in format expression",
                                );
                                Lexer::none()
                            }
                        } else {
                            self.err(Level::Error, "Unexpected end of input after '\\'");
                            Lexer::none()
                        }
                    } else {
                        let ident = self.get_identifier();
                        if self.keywords.contains(&ident) {
                            LexResult::new(LexItem::Token(ident), pos)
                        } else {
                            LexResult::new(LexItem::Identifier(ident), pos)
                        }
                    }
                }
            })
        } else if let Some(line_result) = self.lines.next() {
            match line_result {
                Ok(ln) => {
                    self.iter = ln.chars().collect::<Vec<_>>().into_iter().peekable();
                    self.position.line += 1;
                    self.position.pos = 1;
                    Some(LexResult::new(LexItem::None, self.position.clone()))
                }
                Err(e) => {
                    self.position.line += 1;
                    self.err(
                        Level::Fatal,
                        &format!(
                            "Cannot read line {} — is the file valid UTF-8? ({})",
                            self.position.line, e
                        ),
                    );
                    None
                }
            }
        } else {
            None
        }
    }

    pub fn pos(&self) -> &Position {
        &self.position
    }

    pub fn at(&self) -> (u32, u32) {
        (self.position.line, self.position.pos)
    }

    pub fn diagnostic(&mut self, level: Level, message: &str) {
        self.diagnostics.add(
            level,
            &format!(
                "{message} at {}:{}:{}",
                self.position.file, self.position.line, self.position.pos
            ),
        );
    }

    pub fn specific(&mut self, result: &LexResult, level: Level, message: &str) {
        self.diagnostics.add(
            level,
            &format!(
                "{message} at {}:{}:{}",
                self.position.file, result.position.line, result.position.pos
            ),
        );
    }

    pub fn pos_diagnostic(&mut self, level: Level, pos: &Position, message: &str) {
        self.diagnostics.add(
            level,
            &format!("{message} at {}:{}:{}", pos.file, pos.line, pos.pos),
        );
    }

    pub fn diagnostics(&self) -> &Diagnostics {
        &self.diagnostics
    }

    pub fn mode(&self) -> Mode {
        self.mode.clone()
    }

    pub fn set_mode(&mut self, mode: Mode) {
        if mode == Mode::Formatting && self.peek_token("}") {
            self.in_format_expr = false;
            self.mode = mode;
            self.peek = if self.in_backtick {
                self.in_backtick = false;
                self.backtick_string_resume()
            } else {
                self.string()
            };
        } else {
            self.mode = mode;
        }
    }

    #[allow(dead_code)]
    pub fn whitespace(&mut self) {
        while self.peek_token(" ") || self.peek_token("\t") {
            self.cont();
        }
    }

    fn none() -> LexResult {
        LexResult {
            has: LexItem::None,
            position: Position {
                file: String::new(),
                line: 0,
                pos: 0,
            },
        }
    }

    /// parse a character constant for the lexer.
    fn char(&mut self) -> LexResult {
        let pos = self.position.clone();
        let mut res = String::new();
        while let Some(&c) = self.iter.peek() {
            if c == '\'' {
                self.next_char();
                let mut chars = res.chars();
                return LexResult::new(
                    LexItem::Character(if let Some(ch) = chars.next() {
                        if chars.next().is_some() {
                            self.err(Level::Error, "Expected only one character in constant");
                        }
                        ch as u32
                    } else {
                        self.err(Level::Error, "Expected a character in constant");
                        0
                    }),
                    pos,
                );
            }
            if c == '\\' {
                self.next_char();
                if !self.escape_seq(&mut res) {
                    break;
                }
            } else if c == '\n' {
                break;
            } else {
                res.push(c);
            }
            self.next_char();
        }
        self.err(Level::Fatal, "Character not correctly terminated");
        Lexer::none()
    }

    fn escape_seq(&mut self, res: &mut String) -> bool {
        // TODO allow numeric characters
        if let Some(&c) = self.iter.peek() {
            match c {
                '"' | '\'' | '\\' => res.push(c),
                't' => res.push('\t'),
                'r' => res.push('\r'),
                'n' | '\n' => res.push('\n'),
                _ => {
                    self.err(Level::Error, "Unknown escape sequence");
                    res.push('?');
                }
            }
            true
        } else {
            false
        }
    }

    /// parse a string for the lexer.
    fn string(&mut self) -> LexResult {
        let pos = self.position.clone();
        let mut res = String::new();
        while let Some(&c) = self.iter.peek() {
            if c == '"' {
                self.mode = Mode::Code;
                self.next_char();
                return LexResult::new(LexItem::CString(res), pos);
            }
            if c == '\\' {
                self.next_char();
                if !self.escape_seq(&mut res) {
                    break;
                }
            } else if c == '\n' {
                break;
            } else if c == '{' {
                self.next_char();
                if let Some('{') = self.iter.peek() {
                    res.push(c);
                } else {
                    self.mode = Mode::Formatting;
                    self.in_format_expr = true;
                    return LexResult::new(LexItem::CString(res), pos);
                }
            } else if c == '}' {
                self.next_char();
                if let Some('}') = self.iter.peek() {
                    res.push(c);
                } else {
                    self.err(Level::Warning, "Expected two '}' tokens");
                }
            } else {
                res.push(c);
            }
            self.next_char();
        }
        self.err(Level::Fatal, "String not correctly terminated");
        Lexer::none()
    }

    /// Scan a string literal that appears as an expression inside a `{...}` format slot.
    ///
    /// When `escaped_delim` is false (opened by bare `"`), the string closes on
    /// a bare `"` and `\"` is a normal escape producing a literal quote.  This
    /// is the path used by `.loft` source files: `"text {"inner \"quoted\""}"`.
    ///
    /// When `escaped_delim` is true (opened by `\"`), the string closes on `\"`
    /// as well as bare `"`.  This preserves backward compatibility with Rust
    /// test macros where the source already has the outer quotes escaped:
    /// `"text {\"inner\"}"`.
    fn string_nested(&mut self, escaped_delim: bool) -> LexResult {
        let pos = self.position.clone();
        let mut res = String::new();
        while let Some(&c) = self.iter.peek() {
            if c == '"' {
                // Bare " always closes the nested string literal.
                self.next_char();
                return LexResult::new(LexItem::CString(res), pos);
            }
            if c == '\\' {
                self.next_char(); // consume '\'
                if escaped_delim && let Some(&'"') = self.iter.peek() {
                    // Opened by \" → \" also closes.
                    self.next_char();
                    return LexResult::new(LexItem::CString(res), pos);
                }
                // Normal escape sequence (including \" when !escaped_delim).
                if !self.escape_seq(&mut res) {
                    break;
                }
            } else if c == '\n' {
                break;
            } else {
                res.push(c);
            }
            self.next_char();
        }
        self.err(
            Level::Fatal,
            "Nested string literal not correctly terminated",
        );
        Lexer::none()
    }

    /// Advance to the next source line inside a multi-line backtick string.
    /// Returns false at end-of-file.
    fn advance_line(&mut self) -> bool {
        if let Some(line_result) = self.lines.next() {
            match line_result {
                Ok(ln) => {
                    self.iter = ln.chars().collect::<Vec<_>>().into_iter().peekable();
                    self.position.line += 1;
                    self.position.pos = 1;
                    true
                }
                Err(_) => false,
            }
        } else {
            false
        }
    }

    /// Scan a backtick string literal: `` `...` ``.
    ///
    /// Multi-line, supports `{expr}` interpolation and `{{`/`}}` escaping.
    /// Bare `"` is literal (no escaping needed).  `\` escapes work as usual.
    /// Closes on the next `` ` ``.
    ///
    /// **Indent stripping:** the column of the closing `` ` `` defines the base
    /// indentation.  That many leading spaces are removed from every line of the
    /// content.  The first line (on the same line as the opening `` ` ``) and the
    /// last line (on the same line as the closing `` ` ``) are trimmed if they
    /// contain only whitespace.
    fn backtick_string(&mut self) -> LexResult {
        let pos = self.position.clone();
        let mut lines: Vec<String> = Vec::new();
        let mut cur = String::new();

        loop {
            match self.iter.peek() {
                Some(&'`') => {
                    // Closing backtick — record its column for indent stripping.
                    let close_col = self.position.pos;
                    self.next_char();
                    lines.push(cur);

                    // Strip indentation: remove up to (close_col - 1) leading spaces
                    // from each line.
                    let strip = (close_col - 1) as usize;
                    let mut result = String::new();
                    for (i, line) in lines.iter().enumerate() {
                        if i == 0 {
                            // First line: content after opening backtick on same line.
                            if !line.trim().is_empty() {
                                result += line;
                            }
                            continue;
                        }
                        // Last line before closing backtick: skip if whitespace-only.
                        if i == lines.len() - 1 && line.trim().is_empty() {
                            continue;
                        }
                        if !result.is_empty() {
                            result.push('\n');
                        }
                        // Strip up to `strip` leading spaces.
                        let stripped = if strip > 0
                            && line.len() >= strip
                            && line[..strip].chars().all(|c| c == ' ')
                        {
                            &line[strip..]
                        } else {
                            line
                        };
                        result += stripped;
                    }
                    self.mode = Mode::Code;
                    return LexResult::new(LexItem::CString(result), pos);
                }
                Some(&'{') => {
                    self.next_char();
                    if let Some('{') = self.iter.peek() {
                        cur.push('{');
                    } else {
                        // Enter format interpolation — return what we have so far.
                        lines.push(std::mem::take(&mut cur));
                        let mut result = String::new();
                        for (i, line) in lines.iter().enumerate() {
                            if i == 0 && line.trim().is_empty() {
                                continue;
                            }
                            if !result.is_empty() {
                                result.push('\n');
                            }
                            result += line;
                        }
                        self.mode = Mode::Formatting;
                        self.in_format_expr = true;
                        self.in_backtick = true;
                        return LexResult::new(LexItem::CString(result), pos);
                    }
                }
                Some(&'}') => {
                    self.next_char();
                    if let Some('}') = self.iter.peek() {
                        cur.push('}');
                    } else {
                        self.err(Level::Warning, "Expected two '}' tokens");
                    }
                }
                Some(&'\\') => {
                    self.next_char();
                    if !self.escape_seq(&mut cur) {
                        self.err(Level::Fatal, "Backtick string not correctly terminated");
                        return Lexer::none();
                    }
                }
                Some(&c) => {
                    cur.push(c);
                }
                None => {
                    // End of line — advance to next line.
                    lines.push(std::mem::take(&mut cur));
                    if !self.advance_line() {
                        self.err(Level::Fatal, "Backtick string not correctly terminated");
                        return Lexer::none();
                    }
                    // Read the full line into cur to get accurate column positions.
                    // But first, capture the raw line for indent tracking.
                    let mut line_content = String::new();
                    while let Some(&c) = self.iter.peek() {
                        if c == '`' || c == '{' || c == '}' || c == '\\' {
                            break;
                        }
                        line_content.push(c);
                        self.next_char();
                    }
                    cur = line_content;
                    continue; // don't call next_char — we're positioned at the special char
                }
            }
            self.next_char();
        }
    }

    /// Resume a backtick string after a `}` closes a format expression.
    /// Called from the `}` token handler when the backtick string owns the
    /// format context.
    fn backtick_string_resume(&mut self) -> LexResult {
        let pos = self.position.clone();
        let mut cur = String::new();
        loop {
            match self.iter.peek() {
                Some(&'`') => {
                    self.next_char();
                    self.mode = Mode::Code;
                    return LexResult::new(LexItem::CString(cur), pos);
                }
                Some(&'{') => {
                    self.next_char();
                    if let Some('{') = self.iter.peek() {
                        cur.push('{');
                    } else {
                        self.mode = Mode::Formatting;
                        self.in_format_expr = true;
                        self.in_backtick = true;
                        return LexResult::new(LexItem::CString(cur), pos);
                    }
                }
                Some(&'}') => {
                    self.next_char();
                    if let Some('}') = self.iter.peek() {
                        cur.push('}');
                    } else {
                        self.err(Level::Warning, "Expected two '}' tokens");
                    }
                }
                Some(&'\\') => {
                    self.next_char();
                    if !self.escape_seq(&mut cur) {
                        self.err(Level::Fatal, "Backtick string not correctly terminated");
                        return Lexer::none();
                    }
                }
                Some(&c) => {
                    cur.push(c);
                }
                None => {
                    cur.push('\n');
                    if !self.advance_line() {
                        self.err(Level::Fatal, "Backtick string not correctly terminated");
                        return Lexer::none();
                    }
                    continue;
                }
            }
            self.next_char();
        }
    }

    fn next_char(&mut self) {
        self.iter.next();
        self.position.pos += 1;
    }

    fn get_identifier(&mut self) -> String {
        let mut string = String::new();
        while let Some(&ident) = self.iter.peek() {
            if ident.is_ascii_lowercase()
                || ident.is_ascii_uppercase()
                || ident.is_ascii_digit()
                || ident == '_'
            {
                string.push(ident);
                self.next_char();
            } else {
                break;
            }
        }
        string
    }

    fn get_number(&mut self) -> String {
        let mut number = String::new();
        let mut hex = false;
        while let Some(&c) = self.iter.peek() {
            if c.is_ascii_digit() || c == 'b' || c == 'o' {
                number.push(c);
                self.next_char();
            } else if c == 'x' && !hex && number == "0" {
                hex = true;
                number.push(c);
                self.next_char();
            } else if hex && (('a'..='f').contains(&c) || ('A'..='F').contains(&c)) {
                number.push(c);
                self.next_char();
            } else {
                break;
            }
        }
        number
    }

    /// parse a number for the lexer.
    fn number(&mut self) -> LexResult {
        let pos = self.position.clone();
        let mut val = self.get_number();
        let mut f = false;
        if let Some('.') = self.iter.peek() {
            self.next_char();
            if let Some('.') = self.iter.peek() {
                self.next_char();
                self.link = self.memory.len();
                self.memory.push(LexResult::new(
                    LexItem::Token("..".to_string()),
                    pos.clone(),
                ));
                return if let Ok(r) = val.parse::<u32>() {
                    LexResult::new(LexItem::Integer(r, val.starts_with('0')), pos)
                } else {
                    self.err(Level::Error, "Problem parsing float");
                    Lexer::none()
                };
            }
            val.push('.');
            f = true;
            let part = self.get_number();
            if part.is_empty() {
                self.err(Level::Error, "Problem parsing float");
                return Lexer::none();
            }
            val += &part;
        }
        if let Some('e') = self.iter.peek() {
            f = true;
            val.push('e');
            self.next_char();
            if let Some('-') = self.iter.peek() {
                self.next_char();
                val.push('-');
            }
            let exp = self.get_number();
            if exp.is_empty() {
                self.err(Level::Error, "Problem parsing float");
                return Lexer::none();
            }
            val += &exp;
        }
        if f {
            if let Some('f') = self.iter.peek() {
                self.next_char();
                if let Ok(r) = val.parse::<f32>() {
                    LexResult::new(LexItem::Single(r), pos)
                } else {
                    self.err(Level::Error, "Problem parsing single float");
                    LexResult::new(LexItem::Single(0.0), pos)
                }
            } else if let Ok(r) = val.parse::<f64>() {
                LexResult::new(LexItem::Float(r), pos)
            } else {
                self.err(Level::Error, "Problem parsing float");
                LexResult::new(LexItem::Float(0.0), pos)
            }
        } else if let Some(short) = val.strip_prefix("0x") {
            let res = if let Some(r) = hex_parse(short) {
                r
            } else {
                self.err(Level::Error, "Problem parsing hex number");
                0
            };
            self.ret_number(res, pos, false)
        } else if let Some(short) = val.strip_prefix("0b") {
            let res = if let Some(r) = bin_parse(short) {
                r
            } else {
                self.err(Level::Error, "Problem parsing binary number");
                0
            };
            self.ret_number(res, pos, false)
        } else if let Some(short) = val.strip_prefix("0o") {
            let res = if let Some(r) = oct_parse(short) {
                r
            } else {
                self.err(Level::Error, "Problem parsing octal number");
                0
            };
            self.ret_number(res, pos, false)
        } else if let Ok(r) = val.parse::<u64>() {
            self.ret_number(r, pos, val.starts_with('0'))
        } else {
            self.err(Level::Error, "Problem parsing number");
            self.ret_number(0, pos, false)
        }
    }

    #[allow(clippy::cast_possible_truncation)] // r is validated to fit in i32 range (< i32::MAX) before the u64 → i32 cast
    fn ret_number(&mut self, r: u64, p: Position, start_zero: bool) -> LexResult {
        let max = i32::MAX as usize;
        if let Some('l') = self.iter.peek() {
            self.next_char();
            LexResult::new(LexItem::Long(r), p)
        } else if r > max as u64 {
            self.err(Level::Error, "Problem parsing integer");
            LexResult::new(LexItem::Integer(0, start_zero), p)
        } else {
            LexResult::new(LexItem::Integer(r as u32, start_zero), p)
        }
    }

    pub fn parse_string(&mut self, string: &str, filename: &str) {
        let mut v = Vec::new();
        for l in string.split('\n') {
            v.push(Ok(String::from(l)));
        }
        self.lines = Box::new(v.into_iter());
        self.restart(filename);
    }

    pub fn switch(&mut self, filename: &str) {
        let Ok(fp) = File::open(filename) else {
            self.diagnostics
                .add(Level::Fatal, &format!("Unknown file:{filename}"));
            return;
        };
        self.lines = Box::new(BufReader::new(fp).lines());
        self.restart(filename);
    }

    fn restart(&mut self, filename: &str) {
        self.position = Position {
            file: filename.to_string(),
            line: 0,
            pos: 0,
        };
        self.peek = LexResult {
            has: LexItem::None,
            position: self.position.clone(),
        };
        self.memory.clear();
        self.link = 0;
        self.links = Rc::new(RefCell::new(0));
        self.iter = LINE.chars().collect::<Vec<_>>().into_iter().peekable();
        self.mode = Mode::Code;
        self.cont();
    }

    fn err(&mut self, level: Level, error: &str) {
        diagnostic!(self, level, "{error}");
    }

    /// debug feature to check the amount of currently in use links
    pub fn count_links(&self) -> u32 {
        *self.links.borrow()
    }

    /// Return the currently found lexer element.
    pub fn peek(&self) -> LexResult {
        self.peek.clone()
    }

    pub fn peek_token(&self, token: &str) -> bool {
        self.peek.has == LexItem::Token(token.to_string())
    }

    fn end(&mut self) {
        self.peek = LexResult {
            has: LexItem::None,
            position: self.position.clone(),
        }
    }

    /// Continue the lexer to the next step.
    pub fn cont(&mut self) {
        let Some(n) = self.next() else {
            self.end();
            return;
        };
        let mut res = n;
        while res.has == LexItem::Token("//".to_string()) {
            while self.iter.peek().is_some() {
                self.iter.next();
            }
            let Some(n) = self.next() else {
                self.end();
                return;
            };
            res = n;
        }
        if self.link == self.memory.len() {
            if self.count_links() > 0 {
                self.memory.push(res.clone());
                self.link += 1;
            } else {
                self.memory.clear();
                self.link = 0;
            }
        }
        self.peek = res;
    }

    /// Create a link to the current lexer position, it can be used to revert to
    /// this position later.
    pub fn link(&mut self) -> Link {
        let cur: u32 = *self.links.borrow();
        self.links.replace(cur + 1);
        if self.memory.is_empty() {
            self.memory.push(self.peek.clone());
            self.link += 1;
        }
        Link {
            links: Rc::clone(&self.links),
            pos: self.link - 1,
        }
    }

    /// Reset to a previously made link position in the source.
    pub fn revert(&mut self, link: Link) {
        self.link = link.pos;
        drop(link);
        self.cont();
    }

    pub fn token(&mut self, token: &'static str) -> bool {
        if self.has_token(token) {
            true
        } else {
            diagnostic!(self, Level::Error, "Expect token {token}");
            false
        }
    }

    /// Shorthand test if the current element is a specific token and skip it if found.
    pub fn has_token(&mut self, token: &'static str) -> bool {
        if self.peek_token(token) {
            self.cont();
            true
        } else {
            false
        }
    }

    /// Shorthand test if the current element is a specific local keyword, so not one of the reserved
    pub fn has_keyword(&mut self, keyword: &'static str) -> bool {
        if self.peek.has == LexItem::Identifier(keyword.to_string()) {
            self.cont();
            true
        } else {
            false
        }
    }

    /// Shorthand test if the current element is a number and skip it if found.
    pub fn has_integer(&mut self) -> Option<u32> {
        if let LexItem::Integer(n, _) = self.peek().has {
            self.cont();
            Some(n)
        } else {
            None
        }
    }

    /// Shorthand test if the current element is a number and skip it if found.
    pub fn has_long(&mut self) -> Option<u64> {
        if let LexItem::Long(n) = self.peek().has {
            self.cont();
            Some(n)
        } else if let LexItem::Integer(n, _zero) = self.peek().has {
            self.cont();
            Some(u64::from(n))
        } else {
            None
        }
    }

    pub fn has_char(&mut self) -> Option<u32> {
        if let LexItem::Character(c) = self.peek().has {
            self.cont();
            Some(c)
        } else {
            None
        }
    }

    /// Shorthand test if the current element is a constant string and skip it if found.
    pub fn has_cstring(&mut self) -> Option<String> {
        if let LexItem::CString(n) = self.peek().has {
            self.cont();
            Some(n)
        } else {
            None
        }
    }

    /// Shorthand test if the current element is a float and skip it if found.
    pub fn has_float(&mut self) -> Option<f64> {
        if let LexItem::Float(n) = self.peek().has {
            self.cont();
            Some(n)
        } else {
            None
        }
    }

    /// Shorthand test if the current element is a float and skip it if found.
    pub fn has_single(&mut self) -> Option<f32> {
        if let LexItem::Single(n) = self.peek().has {
            self.cont();
            Some(n)
        } else {
            None
        }
    }

    /// Peek two tokens ahead to detect `identifier :` (named argument syntax).
    /// Returns `Some(name)` if the pattern matches, without consuming any tokens.
    /// Returns `None` if the current token is not an identifier followed by `:`.
    pub fn peek_named_arg(&mut self) -> Option<String> {
        if let LexItem::Identifier(ref name) = self.peek.has {
            let name = name.clone();
            let saved = self.link();
            self.cont(); // consume identifier
            let is_colon = self.peek_token(":");
            let is_double_colon = self.peek_token("::");
            self.revert(saved); // restore to before identifier
            if is_colon && !is_double_colon {
                return Some(name);
            }
        }
        None
    }

    /// Shorthand test if the current element is an identifier and skip it if found.
    pub fn has_identifier(&mut self) -> Option<String> {
        if let LexItem::Identifier(n) = self.peek().has {
            self.cont();
            Some(n)
        } else {
            None
        }
    }

    /// Create a lexer from a static string
    #[allow(unused)]
    pub fn from_str(s: &str, filename: &str) -> Lexer {
        let mut v = Vec::new();
        for l in s.split('\n') {
            v.push(Ok(String::from(l)));
        }
        let mut res = Lexer::new(v.into_iter(), filename);
        res.cont();
        res
    }
}

#[cfg(test)]
mod test {
    fn test_id(lexer: &Lexer, id: &str) {
        assert_eq!(lexer.peek().has, LexItem::Identifier(String::from(id)));
    }

    fn links(lexer: &Lexer, nr: u32) {
        assert_eq!(lexer.count_links(), nr);
    }

    #[allow(unreachable_code)]
    fn array(lexer: &mut Lexer) -> Vec<LexItem> {
        let mut rest = Vec::new();
        rest.push(lexer.peek().has);
        loop {
            let Some(res) = lexer.next() else {
                break;
            };
            rest.push(res.has);
        }
        rest
    }

    use super::*;
    fn validate(s: &'static str, data: &[LexItem]) {
        let res = array(&mut Lexer::from_str(s, "validate"));
        assert_eq!(res, data);
    }

    #[cfg(test)]
    fn error(s: &'static str, err: &'static str) {
        let mut l = Lexer::from_str(s, "error");
        l.cont();
        assert_eq!(format!("{:?}", l.diagnostics), err.to_string());
    }

    #[cfg(test)]
    fn tokens(s: &'static str, t: &'static [&'static str]) {
        let mut data: Vec<LexItem> = Vec::new();
        for s in t {
            if s.chars().next().unwrap().is_ascii_digit() {
                if let Ok(res) = s.parse::<u32>() {
                    data.push(LexItem::Integer(res, false));
                } else {
                    panic!("Cannot parse {s}");
                }
            } else if KEYWORDS.contains(s) || TOKENS.contains(s) {
                data.push(LexItem::Token((*s).to_string()));
            } else {
                data.push(LexItem::Identifier((*s).to_string()));
            }
        }
        assert_eq!(array(&mut Lexer::from_str(s, "tokens")), data);
    }

    #[test]
    fn test_lexer() {
        validate("1234", &[LexItem::Integer(1234, false)]);
        validate("0xaf", &[LexItem::Integer(0xaf, false)]);
        validate("1e2", &[LexItem::Float(100.0)]);
        validate(
            "1..4",
            &[
                LexItem::Integer(1, false),
                LexItem::Token("..".to_string()),
                LexItem::Integer(4, false),
            ],
        );
        tokens("=1+2", &["=", "1", "+", "2"]);
        tokens("=if 1 in a", &["=", "if", "1", "in", "a"]);
    }

    #[test]
    fn lexer_errors() {
        error("123.a", "[\"Error: Problem parsing float at error:1:5\"]");
        error("12. ", "[\"Error: Problem parsing float at error:1:4\"]");
        error("1.12ea", "[\"Error: Problem parsing float at error:1:6\"]");
        error(
            "123456789012345678901",
            "[\"Error: Problem parsing number at error:1:22\"]",
        );
        error(
            "\"1\\a2\"",
            "[\"Error: Unknown escape sequence at error:1:4\"]",
        );
        error(
            "\"\\",
            "[\"Fatal: String not correctly terminated at error:1:3\"]",
        );
        error(
            "\"1\\t2",
            "[\"Fatal: String not correctly terminated at error:1:6\"]",
        );
        error(
            "\"12\nss",
            "[\"Fatal: String not correctly terminated at error:1:4\"]",
        );
    }

    #[test]
    fn test_links() {
        let mut lex = Lexer::from_str("{num:1 + a*(2.0e2+= b )", "test_links");
        assert_eq!(lex.count_links(), 0);
        assert_eq!(lex.peek().has, LexItem::Token(String::from("{")));
        {
            lex.cont();
            test_id(&lex, "num");
            let l1 = lex.link();
            links(&lex, 1);
            test_id(&lex, "num");
            lex.cont();
            assert!(lex.has_token(":"));
            assert_eq!(lex.peek().has, LexItem::Integer(1, false));
            links(&lex, 1);
            lex.revert(l1);
            test_id(&lex, "num");
            links(&lex, 0);
        }
        links(&lex, 0);
        test_id(&lex, "num");
        lex.cont();
        links(&lex, 0);
        assert_eq!(lex.peek().has, LexItem::Token(":".to_string()));
        lex.mode = Mode::Code;
        assert!(lex.has_token(":"));
        if let Some(n) = lex.has_integer() {
            assert_eq!(n, 1);
        } else {
            panic!("Expected a number")
        }
        assert!(lex.has_token("+"));
        if let Some(n) = lex.has_identifier() {
            assert_eq!(n, "a");
        } else {
            panic!("Expected an identifier")
        }
        assert!(lex.has_token("*"));
        assert!(lex.has_token("("));
        if let LexResult {
            has: LexItem::Float(f),
            ..
        } = lex.peek()
        {
            assert!(f64::abs(f - 200.0) < 0.00001);
        } else {
            panic!("Expected a float")
        }
        lex.cont();
        assert!(lex.has_token("+="));
    }

    #[test]
    fn test_formats() {
        validate(
            "\"ab{{cd}}ef\"",
            &[LexItem::CString("ab{cd}ef".to_string())],
        );
        validate(
            "\"ab{c:d}ef\"",
            &[
                LexItem::CString("ab".to_string()),
                LexItem::Identifier("c".to_string()),
                LexItem::Token(":".to_string()),
                LexItem::Identifier("d".to_string()),
                LexItem::CString("ef".to_string()),
            ],
        );
    }
}
