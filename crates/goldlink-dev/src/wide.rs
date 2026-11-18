//! DEVELOPMENT ONLY (UC16's emulator pass, dev/docs/unified-client/UC16_WIDE_NEIGHBOURS.md):
//! plays each case `dev/tools/wide_scan.py` found (a scene, a border) and logs where the game's
//! people are, every frame, for `dev/tools/wide_verdicts.py` to judge what the widened screen's
//! strips show.
//!
//! GOLDLINK_DEV_TOUR_ROUTE=wide, GOLDLINK_DEV_TOUR_WIDE_CASES=<cases.json> (a list of
//! [`Case`]). For each case: the story set (flags and variables, written into the save's
//! SaveVarsFlags), the warp (debug command 5, preset COOP_WARP_FROM_BUFFER: the map, place and
//! facing from gCoopDebugMonBuffer), the keys (walking onto a trigger, across a border), A to talk
//! if asked, then A every half second while a script runs (and through a battle, FIGHT and the
//! first move), until the field has been free for two seconds after the case's frames. The log,
//! `<tour dir>/wide/<id>.log`:
//!
//! - `C <frame> <x> <z>`: the camera's target (pret Camera.lookAt.camTarget, FieldSystem.camera
//!   + 0x20), in pixels (fx32 world units >> 12), when it changes;
//! - `O <frame> <slot> <id> <x> <z> <shown> <cmd> <facing> <sprite> <map>`: a person (pret
//!   LocalMapObject, the MapObjectManager's array) when anything of theirs changes; `shown` 0
//!   when inactive or hidden (MAPOBJECTFLAG_VISIBLE, the vanish bit 9);
//! - `S <frame> <scene>` the game's scene, `K <frame> <what>` the case's steps.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use goldlink::goldlink_bridge::layout::*;
use goldlink::goldlink_bridge::{Scene, scene_of};
use goldlink_abi::CoopDebugCommand as C;
use goldlink_core::Keys;
use goldlink_runtime::{Emulator, Input};

/// pret include/constants/vars.h: NUM_VARS; SaveVarsFlags = u16 vars[NUM_VARS], u8 flags[].
const NUM_VARS: usize = 0x170;
const VAR_BASE: u16 = 0x4000;
/// pret include/field_system.h: FieldSystem.camera, .mapObjectManager (checked against
/// hg-engine's own struct: offsetof 0x3C).
const FSYS_CAMERA: u32 = 0x24;
const FSYS_OBJECTS: u32 = 0x3C;
/// pret include/camera.h: Camera.lookAt.camTarget (after CameraPerspective, 20 bytes, and
/// camPos).
const CAMERA_TARGET: u32 = 0x20;
/// pret include/map_object.h: MapObjectManager.objectCount, .objects; LocalMapObject's size and
/// fields.
const MOM_COUNT: u32 = 0x04;
const MOM_OBJECTS: u32 = 0x124;
const MO_SIZE: u32 = 0x12C;
const MO_FLAGS: u32 = 0x00;
const MO_ID: u32 = 0x08;
const MO_MAP: u32 = 0x0C;
const MO_SPRITE: u32 = 0x10;
const MO_FACING: u32 = 0x28;
const MO_POS: u32 = 0x70;
const MO_CMD: u32 = 0xA4;
const FLAG_ACTIVE: u32 = 1 << 0;
const FLAG_VANISH: u32 = 1 << 9;
/// hg-engine include/coop_sync.h COOP_WARP_FROM_BUFFER.
const WARP_FROM_BUFFER: u8 = 0xFF;

#[derive(Debug, Clone, Default)]
pub struct Case {
    pub id: String,
    pub map: u16,
    pub x: u16,
    pub z: u16,
    pub dir: u16,
    pub flags_set: Vec<u16>,
    pub flags_clear: Vec<u16>,
    pub vars: Vec<(u16, u16)>,
    /// After arriving: ("up" | "down" | "left" | "right", frames held), in order.
    pub keys: Vec<(String, u32)>,
    /// A once the keys are done (a person to talk to, faced).
    pub talk: bool,
    /// Frames to watch at least, from the warp.
    pub watch: u32,
    /// Other places to start from when the first key doesn't move the player (a border case:
    /// which tiles can be stood on isn't known from pret).
    pub alts: Vec<(u16, u16)>,
}

