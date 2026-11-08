#include "config.h"
#include "types.h"
#include "constants/file.h"
#include "task.h"
#include "coop_sync.h"
#include "coop_avatars_gen.h"

// ---------------------------------------------------------------------------
// Milestone 6 §6.2: the other players on the Pokegear map, Overlay 152 (loaded with
// the vanilla Overlay 101). Each online player in the world shows as a head where
// they are: a copy of the map's own player icon (Ethan's or Lyra's head, by gender),
// or their avatar's custom head when it has one.
//
// - pret PokegearMap_LoadSprites (ov101_021E8E34) is replaced through `hooks`: the
//   vanilla sprites, then ours, and a SysTask that places them every frame.
// - pret PokegearMap_RemoveAllSprites (ov101_021E8E4C) is replaced too: ours go
//   first, while the sprite system still holds them.
// - The heads are not in the map's object manager: its zoom animation moves every
//   object it holds toward targets only vanilla sets. They follow the player icon
//   instead: its position plus (their cell - our cell) * grid.
// ---------------------------------------------------------------------------

#define COOP_PGMAP_MAGIC   0x31475043 // "CPG1"
#define COOP_PGMAP_HEADS   COOP_REMOTE_SLOTS
#define PGMAP_SPRITE_CURSOR 5        // pret PokegearMapSpriteID
#define PGMAP_SPRITE_PLAYER 6
#define POKEGEAR_REGION_KANTO  0     // pret PokegearRegion
#define POKEGEAR_REGION_INDIGO 1
#define POKEGEAR_REGION_JOHTO  2
// Custom heads (an avatar's map head, the same palette as vanilla's): their characters go in the
// last 1 KB of the first 32 KB of OBJ VRAM (1D 32K mapping addresses no more), one 128-byte spot
// per head, and the head's own image proxy points there so its cell (characters 8-11, Ethan's)
// draws them. Only when that spot is empty when the map loads; cleared when it goes.
#define COOP_OBJ_VRAM_MAIN      0x06400000
#define COOP_PGMAP_HEAD_VRAM    0x7C00
#define COOP_PGMAP_HEAD_BYTES   0x80
#define COOP_PGMAP_HEAD_CHAR    8        // pret pgmap_gra cell 2 (Ethan): characters 8-11
#define COOP_SPRITE_VRAM_MAIN   0xB8     // pret Sprite.imageProxy.vramLocation.baseAddrOfVram[2DMAIN]
#define COOP_FIRST_CUSTOM_AVATAR 3

#ifdef COOP_AVATAR_MAP_HEAD_TILES
static const u8 sCoopHeadTiles[][COOP_PGMAP_HEAD_BYTES] = COOP_AVATAR_MAP_HEAD_TILES;
#endif
#ifdef COOP_AVATAR_MAP_HEAD
static const u8 sCoopHeadRow[] = COOP_AVATAR_MAP_HEAD;
#endif

// The map window on the bottom screen (inside its frame): a head's centre must be in it.
#define COOP_PGMAP_LEFT   18
#define COOP_PGMAP_RIGHT  198
#define COOP_PGMAP_TOP    18
#define COOP_PGMAP_BOTTOM 130

// pret Sprite (include/sprite.h): only the fields used.
struct CoopPgSprite
{
    u8 pad0[0x34];
    u8 drawFlag;   // 0x34
    u8 animActive; // 0x35
};
_Static_assert(__builtin_offsetof(struct CoopPgSprite, drawFlag) == 0x34, "pret Sprite.drawFlag");

// pret PokegearManagedObject (include/application/pokegear/pokegear_internal.h).
struct CoopPgObject
{
    u8 active;
    u8 autoCull;
    u16 autoUpdateDisabled;
    s16 x;                       // 0x04 pos
    s16 y;
    u8 pad8[0x20 - 0x08];
    struct CoopPgSprite *sprite; // 0x20
    u8 pad24[4];
};
_Static_assert(sizeof(struct CoopPgObject) == 0x28, "pret PokegearManagedObject size");
_Static_assert(__builtin_offsetof(struct CoopPgObject, sprite) == 0x20, "pret PokegearManagedObject.sprite");

// pret PokegearObjectsManager.
struct CoopPgObjects
{
    u16 max;
    u16 num;
    u8 pad4[4];
    struct CoopPgObject *objects; // 0x08
};

// pret PokegearAppData: the sprite system the map's sprites belong to.
struct CoopPgApp
{
    u8 pad0[0x8C];
    void *spriteSystem;  // 0x08C
    void *spriteManager; // 0x090
};
_Static_assert(__builtin_offsetof(struct CoopPgApp, spriteManager) == 0x90, "pret PokegearAppData.spriteManager");

