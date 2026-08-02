// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I74 — CDylib extension loader: the `#c` signature every caller derives from (@PLN24 arc A).

//! @PLN24 arc A — the C signature a `#c` binding declares, and the one place it
//! is understood.
//!
//! ```loft
//! pub fn status(conn: integer) -> integer;   #c "PQstatus" "int(void*)"
//! ```
//!
//! **The declaration is the sole authority on the C signature.** That is not a
//! style choice — it is what the architecture probe left standing
//! (`tests/fixtures/c_abi/`). Pointed at deliberately wrong signatures, the
//! runtime caller did not fail: an arity-1 symbol called through an arity-3
//! trampoline returned the right answer (extra register arguments are ignored),
//! and a variadic function through a non-variadic one returned the right answer
//! too, by luck. There is no runtime signal to reconcile a second source of
//! truth against, so the check is here, at compile time, or nowhere.
//!
//! The Rust path (`#native`) is authoritative in the other direction — the
//! `#[loft_native]` macro reads the real Rust signature and generates a marshal
//! bridge, so the loft declaration can be loose. A `#c` library contains no
//! Rust and there is nothing to read; that absence is the whole reason this
//! module exists.
//!
//! Every consumer — the interpreter's trampoline choice, the `--native`
//! `extern "C"` emission, and each target arc E adds — derives its call from
//! [`CSignature::parse`] and never re-derives widths from the loft types. The
//! loft types cannot express them: `integer` is i64 whatever the C function
//! takes.
//!
//! Nothing calls a `#c` function yet (arc A lands inert). This module parses,
//! checks, and reports.

use crate::data::{Data, Type};

/// The data model a target uses for the C integer types, which is the whole of
/// what varies between the platforms loft builds for.
///
/// C type names are **not** fixed-width, and a signature string has to keep
/// meaning the same thing everywhere it is compiled: `long` is 64 bits on Linux
/// and macOS and 32 on Windows, and plain `char` is signed on x86-64 and
/// unsigned on AArch64. So the declaration stays portable and the *target*
/// decides what it means — exactly as a C compiler would read the same header.
/// Resolving instead of rejecting is what lets an author write the signature
/// their system header already shows them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CTarget {
    /// Width of C `long` in bits: 64 on LP64 (Linux, macOS), 32 on LLP64 (Windows).
    pub long_bits: u8,
    /// Whether plain `char` is signed (x86-64) or unsigned (AArch64).
    pub char_signed: bool,
}

impl CTarget {
    /// The target loft is compiling for. Host-derived: `#c` is native-only, and
    /// the cross-compiled targets (wasm) have no C ABI to bind to at all — that
    /// is arc E's subject, not a width question.
    #[must_use]
    pub fn host() -> CTarget {
        CTarget {
            long_bits: if cfg!(windows) { 32 } else { 64 },
            char_signed: !cfg!(any(target_arch = "aarch64", target_arch = "arm")),
        }
    }
}

/// The highest arity a `#c` binding can have.
///
/// The interpreter calls through a fixed ladder of per-arity trampolines
/// (`c_call`), and the ceiling is a fact about the CONTRACT rather than about
/// that caller — the declaration is checked against it, so it lives with the
/// signature and is readable on every build, including the ones with no C
/// caller compiled in at all.
pub const MAX_C_ARITY: usize = 12;

/// A C type as spelled in a `#c` signature.
///
/// Deliberately not a general C type model: it is exactly the vocabulary the
/// binding can carry, so a type that cannot cross is *unrepresentable* rather
/// than represented-and-rejected-later. `Float` is the one exception — it
/// parses so the diagnostic can name the shim, rather than reading as a
/// spelling mistake.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CType {
    Void,
    /// An integer-class scalar: every C integer, and the class that collapses
    /// to one `u64` slot in the caller.
    Int {
        bits: u8,
        signed: bool,
    },
    /// Any pointer. `pointee` is kept for the diagnostic and for the
    /// `--native` extern emission, which has to write a real Rust type.
    Pointer {
        konst: bool,
        pointee: String,
    },
    /// Parsed so it can be REFUSED with a useful message. A float argument
    /// travels in an SSE register, not the integer registers the caller uses —
    /// a different register file, so no amount of casting reaches it. The C
    /// side wraps it in a shim that takes the bit pattern as an integer.
    Float {
        bits: u8,
    },
}

