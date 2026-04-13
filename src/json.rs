// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// P54: JSON parser used by the `json_parse` native function in
// `src/native.rs`.  Walks UTF-8 text once and returns a `Parsed`
// value that the caller materialises into a loft `JsonValue`
// struct-enum record.
//
// Step 4 scope: full RFC 8259 — null, true, false, number, string
// (incl. standard escapes), array, object.  The `Parsed` tree is
// fully recursive here; `native::n_json_parse` flattens it into
// the arena-indexed loft JsonValue form at materialisation time.
//
// Q1 (this commit): parse failures carry a JSON Pointer path
// (RFC 6901) plus the byte offset.  Line:column and the
// surrounding context snippet are computed by `format_error`
// at error-formatting time, not per token, so the success path
// pays nothing.

/// Intermediate tree produced by [`parse`].  The loft-level
/// `JsonValue` variants are built from these values inside
/// `native::n_json_parse` so this module stays free of database
/// concerns.
#[derive(Debug, Clone)]
pub enum Parsed {
    Null,
    Bool(bool),
    Number(f64),
    Str(String),
    Array(Vec<Parsed>),
    Object(Vec<(String, Parsed)>),
}

/// Structured parse error.  `path` is an RFC 6901 JSON Pointer
/// to the location in the input where parsing gave up — `""`
/// means "at the root", `/users/3/age` means "third element of
/// the `users` array's `age` field".  `byte_offset` is the
/// absolute byte position; line:column + context snippet are
/// derived by [`format_error`] at error-formatting time so the
/// success path pays nothing.
#[derive(Debug, Clone)]
pub struct ParseError {
    pub message: String,
    pub byte_offset: usize,
    pub path: String,
}

/// Internal: a single parse step's result — value + advance, or
/// (message, offset).  The path is threaded out-of-band via the
/// `&mut Vec<String>` path stack so most arms don't need to mention it.
type ParseResult = Result<(Parsed, usize), (String, usize)>;

/// Parse the entire `input` as a JSON value.
///
/// Leading and trailing whitespace is allowed.  Characters after
/// the value (other than whitespace) are a syntax error — strict
/// RFC 8259, not a forgiving tokeniser.
///
/// # Errors
/// Returns a [`ParseError`] when the input is not valid JSON.
/// The `path` field localises the failure inside the document
/// (RFC 6901 JSON Pointer); the `byte_offset` field locates it
/// inside the raw text.
pub fn parse(input: &str) -> Result<Parsed, ParseError> {
    let bytes = input.as_bytes();
    let start = skip_ws(bytes, 0);
    let mut path: Vec<String> = Vec::new();
    let res = parse_value(bytes, start, &mut path);
    let (value, mut i) = match res {
        Ok(ok) => ok,
        Err((msg, at)) => {
            return Err(ParseError {
                message: msg,
                byte_offset: at,
                path: render_path(&path),
            });
        }
    };
    i = skip_ws(bytes, i);
    if i != bytes.len() {
        return Err(ParseError {
            message: format!("unexpected trailing byte at offset {i}"),
            byte_offset: i,
            path: render_path(&path),
        });
    }
    Ok(value)
}

/// Render the path stack as an RFC 6901 JSON Pointer.  Empty
/// stack → `""` (root).  Each segment is escaped: `~` → `~0`,
/// `/` → `~1`.
fn render_path(stack: &[String]) -> String {
    if stack.is_empty() {
        return String::new();
    }
    let mut out = String::with_capacity(stack.iter().map(|s| s.len() + 1).sum());
    for seg in stack {
        out.push('/');
        for ch in seg.chars() {
            match ch {
                '~' => out.push_str("~0"),
                '/' => out.push_str("~1"),
                _ => out.push(ch),
            }
        }
    }
    out
}

