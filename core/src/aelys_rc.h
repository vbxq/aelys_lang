#ifndef AELYS_RC_H
#define AELYS_RC_H

/* header is refcount u32 @0, flags u8 @4, type_id u32 @8, data @16; the program holds
   the data pointer, so the runtime recedes by this size to reach the header */
/* must stay in sync with RC_HEADER_SIZE in codegen/src/lowering/stmts.rs */
#define AELYS_RC_HEADER_SIZE 16

/* the codegen inline cow guard loads the refcount at data_ptr - 16 as a u32; a silent drift
   here would read the wrong word, so pin the two halves together */
_Static_assert(AELYS_RC_HEADER_SIZE == 16,
               "AELYS_RC_HEADER_SIZE must match RC_HEADER_SIZE in codegen/src/lowering/stmts.rs");

/* set when an object is registered as a cycle candidate, owned by aelys_rc_cycles.c */
#define AELYS_FLAG_CANDIDATE ((unsigned char)0x01)
/* vec buffers carry an rc header but hold primitive bytes, never child pointers, so the
   collector must never trace them or it would read garbage through the pointer map */
#define AELYS_FLAG_NO_TRACE  ((unsigned char)0x02)

#endif /* AELYS_RC_H */
