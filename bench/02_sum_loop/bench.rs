use std::time::Instant;

fn main() {
    let n: u64 = 10_000_000;
    let t0 = Instant::now();
    let sum: u64 = (0..n).sum();
    let ms = t0.elapsed().as_millis();
    println!("result: {}  time: {}ms", sum, ms);
}
