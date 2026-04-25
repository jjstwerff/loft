"""Benchmark 11: par equivalent — 50-iter Newton's sqrt, 4 processes.

Sums 50-iteration Newton's-method sqrt of (i+1) over n=100_000
elements, partitioned across 4 worker processes (Python's GIL
prevents threads from running CPU-bound work in parallel; this
uses multiprocessing.Pool).

Matches loft's `par(items, work, 4)` shape: 4-way partition, each
worker computes a partial sum, main thread aggregates.
"""

import time
from multiprocessing import Pool


def newton_sqrt(x: float) -> float:
    g = x / 2.0
    for _ in range(50):
        g = (g + x / g) / 2.0
    return g


def chunk_sum(args):
    lo, hi = args
    return sum(newton_sqrt(float(i + 1)) for i in range(lo, hi))


if __name__ == "__main__":
    n = 100_000
    workers = 4
    chunk = n // workers
    ranges = [
        (t * chunk, (t + 1) * chunk if t < workers - 1 else n)
        for t in range(workers)
    ]
    t0 = time.time()
    with Pool(workers) as p:
        total = sum(p.map(chunk_sum, ranges))
    ms = (time.time() - t0) * 1000
    print(f"result: {round(total)}  time: {ms:.0f}ms")
