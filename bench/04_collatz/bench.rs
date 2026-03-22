use std::time::Instant;

fn collatz_len(mut n: u64) -> u64 {
    let mut steps = 1;
    while n != 1 {
        if n % 2 == 0 { n /= 2; } else { n = 3 * n + 1; }
        steps += 1;
    }
    steps
}

fn main() {
    let t0 = Instant::now();
    let (max_n, max_steps) = (1u64..1_000_000)
        .map(|i| (i, collatz_len(i)))
        .max_by_key(|&(_, s)| s)
        .unwrap();
    let ms = t0.elapsed().as_millis();
    println!("result: {} steps={}  time: {}ms", max_n, max_steps, ms);
}
