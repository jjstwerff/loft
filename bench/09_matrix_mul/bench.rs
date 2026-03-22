use std::time::Instant;

fn main() {
    let n = 1_000_000usize;
    let xs: Vec<f64> = (0..n).map(|i| i as f64 / 1000.0).collect();
    let ys: Vec<f64> = (0..n).map(|i| (n - i) as f64 / 1000.0).collect();
    let t0 = Instant::now();
    let dot: f64 = xs.iter().zip(ys.iter()).map(|(x, y)| x * y).sum();
    let ms = t0.elapsed().as_millis();
    println!("result: {}  time: {}ms", dot.round() as i64, ms);
}
