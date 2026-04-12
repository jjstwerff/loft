// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

use std::fmt::{Arguments, Debug, Display, Formatter};

#[derive(PartialOrd, Ord, PartialEq, Eq, Debug, Clone, Copy)]
pub enum Level {
    Debug,
    Warning,
    Error,
    Fatal,
}

/// One diagnostic message with optional source location.
#[derive(Debug, Clone)]
pub struct DiagEntry {
    pub level: Level,
    pub message: String,
    pub file: String,
    pub line: u32,
    pub col: u32,
}

impl DiagEntry {
    /// Format as a single-line string: `Level: message at file:line:col`
    #[must_use]
    pub fn to_string_compact(&self) -> String {
        if self.file.is_empty() {
            format!("{:?}: {}", self.level, self.message)
        } else {
            format!(
                "{:?}: {} at {}:{}:{}",
                self.level, self.message, self.file, self.line, self.col
            )
        }
    }
}

pub struct Diagnostics {
    entries: Vec<DiagEntry>,
    level: Level,
}

impl Default for Diagnostics {
    fn default() -> Self {
        Self::new()
    }
}

impl Debug for Diagnostics {
    fn fmt(&self, fmt: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        let lines: Vec<String> = self.entries.iter().map(DiagEntry::to_string_compact).collect();
        fmt.write_str(&format!("{lines:?}"))
    }
}

impl Display for Diagnostics {
    fn fmt(&self, fmt: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        for (i, entry) in self.entries.iter().enumerate() {
            if i > 0 {
                fmt.write_str("\n")?;
            }
            fmt.write_str(&entry.to_string_compact())?;
        }
        Ok(())
    }
}

impl Diagnostics {
    #[must_use]
    pub fn new() -> Diagnostics {
        Diagnostics {
            entries: Vec::new(),
            level: Level::Debug,
        }
    }

    pub fn add(&mut self, level: Level, message: &str) {
        self.entries.push(DiagEntry {
            level,
            message: message.to_string(),
            file: String::new(),
            line: 0,
            col: 0,
        });
        if level > self.level {
            self.level = level;
        }
    }

    pub fn add_at(&mut self, level: Level, message: &str, file: &str, line: u32, col: u32) {
        self.entries.push(DiagEntry {
            level,
            message: message.to_string(),
            file: file.to_string(),
            line,
            col,
        });
        if level > self.level {
            self.level = level;
        }
    }

    pub fn fill(&mut self, other: &Diagnostics) {
        for e in &other.entries {
            self.entries.push(e.clone());
        }
        if other.level > self.level {
            self.level = other.level;
        }
    }

    /// Backward-compatible: return each entry as a formatted string.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        self.entries.iter().map(DiagEntry::to_string_compact).collect()
    }

    #[must_use]
    pub fn entries(&self) -> &[DiagEntry] {
        &self.entries
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub fn level(&self) -> Level {
        self.level
    }
}

#[must_use]
pub fn diagnostic_format(_level: Level, message: Arguments<'_>) -> String {
    format!("{message}")
}

#[macro_export]
macro_rules! diagnostic {
    ($lexer:expr, $level:expr, $($arg:tt)+) => (
        $lexer.diagnostic($level.clone(), &diagnostic_format($level, format_args!($($arg)+)))
    )
}

#[macro_export]
macro_rules! specific {
    ($lexer:expr, $result:expr, $level:expr, $($arg:tt)+) => (
        $lexer.specific($result, $level.clone(), &diagnostic_format($level, format_args!($($arg)+)))
    )
}

/// Levenshtein edit distance between two strings.
#[must_use]
pub fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        curr[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            curr[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(curr[j] + 1);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

/// Find the closest match to `name` among `candidates` (Levenshtein distance ≤ 2).
#[must_use]
pub fn suggest_similar<'a>(name: &str, candidates: &[&'a str]) -> Option<&'a str> {
    candidates
        .iter()
        .copied()
        .filter(|c| {
            let d = levenshtein(name, c);
            d > 0 && d <= 2
        })
        .min_by_key(|c| levenshtein(name, c))
}
