struct U { name: text, age: integer }
fn drive() -> text { u = U { name: "Al", age: 7 }; acc = "J="; acc += u.to_json(); return acc; }
fn main() { i = 0; r = ""; while i < %N% { r = drive(); i += 1; } print(r); }
