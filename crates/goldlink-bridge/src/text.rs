//! What the bridge tells the player, word for word the C# bridge's (CoopBridgeForm.cs) so both
//! clients read the same.

use crate::layout::*;
use goldlink_proto as wire;

pub const DEFAULT_MESSAGE: &str = "Enter a world code to join your friends, or create a new world.";

/// The relay turned the connection down.
pub fn server_status(code: u8, room: &str, server_protocol: u16) -> String {
    match code {
        wire::ROOM_STATUS_ERR_PROTOCOL => protocol_mismatch(server_protocol),
        1 => format!("World {room} doesn't exist. Check the code, or create it instead."),
        2 => {
            format!("World {room} already exists. Join it, or pick another code to create a world.")
        }
        3 => format!(
            "World {room} is locked to its original players (or this trainer is registered to another install)."
        ),
        4 => format!("World {room} is full, or the server is at capacity."),
        5 => "World codes are 6 letters or numbers.".into(),
        wire::ROOM_STATUS_ERR_MIDGAME_SAVE => format!(
            "World {room} hasn't started yet, so it only takes a New Game (this save is already under way)."
        ),
        wire::ROOM_STATUS_ERR_SAVE_OTHER_ROOM => format!(
            "This save belongs to another world. Load that world's save, or start a New Game for {room}."
        ),
        wire::ROOM_STATUS_ERR_RATE_LIMITED => {
            "Too many failed attempts. Wait a few seconds and try again.".into()
        }
        wire::ROOM_STATUS_ERR_RUN_LOST => format!(
            "The run in world {room} is over: every player's Pokémon have fallen. Create a new world for a new run."
        ),
        _ => format!("The server rejected the connection (code {code})."),
    }
}

/// The relay and this release speak different protocol versions: which one is behind decides
/// who updates.
pub fn protocol_mismatch(server: u16) -> String {
    let ours = wire::PROTOCOL_VERSION;
    if server > ours {
        format!(
            "This server runs relay protocol {server}, newer than this GoldLink release (protocol {ours}). Update GoldLink to the release your group uses."
        )
    } else {
        format!(
            "This server runs relay protocol {server}, older than this GoldLink release (protocol {ours}). The server needs a coop-relay release for protocol {ours}: ask whoever runs it to update."
        )
    }
}

pub fn no_answer() -> String {
    format!(
        "The server didn't answer. Check the address and that its UDP port is open; it must also run a relay for protocol {}.",
        wire::PROTOCOL_VERSION
    )
}

/// Player name from a player state (HG charset: digits, letters, space).
pub fn decode_name(state: &[u8]) -> String {
    let mut s = String::new();
    for i in 0..8 {
        let ch = u16_at(state, PS_NAME + i * 2);
        if ch == 0xFFFF || ch == 0 {
            break;
        }
        s.push(match ch {
            0x121..=0x12A => (b'0' + (ch - 0x121) as u8) as char,
            0x12B..=0x144 => (b'A' + (ch - 0x12B) as u8) as char,
            0x145..=0x15E => (b'a' + (ch - 0x145) as u8) as char,
            0x1DE => ' ',
            _ => '?',
        });
    }
    if s.is_empty() { "Trainer".into() } else { s }
}

pub fn player_state_name(state: u8) -> String {
    match state {
        0 => "walking".into(),
        1 => "cycling".into(),
        2 => "surfing".into(),
        3 => "in Team Rocket disguise".into(),
        4 => "using a field move".into(),
        7 => "fishing".into(),
        9 => "saving".into(),
        s => format!("state {s}"),
    }
}

