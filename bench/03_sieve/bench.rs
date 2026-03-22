use std::time::Instant;

fn is_prime(n: u64) -> bool {
    if n < 2 { return false; }
    if n == 2 { return true; }
    if n % 2 == 0 { return false; }
    let mut i = 3u64;
    while i * i <= n {
        if n % i == 0 { return false; }
        i += 2;
    }
    true
}

fn main() {
    let limit = 100_000u64;
    let t0 = Instant::now();
    let count = (0..limit).filter(|&n| is_prime(n)).count();
    let ms = t0.elapsed().as_millis();
    println!("result: {}  time: {}ms", count, ms);
}
