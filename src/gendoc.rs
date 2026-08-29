// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I79 — Documentation generator
//
// Generate standard library HTML pages from the documented default/*.loft files.
// Run with: cargo run --bin gendoc

use loft::documentation::typst_escape;
use loft::documentation::{
    StdlibSection, TopicSource, build_nav, gather_topic_info, generate_docs, get_topic_sources,
    page_html, render_topic_body, render_topic_typst,
};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs;

// ---  Data model  ---

#[derive(Debug)]
enum Entry {
    /// A named section header (// --- Name ---).
    Section(String),
    /// A public item or section description.
    /// sig is empty for section-description items.
    Item { sig: String, doc: Vec<String> },
}

struct SectionFull {
    id: String,
    name: String,
    items: Vec<(String, Vec<String>)>, // (signature, doc_lines)
}

// ---  Entry point  ---

fn main() -> std::io::Result<()> {
    let version = env!("CARGO_PKG_VERSION");
    let files = [
        "default/01_code.loft",
        "default/02_files.loft",
        "default/03_text.loft",
    ];

    let mut entries: Vec<Entry> = Vec::new();
    for path in &files {
        match fs::read_to_string(path) {
            Ok(content) => parse_loft(&content, &mut entries),
            Err(e) => eprintln!("Cannot read {path}: {e}"),
        }
    }

    let sections: Vec<SectionFull> = build_sections(&entries)
        .into_iter()
        .filter(|s| s.items.iter().any(|(sig, _)| !sig.is_empty()))
        .collect();
    let link_map = build_link_map(&sections);

    let stdlib_info: Vec<StdlibSection> = sections
        .iter()
        .map(|s| StdlibSection {
            id: s.id.clone(),
            name: s.name.clone(),
            description: stdlib_description(&s.id).to_string(),
        })
        .collect();

    generate_docs(&stdlib_info, &link_map, version)?;

    let topic_info = gather_topic_info();

    for section in &sections {
        generate_stdlib_section(section, &stdlib_info, &topic_info)?;
    }

    generate_stdlib_toc(&sections, &stdlib_info, &topic_info)?;
    generate_search_index(&sections, &stdlib_info)?;

    let topic_sources = get_topic_sources();
    generate_print_page(&topic_sources, &sections, &stdlib_info, &link_map, version)?;
    generate_typst(&topic_sources, &sections, version)?;

    let libraries = generate_libraries_page(&stdlib_info, &topic_info)?;

    let sitemap_pages = loft::documentation::generate_sitemap()?;
    println!("Generated doc/sitemap.xml ({sitemap_pages} pages) + doc/robots.txt");
    println!("Generated {} stdlib section pages", sections.len());
    println!("Generated doc/libraries.html ({libraries} registry packages)");
    Ok(())
}

// ---  Parser  ---

fn parse_loft(content: &str, entries: &mut Vec<Entry>) {
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;
    let mut doc: Vec<String> = Vec::new();
    let mut after_section = false;

    while i < lines.len() {
        let trimmed = lines[i].trim();

        if let Some(name) = parse_section(trimmed) {
            entries.push(Entry::Section(name));
            doc.clear();
            after_section = true;
            i += 1;
            continue;
        }

        if trimmed.starts_with("//") {
            let text = trimmed.trim_start_matches('/').trim().to_string();
            doc.push(text);
            i += 1;
            continue;
        }

        if trimmed.starts_with("pub ") {
            let (sig, consumed) = collect_sig(&lines[i..]);
            entries.push(Entry::Item {
                sig,
                doc: std::mem::take(&mut doc),
            });
            after_section = false;
            i += consumed;
            continue;
        }

        // #rust attribute lines do not break doc accumulation.
        if !trimmed.starts_with('#') {
            if after_section && !doc.is_empty() {
                entries.push(Entry::Item {
                    sig: String::new(),
                    doc: std::mem::take(&mut doc),
                });
            } else {
                doc.clear();
            }
            after_section = false;
        }

        i += 1;
    }
}

fn parse_section(trimmed: &str) -> Option<String> {
    if !trimmed.starts_with("// ---") {
        return None;
    }
    let inner = trimmed
        .trim_start_matches('/')
        .trim()
        .trim_matches('-')
        .trim();
    if inner.is_empty() {
        None
    } else {
        Some(inner.to_string())
    }
}

/// Use this rather than `collect_block` directly; it selects between
/// signature-only and full-body capture based on the declaration kind.
fn collect_sig(lines: &[&str]) -> (String, usize) {
    let first = lines[0].trim();
    if first.starts_with("pub struct") || first.starts_with("pub enum") {
        return collect_block(lines);
    }
    (strip_body(first), 1)
}

fn collect_block(lines: &[&str]) -> (String, usize) {
    let mut result: Vec<String> = Vec::new();
    let mut depth = 0i32;
    for (idx, &line) in lines.iter().enumerate() {
        for ch in line.trim().chars() {
            match ch {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
        }
        result.push(strip_offset_comment(line));
        if depth == 0 && idx > 0 {
            return (result.join("\n"), idx + 1);
        }
    }
    (result.join("\n"), lines.len())
}

fn strip_offset_comment(s: &str) -> String {
    if let Some(pos) = s.rfind("//")
        && s[pos + 2..].trim().parse::<i64>().is_ok()
    {
        return s[..pos].trim_end().to_string();
    }
    s.trim_end().to_string()
}

fn strip_body(sig: &str) -> String {
    let mut depth = 0i32;
    let mut result = String::new();
    for ch in sig.chars() {
        match ch {
            '(' => {
                depth += 1;
                result.push(ch);
            }
            ')' => {
                depth -= 1;
                result.push(ch);
            }
            '{' | ';' if depth == 0 => break,
            _ => result.push(ch),
        }
    }
    result.trim_end().to_string()
}

fn stdlib_description(id: &str) -> &'static str {
    match id {
        "types" => "Primitive type conversions and null checks",
        "math" => "Arithmetic, rounding, and trigonometry",
        "text" => "String manipulation and formatting",
        "collections" => "Vector, sorted, index, and hash operations",
        "output-and-diagnostics" => "Print, assert, and diagnostic functions",
        "parallel" => "Parallel for-loop and worker functions",
        "file-system" => "File, directory, and path operations",
        "environment" => "Command-line arguments and program state",
        _ => "",
    }
}

