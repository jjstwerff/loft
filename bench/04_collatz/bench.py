import time

def collatz_len(n):
    steps = 1
    while n != 1:
        if n % 2 == 0:
            n //= 2
        else:
            n = 3 * n + 1
        steps += 1
    return steps

t0 = time.time()
limit = 1_000_000
max_steps = 0
max_n = 0
for i in range(1, limit):
    s = collatz_len(i)
    if s > max_steps:
        max_steps = s
        max_n = i
ms = (time.time() - t0) * 1000
print(f"result: {max_n} steps={max_steps}  time: {ms:.0f}ms")
