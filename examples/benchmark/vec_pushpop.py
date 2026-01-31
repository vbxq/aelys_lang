import time

n = 1000000
print(f"Vec push/pop benchmark (n={n})")

v = []

t1 = time.perf_counter()
for i in range(n):
    v.append(i)
push_time = (time.perf_counter() - t1) * 1000
print(f"Push time: {push_time:.0f}ms")
print(f"Final len: {len(v)}")

t2 = time.perf_counter()
while len(v) > 0:
    v.pop()
pop_time = (time.perf_counter() - t2) * 1000
print(f"Pop time: {pop_time:.0f}ms")
print(f"Final len: {len(v)}")
