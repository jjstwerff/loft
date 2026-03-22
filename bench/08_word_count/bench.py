import time
from collections import defaultdict

words = ["the", "quick", "brown", "fox", "jumps", "over", "the", "lazy", "dog",
         "the", "fox", "and", "the", "dog", "are", "friends", "the", "end"]
n = 600_000

t0 = time.time()
freq = defaultdict(int)
for i in range(n):
    freq[words[i % 18]] += 1
ms = (time.time() - t0) * 1000
print(f"result: total={n} the={freq['the']}  time: {ms:.0f}ms")