// ---  Section grouping  ---

/// Build this before generating HTML or the link map; it groups the flat entry
/// list into the structure both renderers need, merging sections that share a
/// name across multiple source files.
fn build_sections(entries: &[Entry]) -> Vec<SectionFull> {
    let mut sections: Vec<SectionFull> = Vec::new();
    let mut section_idx: Option<usize> = None;

    for entry in entries {
        match entry {
            Entry::Section(name) => {
                if let Some(idx) = sections.iter().position(|s| &s.name == name) {
                    section_idx = Some(idx);
                } else {
                    sections.push(SectionFull {
                        id: section_id(name),
                        name: name.clone(),
                        items: Vec::new(),
                    });
                    section_idx = Some(sections.len() - 1);
                }
            }
            Entry::Item { sig, doc } => {
                if let Some(idx) = section_idx {
                    sections[idx].items.push((sig.clone(), doc.clone()));
                }
            }
        }
    }
    sections
}

// ---  Link map  ---

/// Build this before calling `generate_docs` and `generate_stdlib_section`;
/// the map lets the syntax highlighter inject stdlib links without a second
/// pass over the generated HTML.
fn build_link_map(sections: &[SectionFull]) -> HashMap<String, String> {
    let mut map: HashMap<String, String> = HashMap::new();

    for section in sections {
        let url = format!("stdlib-{}.html", section.id);
        for (sig, _) in &section.items {
            if sig.is_empty() {
                continue;
            }
            if let Some(name) = sig_name(sig) {
                map.entry(name).or_insert_with(|| url.clone());
            }
        }
    }

    // vector and sorted are user-visible type aliases not captured by pub declarations.
    map.entry("vector".into())
        .or_insert_with(|| "stdlib-collections.html".into());
    map.entry("sorted".into())
        .or_insert_with(|| "stdlib-collections.html".into());

    map
}

fn sig_name(sig: &str) -> Option<String> {
    let trimmed = sig.trim();
    let rest = trimmed
        .strip_prefix("pub fn ")
        .or_else(|| trimmed.strip_prefix("fn "))
        .or_else(|| trimmed.strip_prefix("pub type "))
        .or_else(|| trimmed.strip_prefix("pub struct "))
        .or_else(|| trimmed.strip_prefix("pub enum "))
        .or_else(|| trimmed.strip_prefix("pub "))?;
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() { None } else { Some(name) }
}

// ---  HTML rendering  ---