// pret PokegearMapAppData (include/application/pokegear/map/pokegear_map_internal.h).
struct CoopPgMapApp
{
    u8 pad0[0x0E];
    u8 curRegion;                  // 0x00E
    u8 pad0F;
    struct CoopPgApp *pokegear;    // 0x010
    u8 pad14[0x84 - 0x14];
    struct CoopPgObjects *objManager; // 0x084
    u8 pad88[0x108 - 0x88];
    s16 matrixX;                   // 0x108 our cell (y with vanilla's + 2)
    s16 matrixY;                   // 0x10A
    u8 pad10C[0x138 - 0x10C];
    u8 flags138;                   // 0x138 bit 0: zoomed
};
_Static_assert(__builtin_offsetof(struct CoopPgMapApp, pokegear) == 0x10, "pret PokegearMapAppData.pokegear");
_Static_assert(__builtin_offsetof(struct CoopPgMapApp, objManager) == 0x84, "pret PokegearMapAppData.objManager");
_Static_assert(__builtin_offsetof(struct CoopPgMapApp, matrixX) == 0x108, "pret PokegearMapAppData.matrixX");
_Static_assert(__builtin_offsetof(struct CoopPgMapApp, flags138) == 0x138, "pret PokegearMapAppData.zoomed");

// pret UnmanagedSpriteTemplate (include/sprite_system.h).
struct CoopPgTemplate
{
    int resourceSet;
    s16 x, y, z;
    u16 animation;
    int drawPriority;
    int pal;
    int vram;
    int paletteMode;
    int unk_1C, unk_20, unk_24;
};
_Static_assert(sizeof(struct CoopPgTemplate) == 0x28, "pret UnmanagedSpriteTemplate size");

void LONG_CALL ov101_021E88D8(struct CoopPgMapApp *mapApp); // PokegearMap_CreatePersistentSprites
void LONG_CALL ov100_021E6EF4(struct CoopPgObjects *mgr);   // PokegearObjectsManager_Reset
struct CoopPgSprite *LONG_CALL SpriteSystem_CreateSpriteFromResourceHeader(void *spriteSystem, void *spriteManager,
                                                                          const struct CoopPgTemplate *tmpl);
void LONG_CALL Sprite_Delete(struct CoopPgSprite *sprite);
void LONG_CALL Sprite_SetPositionXY(struct CoopPgSprite *sprite, s16 x, s16 y);
void LONG_CALL CLACT_AnmFrameSet(void *clact, int frame); // pret Sprite_SetAnimationFrame
void LONG_CALL Sprite_SetDrawFlag(void *sprite, BOOL draw);
void LONG_CALL MapHeader_GetWorldMapCoords(u32 mapId, s16 *x, s16 *y);

// The map's player icon (pret PokegearMap_CreatePersistentSprites, template 3), drawn
// behind it: one frame per gender.
static const struct CoopPgTemplate sCoopHeadTemplate = {
    .resourceSet = 1, .x = 0x20, .y = 0x60, .z = 0, .animation = 1, .drawPriority = 2, .pal = 4, .vram = 1, // NNS_G2D_VRAM_TYPE_2DMAIN
};

// Pinned at 0x023C4000 (linker.ld). DEVELOPMENT probes read it; the game only uses it.
struct CoopPgMapState
{
    u32 magic;
    struct CoopPgMapApp *mapApp;
    SysTask *task;
    struct CoopPgSprite *heads[COOP_PGMAP_HEADS];
    u32 trainer[COOP_PGMAP_HEADS]; // who each head is
    s16 cellX[COOP_PGMAP_HEADS];
    s16 cellY[COOP_PGMAP_HEADS];
    s16 screenX[COOP_PGMAP_HEADS];
    s16 screenY[COOP_PGMAP_HEADS];
    u8 shown[COOP_PGMAP_HEADS];   // drawn this frame
    u8 frame[COOP_PGMAP_HEADS];   // 0 Ethan, 1 Lyra
    u8 frames;                    // updates run (wraps)
    u8 customOk;                  // the custom heads' VRAM was empty at load
    u8 custom[COOP_PGMAP_HEADS];  // the avatar whose head is drawn there (0: vanilla frame)
    u32 vanillaBase[COOP_PGMAP_HEADS]; // each head's own image proxy base, to put back
};
__attribute__((section(".coop_pgmap_state"), used)) struct CoopPgMapState gCoopPgMap = { .magic = COOP_PGMAP_MAGIC };

// pret Pokegear_RegionFromCoords (ov100), on a cell without vanilla's + 2.
static u32 Coop_PgRegion(s32 x, s32 y)
{
    if (x > 21)
    {
        if (x == 25 && y == 8)
            return POKEGEAR_REGION_JOHTO; // Mt. Silver
        if ((x == 28 && y == 6) || (x == 28 && y > 8 && y < 13))
            return POKEGEAR_REGION_INDIGO; // Indigo Plateau, Victory Road
        return POKEGEAR_REGION_KANTO;
    }
    return POKEGEAR_REGION_JOHTO;
}

