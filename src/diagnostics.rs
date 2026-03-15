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

pub struct Diagnostics {
    lines: Vec<String>,
    level: Level,
}

impl Default for Diagnostics {
    fn default() -> Self {
        Self::new()
    }
}

impl Debug for Diagnostics {
    fn fmt(&self, fmt: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        fmt.write_str(&format!("{:?}", self.lines))
    }
}

impl Display for Diagnostics {
    fn fmt(&self, fmt: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        fmt.write_str(&format!("{:?}", self.lines))
    }
}

impl Diagnostics {
    #[must_use]
    pub fn new() -> Diagnostics {
        Diagnostics {
            lines: Vec::new(),
            level: Level::Debug,
        }
    }

    pub fn add(&mut self, level: Level, message: &str) {
        self.lines.push(message.to_string());
        if level > self.level {
            self.level = level;
        }
    }

    pub fn fill(&mut self, other: &Diagnostics) {
        for o in &other.lines {
            self.lines.push(o.clone());
        }
        if other.level > self.level {
            self.level = other.level;
        }
    }

    #[must_use]
    pub fn lines(&self) -> &Vec<String> {
        &self.lines
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    #[must_use]
    pub fn level(&self) -> Level {
        self.level
    }
}

#[must_use]
pub fn diagnostic_format(level: Level, message: Arguments<'_>) -> String {
    format!("{level:?}: {message}")
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
