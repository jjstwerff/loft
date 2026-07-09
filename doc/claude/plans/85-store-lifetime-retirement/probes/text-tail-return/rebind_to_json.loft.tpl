struct U { name: text, age: integer }
fn run() -> text { u = U { name: "Alice", age: 30 }; r = u.to_json(); return r; }
fn main() { i = 0; while i < %N% { t = run(); i += 1; } print("done"); }
