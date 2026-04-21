// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

use std::collections::HashMap;
use std::fmt::Write;
use std::fs::{DirEntry, File};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

/// A stdlib section visible in the nav and the search index.
pub struct StdlibSection {
    pub id: String,          // URL-safe slug, e.g. "output-and-diagnostics"
    pub name: String,        // Human-readable label, e.g. "Output and diagnostics"
    pub description: String, // One-line description shown on the index page card
}

struct Topic {
    file: PathBuf,
    filename: String, // Stem without extension, e.g. "05-float"
    name: String,     // Short display name from @NAME header
    title: String,    // Descriptive title from @TITLE header
}

#[must_use]
fn gather_topics() -> Vec<Topic> {
    let mut result: Vec<Topic> = Vec::new();
    let dir = std::fs::read_dir("tests/docs").unwrap();
    let mut entries: Vec<_> = dir.filter_map(Result::ok).collect();
    entries.sort_by_key(DirEntry::file_name);
    entries
        .iter()
        .map(DirEntry::path)
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("loft"))
        })
        .for_each(|file| {
            let file_handle = File::open(file.clone()).expect("failed to open file");
            let reader = BufReader::new(file_handle);
            let filename = file.file_stem().unwrap().to_string_lossy().into_owned();
            let mut name = String::new();
            let mut title = String::new();
            reader.lines().for_each(|line_result| {
                if let Ok(line) = line_result {
                    if let Some(s) = line.strip_prefix("// @NAME: ") {
                        name = s.to_string();
                    }
                    if let Some(s) = line.strip_prefix("// @TITLE: ") {
                        title = s.to_string();
                    }
                }
            });
            result.push(Topic {
                file,
                filename,
                name,
                title,
            });
        });
    result
}

/// Call this after building the stdlib sections and link map so that language
/// pages are generated with stdlib links already inlined and their nav matches
/// the stdlib pages generated separately.
/// # Errors
/// When the `doc/` directory is unwritable.
pub fn generate_docs<S: std::hash::BuildHasher>(
    stdlib_sections: &[StdlibSection],
    link_map: &HashMap<String, String, S>,
    version: &str,
) -> std::io::Result<()> {
    let topics = gather_topics();
    write_index(&topics, stdlib_sections, version)?;
    let nav_info = topic_nav_info(&topics);
    for entry in &topics {
        if entry.filename.starts_with("00-") {
            continue;
        }
        let stem = entry
            .file
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        if let Ok(source) = std::fs::read_to_string(&entry.file) {
            let html = render_doc_page(
                &source,
                &entry.name,
                &stem,
                &nav_info,
                stdlib_sections,
                link_map,
            );
            std::fs::write(format!("doc/{stem}.html"), html)?;
        }
    }
    Ok(())
}

fn flush_intro_para(result: &mut String, para_buf: &mut String) {
    if !para_buf.is_empty() {
        writeln!(result, "<p>{}</p>", para_buf.trim()).expect("");
        para_buf.clear();
    }
}

fn index_intro(topic: &Topic) -> std::io::Result<String> {
    let mut result = String::new();
    let file = File::open(&topic.file).expect("failed to open file");
    let source = BufReader::new(file);
    let mut in_header = true;
    let mut in_list = false;
    let mut para_buf = String::new();
    for line_result in source.lines() {
        let line = line_result?;
        let trimmed = line.trim();
        if in_header && skip_header(trimmed) {
            continue;
        }
        in_header = false;
        if !trimmed.starts_with("//") {
            // Blank line or non-comment = paragraph / list break
            if in_list {
                writeln!(result, "</ul>").expect("");
                in_list = false;
            } else {
                flush_intro_para(&mut result, &mut para_buf);
            }
            continue;
        }
        let text = trimmed.strip_prefix("//").unwrap_or("").trim().to_string();
        // Stop at the first section heading — the index page shows only the
        // brief introductory paragraphs, not the full topic content.
        if text.starts_with("## ") || text.starts_with("### ") {
            break;
        }
        if let Some(n) = text.strip_prefix("- ") {
            flush_intro_para(&mut result, &mut para_buf);
            if !in_list {
                write!(result, "<ul>").expect("");
                in_list = true;
            }
            writeln!(result, "<li>{n}</li>").expect("");
        } else if text.is_empty() {
            // `//` alone on a line also breaks the paragraph
            if in_list {
                writeln!(result, "</ul>").expect("");
                in_list = false;
            } else {
                flush_intro_para(&mut result, &mut para_buf);
            }
        } else {
            if in_list {
                writeln!(result, "</ul>").expect("");
                in_list = false;
            }
            if !para_buf.is_empty() {
                para_buf.push(' ');
            }
            para_buf.push_str(&text);
        }
    }
    if in_list {
        writeln!(result, "</ul>").expect("");
    } else {
        flush_intro_para(&mut result, &mut para_buf);
    }
    Ok(result)
}

fn skip_header(trimmed: &str) -> bool {
    trimmed.starts_with("// Copyright")
        || trimmed.starts_with("// SPDX")
        || trimmed.starts_with("// @NAME: ")
        || trimmed.starts_with("// @TITLE: ")
        || trimmed.is_empty()
}

