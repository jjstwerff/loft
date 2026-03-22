import time

n = 5000
t0 = time.time()
parts = []
for i in range(n):
    parts.append(f"item-{i};")
s = "".join(parts)
ms = (time.time() - t0) * 1000
print(f"result: {len(s)}  time: {ms:.0f}ms")