fn section_id(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn group_paragraphs(lines: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    for line in lines {
        if line.is_empty() {
            if !current.is_empty() {
                result.push(current.trim().to_string());
                current = String::new();
            }
        } else {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(line);
        }
    }
    if !current.is_empty() {
        result.push(current.trim().to_string());
    }
    result
}

/// Call once per section after building the link map; generates a self-contained
/// page with full nav so readers can navigate to any other section or topic.
fn generate_stdlib_section(
    section: &SectionFull,
    stdlib_info: &[StdlibSection],
    topic_info: &[(String, String)],
) -> std::io::Result<()> {
    let stem = format!("stdlib-{}", section.id);
    let nav = build_nav(topic_info, stdlib_info, &stem);
    let mut body = String::new();
    for (sig, doc_lines) in &section.items {
        // I11: interface declarations have no rendering path yet — skip gracefully.
        if sig_kind(sig) == "interface" {
            continue;
        }
        let paras = group_paragraphs(doc_lines);
        if sig.is_empty() {
            body.push_str("<div class=\"section-desc\">");
            for p in &paras {
                body.push_str(&format!("<p>{}</p>", esc(p)));
            }
            body.push_str("</div>\n");
        } else {
            body.push_str("<div class=\"item\">\n");
            body.push_str(&format!("<pre><code>{}</code></pre>\n", esc(sig)));
            for p in &paras {
                body.push_str(&format!("<p>{}</p>\n", esc(p)));
            }
            body.push_str("</div>\n");
        }
    }
    // `stdlib_info` carries the hand-written one-line description for this
    // section; fall back to the name when a section has none.
    let desc = stdlib_info.iter().find(|s| s.id == section.id).map_or_else(
        || format!("{} — the Loft standard library.", section.name),
        |s| s.description.clone(),
    );
    let slug = format!("stdlib-{}", section.id);
    let meta = loft::documentation::PageMeta {
        slug: &slug,
        description: &desc,
    };
    let html = page_html(&section.name, &nav, &section.name, &body, &meta);
    fs::create_dir_all("doc")?;
    fs::write(format!("doc/{stem}.html"), html)?;
    println!("Generated doc/{stem}.html");
    Ok(())
}

/// Generate after all section pages exist so the item counts are accurate and
/// readers get a complete overview when landing on the stdlib index.
/// The **Libraries** page — every package in the registry, grouped by category.
///
/// A reader's first question about a distribution is *what is there*, and until this page
/// existed the published site answered it for two of the 42 packages, in an install line.
/// The registry index is the one place that knows the answer, so the page renders that and
/// nothing else: no per-library file to maintain, and it cannot fall behind the registry it
/// is generated from ([USER_DOCS.md](../doc/claude/USER_DOCS.md) Tier 0).
///
/// It is the web twin of `loft api --registry`, over the same `load_index` — the CLI and
/// the page agree because they read one source, not because someone keeps them in step.
///
/// A package appears under each of its categories, the way the maintainer catalogue lists
/// it: a reader arrives looking for *graphics*, not for a name they do not yet know.
fn generate_libraries_page(
    stdlib_sections: &[StdlibSection],
    topic_info: &[(String, String)],
) -> std::io::Result<usize> {
    // Offline-tolerant on purpose: `load_index` falls back to the cached index, so a doc
    // build does not need the network — only that the box has talked to the registry once.
    let opts = loft::install::InstallOptions {
        allow_unsigned: true,
        refresh: false,
        offline: false,
        allow_prerelease: false,
        skip_lockfile: true,
        lock_path: None,
    };
    let index = match loft::install::load_index(&opts) {
        Ok(index) => index,
        Err(e) => {
            // Fail rather than skip.  The nav links this page from every other page, and a
            // site published without it is the exact hole the page exists to close — so a
            // missing registry has to stop the build, not quietly ship a 404.
            eprintln!(
                "gendoc: cannot build the Libraries page — the registry index is \
                 unreachable and no cached copy is available: {e}\n\
                 Run `loft api --registry` once to populate the cache, then re-run gendoc."
            );
            return Err(std::io::Error::other("registry index unavailable"));
        }
    };

    // Category → the packages carrying it.  A package with no category still has to be
    // reachable, so it lands under "Other" rather than vanishing from the one page whose
    // job is completeness.
    let mut by_category: std::collections::BTreeMap<&str, Vec<&loft::registry_index::Package>> =
        std::collections::BTreeMap::new();
    for pkg in index.packages.values() {
        if pkg.categories.is_empty() {
            by_category.entry("other").or_default().push(pkg);
        }
        for cat in &pkg.categories {
            by_category.entry(cat.as_str()).or_default().push(pkg);
        }
    }

    let mut body = String::new();
    body.push_str(
        "<p>Every library in the loft registry. Each one is written in loft itself, so its \
         source reads like your own code. Install one with <code>loft install &lt;name&gt;</code>, \
         then <code>use &lt;name&gt;;</code> in your program.</p>\n",
    );
    let _ = writeln!(
        body,
        "<p class=\"lib-count\">{} packages{}</p>",
        index.packages.len(),
        if by_category.len() > 1 {
            format!(" in {} categories", by_category.len())
        } else {
            String::new()
        }
    );
    body.push_str(
        "<p>The same catalogue on the command line: <code>loft api --registry</code>. \
         One library's public surface: <code>loft api &lt;name&gt;</code>.</p>\n",
    );

    for (cat, packages) in &by_category {
        let _ = writeln!(
            body,
            "<h2 id=\"{}\">{}</h2>",
            esc(cat),
            esc(&category_title(cat))
        );
        body.push_str("<table class=\"lib-table\">\n");
        body.push_str("<tr><th>Library</th><th>Version</th><th>What it gives you</th></tr>\n");
        for pkg in packages {
            let latest = loft::registry_index::find_best_version(pkg, "*", false)
                .map_or_else(|| "\u{2014}".to_string(), |v| esc(&v.semver));
            let name = esc(&pkg.name);
            let described = pkg.description.as_deref().unwrap_or("");
            // The name links to the library's own page, not straight out to its
            // repository: a reader deciding whether to depend on something wants the card
            // first, and the repository link is on it.
            let link = format!("<a href=\"lib-{name}.html\"><code>{name}</code></a>");
            let _ = writeln!(
                body,
                "<tr><td>{link}</td><td>{latest}</td><td>{}</td></tr>",
                esc(described)
            );
        }
        body.push_str("</table>\n");
    }

    let nav = build_nav(topic_info, stdlib_sections, "libraries");
    let meta = loft::documentation::PageMeta {
        slug: "libraries",
        description: "Every library in the loft registry \u{2014} graphics and 3D, an HTTP \
             server and client, cryptography, text engines and geometry, each written in loft \
             and installed with `loft install`.",
    };
    let html = page_html("Libraries", &nav, "Libraries", &body, &meta);
    fs::write("doc/libraries.html", html)?;

    generate_library_cards(&index, stdlib_sections, topic_info)?;
    Ok(index.packages.len())
}

/// One page per library — the **card**: the facts a reader needs before deciding to depend
/// on something, which is a different question from *what is there* (the catalogue) and from
/// *how do I start* (the guide).
///
/// The categories are the tag system and they answer *where does this belong*. Nothing
/// answered *should I depend on this*, even though the registry already declares almost all
/// of it — the version and its date, the loft floor, the compatibility floors, the size, the
/// dependencies, the auto-use triggers and the size of the public surface.
///
/// **The card is a renderer, not a dataset.** No field here is written by hand anywhere, so
/// there is nothing for a maintainer to keep in step and nothing that can drift from the
/// package it describes ([USER_DOCS.md](../doc/claude/USER_DOCS.md) Tier 0).
fn generate_library_cards(
    index: &loft::registry_index::RegistryIndex,
    stdlib_sections: &[StdlibSection],
    topic_info: &[(String, String)],
) -> std::io::Result<()> {
    // "What uses this" — the one field on the card that no package declares about itself.
    // Inverting every package's `deps` is the whole derivation, and it is the field that
    // tells a reader whether they are the first person to try something.
    let mut used_by: std::collections::BTreeMap<&str, Vec<&str>> =
        std::collections::BTreeMap::new();
    for (name, pkg) in &index.packages {
        let Some(v) = loft::registry_index::find_best_version(pkg, "*", false) else {
            continue;
        };
        for dep in v.deps.keys() {
            used_by.entry(dep.as_str()).or_default().push(name.as_str());
        }
    }

    for (name, pkg) in &index.packages {
        let Some(v) = loft::registry_index::find_best_version(pkg, "*", false) else {
            continue;
        };
        let mut body = String::new();

        if let Some(d) = pkg.description.as_deref().filter(|d| !d.is_empty()) {
            let _ = writeln!(body, "<p class=\"lib-lede\">{}</p>", esc(d));
        }
        let _ = writeln!(
            body,
            "<pre><code>loft install {}\n</code></pre>\n<pre><code>use {};\n</code></pre>",
            esc(name),
            esc(name)
        );

        body.push_str("<table class=\"lib-table lib-card\">\n");
        let mut row = |k: &str, val: &str| {
            let _ = writeln!(body, "<tr><th>{k}</th><td>{val}</td></tr>");
        };

        row(
            "Version",
            &format!(
                "{} \u{2014} published {}",
                esc(&v.semver),
                esc(&short_date(&v.published))
            ),
        );
        if !pkg.categories.is_empty() {
            let cats = pkg
                .categories
                .iter()
                .map(|c| {
                    format!(
                        "<a href=\"libraries.html#{}\">{}</a>",
                        esc(c),
                        esc(&category_title(c))
                    )
                })
                .collect::<Vec<_>>()
                .join(" \u{b7} ");
            row("Categories", &cats);
        }
        row("Requires", &format!("loft <code>{}</code>", esc(&v.loft)));
        row(
            "Compatibility",
            &compat_cell(
                &v.semver,
                v.api_compatible_with.as_deref(),
                v.data_compatible_with.as_deref(),
            ),
        );
        row("Download", &human_size(v.size));
        if !v.api.is_empty() {
            row(
                "Public API",
                &format!(
                    "{} items \u{2014} <code>loft api {}</code> prints them with their documentation",
                    v.api.len(),
                    esc(name)
                ),
            );
        }
        row(
            "Dependencies",
            &package_links(
                &v.deps.keys().map(String::as_str).collect::<Vec<_>>(),
                index,
                "none",
            ),
        );
        row(
            "Used by",
            &package_links(
                used_by.get(name.as_str()).map(Vec::as_slice).unwrap_or(&[]),
                index,
                "no other registry package \u{2014} you would be the first",
            ),
        );
        if !v.triggers.is_empty() {
            // A trigger is why a method resolves with no `use` line, which reads as magic
            // until you know the package behind it.
            let names = v
                .triggers
                .iter()
                .map(|t| {
                    let (m, recv) = t.split_once(':').unwrap_or((t.as_str(), ""));
                    if recv.is_empty() {
                        format!("<code>{}</code>", esc(m))
                    } else {
                        format!("<code>{}.{}()</code>", esc(recv), esc(m))
                    }
                })
                .collect::<Vec<_>>()
                .join(" \u{b7} ");
            row(
                "Loads itself for",
                &format!("{names} \u{2014} these resolve with no <code>use</code> line"),
            );
        }
        if !pkg.yanked.is_empty() {
            let ys = pkg
                .yanked
                .iter()
                .map(|y| format!("<code>{}</code>", esc(y)))
                .collect::<Vec<_>>()
                .join(", ");
            row(
                "Yanked versions",
                &format!("{ys} \u{2014} do not pin these"),
            );
        }
        if let Some(url) = pkg.homepage.as_deref().filter(|u| !u.is_empty()) {
            row("Source", &format!("<a href=\"{0}\">{0}</a>", esc(url)));
        }
        body.push_str("</table>\n");

        body.push_str(
            "<p class=\"lib-legend\">A <em>drop-in</em> floor is the oldest release of this \
             package the current one still replaces without a source change. \
             <em>Not declared</em> means exactly that \u{2014} the package has made no promise, \
             which is the honest state for one published before the compatibility contract \
             existed.</p>\n",
        );
        body.push_str("<p><a href=\"libraries.html\">\u{2190} All libraries</a></p>\n");

        let nav = build_nav(topic_info, stdlib_sections, "libraries");
        let desc = pkg
            .description
            .as_deref()
            .filter(|d| !d.is_empty())
            .map_or_else(
                || format!("The {name} library for loft \u{2014} install it with `loft install {name}`."),
                std::string::ToString::to_string,
            );
        let slug = format!("lib-{name}");
        let meta = loft::documentation::PageMeta {
            slug: &slug,
            description: &desc,
        };
        let html = page_html(name, &nav, name, &body, &meta);
        fs::write(format!("doc/lib-{name}.html"), html)?;
    }
    Ok(())
}

/// What the two compatibility floors mean, in the reader's terms.
///
/// The floor is *the oldest release this version is still a drop-in for* — so a floor EQUAL
/// to the version says the opposite of what it looks like: nothing older is compatible,
/// because the break happened here. A version that declares no floor has promised nothing,
/// and that absence is load-bearing rather than an omission to paper over: 18 of the 42
/// packages predate the contract.
fn compat_cell(version: &str, api: Option<&str>, data: Option<&str>) -> String {
    let one = |kind: &str, floor: Option<&str>| -> String {
        match floor {
            None => format!("{kind}: not declared"),
            // Say what the floor literally declares, not what it implies. A floor equal to
            // the version means either "the break happened here" or "this is a first
            // release" — the index cannot tell those apart, and only one of them makes
            // the word "breaks" true.
            Some(f) if f == version => format!("{kind}: no earlier release is a drop-in"),
            Some(f) => format!("{kind}: drop-in for {} and later", esc(f)),
        }
    };
    format!("{} \u{b7} {}", one("API", api), one("stored data", data))
}

/// A dependency / dependent list, each name linked to its own card. `empty` is what the cell
/// says when the list is empty — an empty cell reads as *unknown*, and here it is a fact.
fn package_links(
    names: &[&str],
    index: &loft::registry_index::RegistryIndex,
    empty: &str,
) -> String {
    if names.is_empty() {
        return empty.to_string();
    }
    names
        .iter()
        .map(|n| {
            if index.packages.contains_key(*n) {
                format!("<a href=\"lib-{0}.html\"><code>{0}</code></a>", esc(n))
            } else {
                format!("<code>{}</code>", esc(n))
            }
        })
        .collect::<Vec<_>>()
        .join(" \u{b7} ")
}

/// `2026-08-21T10:48:14Z` \u{2192} `2026-08-21`. A reader wants the day, not the second.
fn short_date(stamp: &str) -> String {
    stamp.split('T').next().unwrap_or(stamp).to_string()
}

/// A download size a person can judge. Packages here run from 11 kB to 200 kB, so kB is the
/// working unit and MB only appears if one ever grows into it.
fn human_size(bytes: u64) -> String {
    if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else {
        format!("{} kB", bytes.div_ceil(1024))
    }
}

/// The heading a category slug is shown as.  The registry's slugs are terse because they
/// are also search keys (`asset-format`, `cli`); a heading is read by a person.
///
/// An unknown slug renders as itself rather than being dropped, so a category added to the
/// registry appears on the page the same day, unstyled but present.
fn category_title(slug: &str) -> String {
    match slug {
        "animation" => "Animation".to_string(),
        "asset-format" => "Asset formats".to_string(),
        "cli" => "Command line".to_string(),
        "crypto" => "Cryptography".to_string(),
        "encoding" => "Encoding".to_string(),
        "game" => "Games".to_string(),
        "geometry" => "Geometry".to_string(),
        "graphics" => "Graphics and 3D".to_string(),
        "math" => "Mathematics".to_string(),
        "net" => "Networking".to_string(),
        "plugins" => "Plugins".to_string(),
        "random" => "Randomness".to_string(),
        "text" => "Text".to_string(),
        "time" => "Time".to_string(),
        "world" => "World building".to_string(),
        "other" => "Other".to_string(),
        s => s.to_string(),
    }
}

fn generate_stdlib_toc(
    sections: &[SectionFull],
    stdlib_info: &[StdlibSection],
    topic_info: &[(String, String)],
) -> std::io::Result<()> {
    let nav = build_nav(topic_info, stdlib_info, "stdlib");
    let mut body = String::new();
    body.push_str("<div class=\"grid\">\n");
    for section in sections {
        let count = section.items.iter().filter(|(s, _)| !s.is_empty()).count();
        body.push_str(&format!(
            "  <a class=\"card\" href=\"stdlib-{id}.html\"><h2>{name}</h2><p>{count} items</p></a>\n",
            id = section.id,
            name = esc(&section.name),
        ));
    }
    body.push_str("</div>\n");
    let toc_meta = loft::documentation::PageMeta {
        slug: "stdlib",
        description: "The Loft standard library — every built-in function and type, by section.",
    };
    let html = page_html(
        "Standard Library",
        &nav,
        "Loft Standard Library",
        &body,
        &toc_meta,
    );
    fs::write("doc/stdlib.html", &html)?;
    println!("Generated doc/stdlib.html");
    Ok(())
}

/// Generate last so all section URLs are stable before they are written into
/// the index; the index is consumed at page load so stale URLs cause silent
/// broken links.
fn generate_search_index(
    sections: &[SectionFull],
    stdlib_info: &[StdlibSection],
) -> std::io::Result<()> {
    let mut entries: Vec<String> = Vec::new();

    for sec in stdlib_info {
        entries.push(format!(
            "{{name:{:?},kind:\"section\",url:\"stdlib-{}.html\"}}",
            sec.name, sec.id
        ));
    }

    for section in sections {
        let url = format!("stdlib-{}.html", section.id);
        for (sig, _) in &section.items {
            if sig.is_empty() {
                continue;
            }
            if let Some(name) = sig_name(sig) {
                let kind = sig_kind(sig);
                entries.push(format!("{{name:{:?},kind:{:?},url:{:?}}}", name, kind, url));
            }
        }
    }

    let js = format!("const SEARCH_INDEX=[\n{}\n];\n", entries.join(",\n"));
    fs::write("doc/search-index.js", js)?;
    println!("Generated doc/search-index.js ({} entries)", entries.len());
    Ok(())
}

fn sig_kind(sig: &str) -> &'static str {
    let trimmed = sig.trim();
    if trimmed.starts_with("pub fn ") || trimmed.starts_with("fn ") {
        "fn"
    } else if trimmed.starts_with("pub type ") {
        "type"
    } else if trimmed.starts_with("pub struct ") {
        "struct"
    } else if trimmed.starts_with("pub enum ") {
        "enum"
    } else if trimmed.starts_with("pub interface ") || trimmed.starts_with("interface ") {
        // I11: interface declarations are not yet rendered; return a distinct kind so
        // callers can skip them gracefully without mislabelling them as "const".
        "interface"
    } else {
        "const"
    }
}