fn write_index(
    topics: &[Topic],
    stdlib_sections: &[StdlibSection],
    version: &str,
) -> std::io::Result<()> {
    let mut lang_cards = String::from(
        "      <a class=\"card card-featured\" href=\"00-vs-rust.html\">\
<h2>vs Rust</h2><p>Key differences for developers coming from Rust.</p></a>\n\
      <a class=\"card card-featured\" href=\"00-vs-python.html\">\
<h2>vs Python</h2><p>Key differences for developers coming from Python.</p></a>\n\
      <a class=\"card card-featured\" href=\"00-performance.html\">\
<h2>Performance</h2><p>Benchmark results across interpreter, native, wasm, and Rust.</p></a>\n",
    );
    for topic in topics {
        if !topic.filename.starts_with("00-") {
            let _ = writeln!(
                lang_cards,
                "      <a class=\"card\" href=\"{}.html\"><h2>{}</h2><p>{}</p></a>",
                topic.filename, topic.name, topic.title
            );
        }
    }
    let lib_cards: String = stdlib_sections
        .iter()
        .fold(String::new(), |mut output, sec| {
            if sec.description.is_empty() {
                let _ = writeln!(
                    output,
                    "      <a class=\"card\" href=\"stdlib-{}.html\"><h2>{}</h2></a>",
                    sec.id, sec.name
                );
            } else {
                let _ = writeln!(
                    output,
                    "      <a class=\"card\" href=\"stdlib-{}.html\"><h2>{}</h2><p>{}</p></a>",
                    sec.id, sec.name, sec.description
                );
            }
            output
        });
    let start_cards = concat!(
        "      <a class=\"card card-featured\" href=\"install.html\">",
        "<h2>Install</h2>",
        "<p>Get loft running and write your first Loft program in minutes.</p></a>\n",
        "      <a class=\"card\" href=\"roadmap.html\">",
        "<h2>Roadmap</h2>",
        "<p>Planned features for version 1.0 and beyond, with syntax previews.</p></a>\n",
    );
    let title = topics[0].title.clone();
    let intro = index_intro(&topics[0])?;
    let html = format!(
        "<!DOCTYPE html>\n\
<html lang=\"en\">\n\
<head>\n\
  <meta charset=\"utf-8\">\n\
  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
  <title>Loft Language</title>\n\
  <link rel=\"stylesheet\" href=\"style.css\">\n\
</head>\n\
<body>\n\
  <header class=\"index-header\">\n\
    <div class=\"index-hero\">\n\
      <h1>Loft</h1>\n\
      <p class=\"tagline\">Build small games and interactive things \u{2014} share a link, anyone plays.</p>\n\
      <p class=\"subtagline\">{title}</p>\n\
      <p class=\"version\">v{version}</p>\n\
      <div class=\"hero-ctas\">\n\
        <a class=\"hero-btn hero-btn-primary\" href=\"playground.html\">Try it in the browser</a>\n\
        <a class=\"hero-btn\" href=\"gallery.html\">See the gallery</a>\n\
        <a class=\"hero-btn\" href=\"install.html\">Install</a>\n\
      </div>\n\
    </div>\n\
    <div class=\"search-wrap index-search\">\n\
      <input id=\"search\" type=\"search\" placeholder=\"Search docs\u{2026}\" autocomplete=\"off\">\n\
      <div class=\"search-results\" id=\"search-results\" hidden></div>\n\
    </div>\n\
  </header>\n\
  <section class=\"showcase\">\n\
    <a class=\"showcase-tile showcase-hero\" href=\"brick-buster.html\">\n\
      <img src=\"images/hero-brick-buster.png\" alt=\"Brick Buster \u{2014} a complete loft game\" loading=\"lazy\">\n\
      <div class=\"showcase-caption\">\n\
        <span class=\"showcase-tag\">Built with loft</span>\n\
        <h2>Brick Buster</h2>\n\
        <p>A complete arcade game \u{2014} hand-designed levels, cel-shaded sprites, heart lives, round ball with a velocity-directional squash, rising balloon bombs, fireball after-images, chiptune music. Written in loft, runs in your browser.</p>\n\
      </div>\n\
    </a>\n\
    <div class=\"showcase-side\">\n\
      <a class=\"showcase-tile showcase-sub\" href=\"playground.html\">\n\
        <div class=\"showcase-caption\">\n\
          <span class=\"showcase-tag\">No install</span>\n\
          <h3>Live playground</h3>\n\
          <p>Type a few lines of loft code. Press run. See output. That is the whole tutorial.</p>\n\
        </div>\n\
      </a>\n\
      <a class=\"showcase-tile showcase-sub\" href=\"gallery.html\">\n\
        <div class=\"showcase-caption\">\n\
          <span class=\"showcase-tag\">24 demos</span>\n\
          <h3>Graphics gallery</h3>\n\
          <p>From a hello-triangle to physically-based rendering with shadows \u{2014} all running live in WebGL.</p>\n\
        </div>\n\
      </a>\n\
    </div>\n\
  </section>\n\
  <section class=\"intro\">\n\
{intro}\
  </section>\n\
  <section class=\"topics\">\n\
    <h2 class=\"topics-heading\">Getting Started</h2>\n\
    <div class=\"grid\">\n\
{start_cards}\
    </div>\n\
  </section>\n\
  <section class=\"topics\">\n\
    <h2 class=\"topics-heading\">Language</h2>\n\
    <div class=\"grid\">\n\
{lang_cards}\
    </div>\n\
  </section>\n\
  <section class=\"topics\">\n\
    <h2 class=\"topics-heading\">Standard Library</h2>\n\
    <div class=\"grid\">\n\
{lib_cards}\
    </div>\n\
  </section>\n\
  <script src=\"search-index.js\"></script>\n\
  <script src=\"search.js\"></script>\n\
</body>\n\
</html>\n"
    );
    std::fs::write("doc/index.html", html)
}

