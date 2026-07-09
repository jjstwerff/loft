struct U { name: text, age: integer }
fn drive() -> text { u = U { name: "Al", age: 7 }; t = u.to_json(); t }
fn main() { i=0; r=""; while i < %N% { r = drive(); i += 1; } print(r); }