/// Convert a byte offset into 1-based (line, column).  Line
/// counts `\n`; column counts bytes since the last newline + 1.
/// Out-of-range offsets clamp to the input length.
#[must_use]
pub fn line_col_of(input: &str, byte_offset: usize) -> (usize, usize) {
    let bytes = input.as_bytes();
    let cap = byte_offset.min(bytes.len());
    let mut line = 1usize;
    let mut col_start = 0usize;
    for (i, b) in bytes[..cap].iter().enumerate() {
        if *b == b'\n' {
            line += 1;
            col_start = i + 1;
        }
    }
    (line, cap - col_start + 1)
}

/// Format a [`ParseError`] into a human-readable diagnostic with
/// path, line:column, message, and a context snippet (N lines
/// before the error, the error line with a caret, M lines after).
#[must_use]
pub fn format_error(input: &str, err: &ParseError, before: usize, after: usize) -> String {
    let (line, col) = line_col_of(input, err.byte_offset);
    let path_disp = if err.path.is_empty() {
        "(root)"
    } else {
        err.path.as_str()
    };
    let snippet = context_snippet(input, line, col, before, after);
    format!(
        "parse error at line {line} col {col} (byte {byte}):\n  path: {path_disp}\n  {msg}\n{snippet}",
        byte = err.byte_offset,
        msg = err.message,
    )
}

/// Build a `before`/error/`after` line snippet around `(line,
/// col)`.  The error line is followed by a caret `^` placed
/// under `col` (1-based).
fn context_snippet(input: &str, line: usize, col: usize, before: usize, after: usize) -> String {
    let lines: Vec<&str> = input.lines().collect();
    let lo = line.saturating_sub(before + 1);
    let hi = (line + after).min(lines.len());
    let width = hi.to_string().len();
    let mut out = String::new();
    use std::fmt::Write;
    for (idx, content) in lines.iter().enumerate().take(hi).skip(lo) {
        let n = idx + 1;
        let _ = writeln!(out, "    {n:>width$} \u{2502} {content}");
        if n == line {
            // caret line — width-wide gutter, vertical-bar, then
            // (col-1) spaces, then ^
            let spaces = " ".repeat(col.saturating_sub(1));
            let _ = writeln!(out, "    {pad:>width$} \u{2502} {spaces}^", pad = "");
        }
    }
    out
}

fn parse_value(bytes: &[u8], i: usize, path: &mut Vec<String>) -> ParseResult {
    if i >= bytes.len() {
        return Err(("unexpected end of input".to_string(), i));
    }
    match bytes[i] {
        b'n' => parse_literal(bytes, i, b"null", Parsed::Null),
        b't' => parse_literal(bytes, i, b"true", Parsed::Bool(true)),
        b'f' => parse_literal(bytes, i, b"false", Parsed::Bool(false)),
        b'"' => parse_string(bytes, i),
        b'-' | b'0'..=b'9' => parse_number(bytes, i),
        b'[' => parse_array(bytes, i, path),
        b'{' => parse_object(bytes, i, path),
        b => Err((format!("unexpected byte {b:#x} at offset {i}"), i)),
    }
}

fn parse_literal(bytes: &[u8], i: usize, word: &[u8], value: Parsed) -> ParseResult {
    if i + word.len() > bytes.len() || &bytes[i..i + word.len()] != word {
        return Err((
            format!(
                "expected `{}` at offset {i}",
                std::str::from_utf8(word).unwrap_or("?")
            ),
            i,
        ));
    }
    Ok((value, i + word.len()))
}

