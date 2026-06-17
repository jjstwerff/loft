#!/usr/bin/env python3
"""Fix doc references after the plan-directory migration to flat @PLN-numbered dirs.

The migration flattened `doc/claude/plans/{future,deferred}/<NN>-slug/` and
`doc/claude/lib_plans/future/<NN>-slug/` to flat issue-numbered
`doc/claude/(lib_)plans/<PLN>-slug/`.  The old->new mapping is DERIVED from the
STAGED git renames (the authoritative record), so nothing is hard-coded.

For every tracked `.md` file it:
  1. Re-bases every markdown link `](target)` by resolving the target against the
     file's OLD location, remapping it if it lands inside a moved dir, and
     recomputing the relative path from the file's NEW location.  This fixes both
     the `../` depth change in moved files and inbound links from elsewhere
     (relative or full-path, with or without the `plans/` prefix).
  2. Rewrites bare path mentions (prose / backticks, not markdown links) of the
     full forms `plans/future/<slug>`, `plans/deferred/<slug>`,
     `lib_plans/future/<slug>` to the new flat path.
  3. Rewrites `@PLAN<NN>` tracker tags to `@PLN<n>` CONTEXTUALLY — the lib_plans
     map inside `doc/claude/lib_plans/`, the plans map everywhere else — because
     the legacy `@PLAN` number space collides between the two trees.

Only `.md` files under `doc/claude/` (+ root `CLAUDE.md`) are tag-rewritten; tool
sources keep their literal strings.
"""

import os
import re
import subprocess
from pathlib import Path

ROOT = Path(subprocess.check_output(["git", "rev-parse", "--show-toplevel"], text=True).strip())
OLD_PD = re.compile(r"^(doc/claude/(?:lib_)?plans/(?:future/|deferred/)?[^/]+)")
NEW_PD = re.compile(r"^(doc/claude/(?:lib_)?plans/[^/]+)")
LINK = re.compile(r"\]\(([^)]+)\)")


def num_of(plandir_basename):
    m = re.match(r"(\d+)", os.path.basename(plandir_basename))
    return str(int(m.group(1))) if m else None


def build():
    out = subprocess.check_output(
        ["git", "diff", "--cached", "--name-status", "-M",
         "--", "doc/claude/plans", "doc/claude/lib_plans"], text=True)
    dirmap = {}
    for line in out.splitlines():
        p = line.split("\t")
        if not p[0].startswith("R") or len(p) < 3:
            continue
        mo, mn = OLD_PD.match(p[1]), NEW_PD.match(p[2])
        if mo and mn and mo.group(1) != mn.group(1):
            dirmap[mo.group(1)] = mn.group(1)
    # contextual @PLAN<old>->@PLN<new>, split by tree
    plans_tag, lib_tag = {}, {}
    for o, n in dirmap.items():
        onum, nnum = num_of(o), num_of(n)
        if onum and nnum:
            (lib_tag if "/lib_plans/" in o else plans_tag)[onum] = nnum
    return dirmap, plans_tag, lib_tag


def main():
    dirmap, plans_tag, lib_tag = build()
    inv = {n: o for o, n in dirmap.items()}
    subst = {o[len("doc/claude/"):]: n[len("doc/claude/"):] for o, n in dirmap.items()}
    ordered = sorted(subst.items(), key=lambda kv: -len(kv[0]))

    def remap(absrepo):
        for o, n in dirmap.items():
            if absrepo == o or absrepo.startswith(o + "/"):
                return n + absrepo[len(o):]
        return absrepo

    files = subprocess.check_output(["git", "ls-files", "*.md"], text=True).split()
    changed = 0
    for rel in files:
        fp = ROOT / rel
        if not fp.exists():
            continue
        text = orig = fp.read_text()

        np = NEW_PD.match(rel)
        new_pd = np.group(1) if np else None
        old_file = (inv[new_pd] + rel[len(new_pd):]) if new_pd in inv else rel
        old_dir, new_dir = os.path.dirname(old_file), os.path.dirname(rel)

        def fix(m):
            tgt = m.group(1)
            if not tgt or tgt[0] in "#/" or tgt.startswith(("http", "mailto:")) or "://" in tgt:
                return m.group(0)
            anchor = ""
            if "#" in tgt:
                tgt, a = tgt.split("#", 1)
                anchor = "#" + a
            if not tgt:
                return m.group(0)
            new_abs = remap(os.path.normpath(os.path.join(old_dir, tgt)))
            return f"]({os.path.relpath(new_abs, new_dir)}{anchor})"

        text = LINK.sub(fix, text)
        for o, n in ordered:                       # bare prose/backtick path mentions
            if o in text:
                text = text.replace(o, n)
        # contextual tag rewrite, only in the doc tree
        if rel == "CLAUDE.md" or rel.startswith("doc/claude/"):
            tagmap = lib_tag if "doc/claude/lib_plans/" in rel else plans_tag
            def tagfix(m, tm=tagmap):
                old = m.group(1)
                key = str(int(old))
                return "@PLN" + tm[key] if key in tm else m.group(0)
            # pattern split so this source line carries no literal @PLAN<digit>
            # (else the tracker indexer scans it as a stray tag ref)
            text = re.sub(r"@PLAN" + r"0*(\d+)(?![0-9])", tagfix, text)

        if text != orig:
            fp.write_text(text)
            changed += 1

    print(f"{len(dirmap)} dir moves | plans_tag={len(plans_tag)} lib_tag={len(lib_tag)} | {changed} files rewritten")


if __name__ == "__main__":
    main()
