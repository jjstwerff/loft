struct A { v: (text, text) }
fn first() -> text { a = A { v: ("hello", "world") }; a.v.0 }
fn main() { i = 0; r = ""; while i < %N% { r = first(); i += 1; } print(r); }