impl Case {
    fn from_json(v: &serde_json::Value) -> Option<Self> {
        let n = |k: &str| v[k].as_u64().map(|x| x as u16);
        let list = |k: &str| -> Vec<u16> {
            v[k].as_array()
                .map(|a| a.iter().filter_map(|x| x.as_u64().map(|x| x as u16)).collect())
                .unwrap_or_default()
        };
        Some(Self {
            id: v["id"].as_str()?.to_owned(),
            map: n("map")?,
            x: n("x")?,
            z: n("z")?,
            dir: n("dir").unwrap_or(0),
            flags_set: list("flags_set"),
            flags_clear: list("flags_clear"),
            vars: v["vars"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|p| Some((p[0].as_u64()? as u16, p[1].as_u64()? as u16)))
                        .collect()
                })
                .unwrap_or_default(),
            keys: v["keys"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|p| Some((p[0].as_str()?.to_owned(), p[1].as_u64()? as u32)))
                        .collect()
                })
                .unwrap_or_default(),
            talk: v["talk"].as_bool().unwrap_or(false),
            watch: v["watch"].as_u64().unwrap_or(600) as u32,
            alts: v["alts"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|p| Some((p[0].as_u64()? as u16, p[1].as_u64()? as u16)))
                        .collect()
                })
                .unwrap_or_default(),
        })
    }
}

#[derive(Debug, Default)]
enum Phase {
    #[default]
    Next,
    /// Waiting for the debug command's answer: stage 0 the pointers (then the warp), 1 the warp
    /// to the case's place (then Settle).
    Ack(u8, u8, u32),
    /// Arrived, without the case's story: once the field is free, the story is written, the log
    /// starts and the same warp again loads the map with it (a scene that starts at once, on the
    /// spot, is logged too).
    Settle(u32),
    Drive,
    Done,
}

#[derive(Default)]
pub struct Pass {
    cases: Vec<Case>,
    index: usize,
    phase: Phase,
    frame: u32,
    key: usize,
    key_frames: u32,
    talked: bool,
    idle: u32,
    seq: u8,
    vars_flags: u32,
    log: Option<std::io::BufWriter<std::fs::File>>,
    last: Vec<Option<[i32; 8]>>,
    camera: Option<(i32, i32)>,
    scene: Option<Scene>,
    /// The second warp's sequence number until it's answered, and the frame it was.
    pending: Option<u8>,
    warped_at: u32,
    /// The player's tile (from the people's log), where the first key started, and which
    /// alternative start is in use.
    player: Option<(i32, i32)>,
    key_from: Option<(i32, i32)>,
    alt: usize,
}

fn u32_at(ram: &[u8], addr: u32) -> Option<u32> {
    let off = addr.checked_sub(0x0200_0000)? as usize;
    Some(u32::from_le_bytes(ram.get(off..off + 4)?.try_into().ok()?))
}

fn in_ram(addr: u32) -> bool {
    (0x0200_0000..0x0240_0000).contains(&addr)
}

fn key_of(name: &str) -> Keys {
    match name {
        "up" => Keys::UP,
        "down" => Keys::DOWN,
        "left" => Keys::LEFT,
        "right" => Keys::RIGHT,
        "b" => Keys::B,
        _ => Keys::NONE,
    }
}

impl Pass {
    pub fn load() -> Self {
        let cases: Vec<Case> = std::env::var_os("GOLDLINK_DEV_TOUR_WIDE_CASES")
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(Case::from_json)
            .collect();
        eprintln!("tour: wide: {} cases", cases.len());
        Self {
            cases,
            ..Self::default()
        }
    }

    fn line(&mut self, text: std::fmt::Arguments) {
        if let Some(w) = self.log.as_mut() {
            let _ = w.write_fmt(text);
            let _ = w.write_all(b"\n");
        }
    }

