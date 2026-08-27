/* this need to be kept as minimal as possible. */

/* shared by every runtime variant; only the rc_* unit changes at link time */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>

#include "aelys_rc.h"
#include "aelys_alloc_immix.h"

#if defined(_MSC_VER)
#define AELYS_NORETURN __declspec(noreturn)
#else
#define AELYS_NORETURN _Noreturn
#endif

/* a byte slice, may contain '\0' */
typedef struct {
    const char *ptr;
    long long len;
} AelysString;

extern long long __aelys_user_main(void);

/* non-static so the variant unit can bump the free counter through extern */
long long __aelys_alloc_count = 0;
long long __aelys_free_count = 0;

/* only the rc+cycles unit ever assigns this; it stays NULL under rc and leak, so a
   program calling __aelys_collect() still links and simply does nothing */
void (*__aelys_collect_hook)(void) = 0;

void __aelys_collect(void) {
    if (__aelys_collect_hook) {
        __aelys_collect_hook();
    }
}

AELYS_NORETURN void __aelys_panic(const char *ptr, long long len);

void __aelys_write(const char *ptr, long long len) {
    fwrite(ptr, 1, (size_t)len, stdout);
    fflush(stdout);
}

void __aelys_write_err(const char *ptr, long long len) {
    fwrite(ptr, 1, (size_t)len, stderr);
}

/* strings cross the ABI as a flat (ptr, len) pair, not a struct, because 16-byte struct
   passing does not agree between MSVC and LLVM on windows x64 */
AelysString __aelys_str_char_at(const char *str_ptr, long long str_len,
                                long long index) {
    if (index < 0) {
        __aelys_panic("index out of bounds", 20);
    }

    const unsigned char *data = (const unsigned char *)str_ptr;
    long long byte_pos = 0;
    long long char_index = 0;

    while (byte_pos < str_len && char_index < index) {
        unsigned char byte = data[byte_pos];

        if ((byte & 0x80) == 0) {
            byte_pos += 1;
        } else if ((byte & 0xE0) == 0xC0) {
            byte_pos += 2;
        } else if ((byte & 0xF0) == 0xE0) {
            byte_pos += 3;
        } else if ((byte & 0xF8) == 0xF0) {
            byte_pos += 4;
        } else {
            __aelys_panic("invalid UTF-8", 13);
        }

        char_index++;
    }

    if (char_index < index || byte_pos >= str_len) {
        __aelys_panic("index out of bounds", 20);
    }

    unsigned char start_byte = data[byte_pos];
    long long char_len = 1;

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

    if (byte_pos + char_len > str_len) {
        __aelys_panic("invalid UTF-8", 13);
    }

    AelysString result;
    result.ptr = str_ptr + byte_pos;
    result.len = char_len;
    return result;
}

/* this wrapper is the only place the alloc counter is bumped; free_count is bumped at
   the rc_* call sites, so the internal immix entry points must never touch either */
void *__aelys_alloc(long long size) {
    void *p = aelys_immix_alloc(size);
    if (!p && size > 0) {
        __aelys_panic("out of memory", 13);
    }
    __aelys_alloc_count++;
    return p;
}

void __aelys_free(void *ptr) {
    aelys_immix_free(ptr);
}

void *__aelys_realloc(void *ptr, long long size) {
    void *p = aelys_immix_realloc(ptr, size);
    if (!p && size > 0) {
        __aelys_panic("out of memory", 13);
    }
    return p;
}

/* a NULL buffer counts as uniquely owned, so an empty vec pushes into a fresh buffer
   instead of dereferencing NULL-16 */
unsigned __aelys_rc_refcount(void *ptr) {
    if (!ptr) {
        return 1;
    }
    return *(uint32_t *)((char *)ptr - AELYS_RC_HEADER_SIZE);
}

/* the buffer gets NO_TRACE and an inert type_id 0: without the flag the collector would
   read its primitive elements as rc child pointers, since a real type also owns id 0 */
void *__aelys_vec_new(long long elem_size, long long cap) {
    long long bytes = AELYS_RC_HEADER_SIZE + (cap > 0 ? elem_size * cap : 0);
    char *base = (char *)__aelys_alloc(bytes > 0 ? bytes : AELYS_RC_HEADER_SIZE);
    *(uint32_t *)(base + 0) = 1u;
    *(uint8_t *)(base + 4) = AELYS_FLAG_NO_TRACE;
    *(uint8_t *)(base + 5) = 0u;
    *(uint8_t *)(base + 6) = 0u;
    *(uint8_t *)(base + 7) = 0u;
    *(uint32_t *)(base + 8) = 0u;
    return base + AELYS_RC_HEADER_SIZE;
}

/* only valid once the caller has proven refcount==1: growing a shared buffer would move
   it under the other owner's feet */
void *__aelys_vec_grow(void *ptr, long long elem_size, long long new_cap) {
    char *base = (char *)ptr - AELYS_RC_HEADER_SIZE;
    long long bytes = AELYS_RC_HEADER_SIZE + elem_size * new_cap;
    char *nbase = (char *)__aelys_realloc(base, bytes);
    return nbase + AELYS_RC_HEADER_SIZE;
}

/* must mirror AirType::Vec's llvm struct in codegen/src/infra/types.rs */
typedef struct {
    void *ptr;
    long long len;
    long long cap;
} AelysVec;

typedef struct {
    void *ptr;
    long long len;
} AelysMutSlice;

AelysMutSlice __aelys_vec_try_as_unique_mut_slice(void *vecptr, long long elem_size) {
    (void)elem_size;
    AelysMutSlice out = {NULL, 0};
    if (!vecptr) {
        return out;
    }
    AelysVec *v = (AelysVec *)vecptr;
    if (__aelys_rc_refcount(v->ptr) == 1) {
        out.ptr = v->ptr;
        out.len = v->len;
    }
    return out;
}

