#include "config.h"
#include "types.h"

#include "coop_sync.h"
#include "coop_link.h"

// ---------------------------------------------------------------------------
// Virtual link (docs/MILESTONE3_PLAN.md §15), Overlay 129.
//
// A virtual radio under the vanilla local-wireless manager (ccd
// src/wireless_manager.c, pret asm/unk_02032844.s; docs/LINK_SYMBOLS.md). The
// manager's internal functions that would call NitroSDK WM_* are hooked
// (`hooks`): each does its job against the link block instead, and leaves
// the matching vanilla callback for Coop_LinkUpdate to deliver next frame,
// as the radio would answer. Everything above (CommWireless, the comm core,
// CommState) runs unchanged.
//
// Callbacks that would call WM_* themselves are not used: the shim applies
// their success outcome (OnInitialized, OnEnded, the channel measurement).
//
// The link block lives in the manager's work buffer, in the WM system
// buffer WM_Initialize would use: zeroed by CommWireless_Init, freed with the
// session. The bridge carries it over the network (exchange_ptr).
// ---------------------------------------------------------------------------

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

// ccd wireless_manager.c WIRELESS_STATE_*.
#define WS_STOPPED  0
#define WS_IDLE     1
#define WS_SCANNING 2
#define WS_BUSY     3
#define WS_PARENT   4 // MP running (the child uses it too)
#define WS_CHILD    6
#define WS_MEASURED 7

#define WM_ERRCODE_FAILED   1
#define WM_ERRCODE_NO_ENTRY 11

struct CoopWirelessManager // ccd WirelessManager
{
    u32 unk_0;
    struct CoopWirelessWork *work;
};

struct CoopCommWireless // ccd CommWireless (head)
{
    u16 tgid;
    u16 unk_02;
    int state; // 0 off, 1 starting up, 2 started (WVR)
    void *work;
};

extern struct CoopWirelessManager sWirelessManager; // 0x021D4124
extern struct CoopCommWireless sCommWireless;       // 0x021D4134

// The manager's WM callbacks (static in ccd; addresses in rom.ld).
void LONG_CALL Wireless_OnParentParameterSet(void *event);
void LONG_CALL Wireless_OnParentEvent(void *event);
void LONG_CALL Wireless_OnParentMPEvent(void *event);
void LONG_CALL Wireless_OnParentMPEnded(void *event);
void LONG_CALL Wireless_OnParentEnded(void *event);
void LONG_CALL Wireless_OnScanEvent(void *event);
void LONG_CALL Wireless_OnScanEnded(void *event);
void LONG_CALL Wireless_OnConnectEvent(void *event);
void LONG_CALL Wireless_OnChildMPEvent(void *event);
void LONG_CALL Wireless_OnChildMPEnded(void *event);
void LONG_CALL Wireless_OnDisconnected(void *event);
void LONG_CALL Wireless_OnReset(void *event);
void LONG_CALL Wireless_OnDataSent(void *event);
void LONG_CALL Wireless_OnPortEvent(void *event);
void LONG_CALL Wireless_OnEntrySet(void *event);
int LONG_CALL CommState_GetLinkMode(void);

// Callbacks due next frame (CoopLinkBlock.pending).
enum
{
    PEND_INIT = 1 << 0,           // -> IDLE (OnInitialized)
    PEND_PARAM_SET = 1 << 1,      // OnParentParameterSet
    PEND_PARENT_STARTED = 1 << 2, // OnParentEvent: started
    PEND_PARENT_MP = 1 << 3,      // OnParentMPEvent: MP started
    PEND_PARENT_MP_END = 1 << 4,  // OnParentMPEnded
    PEND_PARENT_END = 1 << 5,     // OnParentEnded
    PEND_SCAN = 1 << 6,           // OnScanEvent
    PEND_SCAN_END = 1 << 7,       // OnScanEnded
    PEND_CHILD_MP = 1 << 8,       // OnChildMPEvent: MP started
    PEND_CHILD_MP_END = 1 << 9,   // OnChildMPEnded
    PEND_DISCONNECTED = 1 << 10,  // OnDisconnected
    PEND_RESET = 1 << 11,         // OnReset
    PEND_MEASURED = 1 << 12,      // -> MEASURED (the channel measurement)
    PEND_ENDED = 1 << 13,         // -> STOPPED (OnEnded), the block retires
    PEND_ENTRY_SET = 1 << 14,     // OnEntrySet
};