/// Strip all HTML tags from `s`, returning only the text content.
fn html_strip_tags(s: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }
    result
}

/// Decode common HTML entities.
fn html_decode(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&mdash;", "\u{2014}")
        .replace("&ndash;", "\u{2013}")
}

/// Return the substring strictly between the first `open` and the next `close`.
fn between<'a>(s: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = s.find(open)? + open.len();
    let end = s[start..].find(close)? + start;
    Some(&s[start..end])
}

/// Render `doc/00-vs-rust.html` as Typst markup for inclusion in the PDF.
/// The HTML is the single source of truth; this function extracts headings,
/// Loft/Rust code blocks, and verdict paragraphs directly from it.
fn render_vs_rust_typst(html: &str) -> String {
    let mut out = String::new();

    // Intro paragraph (between <article> and first <h2>)
    if let Some(after_article) = html
        .find("<article>")
        .map(|p| &html[p + "<article>".len()..])
        && let Some(intro_raw) = between(after_article, "<p>", "</p>")
    {
        let intro = html_decode(&html_strip_tags(intro_raw));
        out.push_str(&typst_escape(intro.trim()));
        out.push_str("\n\n");
    }

    // Walk through each <h2 ...> section
    let mut pos = 0;
    while let Some(rel) = html[pos..].find("<h2 ") {
        let h2_start = pos + rel;

        // Heading text: content between the first > and </h2>
        let after_h2_tag = h2_start + html[h2_start..].find('>').unwrap_or(0) + 1;
        let h2_close = html[after_h2_tag..].find("</h2>").unwrap_or(0);
        let heading_raw = &html[after_h2_tag..after_h2_tag + h2_close];
        let heading = html_decode(&html_strip_tags(heading_raw));
        // Strip leading "N. " prefix — Typst adds its own numbering via #set heading(numbering: …)
        let heading_text = heading.trim();
        let heading_text = heading_text
            .find(". ")
            .filter(|&dot| heading_text[..dot].chars().all(|c| c.is_ascii_digit()))
            .map(|dot| &heading_text[dot + 2..])
            .unwrap_or(heading_text);
        out.push_str(&format!("=== {}\n\n", typst_escape(heading_text)));

        let section_start = after_h2_tag + h2_close + "</h2>".len();

        // Section ends at the next <h2 or </article>
        let section_end = html[section_start..]
            .find("<h2 ")
            .or_else(|| html[section_start..].find("</article>"))
            .map(|p| section_start + p)
            .unwrap_or(html.len());
        let section = &html[section_start..section_end];

        // Loft code block (plain <pre><code>…</code></pre>)
        if let Some(loft_raw) = between(section, "<pre><code>", "</code></pre>") {
            let loft_code = html_decode(&html_strip_tags(loft_raw));
            out.push_str(&format!("```rust\n{}\n```\n\n", loft_code.trim_end()));
        }

        // Rust code block (<pre class="rust-pre"><code>…</code></pre>)
        if let Some(rust_raw) = between(section, "<pre class=\"rust-pre\"><code>", "</code></pre>")
        {
            let rust_code = html_decode(rust_raw);
            out.push_str(&format!("```rust\n{}\n```\n\n", rust_code.trim_end()));
        }

        // Verdict: upside and downside as separate paragraphs
        if let Some(up_raw) = between(section, "<p class=\"up\">", "</p>") {
            let up = html_decode(&html_strip_tags(up_raw));
            out.push_str(&typst_escape(up.trim()));
            out.push_str("\n\n");
        }
        if let Some(down_raw) = between(section, "<p class=\"down\">", "</p>") {
            let down = html_decode(&html_strip_tags(down_raw));
            out.push_str(&typst_escape(down.trim()));
            out.push_str("\n\n");
        }

        pos = section_end;
    }
    out
}

