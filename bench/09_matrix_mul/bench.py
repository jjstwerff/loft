import time

n = 1_000_000
xs = [i / 1000.0 for i in range(n)]
ys = [(n - i) / 1000.0 for i in range(n)]

t0 = time.time()
dot = sum(xs[i] * ys[i] for i in range(n))
ms = (time.time() - t0) * 1000
print(f"result: {round(dot)}  time: {ms:.0f}ms")
