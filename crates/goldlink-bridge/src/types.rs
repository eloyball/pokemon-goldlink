//! Type matchups for the battle HUD's "Super effective" / "Not very effective" under each move
//! (the user's v0.7.1 play test). The rows are hg-engine's `TypeEffectivenessTable`
//! (src/battle/battle_pokemon.c) as this ROM builds it (config.h: Fairy, Gen 6+ matchups), its
//! Foresight and Ring Target markers left out: (move type, defending type, multiplier x10).
//! Abilities, items and field effects aren't counted (Levitate, Scrappy, a Ring Target).

const TABLE: [(u8, u8, u8); 120] = [
    (0, 5, 5),
    (0, 8, 5),
    (1, 0, 20),
    (1, 2, 5),
    (1, 3, 5),
    (1, 5, 20),
    (1, 6, 5),
    (1, 8, 20),
    (1, 9, 5),
    (1, 14, 5),
    (1, 15, 20),
    (1, 17, 20),
    (2, 1, 20),
    (2, 5, 5),
    (2, 6, 20),
    (2, 8, 5),
    (2, 12, 20),
    (2, 13, 5),
    (3, 3, 5),
    (3, 4, 5),
    (3, 5, 5),
    (3, 7, 5),
    (3, 9, 20),
    (3, 12, 20),
    (4, 3, 20),
    (4, 5, 20),
    (4, 6, 5),
    (4, 8, 20),
    (4, 10, 20),
    (4, 12, 5),
    (4, 13, 20),
    (5, 1, 5),
    (5, 2, 20),
    (5, 4, 5),
    (5, 6, 20),
    (5, 8, 5),
    (5, 10, 20),
    (5, 15, 20),
    (6, 1, 5),
    (6, 2, 5),
    (6, 3, 5),
    (6, 7, 5),
    (6, 8, 5),
    (6, 9, 5),
    (6, 10, 5),
    (6, 12, 20),
    (6, 14, 20),
    (6, 17, 20),
    (7, 7, 20),
    (7, 14, 20),
    (7, 17, 5),
    (8, 5, 20),
    (8, 8, 5),
    (8, 9, 20),
    (8, 10, 5),
    (8, 11, 5),
    (8, 13, 5),
    (8, 15, 20),
    (9, 1, 20),
    (9, 3, 5),
    (9, 8, 5),
    (9, 10, 5),
    (9, 16, 20),
    (9, 17, 20),
    (10, 5, 5),
    (10, 6, 20),
    (10, 8, 20),
    (10, 10, 5),
    (10, 11, 5),
    (10, 12, 20),
    (10, 15, 20),
    (10, 16, 5),
    (11, 4, 20),
    (11, 5, 20),
    (11, 10, 20),
    (11, 11, 5),
    (11, 12, 5),
    (11, 16, 5),
    (12, 2, 5),
    (12, 3, 5),
    (12, 4, 20),
    (12, 5, 20),
    (12, 6, 5),
    (12, 8, 5),
    (12, 10, 5),
    (12, 11, 20),
    (12, 12, 5),
    (12, 16, 5),
    (13, 2, 20),
    (13, 11, 20),
    (13, 12, 5),
    (13, 13, 5),
    (13, 16, 5),
    (14, 1, 20),
    (14, 3, 20),
    (14, 8, 5),
    (14, 14, 5),
    (15, 2, 20),
    (15, 4, 20),
    (15, 8, 5),
    (15, 10, 5),
    (15, 11, 5),
    (15, 12, 20),
    (15, 15, 5),
    (15, 16, 20),
    (16, 8, 5),
    (16, 16, 20),
    (17, 1, 5),
    (17, 7, 20),
    (17, 9, 5),
    (17, 14, 20),
    (17, 17, 5),
    (3, 8, 0),
    (4, 2, 0),
    (7, 0, 0),
    (13, 4, 0),
    (14, 17, 0),
    (16, 9, 0),
    (0, 7, 0),
    (1, 7, 0),
];

/// How a move of `move_type` hits a Pokémon of `types` (its two; the same twice for one type),
/// by the types alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    None,
    NotVery,
    Normal,
    Super,
}

pub fn effect(move_type: u8, types: [u8; 2]) -> Effect {
    // In tenths, as the game multiplies.
    let mut m: u32 = 100;
    let defending: &[u8] = if types[0] == types[1] {
        &types[..1]
    } else {
        &types
    };
    for &(a, d, mul) in &TABLE {
        if a == move_type && defending.contains(&d) {
            m = m * mul as u32 / 10;
        }
    }
    match m {
        0 => Effect::None,
        1..=99 => Effect::NotVery,
        100 => Effect::Normal,
        _ => Effect::Super,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // hg-engine's numbers.
    const NORMAL: u8 = 0;
    const FIGHTING: u8 = 1;
    const FLYING: u8 = 2;
    const GROUND: u8 = 4;
    const GHOST: u8 = 7;
    const STEEL: u8 = 8;
    const FAIRY: u8 = 9;
    const FIRE: u8 = 10;
    const WATER: u8 = 11;
    const GRASS: u8 = 12;
    const ELECTRIC: u8 = 13;
    const PSYCHIC: u8 = 14;
    const DRAGON: u8 = 16;
    const DARK: u8 = 17;

    #[test]
    fn matchups() {
        assert_eq!(effect(WATER, [FIRE, FIRE]), Effect::Super);
        assert_eq!(effect(FIRE, [WATER, WATER]), Effect::NotVery);
        assert_eq!(effect(NORMAL, [GHOST, GHOST]), Effect::None);
        assert_eq!(effect(ELECTRIC, [WATER, GROUND]), Effect::None);
        assert_eq!(effect(DRAGON, [FAIRY, FAIRY]), Effect::None);
        // Gen 6+: Steel no longer resists Ghost or Dark.
        assert_eq!(effect(GHOST, [STEEL, PSYCHIC]), Effect::Super);
        assert_eq!(effect(DARK, [STEEL, STEEL]), Effect::Normal);
        // Both types: Grass on Water/Ground is 4x, Fighting on Flying/Steel cancels.
        assert_eq!(effect(GRASS, [WATER, GROUND]), Effect::Super);
        assert_eq!(effect(FIGHTING, [FLYING, STEEL]), Effect::Normal);
    }
}