/// Render `doc/00-vs-python.html` as Typst markup for inclusion in the PDF.
fn render_vs_python_typst(html: &str) -> String {
    let mut out = String::new();

    // Intro paragraph (between <article> and first <h2>)
    if let Some(after_article) = html
        .find("<article>")
        .map(|p| &html[p + "<article>".len()..])
        && let Some(intro_raw) = between(after_article, "<p>", "</p>")
    {
        let intro = html_decode(&html_strip_tags(intro_raw));
        out.push_str(&typst_escape(intro.trim()));
        out.push_str("\n\n");
    }

    // Walk through each <h2 ...> section
    let mut pos = 0;
    while let Some(rel) = html[pos..].find("<h2 ") {
        let h2_start = pos + rel;

        // Heading text: content between the first > and </h2>
        let after_h2_tag = h2_start + html[h2_start..].find('>').unwrap_or(0) + 1;
        let h2_close = html[after_h2_tag..].find("</h2>").unwrap_or(0);
        let heading_raw = &html[after_h2_tag..after_h2_tag + h2_close];
        let heading = html_decode(&html_strip_tags(heading_raw));
        // Strip leading "N. " prefix — Typst adds its own numbering via #set heading(numbering: …)
        let heading_text = heading.trim();
        let heading_text = heading_text
            .find(". ")
            .filter(|&dot| heading_text[..dot].chars().all(|c| c.is_ascii_digit()))
            .map(|dot| &heading_text[dot + 2..])
            .unwrap_or(heading_text);
        out.push_str(&format!("=== {}\n\n", typst_escape(heading_text)));

        let section_start = after_h2_tag + h2_close + "</h2>".len();

        // Section ends at the next <h2 or </article>
        let section_end = html[section_start..]
            .find("<h2 ")
            .or_else(|| html[section_start..].find("</article>"))
            .map(|p| section_start + p)
            .unwrap_or(html.len());
        let section = &html[section_start..section_end];

        // Loft code block (plain <pre><code>…</code></pre>)
        if let Some(loft_raw) = between(section, "<pre><code>", "</code></pre>") {
            let loft_code = html_decode(&html_strip_tags(loft_raw));
            out.push_str(&format!("```rust\n{}\n```\n\n", loft_code.trim_end()));
        }

        // Python code block (<pre class="python-pre"><code>…</code></pre>)
        if let Some(python_raw) =
            between(section, "<pre class=\"python-pre\"><code>", "</code></pre>")
        {
            let python_code = html_decode(python_raw);
            out.push_str(&format!("```python\n{}\n```\n\n", python_code.trim_end()));
        }

        // Verdict: upside and downside as separate paragraphs
        if let Some(up_raw) = between(section, "<p class=\"up\">", "</p>") {
            let up = html_decode(&html_strip_tags(up_raw));
            out.push_str(&typst_escape(up.trim()));
            out.push_str("\n\n");
        }
        if let Some(down_raw) = between(section, "<p class=\"down\">", "</p>") {
            let down = html_decode(&html_strip_tags(down_raw));
            out.push_str(&typst_escape(down.trim()));
            out.push_str("\n\n");
        }

        pos = section_end;
    }
    out
}

