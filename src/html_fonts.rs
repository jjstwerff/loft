// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @F54 — Browser / WASM target (--html / --native-wasm)

//! @PLN146 F5/F6 — turn `[[font]]` declarations into the two things a browser page
//! needs so text draws in the font the program asked for.
//!
//! Under `--html` a font PATH is not opened: `gl_load_font("fonts/Foo.ttf")` reaches
//! the page as the base name `Foo`, and the bridge resolves that to a CSS family
//! (`doc/loft-gl-wasm.js`, `familyFor`). So the page must already carry the family,
//! under exactly that name. Two mechanics decide whether it works, and each is one
//! half of this module:
//!
//! - **The name must match.** A declared `family` that differs from the base name
//!   the program passes resolves to a generic fallback with nothing on stderr, so
//!   [`validate`] refuses the drift at build time rather than leaving it to be found
//!   as a wrong-looking page.
//! - **The font must have arrived.** A webfont is still loading while the first
//!   frame draws, and the bridge caches a font's family per handle — so one early
//!   `gl_load_font` locks that handle to the fallback for the whole run.
//!   [`boot_await_js`] holds `loft_start` until every declared family is loaded.
//!
//! Three sources, one declaration ([`crate::manifest::FontDecl`]): a family the
//! browser already has (nothing emitted), a font file we serve (`@font-face`), or a
//! provider's stylesheet (`<link>`). The native side of the same declaration is the
//! file beside the game, which is what `native` names.

use crate::manifest::FontDecl;

/// One validated declaration: the family, and where the browser gets it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageFont {
    /// The CSS family, which is also the base name the program looks it up by.
    pub family: String,
    /// A font file the page fetches — emitted as an `@font-face` rule.
    pub url: Option<String>,
    /// A stylesheet that declares the family — emitted as a `<link>`.
    pub stylesheet: Option<String>,
}

/// The base name a font path reaches the browser bridge as: the file name without
/// its directories or extension, which is what `familyFor` keys on.
#[must_use]
pub fn base_name(path: &str) -> String {
    let file = path.rsplit(['/', '\\']).next().unwrap_or(path);
    match file.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem.to_string(),
        _ => file.to_string(),
    }
}

/// Reject a value that cannot be embedded in the page's CSS or HTML. A family lands
/// inside a quoted CSS string and a URL inside a quoted attribute, so a quote or an
/// angle bracket in either would end the construct it sits in.
fn embeddable(what: &str, value: &str) -> Result<(), String> {
    if let Some(bad) = value
        .chars()
        .find(|c| matches!(c, '"' | '\'' | '<' | '>' | '\\' | '\n' | '\r'))
    {
        return Err(format!(
            "loft.toml: [[font]] {what} `{value}` contains `{bad}`, which cannot appear \
             in the page's CSS or HTML"
        ));
    }
    Ok(())
}

/// Check the `[[font]]` declarations and answer the page's font list.
///
/// The load-bearing rule is the last one: `family` must equal the base name of
/// `native`, because `native` is the path the program passes to `gl_load_font` and
/// that path's base name is the only thing the browser sees. When they differ the
/// page registers one family and the program asks for another, text draws in a
/// generic fallback, and nothing anywhere says so.
///
/// # Errors
/// One message per problem found, joined by newlines — a build-stopping diagnostic.
pub fn validate(fonts: &[FontDecl]) -> Result<Vec<PageFont>, String> {
    let mut out: Vec<PageFont> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    for f in fonts {
        let family = f.family.clone().unwrap_or_default();
        if family.trim().is_empty() {
            errors.push(
                "loft.toml: a [[font]] declares no `family` — the family name is also the \
                 name the program looks the font up by, so it cannot be left out"
                    .to_string(),
            );
            continue;
        }
        if let Err(e) = embeddable("family", &family) {
            errors.push(e);
            continue;
        }
        for (what, v) in [("url", &f.url), ("stylesheet", &f.stylesheet)] {
            if let Some(v) = v
                && let Err(e) = embeddable(what, v)
            {
                errors.push(e);
            }
        }
        if f.url.is_some() && f.stylesheet.is_some() {
            errors.push(format!(
                "loft.toml: [[font]] `{family}` declares both `url` and `stylesheet` — a \
                 family comes from one source or the other, so name the one the browser \
                 should use"
            ));
            continue;
        }
        if let Some(native) = f.native.as_deref() {
            let base = base_name(native);
            if base != family {
                errors.push(format!(
                    "loft.toml: [[font]] `family = \"{family}\"` does not match \
                     `native = \"{native}\"`, whose base name is `{base}`.\n  \
                     The browser resolves a font by that base name, so the page would \
                     register `{family}` while the program asks for `{base}` and text \
                     would silently draw in a fallback.\n  \
                     Rename one of the two so they agree."
                ));
                continue;
            }
        }
        let entry = PageFont {
            family: family.clone(),
            url: f.url.clone(),
            stylesheet: f.stylesheet.clone(),
        };
        match out.iter().find(|p| p.family == family) {
            // Declared twice identically — the app and a library it uses can name the
            // same font, and agreeing about it is not an error.
            Some(prev) if *prev == entry => {}
            Some(_) => errors.push(format!(
                "loft.toml: [[font]] `{family}` is declared twice with different sources — \
                 the page can serve one family from one place only"
            )),
            None => out.push(entry),
        }
    }
    if errors.is_empty() {
        Ok(out)
    } else {
        Err(errors.join("\n"))
    }
}

