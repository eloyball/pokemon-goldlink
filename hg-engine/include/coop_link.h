#ifndef COOP_LINK_H
#define COOP_LINK_H

#include "types.h"

// Virtual link (src/coop_link.c, Overlay 129; docs/MILESTONE3_PLAN.md §15).
// Delivers the virtual radio's callbacks and messages; once a frame, from the
// co-op SysTask, in and out of the field.
void LONG_CALL Coop_LinkUpdate(void);

#endif
