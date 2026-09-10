/* this file is the only translation unit that differs between runtime variants */

#include <stddef.h> /* NULL */
#include <stdint.h>

#include "aelys_rc.h" /* AELYS_RC_HEADER_SIZE */

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
    header[0] -= 1;
}