// ---  Print page  ---

/// Generate a single-file HTML page containing all documentation for offline
/// reading and PDF printing. The page links back to individual online pages
/// and includes a clickable table of contents.
fn generate_print_page(
    topic_sources: &[TopicSource],
    sections: &[SectionFull],
    stdlib_info: &[StdlibSection],
    link_map: &HashMap<String, String>,
    version: &str,
) -> std::io::Result<()> {
    let mut toc = String::from("<nav class=\"print-toc\">\n<h2>Contents</h2>\n<ul>\n");
    toc.push_str(
        "<li><a href=\"00-vs-rust.html\">vs Rust — Key Differences</a> <span class=\"toc-note\">(online only)</span></li>\n",
    );
    toc.push_str(
        "<li><a href=\"00-vs-python.html\">vs Python — Key Differences</a> <span class=\"toc-note\">(online only)</span></li>\n",
    );
    toc.push_str(
        "<li><a href=\"#00-performance\">Performance</a> — <span class=\"toc-desc\">Benchmark results across interpreter, native, wasm, and Rust</span></li>\n",
    );
    for ts in topic_sources {
        toc.push_str(&format!(
            "<li><a href=\"#{fn}\">{name}</a> — <span class=\"toc-desc\">{title}</span></li>\n",
            fn = ts.filename,
            name = esc(&ts.name),
            title = esc(&ts.title),
        ));
    }
    toc.push_str("<li class=\"toc-section\"><a href=\"#stdlib\">Standard Library</a></li>\n");
    for sec in stdlib_info {
        toc.push_str(&format!(
            "<li class=\"toc-indent\"><a href=\"#stdlib-{id}\">{name}</a></li>\n",
            id = sec.id,
            name = esc(&sec.name),
        ));
    }
    toc.push_str("</ul>\n</nav>\n");

    let mut content = toc;

    for ts in topic_sources {
        content.push_str(&format!(
            "<section class=\"print-section\" id=\"{fn}\">\n<h1>{name}</h1>\n",
            fn = ts.filename,
            name = esc(&ts.name),
        ));
        content.push_str(&render_topic_body(&ts.source, link_map));
        content.push_str("</section>\n");
    }

    // Embed the hand-maintained performance page as a print section.
    let perf_html = fs::read_to_string("doc/00-performance.html").unwrap_or_default();
    let article_body = if let (Some(start), Some(end)) =
        (perf_html.find("<article>"), perf_html.find("</article>"))
    {
        &perf_html[start + "<article>".len()..end]
    } else {
        ""
    };
    content.push_str(
        "<section class=\"print-section\" id=\"00-performance\">\n<h1>Performance</h1>\n",
    );
    content.push_str(article_body);
    content.push_str("</section>\n");

    content
        .push_str("<section class=\"print-section\" id=\"stdlib\">\n<h1>Standard Library</h1>\n");
    for section in sections {
        content.push_str(&format!(
            "<h2 id=\"stdlib-{id}\">{name}</h2>\n",
            id = section.id,
            name = esc(&section.name),
        ));
        for (sig, doc_lines) in &section.items {
            let paras = group_paragraphs(doc_lines);
            if sig.is_empty() {
                for p in &paras {
                    content.push_str(&format!("<p class=\"section-desc\">{}</p>\n", esc(p)));
                }
            } else {
                content.push_str("<div class=\"item\">\n");
                content.push_str(&format!("<pre><code>{}</code></pre>\n", esc(sig)));
                for p in &paras {
                    content.push_str(&format!("<p>{}</p>\n", esc(p)));
                }
                content.push_str("</div>\n");
            }
        }
    }
    content.push_str("</section>\n");

    let count = topic_sources.len() + sections.len();
    let html = format!(
        "<!DOCTYPE html>\n\
<html lang=\"en\">\n\
<head>\n\
  <meta charset=\"utf-8\">\n\
  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
  <title>Loft Language — Complete Reference</title>\n\
  <link rel=\"stylesheet\" href=\"style.css\">\n\
</head>\n\
<body class=\"print-page\">\n\
  <header class=\"print-header\">\n\
    <h1>Loft Language Reference</h1>\n\
    <p class=\"tagline\">Complete language and standard library documentation</p>\n\
    <p class=\"version\">v{version}</p>\n\
    <p><a href=\"index.html\">&#8592; Back to online documentation</a> \
       &nbsp;|&nbsp; <a href=\"00-vs-rust.html\">vs Rust comparison</a></p>\n\
  </header>\n\
{content}\
</body>\n\
</html>\n",
        content = content,
    );
    fs::write("doc/print.html", &html)?;
    println!("Generated doc/print.html ({count} sections)");
    Ok(())
}