fn parse_string(bytes: &[u8], start: usize) -> ParseResult {
    debug_assert_eq!(bytes[start], b'"');
    let mut i = start + 1;
    let mut out = String::new();
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'"' => return Ok((Parsed::Str(out), i + 1)),
            b'\\' => {
                if i + 1 >= bytes.len() {
                    return Err(("unterminated escape".to_string(), i));
                }
                i += 1;
                match bytes[i] {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'b' => out.push('\u{0008}'),
                    b'f' => out.push('\u{000c}'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        if i + 4 >= bytes.len() {
                            return Err(("truncated \\uXXXX escape".to_string(), i));
                        }
                        let hex = std::str::from_utf8(&bytes[i + 1..i + 5])
                            .map_err(|_| ("non-ASCII in \\uXXXX escape".to_string(), i))?;
                        let cp = u32::from_str_radix(hex, 16)
                            .map_err(|_| ("invalid hex in \\uXXXX escape".to_string(), i))?;
                        if let Some(c) = char::from_u32(cp) {
                            out.push(c);
                        } else {
                            out.push('\u{fffd}');
                        }
                        i += 4;
                    }
                    other => return Err((format!("invalid escape \\{}", other as char), i)),
                }
                i += 1;
            }
            c if c < 0x20 => {
                return Err((format!("raw control byte {c:#x} in string"), i));
            }
            _ => {
                // UTF-8 continuation bytes are passed through verbatim
                // by pushing them into the String via bytes-to-str
                // reconstruction.  We know the input is `&str`, so the
                // slice is valid UTF-8.
                out.push(bytes[i] as char);
                i += 1;
            }
        }
    }
    Err(("unterminated string".to_string(), start))
}

fn parse_number(bytes: &[u8], start: usize) -> ParseResult {
    let mut i = start;
    if bytes[i] == b'-' {
        i += 1;
    }
    // integer part
    if i >= bytes.len() || !bytes[i].is_ascii_digit() {
        return Err(("expected digit in number".to_string(), i));
    }
    if bytes[i] == b'0' {
        i += 1;
    } else {
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
    }
    // fraction
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        if i >= bytes.len() || !bytes[i].is_ascii_digit() {
            return Err(("expected digit after `.`".to_string(), i));
        }
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
    }
    // exponent
    if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
        i += 1;
        if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
            i += 1;
        }
        if i >= bytes.len() || !bytes[i].is_ascii_digit() {
            return Err(("expected digit in exponent".to_string(), i));
        }
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
    }
    let slice = std::str::from_utf8(&bytes[start..i])
        .map_err(|_| ("non-ASCII in number".to_string(), start))?;
    let n: f64 = slice
        .parse()
        .map_err(|_| (format!("invalid number `{slice}`"), start))?;
    Ok((Parsed::Number(n), i))
}

fn parse_array(bytes: &[u8], start: usize, path: &mut Vec<String>) -> ParseResult {
    debug_assert_eq!(bytes[start], b'[');
    let mut i = skip_ws(bytes, start + 1);
    let mut items: Vec<Parsed> = Vec::new();
    if i < bytes.len() && bytes[i] == b']' {
        return Ok((Parsed::Array(items), i + 1));
    }
    let mut idx: usize = 0;
    loop {
        path.push(idx.to_string());
        let res = parse_value(bytes, i, path);
        let (v, j) = match res {
            Ok(ok) => ok,
            Err(e) => {
                // Leave path in place — render_path captures it
                // for the diagnostic.
                return Err(e);
            }
        };
        path.pop();
        items.push(v);
        i = skip_ws(bytes, j);
        if i >= bytes.len() {
            return Err(("unterminated array".to_string(), start));
        }
        match bytes[i] {
            b',' => {
                i = skip_ws(bytes, i + 1);
                idx += 1;
            }
            b']' => return Ok((Parsed::Array(items), i + 1)),
            b => return Err((format!("expected `,` or `]` in array, got {b:#x}"), i)),
        }
    }
}

