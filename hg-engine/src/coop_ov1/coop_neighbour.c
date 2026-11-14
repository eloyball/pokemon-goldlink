#include "config.h"
#include "types.h"

// The wider picture (include/coop_wide.h), Overlay 1's unused space (src/coop_ov1/linker.ld).
//
// The field loads one area's graphics at a time: every map block in view binds its textures by
// name to that area's set (ccd src/field/map_load_manager.c, Task_LoadMapModel and
// LoadMapModel). A neighbouring area's block finds most of them (grass, roads, trees) but not
// its own, which draw untextured (white or grey); the DS never shows such a block, the client's
// side strips do. Here, once a block is bound, if any texture is missing:
// - each missing texture takes the bound one in the same block with the longest common name
//   ("roadkk_ri1" gets "roadkk_ro"), or, with no relative (4 letters in common), its
//   materials aren't drawn.
// The area's own blocks find all their textures: they're left exactly as they were.
//
// NitroSystem's model layout (ccd lib/include/nnsys/g3d/binres/res_struct.h, g3d_internal.h,
// lib/NitroSystem/src/g3d_mdl_bind.c), read byte by byte.

#define RES_BIND_BOUND           1          // G3dResBindEntry.flag
#define MAT_POLYGON_ATTR         0x0C       // G3dResMatData
#define MAT_POLYGON_ATTR_MASK    0x10
#define MAT_TEX_IMAGE_PARAM      0x14
#define MAT_TEX_PLTT_BASE        0x1C
#define MAT_SCALE_S              0x24
#define MAT_SCALE_T              0x28
#define TEX_IMAGE_PARAM_KEEP     0xC00F0000 // bits a texture binding doesn't set
#define POLYGON_ATTR_SHOW_FACES  0x000000C0 // back and front faces shown
#define RES_NAME_SIZE            16

// pret 2e7767dd0^ sub_0201F64C (ccd src/gf_3d_render.c): binds a model file's models to a
// texture set by name.
BOOL LONG_CALL GF3dRender_BindModelSet(void *header, const void *texture);

// NitroSystem's resources keep their words and halfwords aligned: copies the compiler can do
// with one load or store (AGENTS.md: no type-punning).
static u32 Read32(const u8 *p)
{
    u32 v;
    __builtin_memcpy(&v, __builtin_assume_aligned(p, 4), 4);
    return v;
}

static u32 Read16(const u8 *p)
{
    u16 v;
    __builtin_memcpy(&v, __builtin_assume_aligned(p, 2), 2);
    return v;
}

static void Write32(u8 *p, u32 v)
{
    __builtin_memcpy(__builtin_assume_aligned(p, 4), &v, 4);
}

// NNSG3dResDict: numEntry at +1, the entries' header at +ofsEntry (+6): sizeUnit, ofsName, then
// the data, sizeUnit bytes each; the names (16 bytes each) at the header + ofsName.
static u8 *DictData(u8 *dict, u32 i)
{
    u8 *head = dict + Read16(dict + 6);
    return head + 4 + i * Read16(head);
}

static const u8 *DictName(u8 *dict, u32 i)
{
    u8 *head = dict + Read16(dict + 6);
    return head + Read16(head + 2) + i * RES_NAME_SIZE;
}

// G3dResMat: its material dictionary at +4 holds each material's offset from the G3dResMat.
static u8 *MatData(u8 *mat, u32 index)
{
    return mat + Read32(DictData(mat + 4, index));
}

static u32 CommonPrefix(const u8 *a, const u8 *b)
{
    u32 n = 0;
    while (n < RES_NAME_SIZE && a[n] != 0 && a[n] == b[n])
        n++;
    return n;
}

BOOL LONG_CALL Coop_BindMapModel(void *header, const void *texture)
{
    BOOL result = GF3dRender_BindModelSet(header, texture);
    u8 *file = header;
    // NNSG3dResFileHeader (16 bytes), then the blocks' offsets: the model set is the first. Its
    // dictionary follows its 8-byte block header; a map file holds one model.
    u8 *set = file + Read32(file + 16);
    if (set[8 + 1] == 0)
        return result;
    u8 *model = set + Read32(DictData(set + 8, 0));
    u8 *mat = model + Read32(model + 8);   // NNSG3dResMdl.ofsMat
    u8 *textures = mat + Read16(mat);      // G3dResMat.ofsDictTexToMatList
    u32 count = textures[1], i, j, k;

    // (A block with every texture bound, the area's own, goes through untouched.)
    for (i = 0; i < count; i++)
    {
        u8 *bind = DictData(textures, i); // G3dResBindEntry: ofsMatIdxList, numMat, flag
        if (bind[3] & RES_BIND_BOUND)
            continue;
        u8 *relative = NULL;
        u32 best = 3;
        for (j = 0; j < count; j++)
        {
            u8 *other = DictData(textures, j);
            u32 common = CommonPrefix(DictName(textures, i), DictName(textures, j));
            if ((other[3] & RES_BIND_BOUND) && common > best)
            {
                best = common;
                relative = other;
            }
        }
        const u8 *materials = mat + Read16(bind);
        for (k = 0; k < bind[2]; k++)
        {
            u8 *m = MatData(mat, materials[k]);
            if (relative != NULL)
            {
                u8 *r = MatData(mat, (mat + Read16(relative))[0]);
                Write32(m + MAT_TEX_IMAGE_PARAM, (Read32(m + MAT_TEX_IMAGE_PARAM) & TEX_IMAGE_PARAM_KEEP)
                    | (Read32(r + MAT_TEX_IMAGE_PARAM) & ~TEX_IMAGE_PARAM_KEEP));
                Write32(m + MAT_TEX_PLTT_BASE, (Read32(m + MAT_TEX_PLTT_BASE) & 0xFFFF0000)
                    | (Read32(r + MAT_TEX_PLTT_BASE) & 0xFFFF)); // texPlttBase; flag kept
                Write32(m + MAT_SCALE_S, Read32(r + MAT_SCALE_S));
                Write32(m + MAT_SCALE_T, Read32(r + MAT_SCALE_T));
            }
            else
            {
                Write32(m + MAT_POLYGON_ATTR, Read32(m + MAT_POLYGON_ATTR) & ~POLYGON_ATTR_SHOW_FACES);
                Write32(m + MAT_POLYGON_ATTR_MASK, Read32(m + MAT_POLYGON_ATTR_MASK) | POLYGON_ATTR_SHOW_FACES);
            }
        }
        // Released with the block like any binding (NNS_G3dReleaseMdlTex keeps only the
        // material's own bits).
        if (relative != NULL)
            bind[3] |= RES_BIND_BOUND;
    }

    return result;
}