impl CType {
    /// Does this type cross the boundary directly, or does it need a shim?
    #[must_use]
    pub fn is_integer_class(&self) -> bool {
        matches!(self, CType::Int { .. } | CType::Pointer { .. })
    }

    /// The Rust type this C type is declared as in a generated `extern "C"`
    /// block (@PLN24 arc C).
    ///
    /// The declared width is what makes a call correct, and the RETURN width
    /// especially: rustc truncates a 32-bit C return at the ABI boundary, so
    /// `as i64` then sign-extends it properly. A caller that read the same
    /// return as a bare `u64` — which is what a signature-blind trampoline does
    /// — turns -1 into 4294967295.
    ///
    /// Every pointer is `*const c_void`, because pointers share one ABI whatever
    /// they point at. The parser keeps the pointee spelling for diagnostics; the
    /// emission does not need it, which is one prediction the build corrected.
    #[must_use]
    pub fn rust_type(&self) -> &'static str {
        match self {
            CType::Void => "()",
            CType::Int {
                bits: 8,
                signed: true,
            } => "i8",
            CType::Int {
                bits: 8,
                signed: false,
            } => "u8",
            CType::Int {
                bits: 16,
                signed: true,
            } => "i16",
            CType::Int {
                bits: 16,
                signed: false,
            } => "u16",
            CType::Int {
                bits: 32,
                signed: true,
            } => "i32",
            CType::Int {
                bits: 32,
                signed: false,
            } => "u32",
            CType::Int { signed: true, .. } => "i64",
            CType::Int { signed: false, .. } => "u64",
            CType::Pointer { .. } => "*const std::ffi::c_void",
            CType::Float { bits: 32 } => "f32",
            CType::Float { .. } => "f64",
        }
    }

    /// How the type is named back to the author in a diagnostic.
    #[must_use]
    pub fn spelling(&self) -> String {
        match self {
            CType::Void => "void".to_string(),
            CType::Int { bits, signed } => {
                format!(
                    "{}{bits}-bit integer",
                    if *signed { "signed " } else { "unsigned " }
                )
            }
            CType::Pointer { konst, pointee } => {
                format!("{}{pointee} *", if *konst { "const " } else { "" })
            }
            CType::Float { bits } => format!("{bits}-bit float"),
        }
    }
}

/// A parsed `#c` signature: what the C function really takes and returns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CSignature {
    pub symbol: String,
    pub ret: CType,
    pub params: Vec<CType>,
}

impl CSignature {
    /// Parse `"<ret>(<param>, …)"` for `symbol`, resolving widths for `target`.
    ///
    /// # Errors
    ///
    /// Returns the whole message the author should see — a malformed signature,
    /// a C type this binding does not understand, or a variadic parameter list
    /// (which no fixed caller can make). The caller adds only the source
    /// position.
    pub fn parse(symbol: &str, text: &str, target: CTarget) -> Result<CSignature, String> {
        let text = text.trim();
        let open = text.find('(').ok_or_else(|| {
            format!(
                "`{text}` is not a C signature — expected `<return>(<params>)`, e.g. `int(void*)`"
            )
        })?;
        if !text.ends_with(')') {
            return Err(format!(
                "`{text}` is missing its closing `)` — expected `<return>(<params>)`, e.g. `int(void*)`"
            ));
        }
        let ret_src = text[..open].trim();
        let params_src = text[open + 1..text.len() - 1].trim();

        let ret = parse_type(ret_src, target)
            .map_err(|e| format!("in the return type of `{text}`: {e}"))?;

        let mut params = Vec::new();
        if !params_src.is_empty() && params_src != "void" {
            for piece in params_src.split(',') {
                let piece = piece.trim();
                if piece == "..." {
                    return Err(
                        "a variadic C function cannot be bound directly — the call site must set \
                         the ABI's vector-register count, which a fixed caller does not. Wrap it \
                         in an ANSI-C shim with a fixed parameter list and bind that"
                            .to_string(),
                    );
                }
                let t = parse_type(piece, target)
                    .map_err(|e| format!("in parameter `{piece}` of `{text}`: {e}"))?;
                if t == CType::Void {
                    return Err(format!(
                        "`void` is not a parameter type in `{text}` — write `()` for no parameters"
                    ));
                }
                params.push(t);
            }
        }
        Ok(CSignature {
            symbol: symbol.to_string(),
            ret,
            params,
        })
    }

