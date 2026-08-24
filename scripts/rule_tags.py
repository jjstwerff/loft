#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# rule_tags.py — the formal rules as `@`-tags, and the code sites that cite them.
#
# A rule is only an anchor for code if it can be found EXACTLY.  See
# doc/claude/formal/README.md § Rule tags for the convention and CLAUDE.md § Tracker tags
# for the sigil it reuses.  Two constraints, both measured rather than assumed:
#
#   * BOUNDARY-EXACT.  23 of the defined rules are a prefix of another (`@B-View` vs
#     `@B-View-Base`), because a general rule and its refinements share a stem.  `\b` does
#     not help — `-` is already a word boundary — so a citation matches `@Name` only when
#     the next character cannot continue a tag.
#   * ONLY A DEFINED RULE.  `B-Ref`, `D-op`, `D-own`, `D-cap`, `D-op-null` read like rules
#     and are family prefixes that appear only in prose.  Citing one is an error.
#   * A NAMESPACED PREFIX, `@FR-<Rule>` (Formal Rule).  A bare `@Name` is NOT unambiguous
#     here: `@` already carries the tracker tags (`@P259`, `@PLN3`, `@PLAN22`, `@F7`, `@I81`,
#     `@GH247`), the worked-example family (`@AAA-###`), and the corpus annotations (`@ARGS`,
#     `@NAME`, `@IGNORE`, `@EXPECT_ERROR`).  Measured: a bare-`@` reading of src/ returned
#     4142 "citations", not one of them a rule.  `@FR-` sits in the same family shape as the
#     others and cannot be confused with `@F<digits>`, whose next character is a digit.
#
# Subcommands:
#   list           every defined rule and the doc that defines it
#   check          every citation resolves; no rule defined twice   (exit 1 on failure)
#   sites <tag>    the code sites citing one rule (tag with or without the @FR- prefix)
#   dups           rules cited from 2+ sites — the duplication question, asked by MEANING
#                  rather than by code shape (which is what rule_predicate_audit.py does)

import collections
import glob
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FORMAL = os.path.join(ROOT, "doc/claude/formal")
SRC = os.path.join(ROOT, "src")

# A rule is DEFINED by a rules-block line `  (Name)  prose` or a deviation header `### Name —`.
# A RULE is defined by a line `  (Name)  prose` INSIDE A FENCED BLOCK — that is the shape the
# rules blocks use.  Two narrowings, each forced by a false positive rather than foreseen:
# markdown SECTION headers are not a definition form (reading them as one turned `## Rules`,
# `## Notation` and `## Deviations` into "rules defined in 17 docs"), and a parenthesised
# MENTION in prose is not one either (`heap.md` refers to "(D-cap-3)", which read as a second
# definition of a rule `capabilities.md` owns).
DEF_INLINE = re.compile(r"^\s*\(([A-Z][A-Za-z0-9-]{1,40})\)\s", re.M)
# A DEVIATION is a register entry.  Two spellings are in use and BOTH are definitions —
# a `### D-own-7 — …` header (ownership.md) and a `> **D-bind-11 — …` blockquote
# (binding.md).  Reading only the header form left `@FR-D-bind-11` unresolvable, which the
# check reported the moment it was cited; the doc format varying by file is exactly the kind
# of thing a registry has to absorb rather than legislate away.
# The blockquote form must keep the em-dash INSIDE the bold (`> **D-bind-12 — CLOSED …`);
# `> **D-bind-10**:` is a cross-REFERENCE from another doc, not a second definition.
DEF_DEV = re.compile(
    r"(?:^#{2,5}\s+`?(?P<h>D[A-Za-z]*-[a-z]+-\d+|DN\d+[A-Za-z-]*)\b"
    r"|^>\s*\*\*(?P<q>D[A-Za-z]*-[a-z]+-\d+|DN\d+[A-Za-z-]*)\s+—)",
    re.M,
)
# A CITATION is `@FR-<Rule>`, boundary-exact so `@FR-B-View` does not match `@FR-B-View-Base`.
CITE = re.compile(r"@FR-([A-Z][A-Za-z0-9-]{1,40})(?![-A-Za-z0-9])")


def defined_rules():
    """{tag: [defining files]} — a rule defined twice is a bug the check reports."""
    out = collections.defaultdict(list)
    for path in sorted(glob.glob(FORMAL + "/*.md")):
        text = open(path, encoding="utf-8").read()
        fenced = "\n".join(_fenced_lines(text))
        devs = {m.group("h") or m.group("q") for m in DEF_DEV.finditer(text)}
        for name in set(DEF_INLINE.findall(fenced)) | devs:
            out[name].append(os.path.basename(path))
    return out


def _fenced_lines(text):
    """Only the lines inside ``` fences — where the rules blocks live."""
    out, inside = [], False
    for line in text.split("\n"):
        if line.lstrip().startswith("```"):
            inside = not inside
            continue
        if inside:
            out.append(line)
    return out


def citations():
    """{tag: [(file, line)]} for every `@Tag` in src/."""
    out = collections.defaultdict(list)
    for path in glob.glob(SRC + "/**/*.rs", recursive=True):
        for n, line in enumerate(open(path, encoding="utf-8", errors="replace"), 1):
            for tag in CITE.findall(line):
                out[tag].append((os.path.relpath(path, ROOT), n))
    return out


def main():
    cmd = sys.argv[1] if len(sys.argv) > 1 else "check"
    rules = defined_rules()

    if cmd == "list":
        for tag in sorted(rules):
            print(f"@FR-{tag:<28} {', '.join(sorted(set(rules[tag])))}")
        print(f"\n{len(rules)} defined rules")
        return 0

    cites = citations()

    if cmd == "sites":
        tag = sys.argv[2].removeprefix("@FR-").lstrip("@")
        if tag not in rules:
            print(f"@FR-{tag} is not a defined rule")
            return 1
        for f, n in cites.get(tag, []):
            print(f"{f}:{n}")
        print(f"\n@FR-{tag}: {len(cites.get(tag, []))} citation(s)")
        return 0

    if cmd == "dups":
        multi = {t: v for t, v in cites.items() if len({f for f, _ in v}) >= 2}
        print(f"{len(multi)} rule(s) cited from 2+ files\n")
        for tag, where in sorted(multi.items(), key=lambda kv: -len(kv[1])):
            print(f"[{len(where):2d} sites] @FR-{tag}")
            for f, n in where:
                print(f"           {f}:{n}")
        return 0

    # check
    problems = []
    for tag, files in rules.items():
        if len(files) > 1:
            problems.append(f"@FR-{tag} defined in {len(files)} docs: {', '.join(files)}")
    for tag, where in sorted(cites.items()):
        if tag not in rules:
            for f, n in where:
                problems.append(f"{f}:{n}: cites @FR-{tag}, which is not a defined rule")
    cited = sum(1 for t in cites if t in rules)
    print(f"{len(rules)} defined rules · {cited} cited · {sum(len(v) for v in cites.values())} citation sites")
    if problems:
        print(f"\n{len(problems)} problem(s):")
        for p in problems:
            print(f"  {p}")
        return 1
    print("ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