    fn debug(&mut self, emu: &mut Emulator, buffer: u32, words: [u16; 4]) {
        let at = (buffer - 0x0200_0000) as usize;
        let ram = emu.nds.main_ram_mut();
        for (i, w) in words.iter().enumerate() {
            ram[at + 2 * i..at + 2 * i + 2].copy_from_slice(&w.to_le_bytes());
        }
        self.seq = self.seq.wrapping_add(1).max(1);
        let m = emu.mailbox_offset();
        let ram = emu.nds.main_ram_mut();
        ram[m + DEBUG_RESULT] = 0;
        ram[m + DEBUG_CMD] = C::COOP_DEBUG_WARP as u8;
        ram[m + DEBUG_ARG] = WARP_FROM_BUFFER;
        ram[m + DEBUG_SEQ] = self.seq;
    }

    fn acked(&self, emu: &Emulator) -> Option<u8> {
        let m = emu.mailbox_offset();
        let ack = m + DIAG + std::mem::offset_of!(goldlink_abi::CoopDiagnostics, debug_ack);
        let ram = emu.nds.main_ram();
        (ram[ack] == self.seq).then(|| ram[m + DEBUG_RESULT])
    }

    /// The people and the camera, as lines when they change.
    fn watch(&mut self, emu: &Emulator, fsys_ptr: u32) {
        let ram = emu.nds.main_ram();
        let Some(fsys) = u32_at(ram, fsys_ptr).filter(|&f| in_ram(f)) else {
            return;
        };
        let f = self.frame;
        if let Some(cam) = u32_at(ram, fsys + FSYS_CAMERA).filter(|&c| in_ram(c)) {
            let x = u32_at(ram, cam + CAMERA_TARGET).unwrap_or(0) as i32 >> 12;
            let z = u32_at(ram, cam + CAMERA_TARGET + 8).unwrap_or(0) as i32 >> 12;
            if self.camera != Some((x, z)) {
                self.camera = Some((x, z));
                self.line(format_args!("C {f} {x} {z}"));
            }
        }
        let Some(mom) = u32_at(ram, fsys + FSYS_OBJECTS).filter(|&m| in_ram(m)) else {
            return;
        };
        let count = u32_at(ram, mom + MOM_COUNT).unwrap_or(0).min(128);
        let Some(objects) = u32_at(ram, mom + MOM_OBJECTS).filter(|&o| in_ram(o)) else {
            return;
        };
        if self.last.len() < count as usize {
            self.last.resize(count as usize, None);
        }
        for slot in 0..count {
            let o = objects + slot * MO_SIZE;
            let flags = u32_at(ram, o + MO_FLAGS).unwrap_or(0);
            let now = if flags & FLAG_ACTIVE == 0 {
                None
            } else {
                Some([
                    u32_at(ram, o + MO_ID).unwrap_or(0) as i32,
                    u32_at(ram, o + MO_POS).unwrap_or(0) as i32 >> 12,
                    u32_at(ram, o + MO_POS + 8).unwrap_or(0) as i32 >> 12,
                    (flags & FLAG_VANISH == 0) as i32,
                    u32_at(ram, o + MO_CMD).unwrap_or(0) as i32,
                    u32_at(ram, o + MO_FACING).unwrap_or(0) as i32,
                    u32_at(ram, o + MO_SPRITE).unwrap_or(0) as i32,
                    u32_at(ram, o + MO_MAP).unwrap_or(0) as i32,
                ])
            };
            if self.last[slot as usize] != now {
                match now {
                    Some([id, x, z, shown, cmd, facing, sprite, map]) => {
                        if id == 255 {
                            self.player = Some((x >> 4, z >> 4));
                        }
                        self.line(format_args!(
                            "O {f} {slot} {id} {x} {z} {shown} {cmd} {facing} {sprite} {map}"
                        ))
                    }
                    None => {
                        if let Some(p) = self.last[slot as usize] {
                            self.line(format_args!(
                                "O {f} {slot} {} {} {} 0 -1 {} {} {}",
                                p[0], p[1], p[2], p[5], p[6], p[7]
                            ));
                        }
                    }
                }
                self.last[slot as usize] = now;
            }
        }
    }

    fn write_vars(&self, emu: &mut Emulator, case: &Case) {
        if !in_ram(self.vars_flags) {
            return;
        }
        let base = (self.vars_flags - 0x0200_0000) as usize;
        let ram = emu.nds.main_ram_mut();
        for &(var, value) in &case.vars {
            if let Some(i) = var.checked_sub(VAR_BASE).map(usize::from)
                && i < NUM_VARS
            {
                ram[base + 2 * i..base + 2 * i + 2].copy_from_slice(&value.to_le_bytes());
            }
        }
    }

