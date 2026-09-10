/* physical layout of one object: [ prefix 16o ][ base ... slot bytes ... ]
   prefix @0 u32 size_class, @4 u32 magic, @8 u64 pad (holds the payload size if LARGE).
   base = prefix + 16 is what we return; the prefix is private and nobody else sees it */
/* freelist[class] holds only slots of exactly class*16 bytes, so a reuse pop can never
   hand back a slot that is too small */
/* single-threaded: the runtime spawns no threads, so these globals need no lock */
/* every malloc below is this allocator's own substrate, counting it would double-count __aelys_alloc */

#include <stdlib.h>
#include <string.h>
#include <stdint.h>

#include "aelys_alloc_immix.h"

extern void __aelys_panic(const char *ptr, long long len);

/* the poison net is inactive unless the runtime is built with -fsanitize=address */
#if defined(__SANITIZE_ADDRESS__)
#include <sanitizer/asan_interface.h>
#define asan_poison(p, n) __asan_poison_memory_region((p), (n))
#define asan_unpoison(p, n) __asan_unpoison_memory_region((p), (n))
#else
#define asan_poison(p, n) ((void)0)
#define asan_unpoison(p, n) ((void)0)
#endif

#define AELYS_BLOCK_PAYLOAD (32 * 1024)               /* 32 KiB usable per region block */
#define AELYS_PREFIX_SIZE 16                          /* 8o class/magic + 8o pad, keeps base 16-aligned */
#define AELYS_ALIGN 16                                /* alignment of the returned base (malloc parity) */
#define AELYS_N_CLASSES (AELYS_BLOCK_PAYLOAD / 16 + 1) /* free-list buckets (2049) */
#define CLASS_LARGE 0xFFFFFFFFu                       /* sentinel: oversized dedicated block */
#define AELYS_MAGIC 0xAE115A11u                       /* live-object sentinel (anti double-free) */

typedef struct AelysBlock {
    char *base;
    size_t cursor;
    struct AelysBlock *next;
} AelysBlock;

typedef struct FreeSlot {
    struct FreeSlot *next; /* intrusive: stored in the first 8o of the free slot */
} FreeSlot;

static AelysBlock *g_cur_block = NULL;
static AelysBlock *g_all_blocks = NULL;            /* keeps region blocks reachable for LSan */
static FreeSlot *g_freelist[AELYS_N_CLASSES];
static long long g_block_count = 0;
static int g_mode = -1;                            /* -1 uninit, 0 = malloc, 1 = immix */

static void immix_init_mode(void) {
    const char *e = getenv("AELYS_ALLOC");
    g_mode = (e && strcmp(e, "malloc") == 0) ? 0 : 1;
}

static int immix_enabled(void) {
    if (g_mode < 0) {
        immix_init_mode();
    }
    return g_mode == 1;
}

static size_t round_up_16(size_t n) {
    return (n + 15u) & ~(size_t)15u;
}

static uint32_t size_to_class(size_t slot) {
    return (uint32_t)(slot / 16u);
}

/* the prefix stays unpoisoned for the object's whole life, it is closed again at free */
static void write_prefix(char *prefix, uint32_t cls) {
    *(uint32_t *)(prefix + 0) = cls;
    *(uint32_t *)(prefix + 4) = AELYS_MAGIC;
}

static uint32_t read_prefix_class(char *prefix) {
    return *(uint32_t *)(prefix + 0);
}

static uint32_t read_prefix_magic(char *prefix) {
    return *(uint32_t *)(prefix + 4);
}

/* only valid on a CLASS_LARGE object, the caller must branch on the class first */
static void write_oversized_payload(char *prefix, size_t slot) {
    *(uint64_t *)(prefix + 8) = (uint64_t)slot;
}

static size_t oversized_payload(char *prefix) {
    return (size_t)*(uint64_t *)(prefix + 8);
}

/* region blocks only; oversized blocks carry no bookkeeping struct and are freed whole */
static AelysBlock *block_record(char *raw) {
    AelysBlock *b = (AelysBlock *)malloc(sizeof(AelysBlock));
    if (!b) {
        return NULL;
    }
    b->base = raw;
    b->cursor = 0;
    b->next = g_all_blocks;
    g_all_blocks = b;
    g_block_count++;
    return b;
}

/* chaining writes `next` inside the poisoned slot, so open and reclose just that word */
static void freelist_push(uint32_t cls, FreeSlot *s) {
    asan_unpoison(s, sizeof(FreeSlot));
    s->next = g_freelist[cls];
    g_freelist[cls] = s;
    asan_poison(s, sizeof(FreeSlot));
}