// ---  Typst generation  ---

/// Render inline HTML content as Typst markup.
/// Handles `<code>`, `<strong>`, badge spans; strips all other tags.
fn render_inline_typst(s: &str) -> String {
    // Replace badge spans before general tag stripping
    let s = s.replace(
        "<span class=\"badge planned\">planned</span>",
        "_(planned)_",
    );
    let mut out = String::new();
    let mut pos = 0;
    while pos < s.len() {
        let rest = &s[pos..];
        if rest.starts_with("<code>") {
            let after = pos + "<code>".len();
            if let Some(end) = s[after..].find("</code>") {
                let text = html_decode(&html_strip_tags(&s[after..after + end]));
                out.push('`');
                out.push_str(&text.replace('`', "'"));
                out.push('`');
                pos = after + end + "</code>".len();
            } else {
                pos += 1;
            }
        } else if rest.starts_with("<strong>") {
            let after = pos + "<strong>".len();
            if let Some(end) = s[after..].find("</strong>") {
                let text = html_decode(&html_strip_tags(&s[after..after + end]));
                out.push('*');
                out.push_str(&typst_escape(&text));
                out.push('*');
                pos = after + end + "</strong>".len();
            } else {
                pos += 1;
            }
        } else if rest.starts_with('<') {
            if let Some(end) = rest.find('>') {
                pos += end + 1;
            } else {
                pos += 1;
            }
        } else {
            let next_tag = rest.find('<').unwrap_or(rest.len());
            let text = html_decode(&rest[..next_tag]);
            out.push_str(&typst_escape(&text));
            pos += next_tag;
        }
    }
    out
}

/// Render an HTML article page (install.html, roadmap.html) as Typst markup.
/// Handles h2, h3, p, pre/code, ul/li; strips navigation and script elements.
fn render_article_html_typst(html: &str) -> String {
    let article = match between(html, "<article>", "</article>") {
        Some(s) => s,
        None => return String::new(),
    };
    let mut out = String::new();
    let mut pos = 0;
    while pos < article.len() {
        let rest = &article[pos..];
        if rest.starts_with("<h2 ") || rest.starts_with("<h2>") {
            let end_tag = rest.find('>').unwrap_or(0);
            let after_tag = end_tag + 1;
            if let Some(close) = rest[after_tag..].find("</h2>") {
                let heading = html_decode(&html_strip_tags(&rest[after_tag..after_tag + close]));
                out.push_str(&format!("=== {}\n\n", typst_escape(heading.trim())));
                pos += after_tag + close + "</h2>".len();
            } else {
                pos += 1;
            }
        } else if rest.starts_with("<h3 ") || rest.starts_with("<h3>") {
            let end_tag = rest.find('>').unwrap_or(0);
            let after_tag = end_tag + 1;
            if let Some(close) = rest[after_tag..].find("</h3>") {
                let heading = html_decode(&html_strip_tags(&rest[after_tag..after_tag + close]));
                out.push_str(&format!("==== {}\n\n", typst_escape(heading.trim())));
                pos += after_tag + close + "</h3>".len();
            } else {
                pos += 1;
            }
        } else if rest.starts_with("<p>") || rest.starts_with("<p ") {
            let end_tag = rest.find('>').unwrap_or(2);
            let after_tag = end_tag + 1;
            if let Some(close) = rest[after_tag..].find("</p>") {
                let text = render_inline_typst(&rest[after_tag..after_tag + close]);
                if !text.trim().is_empty() {
                    out.push_str(text.trim());
                    out.push_str("\n\n");
                }
                pos += after_tag + close + "</p>".len();
            } else {
                pos += 1;
            }
        } else if rest.starts_with("<pre>") || rest.starts_with("<pre ") {
            let end_tag = rest.find('>').unwrap_or(4);
            let after_pre = end_tag + 1;
            if let Some(close) = rest[after_pre..].find("</pre>") {
                let content = &rest[after_pre..after_pre + close];
                let code_text = if let Some(code) = between(content, "<code>", "</code>") {
                    html_decode(&html_strip_tags(code))
                } else {
                    html_decode(&html_strip_tags(content))
                };
                out.push_str(&format!("```\n{}\n```\n\n", code_text.trim_end()));
                pos += after_pre + close + "</pre>".len();
            } else {
                pos += 1;
            }
        } else if rest.starts_with("<ul>") {
            if let Some(close) = rest.find("</ul>") {
                let ul_content = &rest["<ul>".len()..close];
                let mut li_pos = 0;
                while li_pos < ul_content.len() {
                    let li_rest = &ul_content[li_pos..];
                    if li_rest.starts_with("<li>") || li_rest.starts_with("<li ") {
                        let end_tag = li_rest.find('>').unwrap_or(3);
                        let after_tag = end_tag + 1;
                        if let Some(li_close) = li_rest[after_tag..].find("</li>") {
                            let item =
                                render_inline_typst(&li_rest[after_tag..after_tag + li_close]);
                            out.push_str(&format!("- {}\n", item.trim()));
                            li_pos += after_tag + li_close + "</li>".len();
                        } else {
                            li_pos += 1;
                        }
                    } else {
                        li_pos += 1;
                    }
                }
                out.push('\n');
                pos += close + "</ul>".len();
            } else {
                pos += 1;
            }
        } else if rest.starts_with('<') {
            if let Some(end) = rest.find('>') {
                pos += end + 1;
            } else {
                pos += rest.chars().next().map_or(1, char::len_utf8);
            }
        } else {
            pos += rest.chars().next().map_or(1, char::len_utf8);
        }
    }
    out
}

