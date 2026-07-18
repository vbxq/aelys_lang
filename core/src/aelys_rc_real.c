/* this file is the only translation unit that differs between runtime variants */
/* cycles leak here by design: their mutual refcount never reaches zero */

#include <stddef.h> /* NULL */
#include <stdint.h>

#include "aelys_rc.h" /* AELYS_RC_HEADER_SIZE */

extern void __aelys_free(void *ptr);
extern long long __aelys_free_count;

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
    uint32_t *header = aelys_rc_header(ptr);
    header[0] -= 1;
    if (header[0] == 0) {
        __aelys_free_count++;
        /* free the object base, never the data pointer */
        __aelys_free((char *)ptr - AELYS_RC_HEADER_SIZE);
    }
}
