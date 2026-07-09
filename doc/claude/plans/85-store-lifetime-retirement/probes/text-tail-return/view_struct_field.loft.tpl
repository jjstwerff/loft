struct S { name: text, age: integer }
fn get() -> text { s = S { name: "Alice", age: 30 }; s.name }
fn main() { i = 0; r = ""; while i < %N% { r = get(); i += 1; } print(r); }