#define COOP_CONNECT_FAILSAFE (10 * 60) // frames a child waits for ACCEPT / REFUSE
#define COOP_BEACON_INTERVAL  8         // frames between "beacon sent" events

static const u8 sCoopConnectTag[3] = { 'D', 'P', 0 }; // ccd sWirelessConnectTag

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

static BOOL Coop_LinkSameBytes(const u8 *a, const u8 *b, u32 size)
{
    for (; size != 0; size--)
    {
        if (*a++ != *b++)
            return FALSE;
    }
    return TRUE;
}

static u32 Coop_LinkCountBits(u32 bits)
{
    u32 n = 0;

    for (; bits != 0; bits >>= 1)
        n += bits & 1;
    return n;
}

// ---------------------------------------------------------------------------
// Block access
// ---------------------------------------------------------------------------

static struct CoopWirelessWork *Coop_LinkWork(void)
{
    if (sCommWireless.work == NULL)
        return NULL; // CommWireless_Free ran: the manager's buffer is gone
    return sWirelessManager.work;
}

static struct CoopLinkBlock *Coop_LinkBlock(void)
{
    struct CoopWirelessWork *work = Coop_LinkWork();
    struct CoopLinkBlock *blk;

    if (work == NULL)
        return NULL;
    blk = (struct CoopLinkBlock *)work->systemBuffer;
    return blk->magic == COOP_LINK_MAGIC ? blk : NULL;
}

static void Coop_LinkSetState(struct CoopWirelessWork *work, u32 state)
{
    work->state = state;
}

static void Coop_LinkPend(u32 bits)
{
    struct CoopLinkBlock *blk = Coop_LinkBlock();

    if (blk != NULL)
        blk->pending |= bits;
}

// Queues a message for the bridge; FALSE when the out ring is full.
static BOOL Coop_LinkPush(struct CoopLinkBlock *blk, u32 type, u32 aid, const void *data, u32 length)
{
    struct CoopLinkMsg *msg;

    if ((u8)(blk->out_head - blk->out_tail) >= COOP_LINK_OUT_SLOTS || length > COOP_LINK_DATA_MAX)
        return FALSE;
    msg = &blk->out[blk->out_head % COOP_LINK_OUT_SLOTS];
    msg->type = (u8)type;
    msg->aid = (u8)aid;
    msg->length = (u16)length;
    if (length != 0)
        __builtin_memcpy(msg->data, data, length);
    blk->out_head++; // last: the bridge reads up to out_head
    return TRUE;
}

// The parent's game info, as the radio would broadcast it.
static void Coop_LinkPublishBeacon(struct CoopWirelessWork *work, struct CoopLinkBlock *blk, const u8 *info, u32 length, u32 ggid)
{
    if (info == NULL)
        length = 0;
    if (length > COOP_LINK_BEACON_MAX)
        length = COOP_LINK_BEACON_MAX;
    if (blk->mine.length != length || blk->mine.ggid != ggid || blk->mine.entry != (PARAM_ENTRY(work) != 0)
        || !Coop_LinkSameBytes(blk->mine.data, info, length))
    {
        blk->mine.ggid = ggid;
        blk->mine.entry = PARAM_ENTRY(work) != 0;
        if (length != 0)
            __builtin_memcpy(blk->mine.data, info, length);
        blk->mine.length = (u8)length;
        blk->mine.seq++;
    }
}

