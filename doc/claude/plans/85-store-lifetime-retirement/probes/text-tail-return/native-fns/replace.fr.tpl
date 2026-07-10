
fn drive() -> text { s = "a-b-c padded long enough"; t = s.replace("-", "_"); t }
fn main() { i=0; r=""; while i < %N% { r = drive(); i += 1; } print(r); }
