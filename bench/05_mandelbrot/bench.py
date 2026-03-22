import time

def mandelbrot(cx, cy):
    zx, zy = 0.0, 0.0
    for i in range(256):
        if zx * zx + zy * zy > 4.0:
            return i
        zx, zy = zx * zx - zy * zy + cx, 2.0 * zx * zy + cy
    return 256

size = 200
t0 = time.time()
total = sum(
    mandelbrot(
        (x / size) * 3.5 - 2.5,
        (y / size) * 2.0 - 1.0
    )
    for y in range(size)
    for x in range(size)
)
ms = (time.time() - t0) * 1000
print(f"result: {total}  time: {ms:.0f}ms")
