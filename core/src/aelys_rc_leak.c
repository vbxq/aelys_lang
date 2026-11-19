/* this file is the only translation unit that differs between runtime variants */

#include <stddef.h> /* NULL */
#include <stdint.h>

#include "aelys_rc.h" /* AELYS_RC_HEADER_SIZE */

extern void __aelys_panic(const char *ptr, long long len);

static inline uint32_t *aelys_rc_header(void *ptr) {
    return (uint32_t *)((char *)ptr - AELYS_RC_HEADER_SIZE);
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
    uint32_t *header = aelys_rc_header(ptr);
    /* a pinned (saturated) count stays put; the leak variant never frees, even at zero */
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
}
