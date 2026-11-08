//! The avatar choices (the C# bridge's Avatars.cs, docs/CUSTOM_SPRITES_PLAN.md §1):
//! `coop_avatars.json`, written by scripts/avatar_build.py. ID 0 is the save's own look, 1 Ethan,
//! 2 Lyra, 3 + k the custom sheets. The choice is stored by key, so adding a sheet (which shifts
//! the IDs) keeps everyone's pick. Without the file only the vanilla choices are offered.

use std::path::{Path, PathBuf};

pub const FILE: &str = "coop_avatars.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvatarChoice {
    pub id: u8,
    pub key: String,
    pub name: String,
    pub preview: Option<PathBuf>,
}

/// The choices from the first `coop_avatars.json` found in `candidates`.
pub fn load(candidates: &[PathBuf]) -> Vec<AvatarChoice> {
    let mut list = vec![
        AvatarChoice {
            id: 0,
            key: "default".into(),
            name: "Default (by the save)".into(),
            preview: None,
        },
        AvatarChoice {
            id: 1,
            key: "ethan".into(),
            name: "Ethan".into(),
            preview: None,
        },
        AvatarChoice {
            id: 2,
            key: "lyra".into(),
            name: "Lyra".into(),
            preview: None,
        },
    ];
    for path in candidates {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let dir = path.parent().unwrap_or(Path::new("."));
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
            break;
        };
        let items: Vec<&serde_json::Value> = match &v {
            serde_json::Value::Array(a) => a.iter().collect(),
            serde_json::Value::Object(o) => {
                o.values().filter_map(|x| x.as_array()).flatten().collect()
            }
            _ => Vec::new(),
        };
        for item in items {
            let Some(id) = item["id"].as_u64().and_then(|i| u8::try_from(i).ok()) else {
                continue;
            };
            let preview = item["preview"].as_str().map(|p| dir.join(p));
            if id < 3 {
                list[id as usize].preview = preview;
                continue;
            }
            let (Some(file), Some(name)) = (item["file"].as_str(), item["name"].as_str()) else {
                continue;
            };
            if file.is_empty()
                || !file
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            {
                continue;
            }
            list.push(AvatarChoice {
                id,
                key: file.into(),
                name: name.into(),
                preview,
            });
        }
        break;
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vanilla_plus_sheets() {
        let dir = std::env::temp_dir().join(format!("goldlink-avatars-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join(FILE);
        std::fs::write(
            &f,
            r#"{"avatars": [{"id": 1, "preview": "p/ethan.png"}, {"id": 3, "file": "rafael", "name": "Rafael", "preview": "p/rafael.png"}, {"id": 4, "file": "Bad Name", "name": "x"}]}"#,
        )
        .unwrap();
        let l = load(&[dir.join("missing.json"), f]);
        assert_eq!(l.len(), 4);
        assert_eq!(l[1].preview, Some(dir.join("p/ethan.png")));
        assert_eq!(
            (l[3].id, l[3].key.as_str(), l[3].name.as_str()),
            (3, "rafael", "Rafael")
        );
        assert_eq!(load(&[]).len(), 3);
        std::fs::remove_dir_all(&dir).ok();
    }
}
