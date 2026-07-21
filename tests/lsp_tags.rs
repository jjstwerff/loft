// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN63 T1/T2 — the tracker-tag integration (`loft::lsp::TagIndex` + tag
// detection).  Uses a SYNTHETIC index written to the test temp dir: CI has no
// generated `index/tags.json` (it's gitignored), so the tests can't read the
// repo's real one.

use std::fs;
use std::path::PathBuf;

use loft::lsp::{TagIndex, render_tag_markdown, tag_at, tags_in};

/// Write a minimal `<tmp>/index/{tags,features}.json` and return the index dir.
fn synthetic_index() -> PathBuf {
    let idx = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("tagidx/index");
    fs::create_dir_all(&idx).unwrap();
    fs::write(
        idx.join("tags.json"),
        r#"{
          "@F1":  [{"file":"a.md","line":1,"context":"see @F1 here"}],
          "@GH5": [{"file":"b.md","line":2,"context":"@GH5 tracked"}],
          "@P9":  [{"file":"PROBLEMS.md","line":3,"context":"@P9 the bug"}],
          "broken": [{"tag":"@P99","refs":["z.md:1"]}]
        }"#,
    )
    .unwrap();
    fs::write(
        idx.join("features.json"),
        r#"[{"number":1,"title":"Keyed collections","kind":"feature",
             "body":"Look up records by a key field.\n"}]"#,
    )
    .unwrap();
    idx
}

#[test]
fn tag_lookup_renders_feature_title_url_and_summary() {
    let idx = TagIndex::load(synthetic_index().to_str().unwrap()).expect("loads the index");

    // @F1 — a feature: title + first-paragraph summary from features.json, and
    // the features-repo issue URL.
    let f = idx.lookup("@F1").expect("@F1 resolves");
    assert_eq!(f.kind, "feature");
    assert_eq!(f.title.as_deref(), Some("Keyed collections"));
    assert!(
        f.summary
            .as_deref()
            .unwrap_or_default()
            .contains("Look up records"),
        "summary is the body's first paragraph"
    );
    assert_eq!(
        f.url.as_deref(),
        Some("https://github.com/loft-lang/features/issues/1")
    );
    assert_eq!(f.references, 1);
    let md = render_tag_markdown(&f);
    assert!(
        md.contains("**@F1**")
            && md.contains("Keyed collections")
            && md.contains("features/issues/1"),
        "markdown carries tag + title + link: {md}"
    );

    // @GH5 — the loft-repo issue URL.
    assert_eq!(
        idx.lookup("@GH5").and_then(|t| t.url).as_deref(),
        Some("https://github.com/loft-lang/loft/issues/5")
    );

    // @P9 — a problem: no issue URL, summary from the reference context.
    let p = idx.lookup("@P9").expect("@P9 resolves");
    assert_eq!(p.kind, "problem");
    assert!(p.url.is_none(), "P-issues have no issue URL");
    assert_eq!(p.references, 1);

    // An unindexed local tag → None (T3 territory).
    assert!(idx.lookup("@P999").is_none(), "unindexed @P → None");
}

#[test]
fn broken_tags_come_from_the_index_verdict() {
    let idx = TagIndex::load(synthetic_index().to_str().unwrap()).unwrap();
    // T3 consumes the scanner's `broken` array verbatim.
    assert!(idx.is_broken("@P99"), "@P99 is listed in the broken array");
    assert!(!idx.is_broken("@P9"), "@P9 is a valid, referenced tag");
    assert!(
        !idx.is_broken("@F1"),
        "features are never broken-validated by the scanner"
    );
    assert!(
        !idx.is_broken("@P123"),
        "an unindexed tag is not (yet) known-broken"
    );
}

#[test]
fn tag_detection_finds_tokens_and_ranges() {
    let text = "// tracks @F7 and @GH247 today\ncode line\n";
    // A cursor inside `@F7` (1-based line 1, col 12) resolves the tag; a
    // non-tag position does not.
    assert_eq!(tag_at(text, 1, 12).as_deref(), Some("@F7"));
    assert_eq!(tag_at(text, 2, 1), None);

    let all = tags_in(text);
    let names: Vec<&str> = all.iter().map(|(t, ..)| t.as_str()).collect();
    assert_eq!(names, vec!["@F7", "@GH247"]);
    // Range = (tag, line 1-based, start 0-based, end 0-based) — slices back to the tag.
    let (_, line, s, e) = &all[0];
    assert_eq!(*line, 1);
    assert_eq!(
        &text.lines().next().unwrap()[*s as usize..*e as usize],
        "@F7"
    );
}
