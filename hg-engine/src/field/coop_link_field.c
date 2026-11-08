#include "config.h"
#include "types.h"

#include "coop_sync.h"
#include "coop_link_internal.h"

// ---------------------------------------------------------------------------
// The virtual link's scan report (docs/MILESTONE3_PLAN.md §15), in the field
// extension (Overlay 131): a child scans for parents only from the field, so
// src/coop_link.c (Overlay 129) calls it through CoopFieldEntryHeader.linkScan
// (docs/LINK_BATTLE_REWORK.md phase 1 moved it here for space).
// ---------------------------------------------------------------------------

// Writes a data-cache range back to memory (CP15 "clean line", then "drain
// write buffer"). The radio writes parentInfo straight to memory, so the scan
// callback invalidates its cache lines before reading it; what the shim
// wrote there must reach memory first. NitroSDK's DC_FlushRange is unmapped.
__attribute__((target("arm"), noinline)) static void Coop_LinkCleanCache(const void *start, u32 size)
{
    u32 line = (u32)start & ~31u, end = (u32)start + size;

    for (; line < end; line += 32)
        __asm__ volatile("mcr p15, 0, %0, c7, c10, 1" : : "r"(line) : "memory");
    __asm__ volatile("mcr p15, 0, %0, c7, c10, 4" : : "r"(0) : "memory");
}


// One scan report: the next parent seen (as a WMBssDesc in parentInfo whose
// address ends with its player id), or "channel done".
void Coop_LinkScanReport(struct CoopWirelessWork *work, struct CoopLinkBlock *blk)
{
    u32 words[(0x48 + COOP_LINK_BEACON_MAX) / 4];
    u8 *event = (u8 *)words;
    const struct CoopLinkBeacon *b = NULL;
    const u8 *want = &work->scanParam[8]; // bssid looked for (FF..FF: any)
    u32 i, n;

    __builtin_memset(words, 0, sizeof(words));
    for (n = 0; n < COOP_LINK_SEEN_SLOTS && b == NULL; n++)
    {
        i = (blk->scan_next + n) % COOP_LINK_SEEN_SLOTS;
        if (blk->seen[i].length < 8 || blk->seen[i].length > COOP_LINK_BEACON_MAX)
            continue;
        if (want[0] != 0xFF && want[5] != blk->seen[i].player)
            continue;
        b = &blk->seen[i];
        blk->scan_next = (u8)(i + 1);
    }
    if (b == NULL)
    {
        event[0x08] = 4; // channel done, nothing found
        Wireless_OnScanEvent(words);
        return;
    }
    // WMBssDesc: length, rssi, bssid, ..., channel @0x36, gameInfoLength
    // @0x3C, WMGameInfo @0x40 (ggid @0x44, attribute @0x4B, user data @0x50).
    __builtin_memset(work->parentInfo, 0, sizeof(work->parentInfo));
    work->parentInfo[0x00] = sizeof(work->parentInfo) / 2;
    work->parentInfo[0x04] = 0x02;
    work->parentInfo[0x05] = 'C';
    work->parentInfo[0x06] = 'O';
    work->parentInfo[0x07] = 'O';
    work->parentInfo[0x08] = 'P';
    work->parentInfo[0x09] = b->player;
    work->parentInfo[0x36] = 1;
    work->parentInfo[0x3C] = (u8)(16 + b->length);
    __builtin_memcpy(&work->parentInfo[0x44], &b->ggid, 4);
    work->parentInfo[0x4A] = b->length;
    work->parentInfo[0x4B] = b->entry ? 1 : 0;
    __builtin_memcpy(&work->parentInfo[0x50], b->data, b->length);
    Coop_LinkCleanCache(work->parentInfo, sizeof(work->parentInfo));
    // WMStartScanCallback: state @8, the address @0xA, gameInfoLength @0x36,
    // WMGameInfo @0x38 (ggid @0x3C, attribute @0x43, user data @0x48).
    event[0x08] = 5; // parent found
    __builtin_memcpy(&event[0x0A], &work->parentInfo[0x04], 6);
    event[0x36] = (u8)(16 + b->length);
    __builtin_memcpy(&event[0x3C], &b->ggid, 4);
    event[0x43] = b->entry ? 1 : 0;
    __builtin_memcpy(&event[0x48], b->data, b->length);
    Wireless_OnScanEvent(words);
}
