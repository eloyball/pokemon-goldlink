#ifndef COOP_SESSION_H
#define COOP_SESSION_H

#include "types.h"
#include "coop_sync.h"

struct FieldSystem;

// Player-to-player sessions (docs/MILESTONE3_PLAN.md, Phase 1). Overlay 131
// only (src/field/coop_session.c); Overlay 129 must never call these.

// Talking to an idle ghost once the adventure started: Trade / Battle /
// Team up / Cancel, then the request and its answer. remoteSlot indexes
// remote_players; remote is its validated copy. FALSE when the task could
// not start (field heap full).
BOOL Coop_StartSessionMenu(struct FieldSystem *fsys, u32 remoteSlot, const struct CoopPlayerState *remote);

// Every field frame: shows the Yes / No prompt for a pending incoming
// request once the player is free (no task, message or step in progress).
void Coop_UpdateSessionPrompt(struct FieldSystem *fsys);

// Every field frame (src/field/coop_trade.c): starts the trade task whenever
// the relay holds a trade for this player (gates.trade_pending) and the
// player is free, and reloads the map in place after the lead was traded.
void Coop_UpdateTrade(struct FieldSystem *fsys);

#endif // COOP_SESSION_H
