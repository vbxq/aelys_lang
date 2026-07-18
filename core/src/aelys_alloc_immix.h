#ifndef AELYS_ALLOC_IMMIX_H
#define AELYS_ALLOC_IMMIX_H

/* non-moving: objects are bumped or reused in place and never evacuated, so an address
   handed to the program stays valid for the object's whole life */
/* the block size lives in a private 16-byte prefix before base; nobody else sees it */
/* backing is picked from AELYS_ALLOC (immix or malloc) and frozen at the first call */

/* these do not touch the alloc/free counters, the public wrappers own that */
void *aelys_immix_alloc(long long size);
void aelys_immix_free(void *base);
void *aelys_immix_realloc(void *base, long long size);

/* counts 32 KiB region blocks only, oversized blocks are excluded */
long long aelys_immix_block_count(void);

#endif /* AELYS_ALLOC_IMMIX_H */