    /// The case's variables and flags as the save holds them now.
    fn story(&self, emu: &Emulator, case: &Case) -> String {
        if !in_ram(self.vars_flags) {
            return "?".into();
        }
        let base = (self.vars_flags - 0x0200_0000) as usize;
        let ram = emu.nds.main_ram();
        let mut out = Vec::new();
        for &(var, _) in &case.vars {
            let i = (var - VAR_BASE) as usize;
            out.push(format!("{var:#x}={}", u16::from_le_bytes([ram[base + 2 * i], ram[base + 2 * i + 1]])));
        }
        for &flag in case.flags_clear.iter().chain(&case.flags_set) {
            let b = ram[base + 2 * NUM_VARS + flag as usize / 8] >> (flag % 8) & 1;
            out.push(format!("{flag:#x}:{b}"));
        }
        out.join(" ")
    }

    /// One frame; true once every case is done.
    pub fn frame(
        &mut self,
        emu: &mut Emulator,
        input: &mut Input,
        dir: &Path,
        buffer: u32,
        fsys_ptr: u32,
        field_busy: bool,
    ) -> bool {
        self.frame += 1;
        let m = emu.mailbox_offset();
        let scene = if emu.mailbox().is_some() {
            let ram = emu.nds.main_ram();
            scene_of(&ram[m..m + MAILBOX_SIZE])
        } else {
            Scene::Unknown
        };
        if self.scene != Some(scene) && self.log.is_some() {
            self.scene = Some(scene);
            let f = self.frame;
            self.line(format_args!("S {f} {scene:?}"));
        }
        match std::mem::take(&mut self.phase) {
            Phase::Next => {
                let Some(case) = self.cases.get(self.index).cloned() else {
                    if let Some(mut w) = self.log.take() {
                        let _ = w.flush();
                    }
                    self.phase = Phase::Done;
                    return true;
                };
                if let Some(mut w) = self.log.take() {
                    let _ = w.flush();
                }
                let out: PathBuf = dir.join("wide");
                let _ = std::fs::create_dir_all(&out);
                self.log = std::fs::File::create(out.join(format!("{}.log", case.id)))
                    .ok()
                    .map(std::io::BufWriter::new);
                self.frame = 0;
                self.key = 0;
                self.key_frames = 0;
                self.talked = false;
                self.idle = 0;
                self.last.clear();
                self.camera = None;
                self.scene = None;
                self.player = None;
                self.key_from = None;
                self.alt = 0;
                eprintln!("tour: wide {} ({}/{})", case.id, self.index + 1, self.cases.len());
                // First the pointers (no warp), then the story, then the warp.
                self.debug(emu, buffer, [0, 0, 0, 0]);
                self.phase = Phase::Ack(self.seq, 0, 0);
            }
            Phase::Ack(seq, stage, n) => match self.acked(emu) {
                Some(result) if seq == self.seq => {
                    let case = self.cases[self.index].clone();
                    if stage == 0 {
                        let at = (buffer - 0x0200_0000) as usize + 8;
                        let ram = emu.nds.main_ram();
                        self.vars_flags =
                            u32::from_le_bytes(ram[at..at + 4].try_into().unwrap_or_default());
                        self.debug(emu, buffer, [case.map, case.x, case.z, case.dir]);
                        self.phase = Phase::Ack(self.seq, 1, 0);
                    } else {
                        self.line(format_args!("K 0 first warp result {result}"));
                        self.phase = Phase::Settle(0);
                    }
                }
                _ if n > 3600 => {
                    eprintln!("tour: wide {}: no answer", self.cases[self.index].id);
                    self.line(format_args!("K 0 no answer"));
                    self.index += 1;
                    self.phase = Phase::Next;
                }
                _ => {
                    // A scene the save starts on its own (a door's): through it.
                    if field_busy && n % 30 < 3 {
                        input.keys |= Keys::A;
                    }
                    self.phase = Phase::Ack(seq, stage, n + 1)
                }
            },
            Phase::Settle(n) => {
                let case = self.cases[self.index].clone();
                if n < 90 || scene != Scene::Overworld || field_busy {
                    if field_busy && n % 30 < 3 {
                        input.keys |= Keys::A;
                    }
                    self.phase = Phase::Settle(n + 1);
                    return false;
                }
                // The flags now (the people the map creates on the warp's load), the variables
                // once the warp is under way (a scene table's script would start on the spot).
                let vf = self.vars_flags;
                if in_ram(vf) {
                    let base = (vf - 0x0200_0000) as usize;
                    let ram = emu.nds.main_ram_mut();
                    let flags = base + 2 * NUM_VARS;
                    for &flag in &case.flags_set {
                        ram[flags + flag as usize / 8] |= 1 << (flag % 8);
                    }
                    for &flag in &case.flags_clear {
                        ram[flags + flag as usize / 8] &= !(1 << (flag % 8));
                    }
                }
                self.frame = 0;
                let story = self.story(emu, &case);
                self.line(format_args!("K 0 story at {vf:#010x}: {story}"));
                self.debug(emu, buffer, [case.map, case.x, case.z, case.dir]);
                self.pending = Some(self.seq);
                self.warped_at = 0;
                self.phase = Phase::Drive;
            }
            Phase::Drive => {
                let case = self.cases[self.index].clone();
                self.watch(emu, fsys_ptr);
                let f = self.frame;
                if let Some(seq) = self.pending {
                    if let Some(result) = self.acked(emu).filter(|_| seq == self.seq) {
                        self.write_vars(emu, &case);
                        self.line(format_args!("K {f} warp result {result}"));
                        self.pending = None;
                        self.warped_at = f;
                    } else if f > 3600 {
                        self.line(format_args!("K {f} no answer to the warp"));
                        self.pending = None;
                        self.warped_at = f;
                    }
                }
                if f == self.warped_at + 200 {
                    let story = self.story(emu, &case);
                    self.line(format_args!("K {f} story now {story}"));
                }
                // The warp's fade and the map's load first.
                let settled =
                    self.pending.is_none() && f > self.warped_at + 90 && scene == Scene::Overworld;
                if scene == Scene::Battle {
                    // Through it: A on the menus and messages (FIGHT, the first move).
                    if f % 40 < 3 {
                        input.keys |= Keys::A;
                    }
                    self.idle = 0;
                } else if field_busy {
                    self.idle = 0;
                    if f % 30 < 3 {
                        input.keys |= Keys::A;
                    }
                } else if settled {
                    // The first key didn't move the player: the next start, if any.
                    if self.key == 1 && self.key_frames == 0 && self.key_from.is_some()
                        && self.key_from == self.player
                        && self.alt < case.alts.len()
                    {
                        let (x, z) = case.alts[self.alt];
                        self.alt += 1;
                        self.line(format_args!("K {f} blocked; next start {x},{z}"));
                        self.debug(emu, buffer, [case.map, x, z, case.dir]);
                        self.pending = Some(self.seq);
                        self.key = 0;
                        self.key_from = None;
                        self.phase = Phase::Drive;
                        return false;
                    }
                    if let Some((name, frames)) = case.keys.get(self.key) {
                        if self.key_frames == 0 {
                            if self.key == 0 {
                                self.key_from = self.player;
                            }
                            self.line(format_args!("K {f} keys {name} {frames}"));
                        }
                        input.keys |= key_of(name);
                        self.key_frames += 1;
                        if self.key_frames >= *frames {
                            self.key += 1;
                            self.key_frames = 0;
                        }
                        self.idle = 0;
                    } else if case.talk && !self.talked {
                        self.line(format_args!("K {f} talk"));
                        input.keys |= Keys::A;
                        self.talked = true;
                        self.idle = 0;
                    } else {
                        self.idle += 1;
                    }
                }
                let done = (self.pending.is_none() && f >= self.warped_at + case.watch && self.idle >= 120)
                    || f >= case.watch * 4 + 3600;
                if done {
                    self.line(format_args!("K {f} end"));
                    self.index += 1;
                    self.phase = Phase::Next;
                } else {
                    self.phase = Phase::Drive;
                }
            }
            Phase::Done => {
                if let Some(mut w) = self.log.take() {
                    let _ = w.flush();
                }
                self.phase = Phase::Done;
                return true;
            }
        }
        false
    }
}
