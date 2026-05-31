<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# arguments — CLI argument parsing for loft

Build a structured `Args` from `argv`, with declarative
positional + flag definitions.  Pure-loft, no native code.

## Install

```sh
loft install arguments
```

## Usage

```loft
use arguments;

fn main() {
    args = arguments::create("myprog", "1.0", "A tiny example");
    args.add_arg("name", "positional name argument");
    args.add_flag("verbose", "v", "enable verbose output");
    args.parse();

    name = args.value("name");
    if args.flag("verbose") {
        print("hello {name} (verbose)\n");
    } else {
        print("hello {name}\n");
    }
}
```

Full API documented in [src/arguments.loft](src/arguments.loft).

## Provenance

Extracted from the loft monorepo's `lib/arguments/` 2026-05-24
as part of plan-12 Phase 3.5a (libraries without monorepo
consumers).
