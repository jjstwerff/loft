struct U { name: text }
fn drive() -> text { u = U { name: "Alice padded" }; u.to_json() }
fn main() { i = 0; while i < %N% { r = drive(); i += 1; } print("done"); }
