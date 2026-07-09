fn label<T: Printable>(x: T) -> text { x.to_text() + "!" }
fn drive() -> text { label(42) }
fn main() { i = 0; r = ""; while i < %N% { r = drive(); i += 1; } print(r); }
