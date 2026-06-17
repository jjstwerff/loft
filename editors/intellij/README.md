# Loft for JetBrains IDEs

Syntax highlighting for `.loft` files in any JetBrains IDE that
supports TextMate Bundles: **CLion**, **IntelliJ IDEA**, **PyCharm**,
**RustRover**, **WebStorm**, **Rider**, **GoLand**, **PhpStorm**,
**RubyMine**, **DataGrip**, **AppCode**.

## Quick start

1. Open **`Settings`** (Linux/Windows) or **`Preferences`** (macOS).
   Or `Ctrl/Cmd+,`.
2. Navigate **`Editor → TextMate Bundles`**.
3. Click the **`+`** button above the bundle list.
4. Point the file picker at this directory:
   `editors/intellij/Loft.tmbundle/`
   (the `.tmbundle` folder, **not** the repo's top-level
   `syntaxes/` directory — that's the canonical grammar storage,
   but JetBrains needs the bundle wrapper.)
5. Click **OK** / **Apply**.
6. **Restart the IDE** (`File → Restart` or quit + relaunch).
   JetBrains caches file-type bindings at startup; the bundle
   loads on Apply but the `.loft` extension association from
   `info.plist` only takes effect after a restart.  Without
   this step, `.loft` files keep opening as plain text even
   though the bundle is listed in the TextMate Bundles panel.
7. Open any `.loft` file — colouring should apply.  If it
   still doesn't, right-click the file in the project tree →
   `Override File Type → Loft`.

## What's in here

```
editors/intellij/Loft.tmbundle/
├── info.plist                    ← bundle metadata (name, UUID, file types)
└── Syntaxes/
    └── loft.tmLanguage.json      → symlink to ../../../../syntaxes/loft.tmLanguage.json
```

The grammar itself (`loft.tmLanguage.json`) lives canonically at
the repo's top-level `syntaxes/` directory and is symlinked into
the bundle's `Syntaxes/` subdirectory.  Single source of truth;
JetBrains follows the symlink.

## Why a bundle wrapper?

JetBrains' TextMate Bundles loader expects the original
TextMate `.tmbundle` directory layout:

- A directory with `.tmbundle` (or any name + `info.plist`).
- An `info.plist` (Apple plist XML) at the top level naming the
  bundle.
- A `Syntaxes/` subdirectory (capital S) containing one or more
  grammar files — JetBrains reads JSON tmLanguage as well as
  Apple plist tmLanguage.

Pointing JetBrains at the bare `syntaxes/` directory at the repo
root fails with `Cannot read the following bundle: ...` because
that directory has no `info.plist`.  This wrapper exists purely
to satisfy the bundle layout convention without duplicating the
grammar file.

## Limitations

This is the syntax-highlighting + file-extension-recognition
layer only.  No completion, no go-to-definition, no diagnostics,
no run buttons — those need a Language Server (planned for 0.8.6
per [`lib_plans/63-lsp/`](../../doc/claude/lib_plans/63-lsp/README.md))
or a JetBrains-specific plugin.

## Licence

LGPL-3.0-or-later, matching the loft project.
