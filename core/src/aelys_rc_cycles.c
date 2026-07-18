/* this file is the only translation unit that differs between runtime variants */
/* trial-deletion cycle collector: frees pure cycles, never a live object */

#include <stddef.h> /* NULL, size_t */
#include <stdint.h>
#include <stdlib.h> /* malloc/realloc/free */

#include "aelys_rc.h" /* AELYS_RC_HEADER_SIZE */

extern void __aelys_free(void *ptr);
extern long long __aelys_free_count;
extern void (*__aelys_collect_hook)(void);

static inline uint32_t *aelys_rc_refcount(void *ptr) {
    return (uint32_t *)((char *)ptr - AELYS_RC_HEADER_SIZE);
}
static inline uint8_t *aelys_rc_flags(void *ptr) {
    return (uint8_t *)((char *)ptr - AELYS_RC_HEADER_SIZE + 4);
}
static inline uint32_t aelys_rc_type_id(void *ptr) {
    return *(uint32_t *)((char *)ptr - AELYS_RC_HEADER_SIZE + 8);
}

typedef struct {
    void *ptr;
    uint32_t gc_refcount; /* a side copy, the header refcount is never scribbled on */
    uint8_t live;
} AelysCandidate;

static AelysCandidate *g_candidates = NULL;
static size_t g_count = 0;
static size_t g_capacity = 0;

/* the pointer map emitted by the compiler, indexed by type_id */
extern const uint32_t __aelys_rc_type_table[];

static void candidates_push(void *ptr) {
    if (g_count == g_capacity) {
        size_t new_cap = g_capacity ? g_capacity * 2 : 16;
        AelysCandidate *grown =
            (AelysCandidate *)realloc(g_candidates, new_cap * sizeof(AelysCandidate));
        if (!grown) {
            return; /* on OOM a missed collect only leaks, it is never UB */
        }
        g_candidates = grown;
        g_capacity = new_cap;
    }
    g_candidates[g_count].ptr = ptr;
    g_candidates[g_count].gc_refcount = 0;
    g_candidates[g_count].live = 0;
    g_count++;
}

static AelysCandidate *candidate_find(void *ptr) {
    if (ptr == NULL) {
        return NULL;
    }
    for (size_t i = 0; i < g_count; i++) {
        if (g_candidates[i].ptr == ptr) {
            return &g_candidates[i];
        }
    }
    return NULL;
}

/* nulls the slot rather than compacting, so every phase must skip NULL slots */
static void candidate_remove(void *ptr) {
    for (size_t i = 0; i < g_count; i++) {
        if (g_candidates[i].ptr == ptr) {
            g_candidates[i].ptr = NULL;
            return;
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
    aelys_rc_refcount(ptr)[0] += 1;
}

void __aelys_rc_release(void *ptr) {
    if (ptr == NULL) {
        return;
    }
    uint32_t *rc = aelys_rc_refcount(ptr);
    rc[0] -= 1;
    if (rc[0] == 0) {
        /* drop the stale pointer first, or the next collect would deref freed memory */
        if ((*aelys_rc_flags(ptr) & AELYS_FLAG_CANDIDATE) != 0) {
            candidate_remove(ptr);
        }
        __aelys_free_count++;
        __aelys_free((char *)ptr - AELYS_RC_HEADER_SIZE);
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
    }
}

void __aelys_cycle_collect(void) {
    if (g_count == 0) {
        return;
    }

    /* snapshot each candidate's real refcount into its side node */
    for (size_t i = 0; i < g_count; i++) {
        void *c = g_candidates[i].ptr;
        if (c == NULL) {
            continue;
        }
        g_candidates[i].gc_refcount = aelys_rc_refcount(c)[0];
        g_candidates[i].live = 0;
    }

    /* subtract internal refs, so what remains is the external ref count */
    for (size_t i = 0; i < g_count; i++) {
        void *c = g_candidates[i].ptr;
        if (c == NULL) {
            continue;
        }
        uint32_t n = rc_child_count(c);
        for (uint32_t k = 0; k < n; k++) {
            void *child = rc_child_at(c, k);
            if (child == NULL) {
                continue;
            }
            AelysCandidate *cn = candidate_find(child);
            if (cn != NULL && cn->gc_refcount > 0) {
                cn->gc_refcount -= 1;
            }
        }
    }

    /* anything with an external ref left is reachable from outside, so it is live */
    for (size_t i = 0; i < g_count; i++) {
        if (g_candidates[i].ptr != NULL && g_candidates[i].gc_refcount > 0) {
            g_candidates[i].live = 1;
        }
    }

    /* propagate liveness to a fixpoint, so a cycle hanging off a live root survives
       whole; this is what stops the collector from freeing reachable objects */
    int changed = 1;
    while (changed) {
        changed = 0;
        for (size_t i = 0; i < g_count; i++) {
            if (!g_candidates[i].live || g_candidates[i].ptr == NULL) {
                continue;
            }
            void *c = g_candidates[i].ptr;
            uint32_t n = rc_child_count(c);
            for (uint32_t k = 0; k < n; k++) {
                void *child = rc_child_at(c, k);
                if (child == NULL) {
                    continue;
                }
                AelysCandidate *cn = candidate_find(child);
                if (cn != NULL && !cn->live) {
                    cn->live = 1;
                    changed = 1;
                }
            }
        }
    }

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
        __aelys_free_count++;
        __aelys_free((char *)c - AELYS_RC_HEADER_SIZE);
        g_candidates[i].ptr = NULL;
    }

    /* survivors lose the candidate bit so a later release can register them again */
    for (size_t i = 0; i < gc_garbage_bound; i++) {
        if (g_candidates[i].live && g_candidates[i].ptr != NULL) {
            *aelys_rc_flags(g_candidates[i].ptr) &= (uint8_t)~AELYS_FLAG_CANDIDATE;
        }
    }
    /* keep the candidates registered during P4: dropping them while their dedup bit is
       still set would make them unpushable forever */
    size_t kept = 0;
    for (size_t i = gc_garbage_bound; i < g_count; i++) {
        if (g_candidates[i].ptr != NULL) {
            g_candidates[kept++] = g_candidates[i];
        }
    }
    g_count = kept;
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
