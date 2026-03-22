use std::collections::HashMap;
use std::time::Instant;

fn main() {
    let words = ["the", "quick", "brown", "fox", "jumps", "over", "the", "lazy", "dog",
                 "the", "fox", "and", "the", "dog", "are", "friends", "the", "end"];
    let n = 100_000usize;
    let t0 = Instant::now();
    let mut freq: HashMap<&str, usize> = HashMap::new();
    for i in 0..n {
        *freq.entry(words[i % 18]).or_insert(0) += 1;
    }
    let ms = t0.elapsed().as_millis();
    println!("result: total={} the={}  time: {}ms", n, freq["the"], ms);
}
