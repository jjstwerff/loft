struct U { name: text, age: integer }
fn drive() -> text { u = U { name: "Alice", age: 30 }; u.to_json() }
fn eat(t: text) { z = len(t); }
fn main() { i = 0; while i < %N% { s = drive(); z = len(s); i += 1; } print("done"); }