// A remote's map cell, as the field computes the Pokegear's (pret FieldSystem_InitPokegearArgs):
// outdoors its tile / 32; indoors its map's world coordinates (none: no cell).
static BOOL Coop_PgRemoteCell(const volatile struct CoopPlayerState *remote, s16 *cx, s16 *cy)
{
    s16 x = 0, y = 0;

    if (remote->trainer_id == 0 || remote->map_id == 0)
        return FALSE;
    if (remote->matrix_id == 0)
    {
        *cx = (s16)(remote->x / 32);
        *cy = (s16)(remote->z / 32 + 2);
        return TRUE;
    }
    MapHeader_GetWorldMapCoords(remote->map_id, &x, &y);
    if (x == 0 && y == 0)
        return FALSE;
    *cx = x;
    *cy = (s16)(y + 2);
    return TRUE;
}

// The custom head row of an avatar, or -1 (Ethan's or Lyra's head).
static int Coop_PgHeadRow(u32 avatar)
{
#if defined(COOP_AVATAR_MAP_HEAD) && defined(COOP_AVATAR_MAP_HEAD_TILES)
    if (avatar >= COOP_FIRST_CUSTOM_AVATAR && avatar - COOP_FIRST_CUSTOM_AVATAR < NELEMS(sCoopHeadRow)
        && sCoopHeadRow[avatar - COOP_FIRST_CUSTOM_AVATAR] != 0xFF)
        return sCoopHeadRow[avatar - COOP_FIRST_CUSTOM_AVATAR];
#endif
    (void)avatar;
    return -1;
}

static u32 Coop_PgSpotAddr(u32 i)
{
    return COOP_OBJ_VRAM_MAIN + COOP_PGMAP_HEAD_VRAM + i * COOP_PGMAP_HEAD_BYTES;
}

// Head i shows avatar's custom head (row >= 0) or the vanilla frames (row -1).
static void Coop_PgSetCustom(struct CoopPgSprite *head, u32 i, u32 avatar, int row)
{
    u32 base, k;

    if (row < 0)
    {
        if (gCoopPgMap.custom[i] != 0)
            __builtin_memcpy((u8 *)head + COOP_SPRITE_VRAM_MAIN, &gCoopPgMap.vanillaBase[i], sizeof(u32));
        gCoopPgMap.custom[i] = 0;
        return;
    }
#ifdef COOP_AVATAR_MAP_HEAD_TILES
    if (gCoopPgMap.custom[i] == avatar)
        return;
    for (k = 0; k < COOP_PGMAP_HEAD_BYTES; k += 2) // VRAM takes 16-bit writes
        *(vu16 *)(Coop_PgSpotAddr(i) + k) = (u16)(sCoopHeadTiles[row][k] | (sCoopHeadTiles[row][k + 1] << 8));
    base = COOP_PGMAP_HEAD_VRAM + i * COOP_PGMAP_HEAD_BYTES - COOP_PGMAP_HEAD_CHAR * 32;
    __builtin_memcpy((u8 *)head + COOP_SPRITE_VRAM_MAIN, &base, sizeof(u32));
    CLACT_AnmFrameSet(head, 0); // Ethan's cell: characters 8-11
    gCoopPgMap.frame[i] = 0;
    gCoopPgMap.custom[i] = (u8)avatar;
#else
    (void)head;
    (void)i;
    (void)avatar;
    (void)base;
    (void)k;
#endif
}

