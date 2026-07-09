struct U { name: text, age: integer }
fn inner() -> text { u = U { name: "Alice", age: 30 }; u.to_json() }
fn run() -> text { inner() }
fn main() { i = 0; while i < %N% { t = run(); i += 1; } print("done"); }
