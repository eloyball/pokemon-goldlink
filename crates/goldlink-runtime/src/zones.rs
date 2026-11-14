//! Which map area each cell of the overworld's map matrix belongs to (UC11, the lighting's
//! neighbouring areas): the matrix's map header per cell, read from the game.
//!
//! The path is the vanilla game's (pret pokeheartgold, include/field_system.h and
//! include/map_matrix.h), checked on the running co-op ROM: the main overlay manager runs the
//! field, whose data is the `FieldSystem`; its `mapMatrix` holds the matrix's width and height
//! and, per cell (row by row), the map header the cell belongs to (Violet City's cells 73, the
//! cell east of it Route 31's 35). A matrix without a header layer has every cell set to the
//! map it was loaded for; cells outside any area are 0. Every pointer is checked to stay in main
//! RAM, so a wrong guess reads nothing rather than garbage.

const MAIN_RAM: u32 = 0x0200_0000;
/// pret `sMainWork`: [1] the running main overlay manager (the field's, in a game).
const MAIN_WORK: u32 = 0x0211_1868;
/// `OverlayManager`'s data: the `FieldSystem`.
const MANAGER_DATA: u32 = 0x1C;
/// `FieldSystem.mapMatrix`.
const FIELD_MAP_MATRIX: u32 = 0x30;
/// `MapMatrix`: width, height (u8 each), then `MapMatrixData`, whose `headers` start at +2.
const MATRIX_WIDTH: u32 = 0x00;
const MATRIX_HEIGHT: u32 = 0x01;
const MATRIX_HEADERS: u32 = 0x06;
/// pret `MAP_MATRIX_MAX_SIZE`.
const MATRIX_MAX_CELLS: usize = 799;

fn u32_at(ram: &[u8], addr: u32) -> Option<u32> {
    let i = addr.checked_sub(MAIN_RAM)? as usize;
    Some(u32::from_le_bytes(ram.get(i..i + 4)?.try_into().ok()?))
}

fn ptr(ram: &[u8], addr: u32) -> Option<u32> {
    u32_at(ram, addr).filter(|&p| p >= MAIN_RAM && ((p - MAIN_RAM) as usize) < ram.len())
}

/// The map matrix's cells' map headers into `out` (row by row), and its width; None outside a
/// game's field or when the matrix doesn't look like one.
pub fn read(ram: &[u8], out: &mut Vec<u16>) -> Option<usize> {
    let manager = ptr(ram, MAIN_WORK + 4)?;
    let field = ptr(ram, manager + MANAGER_DATA)?;
    let matrix = ptr(ram, field + FIELD_MAP_MATRIX)?;
    let at = (matrix - MAIN_RAM) as usize;
    let width = *ram.get(at + MATRIX_WIDTH as usize)? as usize;
    let height = *ram.get(at + MATRIX_HEIGHT as usize)? as usize;
    let cells = width * height;
    if cells == 0 || cells > MATRIX_MAX_CELLS {
        return None;
    }
    let start = at + MATRIX_HEADERS as usize;
    let bytes = ram.get(start..start + cells * 2)?;
    out.clear();
    out.extend(
        bytes
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]])),
    );
    Some(width)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_matrix_through_the_field() {
        let mut ram = vec![0u8; 0x0040_0000];
        let put = |ram: &mut Vec<u8>, addr: u32, v: u32| {
            let i = (addr - MAIN_RAM) as usize;
            ram[i..i + 4].copy_from_slice(&v.to_le_bytes());
        };
        let (manager, field, matrix) = (0x0226_F210, 0x022A_C1D8, 0x022A_C350);
        put(&mut ram, MAIN_WORK + 4, manager);
        put(&mut ram, manager + MANAGER_DATA, field);
        put(&mut ram, field + FIELD_MAP_MATRIX, matrix);
        let m = (matrix - MAIN_RAM) as usize;
        ram[m] = 3;
        ram[m + 1] = 2;
        for (k, h) in [0u16, 73, 35, 0, 73, 36].iter().enumerate() {
            ram[m + 6 + 2 * k..m + 8 + 2 * k].copy_from_slice(&h.to_le_bytes());
        }
        let mut cells = Vec::new();
        assert_eq!(read(&ram, &mut cells), Some(3));
        assert_eq!(cells, vec![0, 73, 35, 0, 73, 36]);
        // A pointer out of main RAM reads nothing.
        put(&mut ram, field + FIELD_MAP_MATRIX, 0x0100_0000);
        assert_eq!(read(&ram, &mut cells), None);
    }
}
