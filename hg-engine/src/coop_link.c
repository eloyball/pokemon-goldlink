#include "config.h"
#include "types.h"

#include "pokemon.h"
#include "coop_sync.h"
#include "coop_link.h"
#include "coop_link_internal.h"
#include "system.h"

// ---------------------------------------------------------------------------
// Virtual link (docs/MILESTONE3_PLAN.md §15), Overlay 129.
//
// A virtual radio under the vanilla local-wireless manager (ccd
// src/wireless_manager.c, pret asm/unk_02032844.s; docs/LINK_SYMBOLS.md). The
// manager's internal functions that would call NitroSDK WM_* are hooked
// (`hooks`): each does its job against the link block instead, and leaves
// the matching vanilla callback for Coop_LinkUpdate to deliver next frame,
// as the radio would answer. Everything above (CommWireless, CommState, the
// comm core but for its frame sends: the message link below) runs unchanged.
//
// Callbacks that would call WM_* themselves are not used: the shim applies
// their success outcome (OnInitialized, OnEnded, the channel measurement).
//
// The link block lives in the manager's work buffer, in the WM system
// buffer WM_Initialize would use: zeroed by CommWireless_Init, freed with the
// session. The bridge carries it over the network (exchange_ptr).
// ---------------------------------------------------------------------------

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
// Called from the main loop and, for data, from the VBlank interrupt (the
// comm core's sends): interrupts off, so two pushes never take one slot.
static BOOL Coop_LinkPush(struct CoopLinkBlock *blk, u32 type, u32 aid, const void *data, u32 length)
{
    struct CoopLinkMsg *msg;
    u32 intr;

    if (length > COOP_LINK_DATA_MAX)
        return FALSE;
    intr = OS_DisableInterrupts();
    if ((u8)(blk->out_head - blk->out_tail) >= COOP_LINK_OUT_SLOTS)
    {
        OS_RestoreInterrupts(intr);
        return FALSE;
    }
    msg = &blk->out[blk->out_head % COOP_LINK_OUT_SLOTS];
    msg->type = (u8)type;
    msg->aid = (u8)aid;
    msg->length = (u16)length;
    if (length != 0)
        __builtin_memcpy(msg->data, data, length);
    blk->out_head++; // last: the bridge reads up to out_head
    OS_RestoreInterrupts(intr);
    return TRUE;
}