    /// Every type that cannot cross directly, with the reason. Empty when the
    /// signature is bindable as written.
    ///
    /// Separate from `parse` because a float is a perfectly good C type and a
    /// perfectly bad binding: the author needs to be told which one they hit.
    #[must_use]
    pub fn boundary_refusals(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let CType::Float { bits } = self.ret {
            out.push(format!(
                "returns a {bits}-bit float, which comes back in an SSE register the caller does \
                 not read. Wrap `{}` in an ANSI-C shim that returns the bit pattern as an \
                 integer, and bind that",
                self.symbol
            ));
        }
        for (i, p) in self.params.iter().enumerate() {
            if let CType::Float { bits } = p {
                out.push(format!(
                    "parameter {} is a {bits}-bit float, which travels in an SSE register the \
                     caller does not write. Wrap `{}` in an ANSI-C shim taking the bit pattern as \
                     an integer, and bind that",
                    i + 1,
                    self.symbol
                ));
            }
        }
        out
    }
}

/// Resolve one C type name. Unknown spellings are refused rather than guessed:
/// a `#c` declaration nobody can check at runtime must not contain a type
/// nobody checked at compile time either.
fn parse_type(src: &str, target: CTarget) -> Result<CType, String> {
    let src = src.trim();
    if let Some(base) = src.strip_suffix('*') {
        let base = base.trim();
        let (konst, pointee) = match base.strip_prefix("const ") {
            Some(rest) => (true, rest.trim()),
            None => (false, base),
        };
        if pointee.is_empty() {
            return Err(
                "a pointer needs a pointee type, e.g. `void*` or `const char*`".to_string(),
            );
        }
        // The pointee is not resolved: every pointer is one machine word, and
        // the caller passes it as one. It is kept verbatim so the `--native`
        // extern emission can write a faithful Rust type and the diagnostic can
        // echo what was written.
        return Ok(CType::Pointer {
            konst,
            pointee: pointee.to_string(),
        });
    }
    let norm = src.split_whitespace().collect::<Vec<_>>().join(" ");
    let t = match norm.as_str() {
        "void" => CType::Void,
        "char" => CType::Int {
            bits: 8,
            signed: target.char_signed,
        },
        "signed char" => CType::Int {
            bits: 8,
            signed: true,
        },
        "unsigned char" => CType::Int {
            bits: 8,
            signed: false,
        },
        "short" | "short int" | "signed short" => CType::Int {
            bits: 16,
            signed: true,
        },
        "unsigned short" | "unsigned short int" => CType::Int {
            bits: 16,
            signed: false,
        },
        "int" | "signed" | "signed int" => CType::Int {
            bits: 32,
            signed: true,
        },
        "unsigned" | "unsigned int" => CType::Int {
            bits: 32,
            signed: false,
        },
        "long" | "long int" | "signed long" => CType::Int {
            bits: target.long_bits,
            signed: true,
        },
        "unsigned long" | "unsigned long int" => CType::Int {
            bits: target.long_bits,
            signed: false,
        },
        "long long" | "long long int" | "signed long long" => CType::Int {
            bits: 64,
            signed: true,
        },
        "unsigned long long" | "unsigned long long int" => CType::Int {
            bits: 64,
            signed: false,
        },
        "size_t" | "uintptr_t" => CType::Int {
            bits: 64,
            signed: false,
        },
        "ssize_t" | "intptr_t" | "ptrdiff_t" => CType::Int {
            bits: 64,
            signed: true,
        },
        "int8_t" => CType::Int {
            bits: 8,
            signed: true,
        },
        "int16_t" => CType::Int {
            bits: 16,
            signed: true,
        },
        "int32_t" => CType::Int {
            bits: 32,
            signed: true,
        },
        "int64_t" => CType::Int {
            bits: 64,
            signed: true,
        },
        "uint8_t" => CType::Int {
            bits: 8,
            signed: false,
        },
        "uint16_t" => CType::Int {
            bits: 16,
            signed: false,
        },
        "uint32_t" => CType::Int {
            bits: 32,
            signed: false,
        },
        "uint64_t" => CType::Int {
            bits: 64,
            signed: false,
        },
        "_Bool" | "bool" => CType::Int {
            bits: 8,
            signed: false,
        },
        "float" => CType::Float { bits: 32 },
        "double" => CType::Float { bits: 64 },
        other => {
            return Err(format!(
                "`{other}` is not a C type this binding understands. Allowed: the C integer types \
                 (char, short, int, long, long long, size_t, the int*_t / uint*_t family), any \
                 pointer (`void*`, `const char*`), `void`, and `float`/`double` — which then need \
                 a shim. A struct passed by value, or a type from a header, has to go through an \
                 ANSI-C shim that presents it as one of these"
            ));
        }
    };
    Ok(t)
}