/// Generate a Typst source file for the full language reference.
/// Compile with: typst compile doc/loft-reference.typ doc/loft-reference.pdf
fn generate_typst(
    topic_sources: &[TopicSource],
    sections: &[SectionFull],
    version: &str,
) -> std::io::Result<()> {
    let mut out = String::new();

    // Document preamble
    out.push_str("// Loft Language Reference — generated by `cargo run --bin gendoc`\n");
    out.push_str("// Compile with: typst compile loft-reference.typ loft-reference.pdf\n\n");
    out.push_str(&format!(
        "#set document(title: \"Loft Language Reference\", keywords: (\"v{version}\"))\n"
    ));
    out.push_str("#set page(paper: \"a4\", margin: (x: 2.5cm, y: 3cm))\n");
    out.push_str("#set text(font: (\"Libertinus Serif\", \"Liberation Serif\", \"DejaVu Serif\"), size: 11pt)\n");
    out.push_str("#set par(justify: true, leading: 0.75em)\n");
    out.push_str("#set heading(numbering: \"1.1.\")\n");
    out.push('\n');
    out.push_str("#show raw.where(block: true): it => block(\n");
    out.push_str("  fill: luma(245),\n");
    out.push_str("  inset: (x: 10pt, y: 8pt),\n");
    out.push_str("  radius: 4pt,\n");
    out.push_str("  width: 100%,\n");
    out.push_str("  it,\n");
    out.push_str(")\n\n");
    out.push_str("#show heading.where(level: 1): it => {\n");
    out.push_str("  pagebreak(weak: true)\n");
    out.push_str("  v(0.5em)\n");
    out.push_str("  it\n");
    out.push_str("}\n\n");

    // Title page
    out.push_str("= Loft Language Reference\n\n");
    out.push_str("#v(1em)\n\n");
    out.push_str("_A statically-typed scripting language with null safety and built-in parallel execution._\n\n");
    out.push_str(&format!(
        "#text(size: 10pt, fill: luma(100))[Version {version}]\n\n"
    ));
    out.push_str(
        "Every code example in this document is an executable part of the test suite.\n\n",
    );
    out.push_str("#outline(depth: 3)\n\n");
    out.push_str("#pagebreak()\n\n");

    // Getting Started — generated from install.html
    if let Ok(install_html) = std::fs::read_to_string("doc/install.html") {
        out.push_str("= Getting Started\n\n");
        out.push_str(&render_article_html_typst(&install_html));
        out.push('\n');
    }

    // vs Rust section — generated directly from the static HTML (single source of truth)
    if let Ok(vs_rust_html) = std::fs::read_to_string("doc/00-vs-rust.html") {
        out.push_str("= vs Rust\n\n");
        out.push_str(&render_vs_rust_typst(&vs_rust_html));
        out.push('\n');
    }

    // vs Python section — generated directly from the static HTML (single source of truth)
    if let Ok(vs_python_html) = std::fs::read_to_string("doc/00-vs-python.html") {
        out.push_str("= vs Python\n\n");
        out.push_str(&render_vs_python_typst(&vs_python_html));
        out.push('\n');
    }

    // Language topic sections
    for ts in topic_sources {
        out.push_str(&format!("= {}\n\n", typst_escape(&ts.name)));
        out.push_str(&render_topic_typst(&ts.source));
        out.push('\n');
    }

    // Standard library
    out.push_str("= Standard Library\n\n");
    for section in sections {
        out.push_str(&format!("== {}\n\n", typst_escape(&section.name)));
        for (sig, doc_lines) in &section.items {
            let paras = group_paragraphs(doc_lines);
            if sig.is_empty() {
                for p in &paras {
                    if !p.is_empty() {
                        out.push_str(&typst_escape(p));
                        out.push_str("\n\n");
                    }
                }
            } else {
                // Code signature block
                out.push_str(&format!("```rust\n{}\n```\n\n", sig));
                for p in &paras {
                    if !p.is_empty() {
                        out.push_str(&typst_escape(p));
                        out.push('\n');
                    }
                }
                if paras.iter().any(|p| !p.is_empty()) {
                    out.push('\n');
                }
            }
        }
    }

    // Roadmap appendix — generated from roadmap.html
    if let Ok(roadmap_html) = std::fs::read_to_string("doc/roadmap.html") {
        out.push_str("= Roadmap\n\n");
        out.push_str(&render_article_html_typst(&roadmap_html));
        out.push('\n');
    }

    fs::write("doc/loft-reference.typ", &out)?;
    println!("Generated doc/loft-reference.typ");
    Ok(())
}

// ── I11 — gendoc guard for interface declarations ────────────────────────────

#[cfg(test)]
mod tests {
    use super::sig_kind;

    /// I11: `sig_kind` must return `"interface"` (not `"const"`) for interface
    /// declarations so that they are skipped gracefully in stdlib rendering.
    #[test]
    fn sig_kind_interface_returns_interface() {
        assert_eq!(sig_kind("pub interface Ordered { }"), "interface");
        assert_eq!(sig_kind("interface Foo {}"), "interface");
    }
}
