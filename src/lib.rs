// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
#![warn(clippy::pedantic)]

#[macro_use]
pub mod diagnostics;
mod calc;
pub mod data;
pub mod database;
pub mod hash;
pub mod keys;
mod lexer;
pub mod native;
pub mod scopes;
mod variables;
pub mod vector;

pub mod generation;
pub mod ops;
pub mod parser;
mod png_store;
mod radix_tree;
mod store;
pub mod tree;
mod typedef;

pub mod create;
pub mod fill;
pub mod parallel;
pub mod platform;
pub mod state;

pub mod compile;
pub mod log_config;
pub mod logger;
pub mod manifest;
mod stack;

pub mod documentation;