/// Where a player is and what they're doing.
pub fn describe_player(state: &[u8]) -> String {
    let map = u16_at(state, PS_MAP_ID);
    let x = u16_at(state, PS_X) as i16;
    let z = u16_at(state, PS_Z) as i16;
    let at = format!("map {map} ({x}, {z})");
    match state[PS_ACTIVITY] {
        2 => return format!("in a battle · {at}"),
        3 => return format!("in a menu · {at}"),
        4 => return format!("changing maps · {at}"),
        5 => return format!("busy (talking, a sign or a cutscene) · {at}"),
        _ => {}
    }
    if state[PS_IS_ACTIVE] != 1 {
        return format!("away · {at}");
    }
    let mut text = format!("{at} · {}", player_state_name(state[PS_AVATAR_STATE]));
    let follower = u16_at(state, PS_FOLLOWER_SPECIES);
    if state[PS_FOLLOWER_VISIBLE] == 1 && follower != 0 {
        text += &format!(" · follower #{follower}");
    }
    text
}

pub const RANDOM_MODES: [&str; 4] = ["None", "Semi-random", "Preserve difficulty", "Fully random"];
pub const SOUL_LINK_MODES: [&str; 3] = ["Off", "World-wide", "Duo"];
/// hg-engine's type order (Fairy took the "???" slot 9).
pub const TYPE_NAMES: [&str; 18] = [
    "Normal", "Fighting", "Flying", "Poison", "Ground", "Rock", "Bug", "Ghost", "Steel", "Fairy",
    "Fire", "Water", "Grass", "Electric", "Psychic", "Ice", "Dragon", "Dark",
];

/// A world's rules in words.
pub fn describe_rules(rules: u16, mono_type: u8) -> String {
    let mut parts = Vec::new();
    let random = (rules & wire::CHALLENGE_RULE_RANDOM_MASK) as usize;
    if random != 0 {
        let mut r = format!("Random encounters: {}", RANDOM_MODES[random]);
        if rules & wire::CHALLENGE_RULE_LEGENDS_WILD != 0 {
            r += ", legends in the wild";
        }
        if rules & wire::CHALLENGE_RULE_RANDOM_ABILITIES != 0 {
            r += ", random abilities";
        }
        parts.push(r);
    }
    if rules & wire::CHALLENGE_RULE_NUZLOCKE != 0 {
        let soul = wire::soul_link_mode(rules) as usize;
        parts.push(if soul == 0 {
            "Nuzlocke".into()
        } else {
            format!(
                "Nuzlocke, Soul-Link {}",
                SOUL_LINK_MODES[soul.min(2)].to_lowercase()
            )
        });
    }
    if rules & wire::CHALLENGE_RULE_MONOLOCKE != 0 {
        parts.push(match TYPE_NAMES.get(mono_type as usize) {
            Some(t) => format!("Monolocke ({t})"),
            None => "Monolocke".into(),
        });
    }
    if rules & wire::CHALLENGE_RULE_LEVEL_CAP != 0 {
        parts.push("Level cap".into());
    }
    if rules & wire::CHALLENGE_RULE_RANDOM_ITEMS != 0 {
        parts.push("Random items".into());
    }
    if parts.is_empty() {
        "None".into()
    } else {
        parts.join("; ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_rules() {
        let mut st = [0u8; PLAYER_STATE_SIZE];
        for (i, ch) in [
            0x12Bu16 + 4,
            0x145 + 19,
            0x145 + 7,
            0x145,
            0x145 + 13,
            0x1DE,
            0x121 + 2,
        ]
        .iter()
        .enumerate()
        {
            st[PS_NAME + i * 2..PS_NAME + i * 2 + 2].copy_from_slice(&ch.to_le_bytes());
        }
        assert_eq!(decode_name(&st), "Ethan 2");
        assert_eq!(decode_name(&[0u8; 64]), "Trainer");
        assert_eq!(describe_rules(0, 18), "None");
        assert_eq!(
            describe_rules(0x0010 | (1 << 5) | 0x0080 | 0x0200 | 1 | 4, 9),
            "Random encounters: Semi-random, legends in the wild; Nuzlocke, Soul-Link world-wide; Monolocke (Fairy); Random items"
        );
        assert!(protocol_mismatch(7).contains("older"));
        assert!(protocol_mismatch(9).contains("newer"));
    }
}
