import time

def newton_sqrt(x):
    guess = x / 2.0
    for _ in range(50):
        guess = (guess + x / guess) / 2.0
    return guess

n = 1_000_000
t0 = time.time()
acc = sum(newton_sqrt(i + 1) for i in range(n))
ms = (time.time() - t0) * 1000
print(f"result: {round(acc)}  time: {ms:.0f}ms")