// No more beacons (the parent stopped).
static void Coop_LinkClearBeacon(struct CoopLinkBlock *blk)
{
    if (blk->mine.length == 0)
        return;
    blk->mine.length = 0;
    blk->mine.seq++;
}

// ---------------------------------------------------------------------------
// Hooked manager functions (each replaces the one named in the comment)
// ---------------------------------------------------------------------------

// Wireless_Initialize: WM_Initialize -> OnInitialized -> IDLE.
BOOL Coop_Link_Initialize(int mode)
{
    struct CoopWirelessWork *work = sWirelessManager.work;
    struct CoopLinkBlock *blk = (struct CoopLinkBlock *)work->systemBuffer;

    (void)mode;
    Coop_LinkSetState(work, WS_BUSY);
    __builtin_memset(blk, 0, sizeof(*blk));
    blk->version = COOP_LINK_VERSION;
    blk->size = sizeof(*blk);
    blk->target = 0xFF;
    blk->pending = PEND_INIT;
    blk->magic = COOP_LINK_MAGIC;
    return TRUE;
}

// Wireless_StartMeasureChannels: every channel is as quiet as the next.
BOOL Coop_Link_StartMeasureChannels(void)
{
    struct CoopWirelessWork *work = sWirelessManager.work;

    work->channel = 0;
    work->minBusyRatio = 0;
    work->quietChannels = 1; // channel 1
    Coop_LinkSetState(work, WS_BUSY);
    Coop_LinkPend(PEND_MEASURED);
    return TRUE;
}

// Wireless_SendParentParameter: WM_SetParentParameter.
BOOL Coop_Link_SendParentParameter(void)
{
    Coop_LinkSetState(sWirelessManager.work, WS_BUSY);
    Coop_LinkPend(PEND_PARAM_SET);
    return TRUE;
}

// Wireless_StartParentRun: WM_StartParent; beaconing starts.
BOOL Coop_Link_StartParentRun(void)
{
    struct CoopWirelessWork *work = sWirelessManager.work;
    struct CoopLinkBlock *blk = Coop_LinkBlock();

    if (work->state - WS_PARENT <= 2)
        return TRUE;
    if (blk == NULL)
        return FALSE;
    work->aid = 0;
    work->connectedBitmap = 1;
    blk->role = COOP_LINK_ROLE_PARENT;
    blk->aid = 0;
    blk->pending |= PEND_PARENT_STARTED;
    Coop_LinkPublishBeacon(work, blk, PARAM_GAME_INFO(work), PARAM_GAME_INFO_LEN(work), PARAM_GGID(work));
    return TRUE;
}

// Wireless_StartParentMP: WM_StartMP (parent).
BOOL Coop_Link_StartParentMP(void)
{
    struct CoopWirelessWork *work = sWirelessManager.work;

    if (work->state - WS_PARENT <= 2)
        return TRUE;
    Coop_LinkSetState(work, WS_PARENT);
    Coop_LinkPend(PEND_PARENT_MP);
    return TRUE;
}

// Wireless_EndParent: WM_EndMP (parent). The children learn it from BYE.
BOOL Coop_Link_EndParent(void)
{
    struct CoopLinkBlock *blk = Coop_LinkBlock();

    Coop_LinkSetState(sWirelessManager.work, WS_BUSY);
    if (blk != NULL)
    {
        (void)Coop_LinkPush(blk, COOP_LINK_MSG_BYE, 0, NULL, 0);
        Coop_LinkClearBeacon(blk);
        blk->linked = 0;
        blk->pending |= PEND_PARENT_MP_END;
    }
    return TRUE;
}

// Wireless_EndParentRun: WM_EndParent.
BOOL Coop_Link_EndParentRun(void)
{
    Coop_LinkPend(PEND_PARENT_END);
    return TRUE;
}

