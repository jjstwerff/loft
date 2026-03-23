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
      <p class=\"tagline\">{title}</p>\n\
      <p class=\"version\">v{version}</p>\n\
    </div>\n\
    <div class=\"search-wrap index-search\">\n\
      <input id=\"search\" type=\"search\" placeholder=\"Search docs\u{2026}\" autocomplete=\"off\">\n\
      <div class=\"search-results\" id=\"search-results\" hidden></div>\n\
    </div>\n\
  </header>\n\
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
    "long",
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
</head>\n\
<body>\n\
  <nav>{nav}</nav>\n\
  <h1>{h1}</h1>\n\
  <article>\n{body}\n  </article>\n\
  <script src=\"search-index.js\"></script>\n\
  <script src=\"search.js\"></script>\n\
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
