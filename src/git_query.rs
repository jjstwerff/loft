// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @PLN119 arc F — the git natives behind `lib/git`.

//! loft has no subprocess primitive, and [C101](../doc/claude/DESIGN_DECISIONS.md)
//! declines one: `run(cmd, args)` is a second, weaker interface beside the typed
//! library calls loft already has, and every consumer of it re-parses text loft
//! already knows how to type.
//!
//! So an external command lives INSIDE a vetted library. This module is the
//! mechanics half of `lib/git` — the same arrangement as `lib/engine_host`,
//! whose natives live in the binary rather than in a cdylib, because a
//! privileged host capability is the binary's to grant.
//!
//! # The caller never composes a command line
//!
//! @PLN119 claims this "removes the injection surface **by construction**, rather
//! than by the argument-vector-not-a-string rule". Taken literally that rules out
//! an `args: vector<text>` entry point, because that IS the argv rule. So the
//! surface is a **closed query vocabulary**: [`Query`] names the questions, and
//! each one's argv is built HERE. What the caller supplies is values — a ref, a
//! path, a count — never options and never a subcommand.
//!
//! Two consequences worth stating:
//!
//! * A path always follows `--`, so a file called `--upload-pack=…` is a path.
//! * `git -c <key>=<value>` is unreachable, which matters because several git
//!   config keys (`core.pager`, `core.sshCommand`, `alias.*`) name a program to
//!   run. A general argv would have handed that back.
//!
//! The price, which is real: a new question needs a new [`Query`] and a loft
//! release. That is why this is `lib/git` for loft's own tooling and not a
//! general subprocess library.

use crate::database::Stores;
use crate::keys::{DbRef, Str};

/// The questions `lib/git` may ask. The numbers are a wire between the loft
/// declaration and this table, so they are **append-only**: renumbering one
/// silently re-points a released library at a different question.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
enum Query {
    /// The current branch name (`HEAD` when detached).
    Branch = 0,
    /// The abbreviated sha of `HEAD`.
    HeadSha = 1,
    /// `HEAD`'s subject line.
    HeadSubject = 2,
    /// Does this ref resolve? Answers the sha, or nothing.
    HaveRef = 3,
    /// `<behind>\t<ahead>` for `<ref>...HEAD`.
    AheadBehind = 4,
    /// Name-status of the files that differ from `<ref>`.
    Changed = 5,
    /// The last `<n>` commits, one per line.
    Log = 6,
    /// Working-tree status, porcelain v1.
    Status = 7,
    /// The diff of one path against `<ref>`.
    DiffFile = 8,
    /// One commit in full.
    Show = 9,
    /// One commit's per-file `<adds>\t<dels>\t<path>`.
    ShowNumstat = 10,
    /// The abbreviated shas of the last `<n>` commits, one per line.
    LogShas = 11,
    /// The paths that differ from `<ref>`, one per line.
    ChangedNames = 12,
}

impl Query {
    fn from_code(code: i64) -> Option<Query> {
        Some(match code {
            0 => Query::Branch,
            1 => Query::HeadSha,
            2 => Query::HeadSubject,
            3 => Query::HaveRef,
            4 => Query::AheadBehind,
            5 => Query::Changed,
            6 => Query::Log,
            7 => Query::Status,
            8 => Query::DiffFile,
            9 => Query::Show,
            10 => Query::ShowNumstat,
            11 => Query::LogShas,
            12 => Query::ChangedNames,
            _ => return None,
        })
    }
}

/// The unit separator, `%x1f`.
///
/// `tools/viewer/refresh.sh` splits `git log` output on TAB, and a commit
/// subject may CONTAIN a tab — at which point the fields silently shift. A
/// subject cannot contain `\x1f` (git would have to be asked to put one there),
/// so this is one fewer bug than the thing it replaces.
const US: &str = "%x1f";

/// Build the argv for `query`. `a` and `b` are values — a ref, a path, a sha —
/// and never reach an option position.
fn argv(query: Query, a: &str, b: &str, n: i64) -> Vec<String> {
    let s = |v: &str| v.to_string();
    let count = format!("-{}", n.clamp(1, 100_000));
    match query {
        Query::Branch => vec![s("rev-parse"), s("--abbrev-ref"), s("HEAD")],
        Query::HeadSha => vec![s("rev-parse"), s("--short"), s("HEAD")],
        Query::HeadSubject => vec![s("log"), s("-1"), s("--pretty=%s")],
        // `--verify --quiet` so an absent ref is an exit code rather than a
        // message on stderr the caller would have to recognise.
        Query::HaveRef => vec![s("rev-parse"), s("--verify"), s("--quiet"), s(a)],
        Query::AheadBehind => vec![
            s("rev-list"),
            s("--left-right"),
            s("--count"),
            format!("{a}...HEAD"),
        ],
        Query::Changed => vec![s("diff"), s("--name-status"), format!("{a}...HEAD")],
        Query::ChangedNames => vec![s("diff"), s("--name-only"), format!("{a}...HEAD")],
        Query::Log => vec![
            s("log"),
            count,
            format!("--pretty=%h{US}%ad{US}%s"),
            s("--date=short"),
        ],
        Query::LogShas => vec![s("log"), count, s("--pretty=%h")],
        Query::Status => vec![s("status"), s("--short")],
        // The `--` is what makes `b` a PATH rather than whatever it looks like.
        Query::DiffFile => vec![s("diff"), format!("{a}...HEAD"), s("--"), s(b)],
        Query::Show => vec![s("show"), s(a)],
        Query::ShowNumstat => vec![s("show"), s(a), s("--numstat"), s("--pretty=format:")],
    }
}

