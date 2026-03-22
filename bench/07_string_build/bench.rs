use std::time::Instant;

fn main() {
    let n = 5000;
    let t0 = Instant::now();
    let mut s = String::new();
    for i in 0..n {
        s.push_str(&format!("item-{};", i));
    }
    let ms = t0.elapsed().as_millis();
    println!("result: {}  time: {}ms", s.len(), ms);
}
