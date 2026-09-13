#ifndef AELYS_RC_H
#define AELYS_RC_H

#include <stdint.h>

/* header is refcount u32 @0, flags u8 @4, type_id u32 @8, data @16; the program holds
   the data pointer, so the runtime recedes by this size to reach the header */
/* must stay in sync with RC_HEADER_SIZE in codegen/src/lowering/stmts.rs */
#define AELYS_RC_HEADER_SIZE 16

/* the codegen inline cow guard loads the refcount at data_ptr - 16 as a u32; a silent drift
   here would read the wrong word, so pin the two halves together */
_Static_assert(AELYS_RC_HEADER_SIZE == 16,
               "AELYS_RC_HEADER_SIZE must match RC_HEADER_SIZE in codegen/src/lowering/stmts.rs");

/* the three release variants each carry the same underflow abort, so the text lives once */
#define AELYS_RC_UNDERFLOW_MSG \
    "__aelys_rc_release: refcount already zero, a release with no matching retain"

/* stamped over the refcount just before the free, so the word a second release reads is ours */
#define AELYS_RC_DEAD ((uint32_t)0xAE11DEADu)
#define AELYS_RC_FREED_MSG \
    "__aelys_rc_release: the object was already freed, a release with no matching retain"

/* set when an object is registered as a cycle candidate, owned by aelys_rc_cycles.c */
#define AELYS_FLAG_CANDIDATE ((unsigned char)0x01)
/* vec buffers carry an rc header but hold primitive bytes, never child pointers, so the
   collector must never trace them or it would read garbage through the pointer map */
#define AELYS_FLAG_NO_TRACE  ((unsigned char)0x02)

#endif /* AELYS_RC_H */