/// How many C parameters one loft parameter occupies, and which C types it may
/// occupy them with.
///
/// This is the mapping table, and it lives here alone so that the arity check,
/// the interpreter caller and the `--native` emission cannot drift into three
/// different opinions about what a `vector` looks like from C.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoftCShape {
    /// One integer-class C parameter.
    Scalar,
    /// One pointer.
    Pointer,
    /// A pointer plus an integer count — C has no length-carrying array, so a
    /// loft `vector` is two C parameters. The pointer is valid **for the
    /// duration of the call only**; the elements live in a loft store that may
    /// move afterwards.
    PointerAndCount,
    /// Cannot cross.
    Refused(&'static str),
}

impl LoftCShape {
    #[must_use]
    pub fn params(self) -> usize {
        match self {
            LoftCShape::Scalar | LoftCShape::Pointer => 1,
            LoftCShape::PointerAndCount => 2,
            LoftCShape::Refused(_) => 0,
        }
    }
}

/// What a loft type looks like from C.
#[must_use]
pub fn shape_of(t: &Type) -> LoftCShape {
    match t {
        Type::Integer(_) | Type::Boolean | Type::Character | Type::Enum(_, _, _) => {
            LoftCShape::Scalar
        }
        Type::Text(_) => LoftCShape::Pointer,
        Type::Vector(_, _) => LoftCShape::PointerAndCount,
        // @PLN24 — `#c` is the declared edge of loft's no-runtime-errors rule,
        // and this is where that becomes a compile error instead of a promise.
        // A nullable value has no C representation: the sentinels are ordinary
        // numbers over there (`i64::MIN` is a number, `NULL` is a crash), so a
        // null crossing the boundary is either silent corruption or a fault C
        // takes. Discharging it on the loft side (`?? 0`, `x?`, `match`) keeps
        // the guarantee where loft can actually keep it.
        Type::Optional(_) => LoftCShape::Refused(
            "a nullable value cannot cross into C — C has no null model, so the sentinel would \
             arrive as an ordinary number or a fault. Discharge it first (`?? 0`, `x?`, or a \
             `match`) and declare the parameter non-null",
        ),
        Type::Float | Type::Single => LoftCShape::Refused(
            "a float cannot cross directly — it travels in an SSE register the caller does not \
             touch. Pass the bit pattern as an `integer` through an ANSI-C shim",
        ),
        // A loft record is a position inside a store that the allocator may
        // RELOCATE (`resize_store` reallocs the arena). Handing C an interior
        // pointer that a later claim invalidates is the store-lifetime bug
        // class, not a marshalling detail — so it is refused rather than
        // documented. Pass the fields, or an opaque handle C itself owns.
        Type::Reference(_, _) => LoftCShape::Refused(
            "a reference to a loft record cannot cross — records live in a store that may move \
             them. Pass the fields it needs as scalars, or have C own the object and pass its \
             handle as an `integer`",
        ),
        _ => LoftCShape::Refused(
            "this type has no C representation. Only integers, boolean, character, text, vectors \
             and C-owned handles (as `integer`) cross a `#c` boundary",
        ),
    }
}

