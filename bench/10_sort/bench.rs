// Benchmark 10: Bubble sort (3000 integers, pure Rust).
use std::time::Instant;

fn bubble_sort(arr: &mut Vec<i32>) {
    let n = arr.len();
    for i in 0..n {
        for j in 0..(n - i - 1) {
            if arr[j] > arr[j + 1] {
                arr.swap(j, j + 1);
            }
        }
    }
}

fn main() {
    let n = 3000usize;
    let mut data: Vec<i32> = (0..n as i32).map(|i| (i * 31337 + 17) % 100000).collect();

    let t0 = Instant::now();
    bubble_sort(&mut data);
    let ms = t0.elapsed().as_millis();

    let checksum = data[0] + data[n - 1] + data[n / 2];
    let ok = data.windows(2).all(|w| w[0] <= w[1]);
    println!("result: {checksum} sorted={ok}  time: {ms}ms");
}