static void Coop_PgMapUpdate(SysTask *task, void *data)
{
    struct CoopPgMapApp *mapApp = gCoopPgMap.mapApp;
    struct CoopPgObject *player;
    s32 grid, sx, sy;
    u32 i, frame, region, shown;
    s16 cx, cy;

    (void)task;
    (void)data;
    if (mapApp == NULL || mapApp->objManager == NULL)
        return;
    player = &mapApp->objManager->objects[PGMAP_SPRITE_PLAYER];
    grid = 8 * (1 + (mapApp->flags138 & 1));
    shown = Coop_PgRegion(mapApp->matrixX, mapApp->matrixY - 2);
    gCoopPgMap.frames++;
    for (i = 0; i < COOP_PGMAP_HEADS; i++)
    {
        const volatile struct CoopPlayerState *remote = &gCoopCommBlock.remote_players[i];
        struct CoopPgSprite *head = gCoopPgMap.heads[i];
        BOOL show = FALSE;

        if (head == NULL)
            continue;
        if (gCoopCommBlock.is_connected_to_room && Coop_PgRemoteCell(remote, &cx, &cy))
        {
            region = Coop_PgRegion(cx, cy - 2);
            sx = player->x + (cx - mapApp->matrixX) * grid;
            sy = player->y + (cy - mapApp->matrixY) * grid;
            // The Pokegear map shows the player's own region (curRegion is the Fly map's), and
            // the map window only (bottom screen, inside its frame).
            show = player->sprite != NULL && player->sprite->drawFlag
                && (shown == POKEGEAR_REGION_INDIGO || region == POKEGEAR_REGION_INDIGO || region == shown)
                && sx >= COOP_PGMAP_LEFT && sx <= COOP_PGMAP_RIGHT && sy >= COOP_PGMAP_TOP && sy <= COOP_PGMAP_BOTTOM;
            gCoopPgMap.cellX[i] = cx;
            gCoopPgMap.cellY[i] = cy;
            if (show)
            {
                int row = gCoopPgMap.customOk ? Coop_PgHeadRow(remote->avatar_id) : -1;

                Coop_PgSetCustom(head, i, remote->avatar_id, row);
                // Avatar 1 is Ethan, 2 Lyra; the save's own (0) and custom ones without a head by gender.
                frame = row >= 0 ? 0 : remote->avatar_id == 1 ? 0 : remote->avatar_id == 2 ? 1 : (remote->gender & 1);
                if (gCoopPgMap.frame[i] != frame)
                {
                    CLACT_AnmFrameSet(head, (int)frame);
                    gCoopPgMap.frame[i] = (u8)frame;
                }
                Sprite_SetPositionXY(head, (s16)sx, (s16)sy);
                gCoopPgMap.screenX[i] = (s16)sx;
                gCoopPgMap.screenY[i] = (s16)sy;
            }
        }
        gCoopPgMap.trainer[i] = show ? remote->trainer_id : 0;
        if (gCoopPgMap.shown[i] != show)
        {
            Sprite_SetDrawFlag(head, show);
            gCoopPgMap.shown[i] = (u8)show;
        }
    }
}

static void Coop_PgMapRemoveHeads(void)
{
    u32 i, k;

    if (gCoopPgMap.task != NULL)
        DestroySysTask(gCoopPgMap.task);
    gCoopPgMap.task = NULL;
    for (i = 0; i < COOP_PGMAP_HEADS; i++)
    {
        if (gCoopPgMap.heads[i] != NULL)
            Sprite_Delete(gCoopPgMap.heads[i]);
        gCoopPgMap.heads[i] = NULL;
        gCoopPgMap.shown[i] = 0;
        gCoopPgMap.trainer[i] = 0;
        if (gCoopPgMap.custom[i] != 0)
            for (k = 0; k < COOP_PGMAP_HEAD_BYTES; k += 2) // leave the spot empty for the next visit
                *(vu16 *)(Coop_PgSpotAddr(i) + k) = 0;
        gCoopPgMap.custom[i] = 0;
    }
    gCoopPgMap.mapApp = NULL;
}

void LONG_CALL Coop_PgMap_LoadSprites(struct CoopPgMapApp *mapApp)
{
    struct CoopPgSprite *head;
    u32 i;

    ov101_021E88D8(mapApp); // vanilla
    Coop_PgMapRemoveHeads();  // a reload without the removal (never seen; safe either way)
    gCoopPgMap.mapApp = mapApp;
    // Custom heads only when their VRAM is free (nothing of the map's own is there).
    gCoopPgMap.customOk = TRUE;
    for (i = 0; i < COOP_PGMAP_HEADS * COOP_PGMAP_HEAD_BYTES; i += 4)
        if (*(vu32 *)(Coop_PgSpotAddr(0) + i) != 0)
            gCoopPgMap.customOk = FALSE;
    for (i = 0; i < COOP_PGMAP_HEADS; i++)
    {
        head = SpriteSystem_CreateSpriteFromResourceHeader(mapApp->pokegear->spriteSystem, mapApp->pokegear->spriteManager,
                                                           &sCoopHeadTemplate);
        gCoopPgMap.heads[i] = head;
        if (head == NULL)
            continue;
        __builtin_memcpy(&gCoopPgMap.vanillaBase[i], (u8 *)head + COOP_SPRITE_VRAM_MAIN, sizeof(u32));
        gCoopPgMap.custom[i] = 0;
        head->animActive = FALSE; // one frame per gender, as vanilla's player icon
        CLACT_AnmFrameSet(head, 0);
        gCoopPgMap.frame[i] = 0;
        Sprite_SetDrawFlag(head, FALSE);
        gCoopPgMap.shown[i] = 0;
    }
    gCoopPgMap.task = CreateSysTask(Coop_PgMapUpdate, NULL, 1);
}

void LONG_CALL Coop_PgMap_RemoveAllSprites(struct CoopPgMapApp *mapApp)
{
    Coop_PgMapRemoveHeads();
    ov100_021E6EF4(mapApp->objManager); // vanilla
}
