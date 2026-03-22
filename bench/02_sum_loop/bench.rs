use std::hint::black_box;
use std::time::Instant;

fn main() {
    let n: u64 = 10_000_000;
    let t0 = Instant::now();
    let mut sum: u64 = 0;
    for i in 0..n { sum += black_box(i); }
    let ms = t0.elapsed().as_millis();
    println!("result: {}  time: {}ms", sum, ms);
}
