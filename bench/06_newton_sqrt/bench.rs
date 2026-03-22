use std::time::Instant;

fn newton_sqrt(x: f64) -> f64 {
    let mut guess = x / 2.0;
    for _ in 0..50 {
        guess = (guess + x / guess) / 2.0;
    }
    guess
}

fn main() {
    let n = 1_000_000usize;
    let t0 = Instant::now();
    let acc: f64 = (1..=n).map(|i| newton_sqrt(i as f64)).sum();
    let ms = t0.elapsed().as_millis();
    println!("result: {}  time: {}ms", acc.round() as i64, ms);
}
