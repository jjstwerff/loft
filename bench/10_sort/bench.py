"""Benchmark 10: Insertion sort (3000 integers)."""
import time

def insertion_sort(arr):
    n = len(arr)
    for i in range(1, n):
        key = arr[i]
        j = i
        while j > 0 and arr[j - 1] > key:
            arr[j] = arr[j - 1]
            j -= 1
        arr[j] = key

total = 3000
data = [(i * 31337 + 17) % 100000 for i in range(total)]

t0 = time.perf_counter()
insertion_sort(data)
ms = int((time.perf_counter() - t0) * 1000)

checksum = data[0] + data[total - 1] + data[total // 2]
ok = all(data[i] <= data[i + 1] for i in range(total - 1))
print(f"result: {checksum} sorted={ok}  time: {ms}ms")