/// A ref the caller supplied, or nothing.
///
/// Interpolating a ref into a range (`{a}...HEAD`) does NOT keep it out of an
/// option position, and believing it did was this module's one real hole — found
/// by the test written to state the claim, not by review.
/// `--exec-path=/tmp` becomes `--exec-path=/tmp...HEAD`, which git still reads as
/// `--exec-path` because that is decided by the leading `-`.
///
/// A git ref cannot begin with `-` (`git check-ref-format` says so), so refusing
/// one costs nothing real and closes the position by construction. A PATH may
/// begin with `-` and is not checked here — it always follows `--`, which is
/// exactly what `--` is for.
fn ref_ok(a: &str) -> bool {
    !a.starts_with('-')
}

/// Run one query and answer `(exit code, stdout)`.
///
/// A failure to launch git at all answers `-1` with the reason as the output, so
/// the loft side can tell "no git here" from "git said no" — the two want
/// different things from a caller.
fn run(query: Query, a: &str, b: &str, n: i64, dir: &str) -> (i64, String) {
    if !ref_ok(a) {
        return (
            -1,
            format!("'{a}' cannot be a git ref — a ref may not begin with '-'"),
        );
    }
    let mut cmd = std::process::Command::new("git");
    if !dir.is_empty() {
        cmd.arg("-C").arg(dir);
    }
    cmd.args(argv(query, a, b, n));
    // No shell is involved anywhere on this path, so nothing in `a` or `b` is
    // ever interpreted — it is one `execve` with an argv this process built.
    match cmd.output() {
        Ok(out) => (
            i64::from(out.status.code().unwrap_or(-2)),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        ),
        Err(e) => (-1, format!("cannot run git: {e}")),
    }
}

/// `git_query(kind, a, b, n, dir, out) -> integer` — the single native behind
/// `lib/git`.
///
/// Arguments pop in reverse; `out` is a `&text` destination, the same shape
/// `OpGetFileText` uses for a text answer that does not travel on the stack.
pub fn n_git_query(stores: &mut Stores, stack: &mut DbRef) {
    let out = *stores.get::<DbRef>(stack);
    let dir = *stores.get::<Str>(stack);
    let n = *stores.get::<i64>(stack);
    let b = *stores.get::<Str>(stack);
    let a = *stores.get::<Str>(stack);
    let kind = *stores.get::<i64>(stack);

    let (code, text) = match Query::from_code(kind) {
        Some(q) => run(q, a.str(), b.str(), n, dir.str()),
        // An unknown code is a loft/binary mismatch, not a git failure: a
        // library built against a newer vocabulary asking an older binary.
        None => (
            -1,
            format!(
                "this loft does not know git query {kind} — the library is newer than the binary"
            ),
        ),
    };
    *stores.store_mut(&out).addr_mut::<String>(out.rec, out.pos) = text;
    stores.put(stack, code);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every value a caller supplies lands where git reads a VALUE. The two that
    /// matter are a path that looks like an option, and a ref that does — both
    /// are how a general argv leaks an option position to its caller.
    #[test]
    fn a_value_never_reaches_an_option_position() {
        let v = argv(Query::DiffFile, "main", "--upload-pack=evil", 0);
        let dashdash = v.iter().position(|x| x == "--").expect("a path needs --");
        assert!(
            v.iter().position(|x| x == "--upload-pack=evil").unwrap() > dashdash,
            "a path must follow `--`, or git reads it as an option: {v:?}"
        );
        // A ref is refused rather than interpolated. Interpolating it into a
        // range is NOT enough — `--exec-path=/tmp` becomes
        // `--exec-path=/tmp...HEAD`, which git still reads as an option, because
        // that is decided by the leading `-`. This assertion is what found that;
        // the version of this module it was written against had the hole.
        assert!(!ref_ok("--exec-path=/tmp"), "a ref may not begin with '-'");
        assert!(!ref_ok("-c"), "a ref may not begin with '-'");
        assert!(
            ref_ok("main") && ref_ok("HEAD~3") && ref_ok("v1.0") && ref_ok("feature/x"),
            "an ordinary ref must still be usable"
        );
        // A PATH may legitimately begin with `-`, and does not need refusing —
        // it always follows `--`.
        let v = argv(Query::DiffFile, "main", "-weird-name", 0);
        assert!(
            v.iter().position(|x| x == "-weird-name").unwrap()
                > v.iter().position(|x| x == "--").unwrap(),
            "a path must stay behind `--`: {v:?}"
        );
    }

    /// The subcommand is never the caller's, so there is no allowlist to get
    /// wrong — and `-c`, which names programs git will RUN, is unreachable.
    #[test]
    fn the_caller_chooses_no_subcommand_and_no_config() {
        let reads = ["rev-parse", "log", "diff", "status", "show", "rev-list"];
        for code in 0..=12 {
            let q = Query::from_code(code).expect("every code below the top is a query");
            let v = argv(q, "main", "some/path", 20);
            assert!(
                reads.contains(&v[0].as_str()),
                "query {q:?} runs `git {}`, which is not one of the read-only \
                 subcommands this module is allowed to run",
                v[0]
            );
            assert!(
                !v.iter().any(|x| x == "-c" || x.starts_with("--exec-path")),
                "query {q:?} can reach git's config/exec options: {v:?}"
            );
        }
        assert!(
            Query::from_code(13).is_none(),
            "the vocabulary must be closed at its top, or an unknown code runs \
             whatever the next entry happens to be"
        );
    }

    /// A count comes from the caller and becomes `-<n>`, so it has to be a
    /// number and nothing else.
    #[test]
    fn a_count_is_bounded() {
        for (given, want) in [(0i64, "-1"), (-5, "-1"), (20, "-20"), (1 << 40, "-100000")] {
            let v = argv(Query::Log, "", "", given);
            assert_eq!(v[1], want, "count {given} became {}", v[1]);
        }
    }
}