// Frees the in ring's slots up to the first message still unread (handled
// messages are marked NONE, wherever they were).
static void Coop_LinkReleaseIn(struct CoopLinkBlock *blk)
{
    while (blk->in_tail != blk->in_head && blk->in[blk->in_tail % COOP_LINK_IN_SLOTS].type == COOP_LINK_MSG_NONE)
        blk->in_tail++;
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

// Wireless_SetSendData: WM_SetMPDataToPortEx. Unused: the comm core's radio
// frames are replaced by the message link below (Coop_CommUpdate).
BOOL Coop_Link_SetSendData(void)
{
    return FALSE;
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

// A control message (DATA is the comm core's, Coop_CommUpdate).
static void Coop_LinkDeliver(struct CoopWirelessWork *work, struct CoopLinkBlock *blk, struct CoopLinkMsg *msg)
{
    u32 bit;

    switch (msg->type)
    {
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
    u32 pending;
    u8 i;

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
    {
        // The scan runs in the field: its report lives in the field extension.
        const struct CoopFieldEntryHeader *field = Coop_FieldHeader();
        if (field != NULL)
            field->linkScan(work, blk);
        else
            blk->pending |= PEND_SCAN; // next frame
    }
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

    // The others' control messages, wherever they are in the in ring: DATA
    // waits there for the comm core (Coop_CommUpdate), and a BYE must not wait
    // behind DATA a screen holds.
    for (i = blk->in_tail; i != blk->in_head && blk->magic == COOP_LINK_MAGIC; i++)
    {
        struct CoopLinkMsg *msg = &blk->in[i % COOP_LINK_IN_SLOTS];
        if (msg->type == COOP_LINK_MSG_DATA)
            continue;
        Coop_LinkDeliver(work, blk, msg);
        msg->type = COOP_LINK_MSG_NONE;
    }
    Coop_LinkReleaseIn(blk);

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

// ---------------------------------------------------------------------------
// The message link (docs/LINK_BATTLE_REWORK.md part 1)
//
// The comm core's commands travel as DATA messages instead of radio frames.
// Its per-frame child send step (Comm_UpdateChildSend) is replaced: the
// packet pool is drained with vanilla's own serializer into messages for
// every other member (the relay fans them out) and into my own receive ring,
// as transfer mode 1's parent frames did (every command reaches every game,
// its sender included); the others' messages are written into their
// receive rings. Then vanilla's Comm_ReceiveFromChildren parses them, in the
// same Comm_Update. The parent's frame step (Comm_UpdateParentSend) is a
// no-op, and the send stages stay idle, so no radio frame is ever built.
//
// A message is written into a receive ring only whole and only when the
// ring has room (ccd ByteRing_Write raises a link error otherwise, #89). The
// screen's own commands (22 and up) are only understood with the screen's
// command table, so a message holds either the comm core's base commands or
// a screen's, and the latter is stamped with the sender's table: a receiver
// not on that screen holds the message until it is (#90's class). Held
// messages keep each sender's order. (Nothing is dropped: a sender's commands
// for a screen come before what makes the others leave it, so a receiver
// never leaves a screen with its commands still on the way.)
// ---------------------------------------------------------------------------

#define COMM_BASE_COMMAND_COUNT 22

// ccd ByteRing / PacketNode / PacketPool (communication_packet_pool.h).
struct CoopByteRing
{
    u8 *data;
    s16 read;
    s16 limit;
    s16 write;
    s16 capacity;
};

struct CoopPacketNode
{
    u8 *data;
    struct CoopPacketNode *prev;
    struct CoopPacketNode *next;
    u16 size;   // left to write
    u8 command; // 0: free
    u8 flags;   // bit 0: header written
};

struct CoopPacketPool
{
    struct CoopPacketNode *head; // queue
    struct CoopPacketNode *tail;
    struct CoopPacketNode *head2; // queue2 (unused by the senders)
    struct CoopPacketNode *tail2;
    struct CoopPacketNode *current; // partly written
    void *ring;
    void *nodes;
    int count;
};

struct CoopByteWriter
{
    u8 *dest;
    int remaining;
};

// ccd CommWork (communication_core_internal.h), the fields used here at the
// offsets pret's asm/unk_02035900.s uses (recvRings 0x51C, childQueue
// 0x580, connectedNetIds 0x694, transferMode 0x6AD).
struct CoopCommWork
{
    u8 unk_000[0x51C];
    struct CoopByteRing recvRings[8]; // 0x51C commands received from each net id
    void *sendTask;                   // 0x57C
    struct CoopPacketPool childQueue; // 0x580
    u8 unk_5A0[0x694 - 0x5A0];
    u16 connectedNetIds;              // 0x694 a child's view of who is linked
    u8 unk_696[0x6AD - 0x696];
    u8 transferMode;                  // 0x6AD 1: data (co-op), 2: back to 0
};

_Static_assert(__builtin_offsetof(struct CoopCommWork, recvRings) == 0x51C, "CommWork.recvRings");
_Static_assert(__builtin_offsetof(struct CoopCommWork, childQueue) == 0x580, "CommWork.childQueue");
_Static_assert(__builtin_offsetof(struct CoopCommWork, connectedNetIds) == 0x694, "CommWork.connectedNetIds");
_Static_assert(__builtin_offsetof(struct CoopCommWork, transferMode) == 0x6AD, "CommWork.transferMode");

struct CoopCommCore // ccd CommCore
{
    u8 initialized;
    u8 updateLock;
    u8 unk_02[2];
    void *allocation;
    struct CoopCommWork *work;
};

struct CoopCommCommandTable // ccd CommCommandTable (head)
{
    const void *commands;
};

extern struct CoopCommCore sCommCore;                  // 0x021D4140
extern struct CoopCommCommandTable *sCommCommandTable; // 0x021D412C

BOOL LONG_CALL PacketPool_WriteFrame(struct CoopPacketPool *pool, struct CoopByteWriter *writer, int next);
void LONG_CALL ByteRing_Write(struct CoopByteRing *ring, const u8 *src, int count, int unused);
u32 LONG_CALL Comm_GetCommandSize(int command);

// ROM-private state after the link block, in the WM system buffer
// (CommWireless_Init zeroes it with the session).
struct CoopLinkComm
{
    const void *table; // my screen's command table (the last one registered)
    u8 bitmap;         // parent: the members in my last message
};

_Static_assert(sizeof(struct CoopLinkBlock) + sizeof(struct CoopLinkComm) <= 0xF00, "the message link state fits in the WM system buffer");

// A DATA message: this header, then the command bytes.
struct CoopCommHeader
{
    u8 bitmap;         // the parent's members (a child's Comm_IsNetIdConnected)
    u8 screen;         // 0: base commands; 1: the screen of `table`
    u16 unused;
    const void *table; // the sender's command table
};

#define COOP_COMM_DATA_MAX (COOP_LINK_DATA_MAX - sizeof(struct CoopCommHeader))

// Writes into a receive ring only what fits whole (ccd ByteRing_Write raises a
// link error otherwise); FALSE when it doesn't.
__attribute__((noinline)) static BOOL Coop_CommRingPut(struct CoopByteRing *ring, const u8 *src, u32 len, BOOL write)
{
    s32 room = ring->read > ring->write ? ring->read - ring->write : ring->capacity - (ring->write - ring->read);

    if (room <= (s32)len)
        return FALSE;
    if (write && len != 0)
        ByteRing_Write(ring, src, (int)len, 0);
    return TRUE;
}

// Comm_UpdateChildSend, every frame of a comm session.
void Coop_CommUpdate(void)
{
    struct CoopWirelessWork *work = Coop_LinkWork();
    struct CoopLinkBlock *blk = Coop_LinkBlock();
    struct CoopCommWork *cw = sCommCore.work;
    struct CoopLinkComm *lc;
    struct CoopPacketPool *pool;
    struct CoopPacketNode *node;
    struct CoopByteWriter writer;
    struct CoopCommHeader hdr;
    u32 blocked, user, len;
    u32 buf[COOP_LINK_DATA_MAX / 4];
    u8 slot;

    // Unlinked, nothing goes out; but once a battle's lost link is decided (the watchdog's
    // count), my own commands still loop back: the host's battle ends only when its own
    // game has done them (src/battle/coop_link_battle.c).
    if (blk == NULL || (!blk->linked && gCoopCommBlock.track_link_lost_frames < COOP_LINK_BATTLE_LOST_FRAMES)
        || cw == NULL || (cw->transferMode != 1 && cw->transferMode != 2))
        return;
    lc = (struct CoopLinkComm *)(blk + 1);
    pool = &cw->childQueue;

    // My screen (while a table is freed, the last one: its commands still go out).
    if (sCommCommandTable != NULL)
        lc->table = sCommCommandTable->commands;

    // Send: one run of base or screen commands per message, while the out
    // ring and my own receive ring have room. The parent's messages also
    // tell the children who is linked.
    hdr.bitmap = blk->role == COOP_LINK_ROLE_PARENT ? (u8)work->connectedBitmap : 0;
    hdr.table = lc->table;
    for (;;)
    {
        node = pool->current != NULL ? pool->current : pool->head;
        if ((node == NULL && hdr.bitmap == lc->bitmap) || (u8)(blk->out_head - blk->out_tail) >= COOP_LINK_OUT_SLOTS)
            break;
        user = node != NULL && node->command >= COMM_BASE_COMMAND_COUNT;
        for (len = 0; node != NULL && len < COOP_COMM_DATA_MAX && (node->command >= COMM_BASE_COMMAND_COUNT) == user;
             node = node == pool->current ? pool->head : node->next)
        {
            len += node->size;
            if (!(node->flags & 1))
                len += Comm_GetCommandSize(node->command) == 0xFFFF ? 3 : 1;
        }
        if (len > COOP_COMM_DATA_MAX)
            len = COOP_COMM_DATA_MAX;
        if (!Coop_CommRingPut(&cw->recvRings[blk->aid], NULL, len, FALSE))
            break; // my own ring is full: next frame
        writer.dest = (u8 *)buf + sizeof(hdr);
        writer.remaining = (int)len;
        if (len != 0)
            PacketPool_WriteFrame(pool, &writer, 1);
        hdr.screen = (u8)user;
        __builtin_memcpy(buf, &hdr, sizeof(hdr));
        (void)Coop_LinkPush(blk, COOP_LINK_MSG_DATA, blk->aid, buf, sizeof(hdr) + len);
        (void)Coop_CommRingPut(&cw->recvRings[blk->aid], (u8 *)buf + sizeof(hdr), len, TRUE);
        lc->bitmap = hdr.bitmap;
    }

    // Receive, in each sender's order.
    blocked = 0;
    for (slot = blk->in_tail; slot != blk->in_head; slot++)
    {
        struct CoopLinkMsg *msg = &blk->in[slot % COOP_LINK_IN_SLOTS];
        u32 bit = 1u << (msg->aid & 7);
        if (msg->type != COOP_LINK_MSG_DATA || (blocked & bit))
            continue;
        __builtin_memcpy(&hdr, msg->data, sizeof(hdr));
        len = msg->length > sizeof(hdr) ? msg->length - sizeof(hdr) : 0;
        if (len > COOP_COMM_DATA_MAX)
            len = COOP_COMM_DATA_MAX;
        if (hdr.screen && (sCommCommandTable == NULL || sCommCommandTable->commands != hdr.table))
            len = 0xFFFF; // not on that screen (yet): held
        if (len == 0xFFFF || !Coop_CommRingPut(&cw->recvRings[msg->aid & 7], &msg->data[sizeof(hdr)], len, TRUE))
        {
            blocked |= bit;
            continue;
        }
        if (msg->aid == 0 && blk->role == COOP_LINK_ROLE_CHILD)
            cw->connectedNetIds = hdr.bitmap | 1;
        msg->type = COOP_LINK_MSG_NONE;
    }
    Coop_LinkReleaseIn(blk);
}

// Comm_UpdateParentSend: no parent frames (Coop_CommUpdate).
void Coop_CommNoop(void)
{
}

// ---------------------------------------------------------------------------
// Link battle watchdog (docs/MILESTONE3_PLAN.md §17)
// ---------------------------------------------------------------------------

// Our sessions report no errors (vanilla's error screen resets the console),
// so a link lost in a battle would leave both games waiting forever. While a
// co-op battle runs (src/field/coop_battle.c), the battle extension ends it
// (src/battle/coop_link_battle.c, through its header at 0x023C4000); after
// it, the link result screen's sync is completed here.

#define COOP_OVY_BATTLE       12   // FS_OVERLAY_ID(OVY_12): the battle app (the battle extension comes with it)
#define OVM_OVY_ID            0x0C // ccd OverlayManager: the template's overlay id
#define COOP_SYNC_LINK_SCREEN 0x3E // ccd link_battle_screen.c: "waiting for partner"

BOOL LONG_CALL LinkSync_Exists(void);
void LONG_CALL LinkSync_RecvDone(int sender, int size, void *data, void *context);

// Children the parent links with in the running battle (src/field/coop_battle.c
// sets it): 1, or 3 in a team battle. In .data (Overlay 129 is never zeroed).
u8 gCoopLinkBattleChildren = 1;

// Lost: the relay's BYE (the other bridge went quiet or left), or our own
// bridge gone or out of the room.
BOOL Coop_LinkBattleIsLost(void)
{
    struct CoopWirelessWork *work = Coop_LinkWork();
    struct CoopLinkBlock *blk = Coop_LinkBlock();

    if (!Coop_IsConnectedToRoom() || blk == NULL)
        return TRUE;
    if (blk->role == COOP_LINK_ROLE_CHILD)
        return !blk->linked;
    return Coop_LinkCountBits(work->connectedBitmap & 0xFFFE) < gCoopLinkBattleChildren;
}

void Coop_LinkBattleWatchdog(FieldSystem *fsys)
{
    FieldProcessManager *pm;
    u8 *app;
    u32 ovy;

    if (gCoopCommBlock.track_link_battle == COOP_LINK_BATTLE_LOST)
    {
        // The link result screen after the battle waits for the partner
        // (LinkSync 0x3E): complete it as the host's "done" would.
        u8 done = COOP_SYNC_LINK_SCREEN;
        if (LinkSync_Exists())
            LinkSync_RecvDone(0, 1, &done, NULL);
        return;
    }
    pm = fsys != NULL ? fsys->processManager : NULL;
    app = pm != NULL ? pm->child : NULL;
    if (gCoopCommBlock.track_link_battle != COOP_LINK_BATTLE_RUNNING || app == NULL)
        return;
    __builtin_memcpy(&ovy, app + OVM_OVY_ID, 4);
    if (ovy == COOP_OVY_BATTLE && COOP_BATTLE_EXT->magic == COOP_BATTLE_EXT_MAGIC)
        COOP_BATTLE_EXT->linkWatchdog(app);
}