// Wireless_ScanNextChannel: WM_StartScan on the next channel; one report a frame.
BOOL Coop_Link_ScanNextChannel(void)
{
    Coop_LinkPend(PEND_SCAN);
    return TRUE;
}

// Wireless_EndScan: WM_EndScan.
BOOL Coop_Link_EndScan(void)
{
    Coop_LinkPend(PEND_SCAN_END);
    return TRUE;
}

// Wireless_StartConnect: WM_StartConnectEx to the parent in parentInfo, whose
// player id is the last byte of the address the fake scan gave it.
BOOL Coop_Link_StartConnect(void)
{
    struct CoopWirelessWork *work = sWirelessManager.work;
    struct CoopLinkBlock *blk = Coop_LinkBlock();
    u8 data[5];

    if (work->state - WS_PARENT <= 2)
        return TRUE;
    if (blk == NULL)
        return FALSE;
    Coop_LinkSetState(work, WS_BUSY);
    data[0] = work->parentInfo[4 + 5];
    data[1] = (u8)CommState_GetLinkMode();
    data[2] = sCoopConnectTag[0];
    data[3] = sCoopConnectTag[1];
    data[4] = sCoopConnectTag[2];
    if (!Coop_LinkPush(blk, COOP_LINK_MSG_CONNECT, 0, data, sizeof(data)))
        return FALSE;
    blk->role = COOP_LINK_ROLE_CHILD;
    blk->target = data[0];
    blk->linked = 0;
    blk->timer = 0;
    return TRUE;
}

// Wireless_StartChildMP: WM_StartMP (child).
BOOL Coop_Link_StartChildMP(void)
{
    Coop_LinkPend(PEND_CHILD_MP);
    return TRUE;
}

// Wireless_EndChild: WM_EndMP (child).
BOOL Coop_Link_EndChild(void)
{
    Coop_LinkSetState(sWirelessManager.work, WS_BUSY);
    Coop_LinkPend(PEND_CHILD_MP_END);
    return TRUE;
}

// Wireless_Disconnect: WM_Disconnect (child); the parent learns it from BYE.
BOOL Coop_Link_Disconnect(void)
{
    struct CoopLinkBlock *blk = Coop_LinkBlock();

    Coop_LinkSetState(sWirelessManager.work, WS_BUSY);
    if (blk != NULL)
    {
        if (blk->linked)
            (void)Coop_LinkPush(blk, COOP_LINK_MSG_BYE, 0, NULL, 0);
        blk->linked = 0;
        blk->pending |= PEND_DISCONNECTED;
    }
    return TRUE;
}

// Wireless_Reset: WM_Reset.
BOOL Coop_Link_Reset(void)
{
    struct CoopLinkBlock *blk = Coop_LinkBlock();

    Coop_LinkSetState(sWirelessManager.work, WS_BUSY);
    if (blk != NULL)
    {
        if (blk->linked || blk->mine.length != 0)
            (void)Coop_LinkPush(blk, COOP_LINK_MSG_BYE, 0, NULL, 0);
        Coop_LinkClearBeacon(blk);
        blk->linked = 0;
        blk->pending |= PEND_RESET;
    }
    return TRUE;
}

// Wireless_SetSendData: WM_SetMPDataToPortEx. The callback learns next frame
// that it went out; a full ring is "not sent now", the comm core retries.
BOOL Coop_Link_SetSendData(const void *data, u32 length, int port, void *callback)
{
    struct CoopLinkBlock *blk = Coop_LinkBlock();

    (void)port; // the comm core only uses port 14
    if (blk == NULL || blk->sent_count >= 4 || !Coop_LinkPush(blk, COOP_LINK_MSG_DATA, blk->aid, data, length))
        return FALSE;
    blk->sent[blk->sent_count++] = callback;
    return TRUE;
}

// Wireless_SetReceiveCallback: WM_SetPortCallback.
void Coop_Link_SetReceiveCallback(void *callback, int port)
{
    (void)port;
    sWirelessManager.work->receiveCallback = callback;
}

