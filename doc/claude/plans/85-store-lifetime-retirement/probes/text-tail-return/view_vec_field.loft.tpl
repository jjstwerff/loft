struct D { ts: vector<text> }
fn get() -> text { d = D { ts: ["aa", "bb"] }; d.ts[0] }
fn main() { i = 0; r = ""; while i < %N% { r = get(); i += 1; } print(r); }
