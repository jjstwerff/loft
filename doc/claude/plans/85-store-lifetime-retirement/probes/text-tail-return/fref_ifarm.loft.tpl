struct U { name: text, age: integer }
fn caller(c: boolean) -> integer { pick(c).len() }
fn pick(c: boolean) -> text { u = U { name: "Al", age: 7 }; if c { u.to_json() } else { "fallback" } }
fn main() { i = 0; n = 0; while i < %N% { n = caller(true); i += 1; } print("{n}"); }
