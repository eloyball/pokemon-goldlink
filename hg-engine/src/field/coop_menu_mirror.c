#include "config.h"
#include "types.h"
#include "save.h"
#include "task.h"
#include "coop_sync.h"

// ---------------------------------------------------------------------------
// The one-screen layout's menu box (the user, 2026-11-18): the window writes the field's open
// choice menu in the text boxes' font, so the game says what its choices are. Each field frame
// this copies the menu a script waits on (its words, as String) into gCoopMenuMirror, which the
// field header points to (CoopFieldEntryHeader.menu). Only while the script waits in that
// menu's own callback: a menu that closed leaves its pointer behind, and its freed memory.
// Every pointer is checked to lie in main RAM and every String to carry its magic.
// ---------------------------------------------------------------------------

BOOL LONG_CALL Task_RunScripts(TaskManager *taskman);
BOOL LONG_CALL ScrNative_YesNo(void *ctx);
BOOL LONG_CALL ScrNative_Multichoice(void *ctx);
BOOL LONG_CALL ScrNative_MultichoiceWireless(void *ctx);
BOOL LONG_CALL ScrNative_MenuExec(void *ctx);
BOOL LONG_CALL ScrNative_GetMenuChoice(void *ctx);
struct CoopMenu **LONG_CALL ov01_021F6B20(FieldSystem *fsys);

#define COOP_SCRIPT_ENV_CHECK 222271     // pret include/script.h Unk80_10_C_MAGIC
#define COOP_STRING_MAGIC     0xB6F8D2EC // pret src/pm_string.c STRING_MAGIC
#define COOP_MENU_MAX_ITEMS   28         // the menu's item array (ov01_021EDAFC)

// pret String (include/pm_string.h).
struct CoopString
{
    u16 maxsize;
    u16 size;
    u32 magic;
    u16 data[];
};

// pret ListMenuItem (include/list_menu_items.h).
struct CoopListItem
{
    struct CoopString *text;
    s32 value;
};

// pret ListMenu2D (include/list_menu_2d.h): its template's first fields.
struct CoopListMenu2D
{
    struct CoopListItem *items;
    void *window;
    u8 fontId;
    u8 itemsWide;
    u8 itemsHigh;
};
_Static_assert(__builtin_offsetof(struct CoopListMenu2D, itemsHigh) == 0x0A, "pret ListMenu2DTemplate.itemsHigh");

// pret UnkStruct_ov01_021EDC28 (asm/overlay_01_021EDAFC.s, 0x2E0 bytes): each item's words,
// expanded (MoveTutorMenu_SetListItem_Internal: StringExpandPlaceholders into the String at
// 0x1C + 4 * count, the message data at 0x8C), and the item count at 0x9B.
struct CoopMenu
{
    u8 pad0[0x1C];
    struct CoopString *words[COOP_MENU_MAX_ITEMS]; // 0x1C
    u8 pad8C[0x9B - 0x8C];
    u8 count; // 0x9B
};
_Static_assert(__builtin_offsetof(struct CoopMenu, count) == 0x9B, "the menu's item count at 0x9B");

// pret ScriptContext (include/script.h): the native callback it runs.
struct CoopScriptContext
{
    u8 stackDepth;
    u8 mode;
    u8 comparisonResult;
    u8 id;
    void *native;
};

// pret ScriptEnvironment (include/script.h): the multichoice's slot (unk_10), the yes / no's
// menu and the script contexts.
struct CoopScriptEnvironment
{
    u32 check;
    u8 pad4[0x10 - 0x04];
    struct CoopMenu *menu;              // 0x10 SCRIPTENV_MENU_WINDOW
    u8 window[0x10];                    // 0x14 Window
    struct CoopListMenu2D *listMenu2D;  // 0x24 SCRIPTENV_LIST_MENU_2D
    u8 pad28[0x38 - 0x28];
    struct CoopScriptContext *contexts[3]; // 0x38
};
_Static_assert(__builtin_offsetof(struct CoopScriptEnvironment, listMenu2D) == 0x24, "pret ScriptEnvironment.listMenu2D");
_Static_assert(__builtin_offsetof(struct CoopScriptEnvironment, contexts) == 0x38, "pret ScriptEnvironment.scriptContexts");

