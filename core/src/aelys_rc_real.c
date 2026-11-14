/* this file is the only translation unit that differs between runtime variants */
/* cycles leak here by design: their mutual refcount never reaches zero */

#include <stddef.h> /* NULL, size_t */
#include <stdint.h>
#include <stdlib.h> /* realloc */

#include "aelys_alloc_immix.h" /* aelys_immix_is_dead */
#include "aelys_rc.h" /* AELYS_RC_HEADER_SIZE */

extern void __aelys_free(void *ptr);
extern void __aelys_panic(const char *ptr, long long len);
extern long long __aelys_free_count;

extern const AelysDropGlue __aelys_rc_drop_table[];

static inline uint32_t *aelys_rc_header(void *ptr) {
    return (uint32_t *)((char *)ptr - AELYS_RC_HEADER_SIZE);
}

/* a glue's releases queue here, so a long chain is freed in a loop, not a frame per link */
static void **g_pending = NULL;
static size_t g_pending_len = 0;
static size_t g_pending_cap = 0;
static int g_draining = 0;

static void pending_push(void *ptr) {
    if (g_pending_len == g_pending_cap) {
        size_t cap = g_pending_cap ? g_pending_cap * 2 : 64;
        void **grown = (void **)realloc(g_pending, cap * sizeof(void *));
        if (!grown) {
            __aelys_panic("out of memory", 13);
        }
        g_pending = grown;
        g_pending_cap = cap;
    }
    g_pending[g_pending_len++] = ptr;
}

static void drain_pending(void) {
    g_draining = 1;
    while (g_pending_len > 0) {
        void *ptr = g_pending[--g_pending_len];
        uint32_t *header = aelys_rc_header(ptr);
        AelysDropGlue glue = __aelys_rc_drop_table[header[2]];
        if (glue != NULL) {
            glue(ptr);
        }
        __aelys_free_count++;
        header[0] = AELYS_RC_DEAD;
        /* free the object base, never the data pointer */
        __aelys_free((char *)ptr - AELYS_RC_HEADER_SIZE);
    }
    g_draining = 0;
}

void __aelys_rc_retain(void *ptr) {
    /* an Rc::null() handle has no header, so guard before receding to ptr-16 */
    if (ptr == NULL) {
        return;
    }
    uint32_t *header = aelys_rc_header(ptr);
    /* saturate: a wrap to 0 at 2^32 owners would report unshared and silently disable cow */
    if (header[0] != UINT32_MAX) {
        header[0] += 1;
    }
}

void __aelys_rc_release(void *ptr) {
    if (ptr == NULL) {
        return;
    }
    /* the tombstone answers without reading a header the allocator has already reused */
    if (aelys_immix_is_dead((char *)ptr - AELYS_RC_HEADER_SIZE)) {
        __aelys_panic(AELYS_RC_FREED_MSG, (long long)(sizeof(AELYS_RC_FREED_MSG) - 1));
    }
    uint32_t *header = aelys_rc_header(ptr);
    /* a pinned (saturated) count never decrements, so it can never reach zero: a leak, not a uaf */
    if (header[0] == UINT32_MAX) {
        return;
    }
    /* a release past zero scribbles on freed memory, so a double free must stop here, not later */
    if (header[0] == AELYS_RC_DEAD) {
        __aelys_panic(AELYS_RC_FREED_MSG, (long long)(sizeof(AELYS_RC_FREED_MSG) - 1));
    }
    if (header[0] == 0) {
        __aelys_panic(AELYS_RC_UNDERFLOW_MSG, (long long)(sizeof(AELYS_RC_UNDERFLOW_MSG) - 1));
    }
    header[0] -= 1;
    if (header[0] == 0) {
        pending_push(ptr);
        if (!g_draining) {
            drain_pending();
        }
    }
}