// Wireless_End: WM_End -> OnEnded -> STOPPED.
BOOL Coop_Link_End(void)
{
    Coop_LinkSetState(sWirelessManager.work, WS_BUSY);
    Coop_LinkPend(PEND_ENDED);
    return TRUE;
}

// Wireless_UpdateGameInfo: WM_SetGameInfo, while a parent.
void Coop_Link_UpdateGameInfo(const void *gameInfo, int length, u32 ggid, int tgid)
{
    struct CoopWirelessWork *work = sWirelessManager.work;
    struct CoopLinkBlock *blk = Coop_LinkBlock();

    (void)tgid;
    if (blk != NULL && work->state == WS_PARENT)
        Coop_LinkPublishBeacon(work, blk, gameInfo, (u32)length, ggid);
}

// Wireless_SetEntry: WM_SetEntry, while a parent.
BOOL Coop_Link_SetEntry(BOOL enabled)
{
    struct CoopWirelessWork *work = sWirelessManager.work;
    struct CoopLinkBlock *blk = Coop_LinkBlock();

    work->entrySet = 0;
    if (blk == NULL || work->state != WS_PARENT)
        return FALSE;
    PARAM_ENTRY(work) = enabled ? 1 : 0;
    blk->pending |= PEND_ENTRY_SET;
    return TRUE;
}

// CommWireless_StartUp: WVR_StartUpAsync (the ARM7 wireless driver).
void Coop_Link_CommStartUp(void)
{
    sCommWireless.state = 2; // started
}

// CommWireless_Terminate: WVR_TerminateAsync.
void Coop_Link_CommTerminate(void)
{
    sCommWireless.state = 0; // off
}

// ---------------------------------------------------------------------------
// Delivery (once a frame)
// ---------------------------------------------------------------------------

// A WM callback argument: apiid, errcode, then the fields each callback reads
// (ccd wireless_manager.c event structs).
static void Coop_LinkEvent(void (*callback)(void *), u32 errcode, u32 stateOffset, u32 state)
{
    u32 words[0x28 / 4];
    u16 *event = (u16 *)words;

    __builtin_memset(words, 0, sizeof(words));
    event[1] = (u16)errcode;
    event[stateOffset / 2] = (u16)state;
    callback(words);
}

#define Coop_LinkOk(callback) Coop_LinkEvent((callback), 0, 4, 0)

// Event buffers are u32 arrays: the callbacks read them with word loads.
static void Coop_LinkParentEvent(u32 state, u32 aid, const u8 *connectData)
{
    u32 words[0x20 / 4];
    u8 *event = (u8 *)words;

    __builtin_memset(words, 0, sizeof(words));
    event[0x08] = (u8)state; // u16 state
    event[0x10] = (u8)aid;   // u16 aid
    if (connectData != NULL)
        __builtin_memcpy(&event[0x14], connectData, 4);
    Wireless_OnParentEvent(words);
}

static void Coop_LinkConnectEvent(u32 errcode, u32 state, u32 aid)
{
    u32 words[0x10 / 4];
    u8 *event = (u8 *)words;

    __builtin_memset(words, 0, sizeof(words));
    event[0x02] = (u8)errcode;
    event[0x08] = (u8)state;
    event[0x0A] = (u8)aid;
    Wireless_OnConnectEvent(words);
}

// One scan report: the next parent seen (as a WMBssDesc in parentInfo whose
// address ends with its player id), or "channel done".
static void Coop_LinkScanReport(struct CoopWirelessWork *work, struct CoopLinkBlock *blk)
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

