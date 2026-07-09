struct U { name: text, age: integer }
fn run() -> text? { u = U { name: "Alice", age: 30 }; u.to_json() }
fn main() { i = 0; while i < %N% { t = run(); i += 1; } print("done"); }