#[allow(clippy::many_single_char_names)]
fn parse_object(bytes: &[u8], start: usize, path: &mut Vec<String>) -> ParseResult {
    debug_assert_eq!(bytes[start], b'{');
    let mut i = skip_ws(bytes, start + 1);
    let mut fields: Vec<(String, Parsed)> = Vec::new();
    if i < bytes.len() && bytes[i] == b'}' {
        return Ok((Parsed::Object(fields), i + 1));
    }
    loop {
        if i >= bytes.len() || bytes[i] != b'"' {
            return Err(("expected string key in object".to_string(), i));
        }
        let (key, j) = parse_string(bytes, i)?;
        let name = match key {
            Parsed::Str(s) => s,
            _ => unreachable!("parse_string always returns Parsed::Str"),
        };
        i = skip_ws(bytes, j);
        if i >= bytes.len() || bytes[i] != b':' {
            return Err(("expected `:` after object key".to_string(), i));
        }
        i = skip_ws(bytes, i + 1);
        path.push(name.clone());
        let res = parse_value(bytes, i, path);
        let (v, k) = res?;
        path.pop();
        fields.push((name, v));
        i = skip_ws(bytes, k);
        if i >= bytes.len() {
            return Err(("unterminated object".to_string(), start));
        }
        match bytes[i] {
            b',' => i = skip_ws(bytes, i + 1),
            b'}' => return Ok((Parsed::Object(fields), i + 1)),
            b => return Err((format!("expected `,` or `}}` in object, got {b:#x}"), i)),
        }
    }
}

