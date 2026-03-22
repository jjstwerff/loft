"""Benchmark 10: Bubble sort (3000 integers)."""
import time

def bubble_sort(arr):
    n = len(arr)
    for i in range(n):
        for j in range(n - i - 1):
            if arr[j] > arr[j + 1]:
                arr[j], arr[j + 1] = arr[j + 1], arr[j]

total = 3000
data = [(i * 31337 + 17) % 100000 for i in range(total)]

t0 = time.perf_counter()
bubble_sort(data)
ms = int((time.perf_counter() - t0) * 1000)

checksum = data[0] + data[total - 1] + data[total // 2]
ok = all(data[i] <= data[i + 1] for i in range(total - 1))
print(f"result: {checksum} sorted={ok}  time: {ms}ms")
