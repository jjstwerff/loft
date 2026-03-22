use std::time::Instant;

fn mandelbrot(cx: f64, cy: f64) -> u64 {
    let (mut zx, mut zy) = (0.0f64, 0.0f64);
    for i in 0..256u64 {
        if zx * zx + zy * zy > 4.0 { return i; }
        let tmp = zx * zx - zy * zy + cx;
        zy = 2.0 * zx * zy + cy;
        zx = tmp;
    }
    256
}

fn main() {
    let size = 200usize;
    let t0 = Instant::now();
    let total: u64 = (0..size).flat_map(|y| (0..size).map(move |x| {
        let cx = (x as f64 / size as f64) * 3.5 - 2.5;
        let cy = (y as f64 / size as f64) * 2.0 - 1.0;
        mandelbrot(cx, cy)
    })).sum();
    let ms = t0.elapsed().as_millis();
    println!("result: {}  time: {}ms", total, ms);
}