fn skip_ws(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' | b'\n' | b'\r' => i += 1,
            _ => break,
        }
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitives() {
        assert!(matches!(parse("null").unwrap(), Parsed::Null));
        assert!(matches!(parse("true").unwrap(), Parsed::Bool(true)));
        assert!(matches!(parse("false").unwrap(), Parsed::Bool(false)));
    }

    #[test]
    fn numbers() {
        assert!(matches!(parse("0").unwrap(), Parsed::Number(v) if (v - 0.0).abs() < f64::EPSILON));
        assert!(
            matches!(parse("42").unwrap(), Parsed::Number(v) if (v - 42.0).abs() < f64::EPSILON)
        );
        assert!(
            matches!(parse("-17.5").unwrap(), Parsed::Number(v) if (v - (-17.5)).abs() < f64::EPSILON)
        );
        assert!(
            matches!(parse("1.5e3").unwrap(), Parsed::Number(v) if (v - 1500.0).abs() < f64::EPSILON)
        );
    }

    #[test]
    fn strings() {
        let got = parse(r#""hello""#).unwrap();
        assert!(matches!(got, Parsed::Str(ref s) if s == "hello"));
        let got = parse(r#""\"quote\"""#).unwrap();
        assert!(matches!(got, Parsed::Str(ref s) if s == "\"quote\""));
        let got = parse(r#""line\nfeed""#).unwrap();
        assert!(matches!(got, Parsed::Str(ref s) if s == "line\nfeed"));
    }

    #[test]
    fn whitespace_tolerated() {
        assert!(matches!(parse("  null  ").unwrap(), Parsed::Null));
    }

    #[test]
    fn arrays() {
        assert!(matches!(parse("[]").unwrap(), Parsed::Array(ref v) if v.is_empty()));
        let got = parse("[1, 2, 3]").unwrap();
        let Parsed::Array(v) = got else {
            panic!("expected array");
        };
        assert_eq!(v.len(), 3);
        assert!(matches!(v[0], Parsed::Number(n) if (n - 1.0).abs() < f64::EPSILON));
        let nested = parse("[[1], [2, 3]]").unwrap();
        let Parsed::Array(outer) = nested else {
            panic!("expected array");
        };
        assert_eq!(outer.len(), 2);
    }

    #[test]
    fn objects() {
        assert!(matches!(parse("{}").unwrap(), Parsed::Object(ref v) if v.is_empty()));
        let got = parse(r#"{"a": 1, "b": "hi"}"#).unwrap();
        let Parsed::Object(fields) = got else {
            panic!("expected object");
        };
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].0, "a");
        assert!(matches!(fields[0].1, Parsed::Number(n) if (n - 1.0).abs() < f64::EPSILON));
        assert_eq!(fields[1].0, "b");
        assert!(matches!(fields[1].1, Parsed::Str(ref s) if s == "hi"));
    }

    #[test]
    fn nested_mixed() {
        let got = parse(r#"{"items": [1, {"x": true}], "n": null}"#).unwrap();
        let Parsed::Object(fields) = got else {
            panic!("expected object");
        };
        assert_eq!(fields.len(), 2);
        let Parsed::Array(items) = &fields[0].1 else {
            panic!("expected array");
        };
        assert_eq!(items.len(), 2);
        let Parsed::Object(inner) = &items[1] else {
            panic!("expected inner object");
        };
        assert_eq!(inner[0].0, "x");
        assert!(matches!(inner[0].1, Parsed::Bool(true)));
    }

    // ── Q1: structured errors with path / line:col / snippet ────────

    #[test]
    fn err_root_failure_has_empty_path() {
        let err = parse("xyz").unwrap_err();
        assert_eq!(err.path, "");
        assert_eq!(err.byte_offset, 0);
    }

    #[test]
    fn err_inside_array_carries_index_path() {
        let err = parse("[1, 2, 1.]").unwrap_err();
        assert_eq!(err.path, "/2");
    }

    #[test]
    fn err_inside_object_carries_field_path() {
        let err = parse(r#"{"a": 1, "b": 1.}"#).unwrap_err();
        assert_eq!(err.path, "/b");
    }

    #[test]
    fn err_nested_path_is_full_pointer() {
        let err = parse(r#"{"users": [{"name": "x"}, {"name": 1.}]}"#).unwrap_err();
        assert_eq!(err.path, "/users/1/name");
    }

    #[test]
    fn err_path_escapes_slash_and_tilde() {
        // Field "a/b~c" → "/a~1b~0c" per RFC 6901.
        let err = parse(r#"{"a/b~c": 1.}"#).unwrap_err();
        assert_eq!(err.path, "/a~1b~0c");
    }

    #[test]
    fn line_col_basic() {
        assert_eq!(line_col_of("abc", 0), (1, 1));
        assert_eq!(line_col_of("abc", 2), (1, 3));
        assert_eq!(line_col_of("a\nbc", 2), (2, 1));
        assert_eq!(line_col_of("a\nbc", 3), (2, 2));
        assert_eq!(line_col_of("a\nb\nc", 4), (3, 1));
    }

    #[test]
    fn format_error_includes_path_line_col_and_caret() {
        let raw = "{\n  \"x\": 1.\n}";
        let err = parse(raw).unwrap_err();
        let formatted = format_error(raw, &err, 1, 1);
        // Diagnostic mentions path, line, col, message, and a caret.
        assert!(formatted.contains("/x"), "missing path: {formatted}");
        assert!(
            formatted.contains("line 2"),
            "missing line number: {formatted}"
        );
        assert!(formatted.contains('^'), "missing caret: {formatted}");
        assert!(
            formatted.contains("expected digit after `.`"),
            "missing message: {formatted}"
        );
    }

    #[test]
    fn format_error_root_path_renders_as_root_label() {
        let formatted = format_error("xyz", &parse("xyz").unwrap_err(), 0, 0);
        assert!(
            formatted.contains("(root)"),
            "root path label missing: {formatted}"
        );
    }

    #[test]
    fn malformed_collections() {
        assert!(parse("[").is_err());
        assert!(parse("[1,]").is_err());
        assert!(parse("{").is_err());
        assert!(parse(r#"{"a"}"#).is_err());
        assert!(parse(r#"{"a": 1,}"#).is_err());
        assert!(parse(r"{a: 1}").is_err());
    }

    #[test]
    fn malformed_returns_err() {
        assert!(parse("").is_err());
        assert!(parse("nu").is_err());
        assert!(parse("1.").is_err());
        assert!(parse(r#""no-close"#).is_err());
        assert!(parse("null trailing").is_err());
    }
}
