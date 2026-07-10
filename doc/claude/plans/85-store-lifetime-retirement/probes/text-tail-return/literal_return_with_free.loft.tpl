fn f() -> text { s = "local scratch text"; n = s.len(); "constant-{n}-suffix"; "plainlit" }
fn main() { i = 0; r = ""; while i < %N% { r = f(); i += 1; } print(r); }
