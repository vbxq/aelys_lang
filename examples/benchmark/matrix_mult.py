import time

def create_matrix(n, val):
    matrix = []
    size = n * n
    for i in range(size):
        matrix.append(val)
    return matrix

def matrix_mult(a, b, n):
    c = create_matrix(n, 0)
    for i in range(n):
        for j in range(n):
            s = 0
            for k in range(n):
                s = s + a[i * n + k] * b[k * n + j]
            c[i * n + j] = s
    return c

n = 100
print(f"Matrix multiplication {n}x{n}")

a = create_matrix(n, 2)
b = create_matrix(n, 3)

t = time.perf_counter()
c = matrix_mult(a, b, n)
elapsed = (time.perf_counter() - t) * 1000

print(f"Time: {elapsed:.0f}ms")
print(f"c[0] = {c[0]}")
