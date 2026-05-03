# Loft TextMate grammar

`loft.tmLanguage.json` provides syntax highlighting for `.loft` files
in any editor that supports JSON-format
[TextMate grammars](https://macromates.com/manual/en/language_grammars):

- **VS Code** — via the extension at `editors/vscode/` (SH.2 ✅)
- **IntelliJ-family** (CLion / IDEA / PyCharm / WebStorm) — `Settings → Editor → TextMate Bundles`, point at this `syntaxes/` directory
- **GitHub Linguist** — once published as a Linguist contribution (0.8.6 follow-up)
- **Atom**, **BBEdit**, others with TextMate import — best-effort, not formally tested

**Sublime Text** is **not** directly supported because Sublime
Text 4 reads `.sublime-syntax` (YAML) and `.tmLanguage` (Apple
plist XML), not the JSON tmLanguage format we ship.  The
historical conversion path (Package Control → PackageDev) has
broken since Sublime moved to Python 3.8.  A YAML
`.sublime-syntax` companion file or a runtime conversion script
is a 0.8.6 follow-up; for now Sublime users with strong
preference can convert this file manually using one of the
stand-alone tmLanguage→sublime-syntax tools on PyPI.

## Scope coverage

The grammar implements the scope mapping documented in
[`doc/claude/DX.md` § SH.1](../doc/claude/DX.md):

| Loft construct | TextMate scope |
|---|---|
| `fn`, `struct`, `enum`, `type`, `pub`, `use`, `interface`, `const`, `let` | `keyword.declaration.loft` |
| `if`, `else`, `for`, `while`, `match`, `in`, `return`, `break`, `continue`, `yield` | `keyword.control.loft` |
| `and`, `or`, `not`, `as` | `keyword.operator.word.loft` |
| `true`, `false` | `constant.language.boolean.loft` |
| `null` | `constant.language.null.loft` |
| `assert`, `debug_assert`, `panic`, `sizeof`, `self`, `par`, `print`, `log_*`, `len` | `keyword.other.loft` |
| `integer`, `boolean`, `float`, `single`, `long`, `character`, `text`, `byte`, `void`, `reference`, `enumerate`, `i8/i16/i32`, `u8/u16/u32` | `support.type.primitive.loft` |
| `vector`, `sorted`, `hash`, `index`, `spacial`, `iterator` | `support.type.collection.loft` |
| `not null` modifier | `storage.modifier.loft` |
| `CamelCase` identifiers | `entity.name.type.loft` |
| `UPPER_SNAKE` identifiers | `constant.other.loft` |
| `lower_case` after `fn ` | `entity.name.function.loft` |
| `0x...`, `0b...`, `0o...`, decimals, floats | `constant.numeric.loft` |
| `"..."` strings | `string.quoted.double.loft` |
| `{expr}` inside strings | `meta.interpolation.loft` |
| `{{` / `}}` inside strings | `constant.character.escape.loft` |
| `\n`, `\t`, `\\`, `\"` | `constant.character.escape.loft` |
| `//` line comment | `comment.line.double-slash.loft` |
| `///` doc comment | `comment.line.documentation.loft` |
| `#rust`, `#native`, `#opcode`, `#impure`, `#pure` | `meta.annotation.loft` |
| `@EXPECT_ERROR`, `@EXPECT_WARNING`, `@EXPECT_FAIL`, `@NAME`, `@TITLE`, `@MARK` | `meta.annotation.test.loft` |

## Manual sanity check

```sh
# Sublime Text: Tools > Developer > New Syntax to a tmLanguage.tmLanguage file
# VS Code: install via the editors/vscode/ extension scaffold (SH.2)
```

Open `tests/docs/00-general.loft`; keywords (`fn`, `for`, `if`),
type names (`integer`, `Position`), strings with `{interpolation}`,
and comments should all colour distinctly.

## Why JSON rather than YAML / plist

VS Code accepts `.json`, `.tmLanguage.yaml`, and `.plist`.  JSON is
the leanest editable format and the one VS Code marketplaces convert
to internally — picking it as the source of truth avoids round-trip
loss.
