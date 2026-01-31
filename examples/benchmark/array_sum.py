import time

def fill_array(n):
    arr = []
    for i in range(n):
        arr.append(i)
    return arr

def sum_array(arr):
    s = 0
    for x in arr:
        s = s + x
    return s

n = 1000000
print(f"Array sum benchmark (n={n})")

t1 = time.perf_counter()
arr = fill_array(n)
fill_time = (time.perf_counter() - t1) * 1000
print(f"Fill time: {fill_time:.0f}ms")

t2 = time.perf_counter()
s = sum_array(arr)
sum_time = (time.perf_counter() - t2) * 1000
print(f"Sum time: {sum_time:.0f}ms")
print(f"Sum: {s}")