void __aelys_vec_init(void *vecptr, long long elem_size, long long count) {
    AelysVec *v = (AelysVec *)vecptr;
    v->ptr = __aelys_vec_new(elem_size, count);
    v->len = count;
    v->cap = count;
}

extern void __aelys_rc_retain(void *ptr);
extern void __aelys_rc_release(void *ptr);

void __aelys_vec_retain(void *vecptr) {
    __aelys_rc_retain(((AelysVec *)vecptr)->ptr);
}

void __aelys_vec_release(void *vecptr) {
    __aelys_rc_release(((AelysVec *)vecptr)->ptr);
}

/* make the buffer uniquely owned before any write. a plain memcpy is sound because
   rc-bearing elements are rejected at the push site, so no element needs its own retain.
   `extra` is headroom the caller needs beyond len, so push keeps its one-allocation shape */
void __aelys_vec_detach(void *vecptr, long long elem_size, long long extra) {
    AelysVec *v = (AelysVec *)vecptr;
    if (__aelys_rc_refcount(v->ptr) <= 1) {
        return;
    }

    long long want = v->len + extra;
    long long newcap = v->cap > want ? v->cap : want;
    void *newbuf = __aelys_vec_new(elem_size, newcap);
    if (v->len > 0) {
        memcpy(newbuf, v->ptr, (size_t)(v->len * elem_size));
    }
    __aelys_rc_release(v->ptr); /* drop our share, the aliased vec is untouched */
    v->ptr = newbuf;
    v->cap = newcap;
}

/* copy-on-write push: a shared buffer is copied before it is touched, so the other owner
   keeps value semantics. detaching with extra==1 leaves cap > len, so the grow branch
   below is provably dead on that path and the original `else if` short-circuit survives */
void __aelys_vec_push(void *vecptr, void *elemptr, long long elem_size) {
    AelysVec *v = (AelysVec *)vecptr;
    __aelys_vec_detach(vecptr, elem_size, 1);

    if (v->len == v->cap) {
        long long newcap = v->cap > 0 ? v->cap * 2 : 1;
        v->ptr = __aelys_vec_grow(v->ptr, elem_size, newcap);
        v->cap = newcap;
    }

    memcpy((char *)v->ptr + v->len * elem_size, elemptr, (size_t)elem_size);
    v->len += 1;
}

/* arc is reserved, not implemented; the length must match the 19-byte literal */
void __aelys_arc_retain(void *ptr) {
    (void)ptr;
    __aelys_panic("arc not implemented", 19);
}

void __aelys_arc_release(void *ptr) {
    (void)ptr;
    __aelys_panic("arc not implemented", 19);
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

/////////////////////////////////////////////////////

/* TODO BOOTSTRAP ONLY ! move to std.string when ready */
AelysString __aelys_to_string_i64(long long value) {
    char *buffer = (char *)malloc(21);
    if (!buffer) {
        __aelys_panic("malloc failed in to_string_i64", 31);
    }

    int len = snprintf(buffer, 21, "%lld", value);
    if (len < 0) {
        free(buffer);
        __aelys_panic("snprintf failed in to_string_i64", 33);
    }

    AelysString result;
    result.ptr = buffer;
    result.len = (long long)len;
    return result;
}

/* TODO BOOTSTRAP ONLY ! move to std.string when ready */
AelysString __aelys_to_string_f64(double value) {
    char *buffer = (char *)malloc(64);
    if (!buffer) {
        __aelys_panic("malloc failed in to_string_f64", 31);
    }

    int len = snprintf(buffer, 64, "%.17g", value);
    if (len < 0) {
        free(buffer);
        __aelys_panic("snprintf failed in to_string_f64", 33);
    }

    AelysString result;
    result.ptr = buffer;
    result.len = (long long)len;
    return result;
}

/* TODO BOOTSTRAP ONLY ! move to std.string when ready */
AelysString __aelys_to_string_bool(long long value) {
    if (value) {
        AelysString result;
        result.ptr = "true";
        result.len = 4;
        return result;
    } else {
        AelysString result;
        result.ptr = "false";
        result.len = 5;
        return result;
    }
}

/* TODO BOOTSTRAP ONLY ! move to std.string when ready */
AelysString __aelys_str_concat(const char *a_ptr, long long a_len,
                               const char *b_ptr, long long b_len) {
    long long total = a_len + b_len;
    /* __aelys_alloc panics rather than returning NULL, so only the zero case needs a floor */
    char *buffer = (char *)__aelys_alloc(total > 0 ? total : 1);
    if (a_len > 0) {
        memcpy(buffer, a_ptr, (size_t)a_len);
    }
    if (b_len > 0) {
        memcpy(buffer + a_len, b_ptr, (size_t)b_len);
    }

    AelysString result;
    result.ptr = buffer;
    result.len = total;
    return result;
}

/* TODO BOOTSTRAP ONLY ! move to std.string when ready */
long long __aelys_str_eq(const char *a_ptr, long long a_len,
                         const char *b_ptr, long long b_len) {
    if (a_len != b_len) {
        return 0;
    }

    if (a_len == 0) {
        return 1;
    }

    return (memcmp(a_ptr, b_ptr, (size_t)a_len) == 0) ? 1 : 0;
}

int main(int argc, char **argv) {
    (void)argc;
    (void)argv;

    long long ret = __aelys_user_main();
    int code = (int)(ret & 0xFF);
    /* collect before printing the stats so the collector's frees are counted */
    __aelys_collect();
    if (getenv("AELYS_RC_STATS")) {
        fprintf(stderr, "[rc] allocs=%lld frees=%lld\n", __aelys_alloc_count,
                __aelys_free_count);
    }
    return code;
}
