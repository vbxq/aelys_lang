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
    aelys_rc_header(ptr)[0] += 1;
}

void __aelys_rc_release(void *ptr) {
    if (ptr == NULL) {
        return;
    }
    /* the leak variant never frees, even at zero: a bad release stays inert */
    aelys_rc_header(ptr)[0] -= 1;
}
