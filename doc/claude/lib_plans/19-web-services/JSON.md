<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# JSON — currently shipped

JSON serialization and deserialization are already working in
loft today via the `Type.parse()` mechanism + `:j` format flag.
This file is the **reference for what's shipped**, plus a sketch
of future JSON extensions that would round out the library.

**No `#json` annotation needed.** The existing `Type.parse()`
mechanism handles JSON deserialization for any struct, and `":j"`
handles serialization.  This eliminates the need for compiler-
synthesized `from_json` / `to_json` methods entirely.

## Currently-shipped capabilities

| Capability | Syntax | Status |
|---|---|---|
| JSON output | `"{value:j}"` format flag | Working |
| JSON input (struct) | `MyStruct.parse(json_text)` | Working |
| Parse error tracking | `record#errors` accessor | Working |
| Quoted field names | Both `name: value` and `"name": value` | Working |
| Field constraints | `assert(expr)` in struct definitions | Working |
| Callable fn-refs | `fn worker` passed as value | Working |
| Lambdas | `fn(x: T) -> U { body }` | Working |
| String interpolation | `"{expr}"` | Working |

## Serialization (struct → JSON text)

```loft
struct User { id: integer; name: text; email: text }

u = User { id: 42, name: "Alice", email: "a@example.com" };
json = "{u:j}";
// → {"id":42,"name":"Alice","email":"a@example.com"}
```

The `:j` format flag works on any struct, enum, or vector.  No
annotation required.

## Deserialization (JSON text → struct)

```loft
input = `{"id":42,"name":"Alice","email":"a@example.com"}`;
u = User.parse(input);
assert(u.name == "Alice");
```

`Type.parse()` accepts both JSON-style (`"name": value`) and
loft-style (`name: value`) field names.  Missing fields get null
sentinels.  Extra fields are skipped.

## Error handling

```loft
bad = User.parse(`{"id":"not_a_number"}`);
for e in bad#errors {
    log_info("parse error: {e}");
}
```

The `#errors` accessor returns an iterable of error messages from
the most recent parse.

## Vectors

```loft
struct Score { value: integer }
items = `[{"value":10},{"value":20},{"value":30}]`;
scores = vector<Score>.parse(items);
assert(len(scores) == 3);
```

## Field constraints (validation)

```loft
struct User {
    id: integer
        assert(id > 0)
    name: text
        assert(len(name) > 0, "name must not be empty")
    email: text
}

u = User.parse(`{"id":-1,"name":"","email":"x"}`);
// u#errors contains "id > 0", "name must not be empty"
```

---

## Future JSON extensions — sketch

The shipped capabilities cover the common case (round-trip a
struct or vector to/from JSON).  A "fully functioning" web-
services library may want additional JSON tooling beyond what's
here.  None of these are scheduled; they're noted for future
sessions to consider when the surrounding capability is needed.

| Capability | Description | Status |
|---|---|---|
| Schema validation | Validate against JSON Schema (Draft 7+) | Sketch |
| JSON Pointer | `doc.at("/users/0/name")` for path-based access without parsing whole tree | Sketch |
| Streaming parser | Parse arbitrarily-large arrays without loading whole doc into memory | Sketch |
| Pretty-print control | Indent / compact / sort-keys variants of `:j` | Sketch |
| Custom serializers | Per-type override for serialization (e.g. dates as ISO 8601) | Sketch |
| Diff + patch (RFC 6902) | JSON Patch operations for partial updates | Sketch |
| Merge patch (RFC 7396) | JSON Merge Patch for simpler partial updates | Sketch |

**Trigger to schedule any of these:** a concrete consumer
appears that needs the capability.  Today's HTTP-client +
struct-round-trip use case is fully served by what's shipped.

---

## See also

- [README.md](README.md) — overview of the web-services library plan
- [HTTP_CLIENT.md](HTTP_CLIENT.md) — HTTP client design (the
  primary downstream consumer of JSON)
- [../../STDLIB.md](../../STDLIB.md) — stdlib reference for `Type.parse` /
  `:j` format flag
- [../../LOFT.md](../../LOFT.md) — language reference