/// The `src: … format(…)` hint for a font file, read off its extension. An
/// unrecognised extension gets no hint — browsers sniff the bytes, and a WRONG hint
/// is what makes them refuse the file.
fn format_hint(url: &str) -> Option<&'static str> {
    let ext = url
        .split('?')
        .next()
        .unwrap_or(url)
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "woff2" => Some("woff2"),
        "woff" => Some("woff"),
        "ttf" => Some("truetype"),
        "otf" => Some("opentype"),
        _ => None,
    }
}

/// The `<head>` fragment: one `@font-face` per file source and one `<link>` per
/// stylesheet source. Empty when nothing needs bringing — a family the browser
/// already has costs the page nothing.
#[must_use]
pub fn head_html(fonts: &[PageFont]) -> String {
    use std::fmt::Write as _;
    let mut faces = String::new();
    let mut links = String::new();
    for f in fonts {
        if let Some(url) = &f.url {
            let fam = &f.family;
            let fmt = match format_hint(url) {
                Some(h) => format!(" format(\"{h}\")"),
                None => String::new(),
            };
            let _ = write!(
                faces,
                "@font-face{{font-family:\"{fam}\";src:url(\"{url}\"){fmt}}}"
            );
        }
        if let Some(href) = &f.stylesheet {
            let _ = write!(links, "\n<link rel=\"stylesheet\" href=\"{href}\">");
        }
    }
    let mut out = String::new();
    if !faces.is_empty() {
        let _ = write!(out, "\n<style>{faces}</style>");
    }
    out.push_str(&links);
    out
}