static void Coop_LinkDeliver(struct CoopWirelessWork *work, struct CoopLinkBlock *blk, struct CoopLinkMsg *msg)
{
    u32 words[0x14 / 4];
    u8 *event = (u8 *)words;
    void *data = msg->data;
    u32 bit;

    switch (msg->type)
    {
    case COOP_LINK_MSG_DATA:
        if (work->state != WS_PARENT && work->state != WS_CHILD)
            return; // not in MP: the radio would drop it
        // WMPortRecvCallback: state @4 (0x15 received), data @0xC, length @0x10, aid @0x12.
        __builtin_memset(words, 0, sizeof(words));
        event[0x04] = 0x15;
        __builtin_memcpy(&event[0x0C], &data, 4);
        __builtin_memcpy(&event[0x10], &msg->length, 2);
        event[0x12] = msg->aid;
        Wireless_OnPortEvent(words);
        break;

    case COOP_LINK_MSG_CONNECT:
        if (blk->role != COOP_LINK_ROLE_PARENT || msg->aid == 0 || msg->aid > 15)
            return;
        bit = 1u << msg->aid;
        // The manager refuses with WM_Disconnect, which needs the radio:
        // the same checks are made here, and only an accepted child reaches it.
        if (work->state != WS_PARENT || work->entryClosed || work->entryLocked || PARAM_ENTRY(work) == 0
            || Coop_LinkCountBits(work->connectedBitmap) >= work->maxEntries || msg->data[0] != (u8)CommState_GetLinkMode()
            || !Coop_LinkSameBytes(&msg->data[1], sCoopConnectTag, 3))
        {
            (void)Coop_LinkPush(blk, COOP_LINK_MSG_REFUSE, msg->aid, NULL, 0);
            return;
        }
        Coop_LinkParentEvent(7, msg->aid, msg->data);
        if (work->connectedBitmap & bit)
        {
            blk->linked = 1;
            (void)Coop_LinkPush(blk, COOP_LINK_MSG_ACCEPT, msg->aid, NULL, 0);
        }
        else
            (void)Coop_LinkPush(blk, COOP_LINK_MSG_REFUSE, msg->aid, NULL, 0);
        break;

    case COOP_LINK_MSG_ACCEPT:
        if (blk->role != COOP_LINK_ROLE_CHILD || blk->linked)
            return;
        blk->linked = 1;
        blk->aid = msg->aid;
        Coop_LinkConnectEvent(0, 7, msg->aid); // connected: the manager starts MP
        break;

    case COOP_LINK_MSG_REFUSE:
        if (blk->role == COOP_LINK_ROLE_CHILD && !blk->linked)
            Coop_LinkConnectEvent(WM_ERRCODE_NO_ENTRY, 0, 0);
        break;

    case COOP_LINK_MSG_BYE:
        if (blk->role == COOP_LINK_ROLE_PARENT && msg->aid != 0 && msg->aid <= 15
            && (work->connectedBitmap & (1u << msg->aid)))
        {
            Coop_LinkParentEvent(9, msg->aid, NULL); // child left
            blk->linked = (work->connectedBitmap & 0xFFFE) != 0;
        }
        else if (blk->role == COOP_LINK_ROLE_CHILD && msg->aid == 0 && blk->linked)
        {
            blk->linked = 0;
            Coop_LinkConnectEvent(0, 9, 0); // the parent is gone
        }
        break;
    }
}

