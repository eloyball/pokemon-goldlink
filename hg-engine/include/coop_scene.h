#ifndef COOP_SCENE_H
#define COOP_SCENE_H

#include "types.h"
#include "task.h"

struct PartyPokemon;

// Vanilla trade scene (Overlay 71) and trade evolution scene, played from a
// field task (docs/MILESTONE3_PLAN.md §13). Started from Overlay 131
// (src/field/coop_scene_setup.c); the task itself is resident (Overlay 129,
// src/coop_scene.c).
//
// Calls a sub-task on `taskman` that fades out, leaves the overworld, plays
// the trade animation (`sent` leaves, `*partyMon` arrives, `partnerName` is
// the other trainer), plays the evolution scene when `*partyMon` evolves by
// trade (moves learned included) and copies the result into `*partyMon`,
// then restores the overworld and fades in. The calling task continues once
// the overworld is back, with its windows gone: it must close them first.
//
// The sub-task is resident: Overlay 131 unloads with the field map while it
// runs (the calling task resumes once it is back). Returns FALSE (nothing called) when out of memory.
BOOL LONG_CALL Coop_CallTradeScene(TaskManager *taskman, const struct PartyPokemon *sent, struct PartyPokemon *partyMon, const u16 *partnerName);

// A level-up evolution (team boss rewards, docs/MILESTONE3_PLAN.md §20),
// the same way: the evolution scene on `*partyMon` itself (B stops it, moves
// learned included), `species` and `method` from GetMonEvolution.
BOOL LONG_CALL Coop_CallEvolutionScene(TaskManager *taskman, struct PartyPokemon *partyMon, u16 species, int method);

// The party screen choosing `count` Pokemon for a battle (as vanilla's Union
// Room does), the same way: writes the chosen party slots, 1-based and in the
// chosen order, to orderOut[0..5] (0: none; all 0 when cancelled with B).
BOOL LONG_CALL Coop_CallPartySelectEntries(TaskManager *taskman, u8 count, u8 *orderOut);

// The party screen of vanilla's NPC trades ("Choose a Pokémon"), the same way:
// writes the chosen slot (0..5) to `*slotOut`, or COOP_PARTY_SELECT_CANCEL
// for B. `cursor` is the slot the cursor starts on.
#define COOP_PARTY_SELECT_CANCEL 7 // the party screen's Cancel / B
BOOL LONG_CALL Coop_CallTradePartySelect(TaskManager *taskman, u8 cursor, u8 *slotOut);

#endif
