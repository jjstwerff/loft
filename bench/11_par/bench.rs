// Benchmark 11: par equivalent — 50-iter Newton's sqrt, 4 threads.
//
// Sums 50-iteration Newton's-method sqrt of (i+1) over n=100_000
// elements, partitioned across 4 worker threads.  No external deps
// (the run_bench.sh harness compiles each bench.rs with bare
// `rustc -O`, no cargo).  Matches loft's `par(items, work, 4)`
// shape: 4-way partition, each worker computes a partial sum, main
// thread aggregates.

use std::hint::black_box;
use std::thread;
use std::time::Instant;

fn newton_sqrt(x: f64) -> f64 {
    let mut g = x / 2.0;
    for _ in 0..50 {
        g = (g + x / g) / 2.0;
    }
    g
}

fn chunk_sum(lo: i64, hi: i64) -> f64 {
    let mut sum: f64 = 0.0;
    for i in lo..hi {
        sum += black_box(newton_sqrt((i + 1) as f64));
    }
    sum
}

fn main() {
    let n: i64 = 100_000;
    let workers: i64 = 4;
    let chunk = n / workers;
    let t0 = Instant::now();
    let handles: Vec<_> = (0..workers)
        .map(|t| {
            let lo = t * chunk;
            let hi = if t == workers - 1 { n } else { lo + chunk };
            thread::spawn(move || chunk_sum(lo, hi))
        })
        .collect();
    let total: f64 = handles.into_iter().map(|h| h.join().unwrap()).sum();
    let ms = t0.elapsed().as_millis();
    println!("result: {}  time: {}ms", total.round() as i64, ms);
}
