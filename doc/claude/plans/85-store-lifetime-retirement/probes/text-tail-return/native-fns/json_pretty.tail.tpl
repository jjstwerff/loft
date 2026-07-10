struct U { name: text, age: integer }
fn drive() -> text { u = U { name: "Al", age: 7 }; u.to_json_pretty() }
fn main() { i=0; r=""; while i < %N% { r = drive(); i += 1; } print(r); }
