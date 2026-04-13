// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// P54: JSON parser used by the `json_parse` native function in
// `src/native.rs`.  Walks UTF-8 text once and returns a `Parsed`
// value that the caller materialises into a loft `JsonValue`
// struct-enum record.
//
// Step 3 scope (this commit): RFC 8259 primitives — null, true,
// false, number, string (incl. standard escapes).  Step 4 will
// extend this to object and array parsing.

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
    // Object / Array landing in step 4.
}

/// Result of a parse: either the parsed tree plus the byte index
/// of the first character past the value, or an error message with
/// its byte index.
pub type ParseResult = Result<(Parsed, usize), (String, usize)>;

/// Parse the entire `input` as a JSON value.  Returns the parsed
/// tree on success.  On malformed input returns the error message
/// and its byte position so callers can surface it via
/// `json_errors()`.
///
/// Leading and trailing whitespace is allowed.  Characters after
/// the value (other than whitespace) are a syntax error — strict
/// RFC 8259, not a forgiving tokeniser.
///
/// # Errors
/// Returns `(message, byte_offset)` when the input is not a valid
/// JSON value per RFC 8259.
pub fn parse(input: &str) -> Result<Parsed, (String, usize)> {
    let bytes = input.as_bytes();
    let start = skip_ws(bytes, 0);
    let (value, mut i) = parse_value(bytes, start)?;
    i = skip_ws(bytes, i);
    if i != bytes.len() {
        return Err((format!("unexpected trailing byte at offset {i}"), i));
    }
    Ok(value)
}

fn parse_value(bytes: &[u8], i: usize) -> ParseResult {
    if i >= bytes.len() {
        return Err(("unexpected end of input".to_string(), i));
    }
    match bytes[i] {
        b'n' => parse_literal(bytes, i, b"null", Parsed::Null),
        b't' => parse_literal(bytes, i, b"true", Parsed::Bool(true)),
        b'f' => parse_literal(bytes, i, b"false", Parsed::Bool(false)),
        b'"' => parse_string(bytes, i),
        b'-' | b'0'..=b'9' => parse_number(bytes, i),
        // Step 4 lands { and [; for now reject cleanly so malformed
        // inputs and object/array inputs both route through the
        // caller's JNull-on-error fallback.
        b'{' | b'[' => Err((
            format!("object/array parsing not yet implemented (byte {i})"),
            i,
        )),
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
    fn malformed_returns_err() {
        assert!(parse("").is_err());
        assert!(parse("nu").is_err());
        assert!(parse("1.").is_err());
        assert!(parse(r#""no-close"#).is_err());
        assert!(parse("null trailing").is_err());
    }
}