/// The statement that holds `loft_start` until every declared family is loaded —
/// F6, and two lines of it.
///
/// It awaits **every** declared family, including one the browser already has:
/// `document.fonts.load` on a resident family resolves at once, and asking makes the
/// declaration mean the same thing whichever source it names. A family that never
/// arrives (a blocked CDN, a 404) is caught rather than thrown — a remote font is a
/// third-party dependency and the page degrades to a fallback instead of not
/// starting. Emitted inside an `async` function, so it is a bare `await`.
#[must_use]
pub fn boot_await_js(fonts: &[PageFont]) -> String {
    if fonts.is_empty() {
        return String::new();
    }
    let list = fonts
        .iter()
        .map(|f| format!("\"{}\"", f.family))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "\n  // @PLN146 F6 — a webfont is still loading while the first frame draws, and\n  \
         // the bridge caches a font's family per handle, so one early gl_load_font would\n  \
         // lock that handle to a fallback for the whole run.  Hold loft_start until every\n  \
         // declared family has arrived.\n  \
         await Promise.all([{list}].map(f=>document.fonts.load('16px \"'+f+'\"').catch(()=>{{}})));"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decl(family: &str, native: Option<&str>, url: Option<&str>) -> FontDecl {
        FontDecl {
            family: Some(family.to_string()),
            native: native.map(str::to_string),
            url: url.map(str::to_string),
            stylesheet: None,
        }
    }

    #[test]
    fn base_name_is_what_the_browser_sees() {
        assert_eq!(base_name("fonts/PressStart2P.ttf"), "PressStart2P");
        assert_eq!(base_name("a\\b\\Foo.woff2"), "Foo");
        assert_eq!(base_name("Foo"), "Foo");
        // A leading dot is the whole name, not an empty stem with an extension.
        assert_eq!(base_name(".hidden"), ".hidden");
    }

    #[test]
    fn a_family_that_drifts_from_its_native_path_is_refused() {
        // The whole point of F5: this page would register `Press Start 2P` while the
        // program asks for `PressStart2P`, and text would draw in a fallback.
        let err = validate(&[decl("Press Start 2P", Some("fonts/PressStart2P.ttf"), None)])
            .expect_err("drift must be refused");
        assert!(err.contains("PressStart2P"), "{err}");
        assert!(err.contains("does not match"), "{err}");
        // The control: the same declaration, spelled consistently, passes.
        let ok = validate(&[decl("PressStart2P", Some("fonts/PressStart2P.ttf"), None)])
            .expect("matching names are accepted");
        assert_eq!(ok.len(), 1);
    }

    #[test]
    fn one_family_cannot_come_from_two_places() {
        let both = FontDecl {
            family: Some("Foo".into()),
            native: None,
            url: Some("Foo.woff2".into()),
            stylesheet: Some("https://example.test/foo.css".into()),
        };
        assert!(validate(&[both]).is_err());
        // Two declarations that AGREE are one font, not a conflict — an app and a
        // library it uses can both name the font they draw with.
        let twice = vec![
            decl("Foo", None, Some("Foo.woff2")),
            decl("Foo", None, Some("Foo.woff2")),
        ];
        assert_eq!(validate(&twice).expect("agreeing duplicates").len(), 1);
        // Disagreeing ones are.
        let clash = vec![
            decl("Foo", None, Some("Foo.woff2")),
            decl("Foo", None, Some("Bar.woff2")),
        ];
        assert!(validate(&clash).is_err());
    }

    #[test]
    fn a_name_that_would_break_out_of_the_page_is_refused() {
        assert!(validate(&[decl("Fo\"o", None, None)]).is_err());
        assert!(validate(&[decl("Foo", None, Some("a\"><script>"))]).is_err());
        assert!(validate(&[FontDecl::default()]).is_err());
    }

    #[test]
    fn each_source_emits_its_own_shape() {
        let fonts = validate(&[
            // resident: the browser has it, so the page brings nothing
            decl("DejaVu Sans", Some("fonts/DejaVu Sans.ttf"), None),
            // our own server: an @font-face
            decl(
                "LoftProbeFace",
                Some("fonts/LoftProbeFace.ttf"),
                Some("fonts/LoftProbeFace.woff2"),
            ),
            // a provider: a <link>
            FontDecl {
                family: Some("LoftCdnFace".into()),
                native: None,
                url: None,
                stylesheet: Some("https://example.test/cdn.css".into()),
            },
        ])
        .expect("three valid sources");
        let head = head_html(&fonts);
        assert!(head.contains("@font-face{font-family:\"LoftProbeFace\";src:url(\"fonts/LoftProbeFace.woff2\") format(\"woff2\")}"), "{head}");
        assert!(
            head.contains("<link rel=\"stylesheet\" href=\"https://example.test/cdn.css\">"),
            "{head}"
        );
        // The resident family costs the page nothing — no rule, no link.
        assert!(!head.contains("DejaVu Sans"), "{head}");
        // ...but it IS awaited, because the await is what makes the three
        // declarations mean the same thing.
        let js = boot_await_js(&fonts);
        for fam in ["DejaVu Sans", "LoftProbeFace", "LoftCdnFace"] {
            assert!(js.contains(&format!("\"{fam}\"")), "{js}");
        }
        assert!(js.contains("document.fonts.load"), "{js}");
        // Nothing declared, nothing emitted.
        assert_eq!(head_html(&[]), "");
        assert_eq!(boot_await_js(&[]), "");
    }

    #[test]
    fn a_format_hint_is_given_only_when_it_is_known() {
        let f = |u: &str| head_html(&validate(&[decl("F", None, Some(u))]).expect("valid"));
        assert!(f("a/F.woff2").contains("format(\"woff2\")"));
        assert!(f("a/F.ttf").contains("format(\"truetype\")"));
        assert!(f("a/F.otf").contains("format(\"opentype\")"));
        // A wrong hint makes a browser REFUSE the file, so an unknown extension
        // gets none and the bytes decide.
        assert!(!f("/serve?id=7").contains("format("));
    }
}
