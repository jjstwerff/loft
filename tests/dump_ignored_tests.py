#!/usr/bin/env python3
# Regenerate tests/ignored_tests.baseline from tests/issues.rs.
# Usage: python3 tests/dump_ignored_tests.py > tests/ignored_tests.baseline
import re
import sys

HEADER = """\
# Baseline for tests/issues.rs #[ignore = "..."] entries.
# One `<test_name>\\t<reason>` pair per line, sorted by test_name.
# Updated by tests/doc_hygiene.rs::ignored_tests_baseline_is_current
# whenever the set drifts.  A drift typically means one of:
#   - an ignored test just got its fix landed (un-ignore it + delete its line here)
#   - a new ignored spec landed for a new QUALITY.md item (add its line here)
#   - the reason message changed (update the line here)
# Regenerate with: `python3 tests/dump_ignored_tests.py > tests/ignored_tests.baseline`
"""

def main() -> int:
    with open("tests/issues.rs", encoding="utf-8") as f:
        lines = f.read().splitlines()
    out = []
    for i, line in enumerate(lines):
        m = re.match(r'\s*#\[ignore\s*=\s*"(.+)"\]', line)
        if not m:
            continue
        # Mirror the Rust test's unescape so both sides compare
        # equal: `\"` → `"`, `\\` → `\`.
        reason = m.group(1).replace('\\"', '"').replace("\\\\", "\\")
        for j in range(i + 1, min(i + 10, len(lines))):
            fm = re.match(r"fn (\w+)\(", lines[j])
            if fm:
                out.append((fm.group(1), reason))
                break
    out.sort()
    sys.stdout.write(HEADER)
    for name, reason in out:
        sys.stdout.write(f"{name}\t{reason}\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
