import time

def fib(n):
    if n <= 1:
        return n
    return fib(n - 1) + fib(n - 2)

t0 = time.time()
r = fib(38)
ms = (time.time() - t0) * 1000
print(f"result: {r}  time: {ms:.0f}ms")
