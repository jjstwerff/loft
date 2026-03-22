use std::time::Instant;

fn fib(n: u64) -> u64 {
    if n <= 1 { n } else { fib(n - 1) + fib(n - 2) }
}

fn main() {
    let t0 = Instant::now();
    let r = fib(38);
    let ms = t0.elapsed().as_millis();
    println!("result: {}  time: {}ms", r, ms);
}