/// Check a parsed signature against the loft declaration it annotates.
///
/// Returns every problem, not the first: an author fixing a signature wants the
/// whole list, and each message names the C position so a long parameter list
/// stays navigable.
#[must_use]
pub fn check(
    data: &Data,
    sig: &CSignature,
    params: &[Type],
    ret: &Type,
    void_return: bool,
) -> Vec<String> {
    let mut errs = sig.boundary_refusals();

    // Arity, in C parameters rather than loft ones — a `vector` is two.
    let mut expected = 0usize;
    for (i, p) in params.iter().enumerate() {
        match shape_of(p) {
            LoftCShape::Refused(why) => {
                errs.push(format!("parameter {} cannot be bound: {why}", i + 1));
            }
            shape => expected += shape.params(),
        }
    }
    if errs.is_empty() && expected != sig.params.len() {
        errs.push(format!(
            "the C signature takes {} parameter(s), the loft declaration needs {expected} \
             (a `text` is one `const char*`; a `vector` is a pointer AND a count, because C \
             carries no length)",
            sig.params.len()
        ));
    }

    // Per-parameter class. Only reached when the arity agrees, so the indices
    // line up and a mismatch names a real pair rather than a shifted one.
    if errs.is_empty() {
        let mut c = 0usize;
        for (i, p) in params.iter().enumerate() {
            match shape_of(p) {
                LoftCShape::Scalar => {
                    // A C POINTER is allowed here, and deliberately: it is the
                    // handle convention (`PGconn *` held as a loft `integer`),
                    // and it has to work in both directions or it is not a
                    // convention at all — `lc_open` hands one back, `lc_read`
                    // takes it. loft has no type that distinguishes a handle
                    // from an integer, which is exactly why the plan chose
                    // `integer` for it. The ABI is identical: both are one slot.
                    if !matches!(sig.params[c], CType::Int { .. } | CType::Pointer { .. }) {
                        errs.push(format!(
                            "parameter {} is `{}` in loft but `{}` in C — a scalar needs a C \
                             integer type, or a pointer if it is a handle",
                            i + 1,
                            data.type_name_str(p),
                            sig.params[c].spelling()
                        ));
                    }
                    c += 1;
                }
                LoftCShape::Pointer => {
                    if !matches!(sig.params[c], CType::Pointer { .. }) {
                        errs.push(format!(
                            "parameter {} is `{}` in loft but `{}` in C — a text crosses as a \
                             NUL-terminated `const char*`",
                            i + 1,
                            data.type_name_str(p),
                            sig.params[c].spelling()
                        ));
                    }
                    c += 1;
                }
                LoftCShape::PointerAndCount => {
                    if !matches!(sig.params[c], CType::Pointer { .. }) {
                        errs.push(format!(
                            "parameter {} is `{}` in loft, so C parameter {} must be the element \
                             pointer, not `{}`",
                            i + 1,
                            data.type_name_str(p),
                            c + 1,
                            sig.params[c].spelling()
                        ));
                    }
                    if !matches!(sig.params[c + 1], CType::Int { .. }) {
                        errs.push(format!(
                            "parameter {} is `{}` in loft, so C parameter {} must be the element \
                             count, not `{}`",
                            i + 1,
                            data.type_name_str(p),
                            c + 2,
                            sig.params[c + 1].spelling()
                        ));
                    }
                    c += 2;
                }
                LoftCShape::Refused(_) => {}
            }
        }
    }

    // The return, which is the half the probe found broken. A 32-bit C return
    // read back as 64 bits turns -1 into 4294967295 — no crash, a plausible
    // large positive. The declared width is what the caller truncates to, so a
    // return with no declared width is the one thing that cannot be defaulted.
    if void_return {
        if sig.ret != CType::Void {
            errs.push(format!(
                "the loft declaration returns nothing but the C signature returns `{}`",
                sig.ret.spelling()
            ));
        }
    } else {
        // `shape_of` answers for the ARGUMENT direction, where a nullable type is
        // genuinely unrepresentable — loft has no value to hand C for it. A
        // RETURN is not symmetric: C's NULL is exactly "no string", so a
        // `char *` coming back is the one place a `τ?` is the HONEST type, and
        // the only one where loft can see the null the crossing already carries.
        // Declared `text`, the same NULL still arrives as loft's content
        // sentinel — spelling it `text?` does not add the null, it makes the
        // null-flow analysis demand a discharge for it.
        let ret_shape = match ret {
            Type::Optional(inner)
                if matches!(**inner, Type::Text(_))
                    && matches!(sig.ret, CType::Pointer { ref pointee, .. }
                        if pointee.ends_with("char")) =>
            {
                LoftCShape::Pointer
            }
            other => shape_of(other),
        };
        match (&sig.ret, ret_shape) {
            (CType::Void, _) => errs.push(format!(
                "the loft declaration returns `{}` but the C signature returns `void`",
                data.type_name_str(ret)
            )),
            (_, LoftCShape::Refused(why)) => {
                errs.push(format!("the return type cannot be bound: {why}"));
            }
            // A `char *` return bound to loft `text`. The bytes are COPIED up to
            // the first NUL and the pointer is never freed, which is the one
            // ownership answer C's type system cannot give: `strerror` and
            // `PQerrorMessage` hand back storage the caller must NOT free, while
            // `strdup` hands back storage it must. `const` does not separate them
            // (POSIX spells both `char *`), so guessing from the signature would
            // free static memory on a wrong guess — the failure that cannot be
            // recovered from. Borrowed is therefore the only default, and a
            // caller-frees function goes through the plan's shim, which is what
            // the shims are for. Stated at the boundary in PACKAGES.md.
            (CType::Pointer { pointee, .. }, LoftCShape::Pointer) if pointee.ends_with("char") => {}
            // A text return from a pointer that is not spelled `char *`. The
            // declaration is the sole authority here, so it has to SAY it means a
            // string — `void *` bound to `text` is either a mistake or a handle
            // that wanted `integer`, and nothing at runtime tells the two apart.
            // (This is the one decision the pointee spelling carries; everywhere
            // else it is diagnostics only, because pointers share one ABI.)
            (CType::Pointer { pointee, .. }, LoftCShape::Pointer) => errs.push(format!(
                "`{}` returns `{pointee}*`, which cannot come back as a loft `text` — spell the \
                 return `char*` if it really is a C string, or bind it as an `integer` if it is \
                 an opaque handle",
                sig.symbol
            )),
            (CType::Int { .. }, LoftCShape::Scalar) => {}
            // A C function returning a pointer bound to a loft `integer` is the
            // handle convention (`PGconn *`), and it is deliberate.
            (CType::Pointer { .. }, LoftCShape::Scalar) => {}
            (c, _) => errs.push(format!(
                "the loft declaration returns `{}` but the C signature returns `{}`",
                data.type_name_str(ret),
                c.spelling()
            )),
        }
    }
    errs
}

