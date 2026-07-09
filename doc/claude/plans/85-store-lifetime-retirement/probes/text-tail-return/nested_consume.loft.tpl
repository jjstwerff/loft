struct U { name: text, age: integer }
fn wrap(t: text) -> text { "[" + t + "]" }
fn drive() -> text { u = U { name: "Al", age: 7 }; wrap(u.to_json()) }
fn main() { i = 0; r = ""; while i < %N% { r = drive(); i += 1; } print(r); }
