// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// Which triage labels an issue BODY asks for — the parsing half of
// `.github/workflows/label-guard.yml`.
//
// A file rather than inline YAML because this is the part that can be WRONG in a
// way nothing reports: every failure mode here is "no label applied", which reads
// exactly like "the filer did not answer".  `tools/label_guard_selftest.mjs`
// exercises it against real body shapes; the workflow requires it.
//
// Nothing here touches the network or the API — give it a string, get labels.

"use strict";

/// The only labels that may ever be auto-applied.  A body is written by anyone
/// who can open an issue, so the allowlist is what keeps that from becoming
/// "any label they can spell", including ones that do not exist yet.
const VALID = new Set([
  "sev:high", "sev:medium", "sev:low",
  "wa:clean", "wa:partial", "wa:none",
  "area:parser", "area:codegen", "area:store-lifetime", "area:closures",
  "area:runtime", "area:native", "area:wasm", "area:stdlib", "area:packages",
]);

/// Required label categories for a loft BUG (`.github/LABELS.md`), with the
/// values a filer may choose — the comment quotes these, so the message and the
/// parser cannot drift apart.
const CATEGORIES = [
  {
    key: "sev:",
    name: "severity — one `sev:high` / `sev:medium` / `sev:low`",
    values: ["sev:high", "sev:medium", "sev:low"],
  },
  {
    key: "wa:",
    name: "workaround — one `wa:clean` / `wa:partial` / `wa:none`",
    values: ["wa:clean", "wa:partial", "wa:none"],
  },
  {
    key: "area:",
    name: "area — one or more `area:*` (parser / codegen / store-lifetime / …)",
    values: [...VALID].filter((l) => l.startsWith("area:")),
  },
];

/// The heading a filer who CANNOT set labels writes to ask for them.
const TRIAGE_HEADING = "triage";

/// Split a body into its `### <heading>` sections, lowercased heading → text.
function sectionsOf(body) {
  const out = new Map();
  let head = null;
  for (const line of body.split(/\r?\n/)) {
    const m = /^###\s+(.+?)\s*$/.exec(line);
    if (m) {
      head = m[1].toLowerCase();
      out.set(head, []);
    } else if (head) {
      out.get(head).push(line);
    }
  }
  return out;
}

/// Distinct `<key>:<value>` tokens in `text`, restricted to [`VALID`].
function tokens(text, key) {
  const hits = new Set(
    [...text.matchAll(new RegExp("\\b" + key + ":[a-z0-9-]+", "g"))].map((m) => m[0]),
  );
  return [...hits].filter((t) => VALID.has(t));
}

/// The labels `body` asks for.
///
/// Three channels, in the order a body can supply them:
///
///  1. the bug FORM's own answer sections (`### Severity`, …) — what a web filer
///     produces, and what GitHub does not turn into labels for you;
///  2. a `### Triage` section — what a filer who cannot SET labels writes to ask
///     for them, since GitHub restricts labelling to triage permission and an
///     outside reporter has none (loft#805 arrived with no labels at all);
///  3. for `sev:`/`wa:` only, the body at large.
///
/// `sev:` and `wa:` are SINGLE-CHOICE: a value is taken only when its source
/// names exactly one.  Ambiguity must not be guessed — leaving it unset flags the
/// issue, which is the recoverable outcome.
///
/// `area:` is multi-valued and therefore never read from prose: the form's
/// CHECKED boxes, or the `### Triage` block, and nowhere else.  Scanning prose
/// for it is what labelled #626 with six mutually exclusive tokens, because its
/// body quoted a comment that listed them all.
function chooseLabels(body) {
  const text = body || "";
  const sections = sectionsOf(text);
  const section = (want) => {
    for (const [h, lines] of sections) {
      if (h.includes(want)) {
        const t = lines.join("\n").trim();
        // A present-but-blank section must not short-circuit the fallback chain:
        // an unanswered form field is no answer, not an empty one.
        return t === "" ? null : t;
      }
    }
    return null;
  };
  // A pasted log or a quoted example must not contribute labels.
  const unfenced = text.replace(/```[\s\S]*?```/g, "");
  const triage = section(TRIAGE_HEADING);

  // "clean workaround", not "workaround".  TWO form headings contain the latter
  // — the free-text `### Workaround` textarea comes FIRST, so a substring match
  // returned the prose and the dropdown that actually sets `wa:*` was never
  // read.  Silent by construction: the filer answers a required dropdown and the
  // label simply does not appear.
  const sevText = section("severity") ?? triage ?? unfenced;
  const waText = section("clean workaround") ?? triage ?? section("workaround") ?? unfenced;

  const found = new Set();
  for (const [src, key] of [[sevText, "sev"], [waText, "wa"]]) {
    const hits = tokens(src, key);
    if (hits.length === 1) found.add(hits[0]);
  }
  const areaSection = section("area");
  if (areaSection) {
    for (const m of areaSection.matchAll(/-\s*\[[xX]\]\s*`?(area:[a-z0-9-]+)/g)) {
      if (VALID.has(m[1])) found.add(m[1]);
    }
  }
  if (triage) {
    for (const t of tokens(triage, "area")) found.add(t);
  }
  return [...found];
}

module.exports = { VALID, CATEGORIES, TRIAGE_HEADING, chooseLabels };
