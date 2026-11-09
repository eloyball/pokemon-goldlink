#ifndef COOP_LINK_INTERNAL_H
#define COOP_LINK_INTERNAL_H

#include "types.h"
#include "coop_sync.h"

// The virtual link's view of the vanilla local-wireless manager, shared by
// src/coop_link.c (Overlay 129) and src/field/coop_link_field.c (Overlay 131).

// ccd WirelessWork (the manager's work buffer, 0x1380 bytes).
struct CoopWirelessWork
{
    u8 parentParam[0x40];   // 0x0000 WmParentParam
    u8 systemBuffer[0xF00]; // 0x0040 WM_Initialize: our link block
    u8 sendBuffer[0xE0];    // 0x0F40
    u8 receiveBuffer[0x200];// 0x1020
    u8 parentInfo[0xC0];    // 0x1220 WMBssDesc of the parent found / joined
    u8 scanParam[0x10];     // 0x12E0 buffer, channel, time, bssid[3]
    u8 unk_12F0[0x10];
    void *parentFoundCallback; // 0x1300
    u32 sendBufferSize;
    u32 receiveBufferSize;
    u16 scanChannel;
    u16 connectAfterScan;
    u32 state;              // 0x1310
    u32 mode;               // 0x1314
    void (*receiveCallback)(u16 aid, void *data, u16 length); // 0x1318
    u32 unk_131C;
    void *beaconCallback;   // 0x1320
    void *childLeftCallback;
    void *childJoinedCallback;
    u16 aid;                // 0x132C
    u16 connectedBitmap;    // 0x132E
    u32 error;              // 0x1330
    u8 maxEntries;          // 0x1334
    u8 connectBlocked;
    u8 unk_1336[2];
    u32 random;             // 0x1338
    u16 channel;
    u16 minBusyRatio;
    u16 quietChannels;      // 0x1340
    u8 entryLocked;         // 0x1342
    u8 entryClosed;
    u8 entrySet;            // 0x1344
    u8 beaconCount;         // 0x1345
    u8 unk_1346[0x3A];
};

_Static_assert(sizeof(struct CoopWirelessWork) == 0x1380, "ccd WirelessWork size");
_Static_assert(__builtin_offsetof(struct CoopWirelessWork, state) == 0x1310, "WirelessWork.state");
_Static_assert(__builtin_offsetof(struct CoopWirelessWork, connectedBitmap) == 0x132E, "WirelessWork.connectedBitmap");
_Static_assert(__builtin_offsetof(struct CoopWirelessWork, beaconCount) == 0x1345, "WirelessWork.beaconCount");
_Static_assert(sizeof(struct CoopLinkBlock) <= 0xF00, "the link block fits in the WM system buffer");

// WmParentParam fields the shim reads (ccd lib/include/nitro/wm/wm_internal.h).
#define PARAM_GAME_INFO(w)     (*(const u8 **)&(w)->parentParam[0x00])
#define PARAM_GAME_INFO_LEN(w) (*(u16 *)&(w)->parentParam[0x04])
#define PARAM_GGID(w)          (*(u32 *)&(w)->parentParam[0x08])
#define PARAM_ENTRY(w)         (*(u16 *)&(w)->parentParam[0x0E])

void LONG_CALL Wireless_OnScanEvent(void *event);

// The scan report (Overlay 131, through CoopFieldEntryHeader.linkScan): the
// next parent seen, or "channel done".
void Coop_LinkScanReport(struct CoopWirelessWork *work, struct CoopLinkBlock *blk);

#endif
