#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#if defined(_MSC_VER)
#define AELYS_NORETURN __declspec(noreturn)
#else
#define AELYS_NORETURN _Noreturn
#endif

// bootstrap: move to stdlib when ready
long long println(const char *s) {
    puts(s);
    fflush(stdout);
    return 0;
}

// bootstrap: move to stdlib when ready
long long print(const char *s) {
    fputs(s, stdout);
    fflush(stdout);
    return 0;
}

void __aelys_write(const char *ptr, long long len) {
    fwrite(ptr, 1, (size_t)len, stdout);
}

void __aelys_write_err(const char *ptr, long long len) {
    fwrite(ptr, 1, (size_t)len, stderr);
}

long long __aelys_read_stdin(char *buf, long long max_len) {
    if (!fgets(buf, (int)max_len, stdin)) {
        return 0;
    }
    return (long long)strlen(buf);
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
