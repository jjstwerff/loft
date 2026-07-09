pub enum V { A { v: integer }, B { v: text } }
fn extract(x: V) -> text { out = ""; match x { A { v } => {}, B { v } => { out = v; } } out }
fn main() { i = 0; r = ""; while i < %N% { r = extract(B { v: "hello" }); i += 1; } print(r); }