static FreeSlot *freelist_pop(uint32_t cls) {
    FreeSlot *s = g_freelist[cls];
    if (!s) {
        return NULL;
    }
    asan_unpoison(s, sizeof(FreeSlot));
    g_freelist[cls] = s->next;
    /* the caller reopens the whole slot */
    return s;
}

void *aelys_immix_alloc(long long size) {
    if (!immix_enabled()) {
        return malloc((size_t)(size > 0 ? size : 0));
    }

    size_t n = (size > 0) ? (size_t)size : 1;
    size_t slot = round_up_16(n);

    if (AELYS_PREFIX_SIZE + slot > AELYS_BLOCK_PAYLOAD) {
        char *raw = (char *)malloc(AELYS_PREFIX_SIZE + slot);
        if (!raw) {
            return NULL; /* common panics on NULL && size>0 */
        }
        asan_unpoison(raw, AELYS_PREFIX_SIZE + slot);
        write_prefix(raw, CLASS_LARGE);
        write_oversized_payload(raw, slot);
        return raw + AELYS_PREFIX_SIZE;
    }

    uint32_t cls = size_to_class(slot);

    FreeSlot *fs = freelist_pop(cls);
    if (fs) {
        char *base = (char *)fs; /* the slot is the base */
        char *prefix = base - AELYS_PREFIX_SIZE;
        asan_unpoison(prefix, AELYS_PREFIX_SIZE + slot);
        write_prefix(prefix, cls);
        return base;
    }

    size_t need = AELYS_PREFIX_SIZE + slot;
    if (!g_cur_block || g_cur_block->cursor + need > AELYS_BLOCK_PAYLOAD) {
        char *raw = (char *)malloc(AELYS_BLOCK_PAYLOAD);
        if (!raw) {
            return NULL;
        }
        AelysBlock *b = block_record(raw);
        if (!b) {
            free(raw);
            return NULL;
        }
        asan_poison(raw, AELYS_BLOCK_PAYLOAD);
        g_cur_block = b;
    }
    char *prefix = g_cur_block->base + g_cur_block->cursor;
    g_cur_block->cursor += need; /* need is a multiple of 16, so the cursor stays aligned */
    char *base = prefix + AELYS_PREFIX_SIZE;
    asan_unpoison(prefix, AELYS_PREFIX_SIZE + slot);
    write_prefix(prefix, cls);
    return base;
}

void aelys_immix_free(void *base) {
    if (!immix_enabled()) {
        free(base);
        return;
    }
    if (!base) {
        return;
    }
    char *prefix = (char *)base - AELYS_PREFIX_SIZE;

    /* asan cannot see a double-free through a region allocator, the magic can */
    if (read_prefix_magic(prefix) != AELYS_MAGIC) {
        __aelys_panic("double-free or bad free", 23);
    }
    uint32_t cls = read_prefix_class(prefix);
    *(uint32_t *)(prefix + 4) = 0u; /* clear the magic before pushing */

    if (cls == CLASS_LARGE) {
        size_t oslot = oversized_payload(prefix);
        (void)oslot; /* only read by asan_poison */
        asan_poison(prefix, AELYS_PREFIX_SIZE + oslot);
        free(prefix);
        return;
    }

    size_t slot = (size_t)cls * 16u;
    (void)slot; /* only read by asan_poison */
    asan_poison(prefix, AELYS_PREFIX_SIZE + slot); /* the slot becomes a UAF trap */
    freelist_push(cls, (FreeSlot *)base);
}

void *aelys_immix_realloc(void *base, long long size) {
    if (!immix_enabled()) {
        return realloc(base, (size_t)(size > 0 ? size : 0));
    }
    if (!base) {
        return aelys_immix_alloc(size);
    }
    if (size <= 0) {
        aelys_immix_free(base);
        return NULL;
    }

    char *oprefix = (char *)base - AELYS_PREFIX_SIZE;
    if (read_prefix_magic(oprefix) != AELYS_MAGIC) {
        __aelys_panic("double-free or bad free", 23);
    }
    uint32_t ocls = read_prefix_class(oprefix);
    size_t oldslot = (ocls == CLASS_LARGE) ? oversized_payload(oprefix)
                                           : (size_t)ocls * 16u;

    /* these are the internal entry points, so the public alloc/free counters stay put */
    void *nbase = aelys_immix_alloc(size);
    if (!nbase) {
        return NULL;
    }
    size_t copy = (oldslot < (size_t)size) ? oldslot : (size_t)size;
    memcpy(nbase, base, copy); /* carries the rc header over with the payload */
    aelys_immix_free(base);
    return nbase;
}

long long aelys_immix_block_count(void) {
    return g_block_count;
}
