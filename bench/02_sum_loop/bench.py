import time

n = 10_000_000
t0 = time.time()
total = sum(range(n))
ms = (time.time() - t0) * 1000
print(f"result: {total}  time: {ms:.0f}ms")
