struct U { name: text, age: integer }
fn mk() -> U { U { name: "Alice", age: 30 } }
fn main() { i = 0; while i < %N% { t = mk().to_json(); i += 1; } print("done"); }
