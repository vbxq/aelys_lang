import time

def fill_descending(n):
    arr = []
    for i in range(n, 0, -1):
        arr.append(i)
    return arr

def bubble_sort(arr):
    n = len(arr)
    for i in range(n - 1):
        for j in range(n - i - 1):
            if arr[j] > arr[j + 1]:
                arr[j], arr[j + 1] = arr[j + 1], arr[j]

n = 5000
print(f"Bubble sort benchmark (n={n})")

arr = fill_descending(n)
print("Array filled (worst case: descending)")

t = time.perf_counter()
bubble_sort(arr)
elapsed = (time.perf_counter() - t) * 1000

print(f"Sort time: {elapsed:.0f}ms")
print(f"First 5: {arr[0]} {arr[1]} {arr[2]} {arr[3]} {arr[4]}")
