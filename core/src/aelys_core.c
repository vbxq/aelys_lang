/* this need to be kept as minimal as possible. */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>

#if defined(_MSC_VER)
#define AELYS_NORETURN __declspec(noreturn)
#else
#define AELYS_NORETURN _Noreturn
#endif

/* string ABI: a structure {ptr, len} for a byte slice (may contain '\0'). */
typedef struct {
    const char *ptr;
    long long len;
} AelysString;

extern long long __aelys_user_main(void);

/* forward declarations for functions used before their definition. */
AELYS_NORETURN void __aelys_panic(const char *ptr, long long len);

void __aelys_write(const char *ptr, long long len) {
    fwrite(ptr, 1, (size_t)len, stdout);
}

void __aelys_write_err(const char *ptr, long long len) {
    fwrite(ptr, 1, (size_t)len, stderr);
}

/* UTF-8 character indexing: return the i-th Unicode codepoint as a single character string */
AelysString __aelys_str_char_at(AelysString str, long long index) {
    if (index < 0) {
        __aelys_panic("index out of bounds", 20);
    }

    const unsigned char *data = (const unsigned char *)str.ptr;
    long long byte_pos = 0;
    long long char_index = 0;

    /* scan UTF-8 to find the index-th codepoint. */
    while (byte_pos < str.len && char_index < index) {
        unsigned char byte = data[byte_pos];

        /* skip continuation bytes to find the next codepoint start. */
        if ((byte & 0x80) == 0) {
            /* ASCII (0xxxxxxx) */
            byte_pos += 1;
        } else if ((byte & 0xE0) == 0xC0) {
            /* 2-byte (110xxxxx 10xxxxxx) */
            byte_pos += 2;
        } else if ((byte & 0xF0) == 0xE0) {
            /* 3-byte (1110xxxx 10xxxxxx 10xxxxxx) */
            byte_pos += 3;
        } else if ((byte & 0xF8) == 0xF0) {
            /* 4-byte (11110xxx 10xxxxxx 10xxxxxx 10xxxxxx) */
            byte_pos += 4;
        } else {
            /* invalid UTF-8 (as start of sequence) */
            __aelys_panic("invalid UTF-8", 13);
        }

        char_index++;
    }

    /* check bounds: did we reach the requested codepoint */
    if (char_index < index || byte_pos >= str.len) {
        __aelys_panic("index out of bounds", 20);
    }

    /* now byte_pos points to the start of the index-th codepoint, determine its byte length : */
    unsigned char start_byte = data[byte_pos];
    long long char_len = 1;

    /* read if cute */
    if ((start_byte & 0x80) == 0) {
        char_len = 1;
    } else if ((start_byte & 0xE0) == 0xC0) {
        char_len = 2;
    } else if ((start_byte & 0xF0) == 0xE0) {
        char_len = 3;
    } else if ((start_byte & 0xF8) == 0xF0) {
        char_len = 4;
    } else {
        __aelys_panic("invalid UTF-8", 13);
    }

    /* bounds check: ensure the full character fits in the string */
    if (byte_pos + char_len > str.len) {
        __aelys_panic("invalid UTF-8", 13);
    }

    /* return the single-character substring. */
    AelysString result;
    result.ptr = str.ptr + byte_pos;
    result.len = char_len;
    return result;
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