struct CoopMenuMirror gCoopMenuMirror;

static BOOL Coop_InRam(const void *p, u32 size)
{
    u32 a = (u32)p;

    return a >= 0x02000000 && a + size <= 0x02400000;
}

// The touch screen's YES and NO (ScrCmd_GetMenuChoice's buttons): HeartGold's characters.
static const u16 sCoopYes[] = { 0x143, 0x12F, 0x13D, 0xFFFF };
static const u16 sCoopNo[] = { 0x138, 0x139, 0xFFFF };

static void Coop_MirrorYesNo(void)
{
    u32 i;

    for (i = 0; i < NELEMS(sCoopYes); i++)
        gCoopMenuMirror.text[0][i] = sCoopYes[i];
    for (i = 0; i < NELEMS(sCoopNo); i++)
        gCoopMenuMirror.text[1][i] = sCoopNo[i];
    gCoopMenuMirror.kind = 1;
    gCoopMenuMirror.count = 2;
}

// Copies n items' words (each a String, or a list item's); stops at the first that isn't one.
static void Coop_MirrorItems(struct CoopString *const *words, const struct CoopListItem *items, u32 n, u32 kind)
{
    u32 k, i, len;

    if (!Coop_InRam(words != NULL ? (const void *)words : (const void *)items, n * (words != NULL ? 4 : 8)))
        return;
    for (k = 0; k < n && k < COOP_MENU_ITEMS; k++)
    {
        const struct CoopString *s = words != NULL ? words[k] : items[k].text;

        if (!Coop_InRam(s, sizeof(*s)) || s->magic != COOP_STRING_MAGIC || s->size > s->maxsize
            || !Coop_InRam(s->data, s->size * 2u))
            break;
        len = s->size < COOP_MENU_GLYPHS - 1 ? s->size : COOP_MENU_GLYPHS - 1;
        for (i = 0; i < len && s->data[i] != 0xFFFF; i++)
            gCoopMenuMirror.text[k][i] = s->data[i];
        gCoopMenuMirror.text[k][i] = 0xFFFF;
    }
    gCoopMenuMirror.kind = (u8)kind;
    gCoopMenuMirror.count = (u8)k;
}

void Coop_UpdateMenuMirror(FieldSystem *fsys)
{
    const struct CoopScriptEnvironment *env = NULL;
    TaskManager *tm;
    u32 depth, i;

    gCoopMenuMirror.magic = COOP_MENU_MIRROR_MAGIC;
    gCoopMenuMirror.count = 0;
    for (tm = fsys->taskman, depth = 0; tm != NULL && depth < 8; tm = tm->prev, depth++)
    {
        if (tm->func == Task_RunScripts)
        {
            env = tm->env;
            break;
        }
    }
    if (!Coop_InRam(env, sizeof(*env)) || env->check != COOP_SCRIPT_ENV_CHECK)
        return;
    for (i = 0; i < NELEMS(env->contexts); i++)
    {
        const struct CoopScriptContext *ctx = env->contexts[i];
        const struct CoopMenu *menu = NULL;

        if (!Coop_InRam(ctx, sizeof(*ctx)))
            continue;
        if (ctx->native == (void *)ScrNative_YesNo)
        {
            const struct CoopListMenu2D *list = env->listMenu2D;

            if (Coop_InRam(list, sizeof(*list)))
                Coop_MirrorItems(NULL, list->items, (u32)list->itemsWide * list->itemsHigh, 1);
            return;
        }
        if (ctx->native == (void *)ScrNative_GetMenuChoice)
        {
            Coop_MirrorYesNo();
            return;
        }
        if (ctx->native == (void *)ScrNative_Multichoice || ctx->native == (void *)ScrNative_MultichoiceWireless)
            menu = env->menu;
        else if (ctx->native == (void *)ScrNative_MenuExec)
        {
            struct CoopMenu **slot = ov01_021F6B20(fsys);

            menu = Coop_InRam(slot, sizeof(*slot)) ? *slot : NULL;
        }
        else
            continue;
        if (Coop_InRam(menu, sizeof(*menu)))
            Coop_MirrorItems(menu->words, NULL, menu->count, menu == env->menu ? 2 : 3);
        return;
    }
}
