struct A { v: (text, text) }
fn measure() -> integer { first().len() }
fn first() -> text { a = A { v: ("hello", "world") }; a.v.0 }
fn main() { i = 0; r = 0; while i < %N% { r = measure(); i += 1; } print("{r}"); }
