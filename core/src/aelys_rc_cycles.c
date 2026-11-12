/* this file is the only translation unit that differs between runtime variants */
/* trial-deletion cycle collector: frees pure cycles, never a live object */

#include <stddef.h> /* NULL, size_t */
#include <stdint.h>
#include <stdlib.h> /* malloc/realloc/free */

#include "aelys_alloc_immix.h" /* aelys_immix_is_dead */
#include "aelys_rc.h" /* AELYS_RC_HEADER_SIZE */

extern void __aelys_free(void *ptr);
extern void __aelys_panic(const char *ptr, long long len);
extern long long __aelys_free_count;
extern long long __aelys_cycle_collections;
extern long long __aelys_cycle_clamped;
extern void (*__aelys_collect_hook)(void);

static inline uint32_t *aelys_rc_refcount(void *ptr) {
    return (uint32_t *)((char *)ptr - AELYS_RC_HEADER_SIZE);
}
static inline uint8_t *aelys_rc_flags(void *ptr) {
    return (uint8_t *)((char *)ptr - AELYS_RC_HEADER_SIZE + 4);
}
/* header word 12 holds the candidate index, read only after comparing the entry's pointer */
static inline uint32_t *aelys_rc_candidate_slot(void *ptr) {
    return (uint32_t *)((char *)ptr - AELYS_RC_HEADER_SIZE + 12);
}
static inline uint32_t aelys_rc_type_id(void *ptr) {
    return *(uint32_t *)((char *)ptr - AELYS_RC_HEADER_SIZE + 8);
}

typedef struct {
    void *ptr;
    uint32_t gc_refcount; /* a side copy, the header refcount is never scribbled on */
    uint8_t live;
    uint8_t transient;
    uint8_t rereg;
} AelysCandidate;

static AelysCandidate *g_candidates = NULL;
static size_t g_count = 0;
static size_t g_capacity = 0;
static size_t g_live = 0;
/* a collect holds candidate indices across releases that push, so never compact during one */
static int g_collecting = 0;

/* the pointer map emitted by the compiler, indexed by type_id */
extern const uint32_t __aelys_rc_type_table[];
extern const AelysDropGlue __aelys_rc_drop_table[];

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

static void run_drop_glue(void *ptr) {
    AelysDropGlue glue = __aelys_rc_drop_table[aelys_rc_type_id(ptr)];
    if (glue != NULL) {
        glue(ptr);
    }
}

static void drain_pending(void) {
    g_draining = 1;
    while (g_pending_len > 0) {
        void *ptr = g_pending[--g_pending_len];
        run_drop_glue(ptr);
        __aelys_free_count++;
        *aelys_rc_refcount(ptr) = AELYS_RC_DEAD;
        __aelys_free((char *)ptr - AELYS_RC_HEADER_SIZE);
    }
    g_draining = 0;
}

static void candidates_compact_from(size_t start) {
    size_t kept = 0;
    for (size_t i = start; i < g_count; i++) {
        if (g_candidates[i].ptr != NULL) {
            *aelys_rc_candidate_slot(g_candidates[i].ptr) = (uint32_t)kept;
            g_candidates[kept++] = g_candidates[i];
        }
    }
    g_count = kept;
}

/* collector scratch, never counted: a bookkeeping bump would falsify every balanced row */
static void candidates_push(void *ptr) {
    if (g_count == g_capacity) {
        if (!g_collecting) {
            candidates_compact_from(0);
            g_live = g_count;
        }
        if (g_capacity == 0 || g_count > g_capacity / 2) {
            size_t new_cap = g_capacity ? g_capacity * 2 : 16;
            AelysCandidate *grown =
                (AelysCandidate *)realloc(g_candidates, new_cap * sizeof(AelysCandidate));
            if (!grown) {
                return; /* on OOM a missed collect only leaks, it is never UB */
            }
            g_candidates = grown;
            g_capacity = new_cap;
        }
    }
    g_candidates[g_count].ptr = ptr;
    g_candidates[g_count].gc_refcount = 0;
    g_candidates[g_count].live = 0;
    g_candidates[g_count].transient = 0;
    g_candidates[g_count].rereg = 0;
    *aelys_rc_candidate_slot(ptr) = (uint32_t)g_count;
    g_count++;
    g_live++;
}

static AelysCandidate *candidate_find(void *ptr) {
    if (ptr == NULL) {
        return NULL;
    }
    uint32_t i = *aelys_rc_candidate_slot(ptr);
    if (i < g_count && g_candidates[i].ptr == ptr) {
        return &g_candidates[i];
    }
    return NULL;
}

/* nulls the slot rather than compacting, so every phase must skip NULL slots */
static void candidate_remove(void *ptr) {
    AelysCandidate *c = candidate_find(ptr);
    if (c != NULL) {
        c->ptr = NULL;
        if (!g_collecting) {
            g_live--;
        }
    }
}

static uint32_t rc_child_count(void *c) {
    uint32_t type_id = aelys_rc_type_id(c);
    return __aelys_rc_type_table[1 + 2 * type_id];
}