/// Read the signature a definition declared, if any. The one accessor, so a
/// consumer cannot accidentally use the raw string.
#[must_use]
pub fn of(data: &Data, def_nr: u32, target: CTarget) -> Option<Result<CSignature, String>> {
    let d = data.def(def_nr);
    if d.c_sig.is_empty() {
        return None;
    }
    Some(CSignature::parse(&d.c_symbol, &d.c_sig, target))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LP64: CTarget = CTarget {
        long_bits: 64,
        char_signed: true,
    };
    const WIN: CTarget = CTarget {
        long_bits: 32,
        char_signed: true,
    };

    fn data() -> Data {
        Data::new()
    }

    fn int() -> Type {
        Type::Integer(crate::data::IntegerSpec::wide())
    }

    fn sig(s: &str) -> CSignature {
        CSignature::parse("sym", s, LP64).expect("parses")
    }

    #[test]
    fn parses_the_shapes_a_c_header_actually_shows() {
        let s = sig("int(void*)");
        assert_eq!(
            s.ret,
            CType::Int {
                bits: 32,
                signed: true
            }
        );
        assert_eq!(s.params.len(), 1);
        assert!(matches!(s.params[0], CType::Pointer { konst: false, .. }));

        let s = sig("const char*(void*, int)");
        assert!(matches!(s.ret, CType::Pointer { konst: true, .. }));
        assert_eq!(s.params.len(), 2);

        assert_eq!(sig("void()").params.len(), 0);
        assert_eq!(
            sig("void(void)").params.len(),
            0,
            "`(void)` is C for no parameters"
        );
        assert_eq!(
            sig("size_t(const char*)").ret,
            CType::Int {
                bits: 64,
                signed: false
            }
        );
    }

    /// The reason widths are resolved against a target rather than fixed: the
    /// SAME signature string has to keep meaning what the system header means.
    #[test]
    fn long_is_not_one_width() {
        assert_eq!(
            CSignature::parse("s", "long(int)", LP64).unwrap().ret,
            CType::Int {
                bits: 64,
                signed: true
            }
        );
        assert_eq!(
            CSignature::parse("s", "long(int)", WIN).unwrap().ret,
            CType::Int {
                bits: 32,
                signed: true
            },
            "LLP64 — a `long` return truncated to 64 bits on Windows would be the \
             loft-libs-net bug with a different sign"
        );
        assert_eq!(
            CSignature::parse("s", "long long(int)", WIN).unwrap().ret,
            CType::Int {
                bits: 64,
                signed: true
            },
            "`long long` is 64 everywhere, which is why it is spelled differently"
        );
    }

    #[test]
    fn char_signedness_follows_the_target() {
        let signed = CTarget {
            long_bits: 64,
            char_signed: true,
        };
        let unsigned = CTarget {
            long_bits: 64,
            char_signed: false,
        };
        assert_eq!(
            CSignature::parse("s", "char()", signed).unwrap().ret,
            CType::Int {
                bits: 8,
                signed: true
            }
        );
        assert_eq!(
            CSignature::parse("s", "char()", unsigned).unwrap().ret,
            CType::Int {
                bits: 8,
                signed: false
            },
            "AArch64 — and the difference decides whether a returned byte is negative"
        );
        assert_eq!(
            CSignature::parse("s", "signed char()", unsigned)
                .unwrap()
                .ret,
            CType::Int {
                bits: 8,
                signed: true
            },
            "an explicit spelling overrides the target, as in C"
        );
    }

    #[test]
    fn refuses_what_it_cannot_check() {
        let e = CSignature::parse("s", "struct timeval(int)", LP64).unwrap_err();
        assert!(e.contains("not a C type"), "{e}");
        assert!(
            e.contains("shim"),
            "the message has to say what to do instead: {e}"
        );

        let e = CSignature::parse("s", "int(int, ...)", LP64).unwrap_err();
        assert!(e.contains("variadic"), "{e}");

        let e = CSignature::parse("s", "int", LP64).unwrap_err();
        assert!(e.contains("not a C signature"), "{e}");
    }

    /// A float is a good C type and a bad binding. It parses so the author is
    /// told which of those they hit.
    #[test]
    fn a_float_parses_and_is_then_refused_with_the_cure() {
        let s = sig("double(double)");
        let refusals = s.boundary_refusals();
        assert_eq!(
            refusals.len(),
            2,
            "the return AND the parameter: {refusals:?}"
        );
        assert!(refusals[0].contains("SSE register"), "{:?}", refusals[0]);
        assert!(refusals[0].contains("shim"), "{:?}", refusals[0]);
        assert!(sig("int(int)").boundary_refusals().is_empty());
    }

    #[test]
    fn arity_counts_c_parameters_not_loft_ones() {
        let ints = vec![Type::Vector(Box::new(int()), crate::data::Deps::none())];
        let ret = Type::Integer(crate::data::IntegerSpec::wide());
        // A vector is a pointer AND a count.
        assert!(check(&data(), &sig("long(const long*, long)"), &ints, &ret, false).is_empty());
        let errs = check(&data(), &sig("long(const long*)"), &ints, &ret, false);
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("carries no length"), "{}", errs[0]);
    }

    #[test]
    fn a_loft_record_is_refused_because_the_store_can_move_it() {
        let params = vec![Type::Reference(0, crate::data::Deps::none())];
        let ret = Type::Integer(crate::data::IntegerSpec::wide());
        let errs = check(&data(), &sig("long(void*)"), &params, &ret, false);
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("may move them"), "{}", errs[0]);
    }

    /// The handle convention: C returns `PGconn *`, loft holds it as `integer`.
    #[test]
    fn a_returned_pointer_may_be_held_as_an_integer() {
        let ret = Type::Integer(crate::data::IntegerSpec::wide());
        assert!(
            check(
                &data(),
                &sig("void*(const char*)"),
                &[Type::Text(crate::data::Deps::none())],
                &ret,
                false
            )
            .is_empty()
        );
    }

    /// The concession, made concrete: `#c` is the declared edge of loft's
    /// no-runtime-errors rule, and this is the compile error that keeps the
    /// guarantee where loft can actually keep it.
    #[test]
    fn a_nullable_value_cannot_cross() {
        let params = vec![Type::Optional(Box::new(int()))];
        let errs = check(&data(), &sig("long(long)"), &params, &int(), false);
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("no null model"), "{}", errs[0]);
        assert!(
            errs[0].contains("??"),
            "the message must name the cure: {}",
            errs[0]
        );
        assert!(
            check(&data(), &sig("long(long)"), &[int()], &int(), false).is_empty(),
            "and the non-null sibling is fine — otherwise this proves nothing"
        );
    }

    /// The handle convention has to work in BOTH directions: `lc_open` returns
    /// `void*` into a loft `integer`, and `lc_read` takes that integer back as
    /// `void*`. Allowing only the return half made the fixture's open/read/close
    /// cycle unbindable — half a convention.
    #[test]
    fn a_handle_crosses_in_both_directions() {
        let ret = int();
        assert!(
            check(&data(), &sig("void*(long)"), &[int()], &ret, false).is_empty(),
            "out: a pointer return held as an integer"
        );
        assert!(
            check(&data(), &sig("long(void*)"), &[int()], &ret, false).is_empty(),
            "and back in: that integer passed as the pointer"
        );
    }

    #[test]
    fn a_void_return_and_a_value_return_are_not_interchangeable() {
        let ret = Type::Integer(crate::data::IntegerSpec::wide());
        let errs = check(
            &data(),
            &sig("void(int)"),
            std::slice::from_ref(&ret),
            &ret,
            false,
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("returns `void`"), "{}", errs[0]);

        let errs = check(
            &data(),
            &sig("int(int)"),
            std::slice::from_ref(&ret),
            &Type::Void,
            true,
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("returns nothing"), "{}", errs[0]);
    }
}