void Coop_LinkUpdate(void)
{
    struct CoopWirelessWork *work = Coop_LinkWork();
    struct CoopLinkBlock *blk = Coop_LinkBlock();
    u32 pending, i, count;
    void *sent[4];

    if (blk == NULL)
    {
        if ((gCoopCommBlock.exchange_ptr & 3) == 0 && gCoopCommBlock.exchange_ptr != 0
            && *(const u32 *)gCoopCommBlock.exchange_ptr == COOP_LINK_MAGIC)
            gCoopCommBlock.exchange_ptr = 0; // the session ended
        return;
    }
    if (gCoopCommBlock.exchange_ptr == 0)
        gCoopCommBlock.exchange_ptr = (u32)blk;

    // Callbacks due from last frame's requests (new ones wait a frame).
    pending = blk->pending;
    blk->pending = 0;
    if (pending & PEND_INIT)
        Coop_LinkSetState(work, WS_IDLE);
    if (pending & PEND_MEASURED)
        Coop_LinkSetState(work, WS_MEASURED);
    if (pending & PEND_PARAM_SET)
        Coop_LinkOk(Wireless_OnParentParameterSet);
    if (pending & PEND_PARENT_STARTED)
        Coop_LinkParentEvent(0, 0, NULL);
    if (pending & PEND_PARENT_MP)
        Coop_LinkEvent(Wireless_OnParentMPEvent, 0, 4, 10);
    if (pending & PEND_CHILD_MP)
        Coop_LinkEvent(Wireless_OnChildMPEvent, 0, 4, 10);
    if (pending & PEND_SCAN)
        Coop_LinkScanReport(work, blk);
    if (pending & PEND_SCAN_END)
        Coop_LinkOk(Wireless_OnScanEnded);
    if (pending & PEND_ENTRY_SET)
        Coop_LinkOk(Wireless_OnEntrySet);
    if (pending & PEND_PARENT_MP_END)
        Coop_LinkOk(Wireless_OnParentMPEnded);
    if (pending & PEND_PARENT_END)
        Coop_LinkOk(Wireless_OnParentEnded);
    if (pending & PEND_CHILD_MP_END)
        Coop_LinkOk(Wireless_OnChildMPEnded);
    if (pending & PEND_DISCONNECTED)
        Coop_LinkOk(Wireless_OnDisconnected);
    if (pending & PEND_RESET)
        Coop_LinkOk(Wireless_OnReset);

    // Sends of last frame went out.
    count = blk->sent_count;
    for (i = 0; i < count; i++)
        sent[i] = blk->sent[i];
    blk->sent_count = 0;
    for (i = 0; i < count; i++)
    {
        u32 event[0x24 / 4];
        __builtin_memset(event, 0, sizeof(event));
        event[0x20 / 4] = (u32)sent[i]; // WMPortSendCallback: the request's argument
        Wireless_OnDataSent(event);
    }

    // The others' messages.
    while (blk->in_tail != blk->in_head && blk->magic == COOP_LINK_MAGIC)
    {
        struct CoopLinkMsg *msg = &blk->in[blk->in_tail % COOP_LINK_IN_SLOTS];
        if (msg->length > COOP_LINK_DATA_MAX)
            msg->length = COOP_LINK_DATA_MAX;
        Coop_LinkDeliver(work, blk, msg);
        blk->in_tail++;
    }

    if (blk->role == COOP_LINK_ROLE_PARENT && work->state == WS_PARENT)
    {
        // The beacon the radio would send, and its "beacon sent" events
        // (CommWireless waits for a few before it lists the parent as open).
        Coop_LinkPublishBeacon(work, blk, PARAM_GAME_INFO(work), PARAM_GAME_INFO_LEN(work), PARAM_GGID(work));
        if (++blk->timer >= COOP_BEACON_INTERVAL)
        {
            blk->timer = 0;
            Coop_LinkParentEvent(2, 0, NULL);
        }
    }
    else if (blk->role == COOP_LINK_ROLE_CHILD && !blk->linked && work->state == WS_BUSY
             && ++blk->timer >= COOP_CONNECT_FAILSAFE)
    {
        blk->timer = 0;
        Coop_LinkConnectEvent(WM_ERRCODE_FAILED, 0, 0); // no answer from the parent
    }

    if (pending & PEND_ENDED)
    {
        // The manager is done with the radio; its buffer is freed next.
        Coop_LinkSetState(work, WS_STOPPED);
        blk->magic = 0;
        if (gCoopCommBlock.exchange_ptr == (u32)blk)
            gCoopCommBlock.exchange_ptr = 0;
    }
}
