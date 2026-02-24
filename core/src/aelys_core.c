#include <stdio.h>
#include <stdlib.h>

#if defined(_MSC_VER)
#define AELYS_NORETURN __declspec(noreturn)
#else
#define AELYS_NORETURN _Noreturn
#endif

extern long long __aelys_user_main(void);

/* String ABI is a byte slice (ptr, len); data may contain '\0'. */
void __aelys_write(const char *ptr, long long len) {
    fwrite(ptr, 1, (size_t)len, stdout);
}

void __aelys_write_err(const char *ptr, long long len) {
    fwrite(ptr, 1, (size_t)len, stderr);
}

void *__aelys_alloc(long long size) {
    return malloc((size_t)size);
}

void __aelys_free(void *ptr) {
    free(ptr);
}

void *__aelys_realloc(void *ptr, long long size) {
    return realloc(ptr, (size_t)size);
}

AELYS_NORETURN void __aelys_panic(const char *ptr, long long len) {
    fwrite(ptr, 1, (size_t)len, stderr);
    fputc('\n', stderr);
    fflush(stderr);
    abort();
}

AELYS_NORETURN void __aelys_exit(int code) {
    exit(code);
}

int main(int argc, char **argv) {
    (void)argc;
    (void)argv;

    long long ret = __aelys_user_main();
    int code = (int)(ret & 0xFF);
    return code;
}