static void *rc_child_at(void *c, uint32_t i) {
    uint32_t type_id = aelys_rc_type_id(c);
    uint32_t off_idx = __aelys_rc_type_table[1 + 2 * type_id + 1];
    uint32_t byte_off = __aelys_rc_type_table[off_idx + i];
    return *(void **)((char *)c + byte_off);
}

static void rc_child_null(void *c, uint32_t i) {
    uint32_t type_id = aelys_rc_type_id(c);
    uint32_t off_idx = __aelys_rc_type_table[1 + 2 * type_id + 1];
    uint32_t byte_off = __aelys_rc_type_table[off_idx + i];
    *(void **)((char *)c + byte_off) = NULL;
}

void __aelys_rc_retain(void *ptr) {
    /* an Rc::null() handle has no header, so guard before receding to ptr-16 */
    if (ptr == NULL) {
        return;
    }
    uint32_t *rc = aelys_rc_refcount(ptr);
    /* saturate: a wrap to 0 at 2^32 owners would report unshared and silently disable cow */
    if (rc[0] != UINT32_MAX) {
        rc[0] += 1;
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
    uint32_t *rc = aelys_rc_refcount(ptr);
    /* a pinned (saturated) count never decrements, so it can never reach zero and the collector
       sees gc_refcount stay above zero, keeping it live: a permanent leak, never a double free */
    if (rc[0] == UINT32_MAX) {
        return;
    }
    /* a release past zero scribbles on freed memory, so a double free must stop here, not later */
    if (rc[0] == AELYS_RC_DEAD) {
        __aelys_panic(AELYS_RC_FREED_MSG, (long long)(sizeof(AELYS_RC_FREED_MSG) - 1));
    }
    if (rc[0] == 0) {
        __aelys_panic(AELYS_RC_UNDERFLOW_MSG, (long long)(sizeof(AELYS_RC_UNDERFLOW_MSG) - 1));
    }
    rc[0] -= 1;
    if (rc[0] == 0) {
        /* drop the stale pointer first, or the next collect would deref freed memory */
        if ((*aelys_rc_flags(ptr) & AELYS_FLAG_CANDIDATE) != 0) {
            candidate_remove(ptr);
        }
        pending_push(ptr);
        if (!g_draining) {
            drain_pending();
        }
        return;
    }
    uint8_t *flags = aelys_rc_flags(ptr);
    /* a NO_TRACE buffer holds primitive bytes and can never be in a cycle; tracing it
       through the type_id-0 pointer map would read its elements as child pointers */
    if ((*flags & AELYS_FLAG_NO_TRACE) != 0) {
        return;
    }
    /* refcount > 0 means a residual ref that may be an internal cycle ref */
    if ((*flags & AELYS_FLAG_CANDIDATE) == 0) {
        *flags |= AELYS_FLAG_CANDIDATE;
        candidates_push(ptr);
    } else if (g_collecting) {
        AelysCandidate *c = candidate_find(ptr);
        if (c != NULL) {
            c->rereg = 1;
        }
    }
}

static int transient_push(void *ptr) {
    if (g_count == g_capacity) {
        size_t new_cap = g_capacity ? g_capacity * 2 : 16;
        AelysCandidate *grown =
            (AelysCandidate *)realloc(g_candidates, new_cap * sizeof(AelysCandidate));
        if (!grown) {
            return 0;
        }
        g_candidates = grown;
        g_capacity = new_cap;
    }
    *aelys_rc_flags(ptr) |= AELYS_FLAG_CANDIDATE;
    g_candidates[g_count].ptr = ptr;
    g_candidates[g_count].gc_refcount = aelys_rc_refcount(ptr)[0];
    g_candidates[g_count].live = 0;
    g_candidates[g_count].transient = 1;
    g_candidates[g_count].rereg = 0;
    *aelys_rc_candidate_slot(ptr) = (uint32_t)g_count;
    g_count++;
    return 1;
}

/* only valid before the first release of the collect phase */
static void collect_abort(void) {
    for (size_t i = 0; i < g_count; i++) {
        if (g_candidates[i].ptr != NULL && g_candidates[i].transient) {
            *aelys_rc_flags(g_candidates[i].ptr) &= (uint8_t)~AELYS_FLAG_CANDIDATE;
            g_candidates[i].ptr = NULL;
        }
    }
    candidates_compact_from(0);
    g_live = g_count;
    g_collecting = 0;
}

static void candidates_shrink(void) {
    size_t floor = g_live > 16384 ? g_live : 16384;
    if (g_capacity <= 4 * floor) {
        return;
    }
    size_t new_cap = 2 * floor;
    AelysCandidate *shrunk =
        (AelysCandidate *)realloc(g_candidates, new_cap * sizeof(AelysCandidate));
    if (shrunk) {
        g_candidates = shrunk;
        g_capacity = new_cap;
    }
}

void __aelys_cycle_collect(void) {
    if (g_collecting) {
        return;
    }
    __aelys_cycle_collections++;
    if (g_count == 0) {
        return;
    }
    g_collecting = 1;

    /* snapshot each candidate's real refcount into its side node */
    for (size_t i = 0; i < g_count; i++) {
        void *c = g_candidates[i].ptr;
        if (c == NULL) {
            continue;
        }
        g_candidates[i].gc_refcount = aelys_rc_refcount(c)[0];
        g_candidates[i].live = 0;
        g_candidates[i].transient = 0;
        g_candidates[i].rereg = 0;
    }

    /* subtract internal refs, so what remains is the external ref count */
    /* the bound is reread each pass: a child appended here must be expanded too */
    for (size_t i = 0; i < g_count; i++) {
        void *c = g_candidates[i].ptr;
        if (c == NULL) {
            continue;
        }
        uint32_t n = rc_child_count(c);
        for (uint32_t k = 0; k < n; k++) {
            void *child = rc_child_at(c, k);
            if (child == NULL || (*aelys_rc_flags(child) & AELYS_FLAG_NO_TRACE) != 0) {
                continue;
            }
            AelysCandidate *cn = candidate_find(child);
            if (cn == NULL) {
                if (!transient_push(child)) {
                    collect_abort();
                    return;
                }
                cn = &g_candidates[g_count - 1];
            }
            if (cn->gc_refcount > 0) {
                cn->gc_refcount -= 1;
            } else {
                __aelys_cycle_clamped++;
            }
        }
    }

    /* anything with an external ref left is reachable from outside, so it is live */
    size_t reached = g_count;
    uint32_t *stack = (uint32_t *)malloc(reached * sizeof(uint32_t));
    if (!stack) {
        collect_abort();
        return;
    }
    size_t sp = 0;
    for (size_t i = 0; i < reached; i++) {
        if (g_candidates[i].ptr != NULL && g_candidates[i].gc_refcount > 0) {
            g_candidates[i].live = 1;
            stack[sp++] = (uint32_t)i;
        }
    }
    while (sp > 0) {
        void *c = g_candidates[stack[--sp]].ptr;
        uint32_t n = rc_child_count(c);
        for (uint32_t k = 0; k < n; k++) {
            void *child = rc_child_at(c, k);
            if (child == NULL) {
                continue;
            }
            AelysCandidate *cn = candidate_find(child);
            if (cn != NULL && !cn->live) {
                cn->live = 1;
                stack[sp++] = (uint32_t)(cn - g_candidates);
            }
        }
    }
    free(stack);

    /* everything not live is a pure cycle; clear all their fields before freeing any
       of them, otherwise a child dangles when its parent goes first */
    /* releasing children below can push fresh candidates, and those are survivors, not
       garbage, so freeze the bound here and never walk past it in this collect */
    size_t gc_garbage_bound = g_count;

    for (size_t i = 0; i < gc_garbage_bound; i++) {
        if (g_candidates[i].live || g_candidates[i].ptr == NULL) {
            continue;
        }
        void *c = g_candidates[i].ptr;
        uint32_t n = rc_child_count(c);
        for (uint32_t k = 0; k < n; k++) {
            /* release only children outside the garbage set: skipping them would strand a
               phantom +1 on a survivor, releasing a garbage one would double-free it */
            void *child = rc_child_at(c, k);
            if (child != NULL) {
                AelysCandidate *cn = candidate_find(child);
                int intra_garbage = (cn != NULL && !cn->live);
                if (!intra_garbage) {
                    __aelys_rc_release(child);
                }
            }
            rc_child_null(c, k);
        }
    }
    for (size_t i = 0; i < gc_garbage_bound; i++) {
        if (g_candidates[i].live || g_candidates[i].ptr == NULL) {
            continue;
        }
        void *c = g_candidates[i].ptr;
        g_candidates[i].ptr = NULL;
        /* the rc fields are already null, so the glue lets go of the strings and closures alone */
        pending_push(c);
    }
    if (!g_draining) {
        drain_pending();
    }

    /* survivors lose the candidate bit so a later release can register them again */
    for (size_t i = 0; i < gc_garbage_bound; i++) {
        if (g_candidates[i].ptr != NULL && !g_candidates[i].rereg) {
            *aelys_rc_flags(g_candidates[i].ptr) &= (uint8_t)~AELYS_FLAG_CANDIDATE;
            g_candidates[i].ptr = NULL;
        }
    }
    /* a candidate registered while garbage was released keeps its dedup bit, so dropping it would make it unpushable forever */
    candidates_compact_from(0);
    g_live = g_count;
    candidates_shrink();
    g_collecting = 0;
}

/* under rc/leak this file is absent, so the hook stays NULL and collecting is a no-op */
#if defined(_MSC_VER)
static void aelys_install_cycle_hook(void) {
    __aelys_collect_hook = __aelys_cycle_collect;
}
#pragma section(".CRT$XCU", read)
__declspec(allocate(".CRT$XCU")) void (*aelys_cycle_ctor_)(void) =
    aelys_install_cycle_hook;
#else
__attribute__((constructor)) static void aelys_install_cycle_hook(void) {
    __aelys_collect_hook = __aelys_cycle_collect;
}
#endif
