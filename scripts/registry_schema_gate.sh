#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Refuse an index that the registry's OWN PR validation would reject.
#
#   scripts/registry_schema_gate.sh <registry-dir>
#
# Runs gate 1 (schema lint) of `<registry-dir>/tools/validate.py` against
# `<registry-dir>/index.json`.  Exit 0 = the index is acceptable; non-zero = it
# is not, with validate.py's own `::error::` line on stdout.
#
# WHY IT READS THE CHECKOUT'S OWN COPY.  `tools/validate.py` lives in
# `loft-lang/registry` and is what `pr-validate.yml` runs; re-stating its rules
# here would be a second list of one type's facts, and the two would drift.
# Importing the file cannot drift.  (`doc/claude/registry_ci_template/` holds a
# SIBLING copy that has already drifted in both directions — do not use it as
# the authority; loft#1052.)
#
# WHY IT GATES SIGNING RATHER THAN PUBLISHING.  A signed index that fails gate 1
# turns EVERY later submission PR red, on a check that has nothing to do with
# that submission, and the person who sees it cannot clear it — that needs the
# signing key.  Measured: `zttext` and `fixstep` went in with `"categories": []`
# and blocked an unrelated PR for three days.
set -euo pipefail

REG_DIR="${1:?usage: registry_schema_gate.sh <registry-dir>}"
INDEX="$REG_DIR/index.json"
VALIDATOR="$REG_DIR/tools/validate.py"

[ -f "$INDEX" ] || { echo "no index.json in $REG_DIR" >&2; exit 2; }
# An absent validator is a REFUSAL, never a skip: a gate that skips looks
# exactly like a gate that passes, and this one guards the trust root.
[ -f "$VALIDATOR" ] || {
    echo "no tools/validate.py in $REG_DIR — cannot check the index schema; refusing." >&2
    exit 2
}

python3 - "$VALIDATOR" "$INDEX" <<'PY'
import importlib.util, json, sys

mod_path, index_path = sys.argv[1:3]
spec = importlib.util.spec_from_file_location("registry_validate", mod_path)
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)
if not hasattr(mod, "gate_schema"):
    print(f"::error::{mod_path} defines no gate_schema() — cannot check the index schema")
    sys.exit(2)
with open(index_path, encoding="utf-8") as f:
    mod.gate_schema(json.load(f))
PY
