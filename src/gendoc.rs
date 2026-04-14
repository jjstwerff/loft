// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// Generate standard library HTML pages from the documented default/*.loft files.
// Run with: cargo run --bin gendoc

use loft::documentation::{
    StdlibSection, TopicSource, build_nav, gather_topic_info, generate_docs, get_topic_sources,
    page_html, render_topic_body, render_topic_typst,
};
use std::collections::HashMap;
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
        "default/02_images.loft",
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

    println!("Generated {} stdlib section pages", sections.len());
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
    let html = page_html(&section.name, &nav, &section.name, &body);
    fs::create_dir_all("doc")?;
    fs::write(format!("doc/{stem}.html"), html)?;
    println!("Generated doc/{stem}.html");
    Ok(())
}

/// Generate after all section pages exist so the item counts are accurate and
/// readers get a complete overview when landing on the stdlib index.
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
    let html = page_html("Standard Library", &nav, "Loft Standard Library", &body);
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

fn typst_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('#', "\\#")
        .replace('@', "\\@")
        .replace('$', "\\$")
        .replace('[', "\\[")
        .replace(']', "\\]")
        .replace('<', "\\<")
        .replace('>', "\\>")
        .replace('*', "\\*")
        .replace('_', "\\_")
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
