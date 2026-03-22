import time
import math

def is_prime(n):
    if n < 2:
        return False
    if n == 2:
        return True
    if n % 2 == 0:
        return False
    for i in range(3, int(math.sqrt(n)) + 1, 2):
        if n % i == 0:
            return False
    return True

limit = 100_000
t0 = time.time()
count = sum(1 for n in range(limit) if is_prime(n))
ms = (time.time() - t0) * 1000
print(f"result: {count}  time: {ms:.0f}ms")