fn html_esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn to_anchor_id(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// Convert inline markdown in already-escaped HTML text:
/// `**bold**` → `<strong>bold</strong>`, `'code'` → `<code>code</code>`.
fn inline_format(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c == '*'
            && s[i + 1..].starts_with('*')
            && let Some(end) = s[i + 2..].find("**")
        {
            out.push_str("<strong>");
            out.push_str(&s[i + 2..i + 2 + end]);
            out.push_str("</strong>");
            // Skip past the closing **
            let skip_to = i + 2 + end + 2;
            while chars.peek().is_some_and(|(j, _)| *j < skip_to) {
                chars.next();
            }
        } else if (c == '\'' || c == '`')
            && let Some(end) = s[i + 1..].find(c)
            && end > 0
        {
            out.push_str("<code>");
            out.push_str(&s[i + 1..i + 1 + end]);
            out.push_str("</code>");
            let skip_to = i + 1 + end + 1;
            while chars.peek().is_some_and(|(j, _)| *j < skip_to) {
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn flush_para(para: &mut Vec<String>, body: &mut String) {
    if !para.is_empty() {
        let text = para.join(" ");
        body.push_str("<p>");
        body.push_str(&inline_format(&html_esc(&text)));
        body.push_str("</p>\n");
        para.clear();
    }
}

fn flush_list(in_list: &mut bool, body: &mut String) {
    if *in_list {
        body.push_str("</ul>\n");
        *in_list = false;
    }
}

fn flush_indented(block: &mut Vec<String>, body: &mut String) {
    if block.is_empty() {
        return;
    }
    body.push_str("<pre><code>");
    for line in block.iter() {
        body.push_str(&html_esc(line));
        body.push('\n');
    }
    body.push_str("</code></pre>\n");
    block.clear();
}

fn flush_list_item(item: &mut Vec<String>, body: &mut String) {
    if item.is_empty() {
        return;
    }
    let text = item.join(" ");
    let _ = writeln!(body, "<li>{}</li>", inline_format(&html_esc(&text)));
    item.clear();
}

/// Render prose lines into HTML, supporting `## Heading`, `### Sub-heading`,
/// `- list item` (with 2-space continuation), indented code blocks (2+ leading
/// spaces outside lists), and regular paragraph text.
fn render_prose_lines(lines: &[String], body: &mut String) {
    let mut para: Vec<String> = Vec::new();
    let mut in_list = false;
    let mut list_item: Vec<String> = Vec::new();
    let mut indented: Vec<String> = Vec::new();
    for line in lines {
        if let Some(heading) = line.strip_prefix("## ") {
            flush_indented(&mut indented, body);
            flush_list_item(&mut list_item, body);
            flush_list(&mut in_list, body);
            flush_para(&mut para, body);
            let id = to_anchor_id(heading);
            let _ = writeln!(
                body,
                "<h2 id=\"{id}\">{}</h2>",
                inline_format(&html_esc(heading))
            );
        } else if let Some(heading) = line.strip_prefix("### ") {
            flush_indented(&mut indented, body);
            flush_list_item(&mut list_item, body);
            flush_list(&mut in_list, body);
            flush_para(&mut para, body);
            let id = to_anchor_id(heading);
            let _ = writeln!(
                body,
                "<h3 id=\"{id}\">{}</h3>",
                inline_format(&html_esc(heading))
            );
        } else if let Some(item) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            flush_indented(&mut indented, body);
            flush_list_item(&mut list_item, body);
            flush_para(&mut para, body);
            if !in_list {
                body.push_str("<ul>\n");
                in_list = true;
            }
            list_item.push(item.to_string());
        } else if let Some(rest) = line.strip_prefix("  ") {
            if in_list {
                // Continuation of the current list item.
                list_item.push(rest.trim_start().to_string());
            } else if !para.is_empty() && indented.is_empty() {
                // Continuation of the current paragraph.
                para.push(rest.trim_start().to_string());
            } else {
                // Indented code block (only after an empty line / heading).
                flush_para(&mut para, body);
                indented.push(rest.to_string());
            }
        } else if line.is_empty() {
            flush_indented(&mut indented, body);
            flush_list_item(&mut list_item, body);
            flush_list(&mut in_list, body);
            flush_para(&mut para, body);
        } else {
            flush_indented(&mut indented, body);
            flush_list_item(&mut list_item, body);
            flush_list(&mut in_list, body);
            para.push(line.clone());
        }
    }
    flush_indented(&mut indented, body);
    flush_list_item(&mut list_item, body);
    flush_list(&mut in_list, body);
    flush_para(&mut para, body);
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
}

fn code_to_typst(code: &str) -> String {
    // Use "rust" lang tag as approximation — Typst doesn't know "loft" natively
    format!("```rust\n{code}\n```\n\n")
}

fn flush_typst_indented(block: &mut Vec<String>, result: &mut String) {
    if block.is_empty() {
        return;
    }
    result.push_str("```\n");
    for line in block.iter() {
        result.push_str(line);
        result.push('\n');
    }
    result.push_str("```\n\n");
    block.clear();
}

fn prose_to_typst(lines: &[String]) -> String {
    let mut result = String::new();
    let mut para: Vec<String> = Vec::new();
    let mut in_list = false;
    let mut indented: Vec<String> = Vec::new();
    for line in lines {
        if let Some(heading) = line.strip_prefix("## ") {
            flush_typst_indented(&mut indented, &mut result);
            if in_list {
                result.push('\n');
                in_list = false;
            }
            if !para.is_empty() {
                result.push_str(&para.join(" "));
                result.push_str("\n\n");
                para.clear();
            }
            let _ = write!(result, "=== {}\n\n", typst_escape(heading));
        } else if let Some(heading) = line.strip_prefix("### ") {
            flush_typst_indented(&mut indented, &mut result);
            if in_list {
                result.push('\n');
                in_list = false;
            }
            if !para.is_empty() {
                result.push_str(&para.join(" "));
                result.push_str("\n\n");
                para.clear();
            }
            let _ = write!(result, "==== {}\n\n", typst_escape(heading));
        } else if let Some(item) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            flush_typst_indented(&mut indented, &mut result);
            if !para.is_empty() {
                result.push_str(&para.join(" "));
                result.push_str("\n\n");
                para.clear();
            }
            in_list = true;
            let _ = writeln!(result, "- {}", typst_escape(item));
        } else if let Some(rest) = line.strip_prefix("  ") {
            if in_list {
                result.push('\n');
                in_list = false;
            }
            if !para.is_empty() {
                result.push_str(&para.join(" "));
                result.push_str("\n\n");
                para.clear();
            }
            indented.push(rest.to_string());
        } else if line.is_empty() {
            flush_typst_indented(&mut indented, &mut result);
            if in_list {
                result.push('\n');
                in_list = false;
            }
            if !para.is_empty() {
                result.push_str(&para.join(" "));
                result.push_str("\n\n");
                para.clear();
            }
        } else {
            flush_typst_indented(&mut indented, &mut result);
            if in_list {
                result.push('\n');
                in_list = false;
            }
            para.push(typst_escape(line));
        }
    }
    flush_typst_indented(&mut indented, &mut result);
    if in_list {
        result.push('\n');
    }
    if !para.is_empty() {
        result.push_str(&para.join(" "));
        result.push_str("\n\n");
    }
    result
}

// ─── Section parser ───────────────────────────────────────────────────────────

enum DocSection {
    /// Consecutive `//` comment lines with the `// ` prefix stripped.
    Prose(Vec<String>),
    /// Consecutive non-comment source lines kept verbatim.
    Code(Vec<String>),
}

fn parse_sections(source: &str) -> Vec<DocSection> {
    let mut sections: Vec<DocSection> = Vec::new();
    let mut prose: Vec<String> = Vec::new();
    let mut code: Vec<String> = Vec::new();
    let mut in_header = true;

    for line in source.lines() {
        let trimmed = line.trim();
        if in_header && skip_header(trimmed) {
            continue;
        }
        in_header = false;

        if trimmed.starts_with("//") {
            if !code.is_empty() {
                sections.push(DocSection::Code(std::mem::take(&mut code)));
            }
            let after_slashes = trimmed.strip_prefix("//").unwrap_or("");
            // Strip at most one leading space to preserve indentation.
            let text = after_slashes
                .strip_prefix(' ')
                .unwrap_or(after_slashes)
                .to_string();
            prose.push(text);
        } else if trimmed.is_empty() {
            if !prose.is_empty() {
                sections.push(DocSection::Prose(std::mem::take(&mut prose)));
            }
            if !code.is_empty() {
                sections.push(DocSection::Code(std::mem::take(&mut code)));
            }
        } else {
            if !prose.is_empty() {
                sections.push(DocSection::Prose(std::mem::take(&mut prose)));
            }
            code.push(line.to_string());
        }
    }
    if !prose.is_empty() {
        sections.push(DocSection::Prose(prose));
    }
    if !code.is_empty() {
        sections.push(DocSection::Code(code));
    }
    sections
}

// ─── Syntax highlighter ───────────────────────────────────────────────────────

const KW: &[&str] = &[
    "fn", "if", "else", "for", "in", "return", "break", "continue", "struct", "enum", "pub", "use",
    "type", "as", "not", "null", "true", "false", "and", "or", "limit", "default", "virtual",
];
const TY: &[&str] = &[
    "integer",
    "text",
    "boolean",
    "float",
    "single",
    "character",
    "vector",
    "sorted",
    "index",
    "hash",
    "reference",
    "u8",
    "u16",
    "u32",
    "i8",
    "i16",
    "i32",
    "i64",
];
const BI: &[&str] = &[
    "assert", "panic", "len", "round", "ceil", "floor", "abs", "sin", "cos", "log", "rev", "file",
    "min", "max", "sqrt", "typeof", "typedef",
];

fn scan_quoted(chars: &[char], i: usize, n: usize, delim: char) -> usize {
    let mut j = i + 1;
    while j < n && chars[j] != delim {
        j += 1;
    }
    if j < n { j + 1 } else { j }
}

fn scan_number(chars: &[char], i: usize, n: usize) -> usize {
    let mut j = i;
    if chars[i] == '0' && i + 1 < n {
        match chars[i + 1] {
            'x' | 'X' => {
                j += 2;
                while j < n && chars[j].is_ascii_hexdigit() {
                    j += 1;
                }
            }
            'b' | 'B' => {
                j += 2;
                while j < n && (chars[j] == '0' || chars[j] == '1') {
                    j += 1;
                }
            }
            'o' | 'O' => {
                j += 2;
                while j < n && chars[j].is_ascii_digit() {
                    j += 1;
                }
            }
            _ => {
                while j < n && (chars[j].is_ascii_digit() || chars[j] == '.' || chars[j] == '_') {
                    j += 1;
                }
            }
        }
    } else {
        while j < n && (chars[j].is_ascii_digit() || chars[j] == '.' || chars[j] == '_') {
            j += 1;
        }
    }
    if j < n && (chars[j] == 'l' || chars[j] == 'f') {
        j += 1;
    }
    j
}

fn word_class(word: &str, is_call: bool) -> &'static str {
    if KW.contains(&word) {
        "kw"
    } else if TY.contains(&word) {
        "ty"
    } else if BI.contains(&word) {
        "bi"
    } else if word.starts_with(|c: char| c.is_uppercase()) {
        "en"
    } else if is_call {
        "fn-call"
    } else {
        ""
    }
}

fn emit_span<S: std::hash::BuildHasher>(
    out: &mut String,
    cls: &str,
    word: &str,
    link_map: &HashMap<String, String, S>,
) {
    out.push_str("<span class=\"");
    out.push_str(cls);
    out.push_str("\">");
    if let Some(url) = link_map.get(word) {
        out.push_str("<a href=\"");
        out.push_str(url);
        out.push_str("\">");
        out.push_str(&html_esc(word));
        out.push_str("</a>");
    } else {
        out.push_str(&html_esc(word));
    }
    out.push_str("</span>");
}

/// Use this instead of raw HTML concatenation for code blocks; stdlib links are
/// injected here so no separate post-processing pass over the HTML is needed.
fn highlight_loft<S: std::hash::BuildHasher>(
    code: &str,
    link_map: &HashMap<String, String, S>,
) -> String {
    let mut out = String::with_capacity(code.len() * 2);

    for line in code.lines() {
        let chars: Vec<char> = line.chars().collect();
        let char_count = chars.len();
        let mut pos = 0;

        while pos < char_count {
            if pos + 1 < char_count && chars[pos] == '/' && chars[pos + 1] == '/' {
                let rest: String = chars[pos..].iter().collect();
                out.push_str("<span class=\"cm\">");
                out.push_str(&html_esc(&rest));
                out.push_str("</span>");
                pos = char_count;
                continue;
            }

            if chars[pos] == '"' {
                let end = scan_quoted(&chars, pos, char_count, '"');
                let token: String = chars[pos..end].iter().collect();
                out.push_str("<span class=\"st\">");
                out.push_str(&html_esc(&token));
                out.push_str("</span>");
                pos = end;
                continue;
            }

            if chars[pos] == '\'' {
                let end = scan_quoted(&chars, pos, char_count, '\'');
                let token: String = chars[pos..end].iter().collect();
                out.push_str("<span class=\"ch\">");
                out.push_str(&html_esc(&token));
                out.push_str("</span>");
                pos = end;
                continue;
            }

            if chars[pos].is_ascii_digit() {
                let end = scan_number(&chars, pos, char_count);
                let token: String = chars[pos..end].iter().collect();
                out.push_str("<span class=\"nm\">");
                out.push_str(&html_esc(&token));
                out.push_str("</span>");
                pos = end;
                continue;
            }

            if chars[pos].is_alphabetic() || chars[pos] == '_' {
                let mut end = pos;
                while end < char_count && (chars[end].is_alphanumeric() || chars[end] == '_') {
                    end += 1;
                }
                let word: String = chars[pos..end].iter().collect();
                let mut peek = end;
                while peek < char_count && chars[peek] == ' ' {
                    peek += 1;
                }
                let cls = word_class(&word, peek < char_count && chars[peek] == '(');
                if cls.is_empty() {
                    out.push_str(&html_esc(&word));
                } else {
                    emit_span(&mut out, cls, &word, link_map);
                }
                pos = end;
                continue;
            }

            out.push_str(&html_esc(&chars[pos].to_string()));
            pos += 1;
        }
        out.push('\n');
    }

    if out.ends_with('\n') {
        out.pop();
    }
    out
}

// ─── Nav builder ──────────────────────────────────────────────────────────────

/// Shared transformation from a loaded topic list to (filename, name) pairs.
/// Separates the disk-reading concern in `gather_topic_info` from the filtering
/// logic used by `render_doc_page` on its already-loaded topic slice.
fn topic_nav_info(topics: &[Topic]) -> Vec<(String, String)> {
    topics
        .iter()
        .filter(|t| !t.filename.starts_with("00-"))
        .map(|t| (t.filename.clone(), t.name.clone()))
        .collect()
}

/// Use when building stdlib pages outside of `generate_docs`, where the full
/// `Topic` list is not already in scope.
#[must_use]
pub fn gather_topic_info() -> Vec<(String, String)> {
    topic_nav_info(&gather_topics())
}

/// Use to get consistent nav HTML for any page — language topic or stdlib
/// section — so that switching page types does not break the nav structure.
/// `active` is the filename stem of the current page.
#[must_use]
pub fn build_nav(
    topic_info: &[(String, String)],
    stdlib_sections: &[StdlibSection],
    active: &str,
) -> String {
    let mut parts: Vec<String> = Vec::new();

    parts.push("<a href=\"index.html\">Home</a>".to_string());

    // Hand-maintained utility pages before the Language section.
    if active == "install" {
        parts.push("<span class=\"cur\">Install</span>".to_string());
    } else {
        parts.push("<a href=\"install.html\">Install</a>".to_string());
    }
    if active == "roadmap" {
        parts.push("<span class=\"cur\">Roadmap</span>".to_string());
    } else {
        parts.push("<a href=\"roadmap.html\">Roadmap</a>".to_string());
    }

    parts.push("<span class=\"nav-sep\">Language:</span>".to_string());

    // vs-Rust and vs-Python are hand-maintained pages with no corresponding .loft file.
    if active == "00-vs-rust" {
        parts.push("<span class=\"cur\">vs Rust</span>".to_string());
    } else {
        parts.push("<a href=\"00-vs-rust.html\">vs Rust</a>".to_string());
    }
    if active == "00-vs-python" {
        parts.push("<span class=\"cur\">vs Python</span>".to_string());
    } else {
        parts.push("<a href=\"00-vs-python.html\">vs Python</a>".to_string());
    }
    if active == "00-performance" {
        parts.push("<span class=\"cur\">Performance</span>".to_string());
    } else {
        parts.push("<a href=\"00-performance.html\">Performance</a>".to_string());
    }

    for (filename, name) in topic_info {
        if filename == active {
            parts.push(format!("<span class=\"cur\">{name}</span>"));
        } else {
            parts.push(format!("<a href=\"{filename}.html\">{name}</a>"));
        }
    }

    if !stdlib_sections.is_empty() {
        parts.push("<span class=\"nav-sep\">Library:</span>".to_string());
        for sec in stdlib_sections {
            let stem = format!("stdlib-{}", sec.id);
            if stem == active {
                parts.push(format!("<span class=\"cur\">{}</span>", sec.name));
            } else {
                parts.push(format!(
                    "<a href=\"stdlib-{}.html\">{}</a>",
                    sec.id, sec.name
                ));
            }
        }
    }

    let links = parts.join(" · ");
    format!(
        "<div class=\"nav-links\">{links}</div>\
<div class=\"search-wrap\">\
<input type=\"search\" id=\"search\" placeholder=\"Search functions, types…\" autocomplete=\"off\">\
<div id=\"search-results\" class=\"search-results\" hidden></div>\
</div>"
    )
}

// ─── HTML page renderer ───────────────────────────────────────────────────────

/// Use to get consistent page structure for both language topic pages and stdlib
/// section pages; avoids duplicating the HTML boilerplate in two places.
#[must_use]
pub fn page_html(title: &str, nav: &str, h1: &str, body: &str) -> String {
    format!(
        "<!DOCTYPE html>\n\
<html lang=\"en\">\n\
<head>\n\
  <meta charset=\"utf-8\">\n\
  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
  <title>Loft - {title}</title>\n\
  <link rel=\"stylesheet\" href=\"style.css\">\n\
  <style>.playground-link{{display:inline-block;margin:0.3em 0 0.8em;padding:4px 12px;\
background:#2563eb;color:#fff;border-radius:4px;text-decoration:none;font-size:0.85em}}\
.playground-link:hover{{filter:brightness(1.15)}}\
@media print{{.playground-link{{display:none}}}}</style>\n\
</head>\n\
<body>\n\
  <nav>{nav}</nav>\n\
  <h1>{h1}</h1>\n\
  <article>\n{body}\n  </article>\n\
  <script src=\"search-index.js\"></script>\n\
  <script src=\"search.js\"></script>\n\
  <script>\n\
(function(){{\n\
  var m={{\"Keywords\":\"keywords\",\"Texts\":\"texts\",\"Integers\":\"integers\",\
\"Boolean\":\"boolean\",\"Float\":\"float\",\"Functions\":\"functions\",\
\"Vector\":\"vector\",\"Structs\":\"structs\",\"Enums\":\"enums\",\
\"Sorted\":\"sorted\",\"Index\":\"index\",\"Hash\":\"hash\",\"File\":\"file\",\
\"Lexer\":\"lexer\",\"Parser\":\"parser\",\"Libraries\":\"libraries\",\
\"Store Locks\":\"store_locks\",\"Parallel execution\":\"parallel_execution\",\
\"Logging\":\"logging\",\"Time\":\"time\",\"Safety\":\"safety\",\"JSON\":\"json\",\
\"Generics\":\"generics\",\"Closures\":\"closures\",\"Coroutines\":\"coroutines\",\
\"Tuples\":\"tuples\",\"Match\":\"match\",\"Formatting\":\"formatting\"}};\n\
  var h=document.querySelector('h1');\n\
  if(!h)return;\n\
  var key=m[h.textContent];\n\
  if(!key)return;\n\
  var a=document.createElement('a');\n\
  a.className='playground-link';\n\
  a.href='playground.html?example='+key;\n\
  a.textContent='\\u25B6 Try in Playground';\n\
  h.parentNode.insertBefore(a,h.nextSibling);\n\
}})();\n\
  </script>\n\
</body>\n\
</html>\n"
    )
}

/// Render the body content of a topic page (prose + code, no nav or HTML frame).
/// Supports `// ## Section`, `// ### Sub-section`, and `// - list item` in prose.
#[must_use]
pub fn render_topic_body<S: std::hash::BuildHasher>(
    source: &str,
    link_map: &HashMap<String, String, S>,
) -> String {
    let mut body = String::new();
    for section in parse_sections(source) {
        match section {
            DocSection::Prose(lines) => render_prose_lines(&lines, &mut body),
            DocSection::Code(lines) => {
                let highlighted = highlight_loft(&lines.join("\n"), link_map);
                body.push_str("<pre><code>");
                body.push_str(&highlighted);
                body.push_str("</code></pre>\n");
            }
        }
    }
    body
}

/// Render a topic page's content as Typst markup (no document header; use within a `=` section).
/// `## heading` maps to `===`, `### heading` to `====`.
#[must_use]
pub fn render_topic_typst(source: &str) -> String {
    let mut out = String::new();
    for section in parse_sections(source) {
        match section {
            DocSection::Prose(lines) => out.push_str(&prose_to_typst(&lines)),
            DocSection::Code(lines) => out.push_str(&code_to_typst(&lines.join("\n"))),
        }
    }
    out
}

/// A topic source file's metadata and content, ready for print/typst rendering.
pub struct TopicSource {
    pub filename: String,
    pub name: String,
    pub title: String,
    pub source: String,
}

/// Collect all non-`00-` topic source files for use in print/typst generation.
#[must_use]
pub fn get_topic_sources() -> Vec<TopicSource> {
    gather_topics()
        .into_iter()
        .filter(|t| !t.filename.starts_with("00-"))
        .filter_map(|t| {
            std::fs::read_to_string(&t.file)
                .ok()
                .map(|source| TopicSource {
                    filename: t.filename,
                    name: t.name,
                    title: t.title,
                    source,
                })
        })
        .collect()
}

fn render_doc_page<S: std::hash::BuildHasher>(
    source: &str,
    name: &str,
    active: &str,
    topic_info: &[(String, String)],
    stdlib_sections: &[StdlibSection],
    link_map: &HashMap<String, String, S>,
) -> String {
    let nav = build_nav(topic_info, stdlib_sections, active);
    let body = render_topic_body(source, link_map);
    page_html(name, &nav, name, &body)
}

// ─── Package documentation generation ────────────────────────────────────────

/// Build a navigation bar for a package's documentation pages.
/// Contains: package name home link, topic pages, API section links.
fn build_pkg_nav(
    pkg_name: &str,
    topic_info: &[(String, String)],
    api_sections: &[String],
    active: &str,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    if active == "index" {
        parts.push(format!("<span class=\"cur\">{pkg_name}</span>"));
    } else {
        parts.push(format!("<a href=\"index.html\">{pkg_name}</a>"));
    }

    if !topic_info.is_empty() {
        parts.push("<span class=\"nav-sep\">Guides:</span>".to_string());
        for (filename, name) in topic_info {
            if filename == active {
                parts.push(format!("<span class=\"cur\">{name}</span>"));
            } else {
                parts.push(format!("<a href=\"{filename}.html\">{name}</a>"));
            }
        }
    }

    if !api_sections.is_empty() {
        parts.push("<span class=\"nav-sep\">API:</span>".to_string());
        for section_name in api_sections {
            let id = slugify(section_name);
            let stem = format!("api-{id}");
            if stem == active {
                parts.push(format!("<span class=\"cur\">{section_name}</span>"));
            } else {
                parts.push(format!("<a href=\"api-{id}.html\">{section_name}</a>"));
            }
        }
    }

    parts.join(" · ")
}

/// Convert a section name to a URL-safe slug.
fn slugify(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// Parsed API section from a package source file.
struct PkgApiSection {
    name: String,
    items: Vec<(String, Vec<String>)>, // (signature, doc_lines)
}

/// Parse `pub` items and `// --- Section ---` headers from a source file.
fn parse_pkg_api(content: &str) -> Vec<PkgApiSection> {
    let mut sections = Vec::new();
    let mut current_name = "General".to_string();
    let mut items: Vec<(String, Vec<String>)> = Vec::new();
    let mut doc: Vec<String> = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        // Section header
        if trimmed.starts_with("// ---") && trimmed.ends_with("---") {
            let inner = trimmed
                .trim_start_matches('/')
                .trim()
                .trim_matches('-')
                .trim();
            if !inner.is_empty() {
                if !items.is_empty() {
                    sections.push(PkgApiSection {
                        name: current_name.clone(),
                        items: std::mem::take(&mut items),
                    });
                }
                current_name = inner.to_string();
                doc.clear();
                continue;
            }
        }
        // Comment → accumulate doc
        if trimmed.starts_with("//") {
            let text = trimmed.trim_start_matches('/').trim().to_string();
            doc.push(text);
            continue;
        }
        // Public item
        if trimmed.starts_with("pub ") {
            let sig = strip_pub_body(trimmed);
            items.push((sig, std::mem::take(&mut doc)));
            continue;
        }
        // Other lines: clear doc accumulation (unless #rust annotation)
        if !trimmed.starts_with('#') {
            doc.clear();
        }
    }
    if !items.is_empty() {
        sections.push(PkgApiSection {
            name: current_name,
            items,
        });
    }
    sections
}

/// Strip function body from a pub declaration, keeping just the signature.
fn strip_pub_body(line: &str) -> String {
    // For structs/enums, keep the full first line
    if line.starts_with("pub struct") || line.starts_with("pub enum") {
        return line.to_string();
    }
    // For functions, strip body after `{`
    if let Some(pos) = line.find('{') {
        line[..pos].trim().to_string()
    } else {
        line.trim_end_matches(';').to_string()
    }
}

/// Render an API section page as HTML body content.
fn render_api_section_body(section: &PkgApiSection) -> String {
    let mut body = String::new();
    for (sig, doc_lines) in &section.items {
        body.push_str("<div class=\"item\">\n");
        if !sig.is_empty() {
            writeln!(body, "<pre><code>{}</code></pre>", html_escape(sig)).expect("");
        }
        if !doc_lines.is_empty() {
            body.push_str("<p>");
            body.push_str(&doc_lines.join(" "));
            body.push_str("</p>\n");
        }
        body.push_str("</div>\n");
    }
    body
}

/// Escape HTML special characters in a string.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Generate documentation for a package directory.
///
/// Expects the standard package layout:
/// - `loft.toml` — package manifest (name, version)
/// - `src/*.loft` — source files (scanned for `pub` API docs)
/// - `docs/*.loft` — topic/guide pages (optional)
///
/// Generates HTML files in `doc/` (created if missing):
/// - `doc/index.html` — package overview
/// - `doc/<topic>.html` — guide pages from `docs/*.loft`
/// - `doc/api-<section>.html` — API reference from `src/*.loft`
///
/// # Errors
/// Returns `Err` when files cannot be read or written.
#[allow(clippy::too_many_lines)]
pub fn generate_pkg_docs(pkg_dir: &std::path::Path) -> std::io::Result<()> {
    let manifest_path = pkg_dir.join("loft.toml");
    let manifest = if manifest_path.exists() {
        crate::manifest::read_manifest(&manifest_path.to_string_lossy()).unwrap_or_default()
    } else {
        crate::manifest::Manifest::default()
    };
    let pkg_name = manifest.name.unwrap_or_else(|| {
        pkg_dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    });
    let version = manifest.version.unwrap_or_else(|| "0.0.0".to_string());

    // Create doc/ output directory.
    let out_dir = pkg_dir.join("doc");
    std::fs::create_dir_all(&out_dir)?;

    // Copy style.css from the main doc directory if it exists.
    let main_style = std::path::Path::new("doc/style.css");
    let pkg_style = out_dir.join("style.css");
    if main_style.exists() && !pkg_style.exists() {
        std::fs::copy(main_style, &pkg_style)?;
    }

    // Gather topic pages from docs/*.loft
    let docs_dir = pkg_dir.join("docs");
    let topics = if docs_dir.is_dir() {
        gather_pkg_topics(&docs_dir)
    } else {
        Vec::new()
    };
    let topic_info: Vec<(String, String)> = topics
        .iter()
        .map(|t| (t.filename.clone(), t.name.clone()))
        .collect();

    // Parse API sections from src/*.loft
    let src_dir = pkg_dir.join("src");
    let mut all_api_sections = Vec::new();
    if src_dir.is_dir() {
        let mut src_files: Vec<_> = std::fs::read_dir(&src_dir)?
            .filter_map(Result::ok)
            .filter(|e| {
                e.path()
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("loft"))
            })
            .collect();
        src_files.sort_by_key(std::fs::DirEntry::file_name);
        for entry in src_files {
            if let Ok(content) = std::fs::read_to_string(entry.path()) {
                all_api_sections.extend(parse_pkg_api(&content));
            }
        }
    }
    let section_names: Vec<String> = all_api_sections.iter().map(|s| s.name.clone()).collect();

    // Generate index page.
    let nav = build_pkg_nav(&pkg_name, &topic_info, &section_names, "index");
    let mut index_body = String::new();
    writeln!(index_body, "<p><strong>{pkg_name}</strong> v{version}</p>").expect("");
    if !topic_info.is_empty() {
        index_body.push_str("<h2>Guides</h2>\n<ul>\n");
        for (filename, name) in &topic_info {
            writeln!(
                index_body,
                "<li><a href=\"{filename}.html\">{name}</a></li>"
            )
            .expect("");
        }
        index_body.push_str("</ul>\n");
    }
    if !section_names.is_empty() {
        index_body.push_str("<h2>API Reference</h2>\n<ul>\n");
        for name in &section_names {
            let id = slugify(name);
            writeln!(index_body, "<li><a href=\"api-{id}.html\">{name}</a></li>").expect("");
        }
        index_body.push_str("</ul>\n");
    }
    let index_html = page_html(&pkg_name, &nav, &pkg_name, &index_body);
    std::fs::write(out_dir.join("index.html"), index_html)?;

    // Generate topic pages.
    let link_map: HashMap<String, String> = HashMap::new();
    for topic in &topics {
        let stem = &topic.filename;
        let nav = build_pkg_nav(&pkg_name, &topic_info, &section_names, stem);
        if let Ok(source) = std::fs::read_to_string(&topic.file) {
            let body = render_topic_body(&source, &link_map);
            let html = page_html(&topic.name, &nav, &topic.name, &body);
            std::fs::write(out_dir.join(format!("{stem}.html")), html)?;
        }
    }

    // Generate API section pages.
    for section in &all_api_sections {
        let id = slugify(&section.name);
        let stem = format!("api-{id}");
        let nav = build_pkg_nav(&pkg_name, &topic_info, &section_names, &stem);
        let body = render_api_section_body(section);
        let html = page_html(&section.name, &nav, &section.name, &body);
        std::fs::write(out_dir.join(format!("{stem}.html")), html)?;
    }

    let topic_count = topics.len();
    let api_count = all_api_sections.len();
    println!(
        "Generated docs for {pkg_name}: {topic_count} guide(s), {api_count} API section(s) → {}",
        out_dir.display()
    );
    Ok(())
}

/// Gather topic files from a package's docs/ directory.
fn gather_pkg_topics(docs_dir: &std::path::Path) -> Vec<Topic> {
    let mut result = Vec::new();
    let Ok(dir) = std::fs::read_dir(docs_dir) else {
        return result;
    };
    let mut entries: Vec<_> = dir.filter_map(Result::ok).collect();
    entries.sort_by_key(DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        if !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("loft"))
        {
            continue;
        }
        let filename = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let mut name = filename.clone();
        let mut title = String::new();
        if let Ok(file) = File::open(&path) {
            let reader = BufReader::new(file);
            for line in reader.lines().map_while(Result::ok) {
                if let Some(s) = line.strip_prefix("// @NAME: ") {
                    name = s.to_string();
                }
                if let Some(s) = line.strip_prefix("// @TITLE: ") {
                    title = s.to_string();
                }
            }
        }
        result.push(Topic {
            file: path,
            filename,
            name,
            title,
        });
    }
    result
}
