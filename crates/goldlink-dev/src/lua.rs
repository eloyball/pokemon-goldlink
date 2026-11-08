//! BizHawk's Lua API, the subset the probes use (dev/docs/unified-client/LUA_SHIM_SPEC.md §4), so
//! `dev/bizhawk/*.lua` run unchanged on the new core.
//!
//! The script runs as a coroutine: `emu.frameadvance()` yields, the runner emulates one frame,
//! then resumes it. Code between two frameadvance calls runs between frames, as in BizHawk.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::rc::Rc;

use goldlink_core::Keys;
use mlua::{Lua, Table, ThreadStatus, Value};

use crate::machine::{Machine, TOUCH_CENTRE};

pub struct Shared {
    pub machine: Machine,
    pub exit: bool,
    warned: HashSet<String>,
    /// memorysavestate's states, by id.
    states: HashMap<String, Vec<u8>>,
    next_state: u64,
}

pub type State = Rc<RefCell<Shared>>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Domain {
    MainRam,
    Bus,
    Sram,
}

fn domain(name: Option<String>) -> mlua::Result<Domain> {
    match name.as_deref() {
        None | Some("Main RAM") => Ok(Domain::MainRam),
        Some("ARM9 System Bus") => Ok(Domain::Bus),
        Some("SRAM") => Ok(Domain::Sram),
        Some(other) => Err(mlua::Error::runtime(format!(
            "memory domain \"{other}\" is not provided by the goldlink dev shim"
        ))),
    }
}

impl Shared {
    /// BizHawk logs a warning and reads 0 outside a domain (LUA_SHIM_SPEC §4.1); once per kind.
    fn warn_once(&mut self, key: String, text: impl FnOnce() -> String) {
        if self.warned.insert(key) {
            eprintln!("warning: {}", text());
        }
    }

    fn read(&mut self, dom: Domain, addr: i64) -> u8 {
        let byte = match dom {
            Domain::MainRam => usize::try_from(addr)
                .ok()
                .and_then(|a| self.machine.nds.main_ram().get(a).copied()),
            Domain::Sram => usize::try_from(addr)
                .ok()
                .and_then(|a| self.machine.nds.save().get(a).copied()),
            Domain::Bus => u32::try_from(addr)
                .ok()
                .map(|a| self.machine.nds.arm9_peek8(a)),
        };
        byte.unwrap_or_else(|| {
            self.warn_once(format!("r{dom:?}"), || {
                format!("read at 0x{addr:X} outside {dom:?}: 0")
            });
            0
        })
    }

    fn write(&mut self, dom: Domain, addr: i64, value: u8) -> mlua::Result<()> {
        let slot = match dom {
            Domain::MainRam => usize::try_from(addr)
                .ok()
                .and_then(|a| self.machine.nds.main_ram_mut().get_mut(a)),
            Domain::Sram => usize::try_from(addr)
                .ok()
                .and_then(|a| self.machine.nds.save_mut().get_mut(a)),
            Domain::Bus => {
                return Err(mlua::Error::runtime(
                    "writes to \"ARM9 System Bus\" are not provided by the goldlink dev shim",
                ));
            }
        };
        match slot {
            Some(b) => *b = value,
            None => self.warn_once(format!("w{dom:?}"), || {
                format!("write at 0x{addr:X} outside {dom:?}: dropped")
            }),
        }
        Ok(())
    }

    fn read_le(&mut self, dom: Domain, addr: i64, n: usize) -> u64 {
        (0..n).fold(0u64, |acc, i| {
            acc | (self.read(dom, addr + i as i64) as u64) << (8 * i)
        })
    }

    fn write_le(&mut self, dom: Domain, addr: i64, n: usize, value: i64) -> mlua::Result<()> {
        for i in 0..n {
            self.write(dom, addr + i as i64, (value >> (8 * i)) as u8)?;
        }
        Ok(())
    }
}

/// Registers BizHawk's tables on `lua`.
pub fn install(lua: &Lua, state: &State) -> mlua::Result<()> {
    let g = lua.globals();

    // -- memory ---------------------------------------------------------------------------------
    let memory = lua.create_table()?;
    macro_rules! reader {
        ($name:literal, $n:expr, $signed:expr) => {{
            let st = state.clone();
            memory.set(
                $name,
                lua.create_function(move |_, (addr, dom): (i64, Option<String>)| {
                    let v = st.borrow_mut().read_le(domain(dom)?, addr, $n);
                    Ok(if $signed {
                        let shift = 64 - 8 * $n;
                        ((v << shift) as i64) >> shift
                    } else {
                        v as i64
                    })
                })?,
            )?;
        }};
    }
    reader!("read_u8", 1, false);
    reader!("read_u16_le", 2, false);
    reader!("read_u32_le", 4, false);
    reader!("read_s8", 1, true);
    reader!("read_s16_le", 2, true);
    reader!("read_s32_le", 4, true);
    macro_rules! writer {
        ($name:literal, $n:expr) => {{
            let st = state.clone();
            memory.set(
                $name,
                lua.create_function(move |_, (addr, value, dom): (i64, i64, Option<String>)| {
                    st.borrow_mut().write_le(domain(dom)?, addr, $n, value)
                })?,
            )?;
        }};
    }
    writer!("write_u8", 1);
    writer!("write_u16_le", 2);
    writer!("write_u32_le", 4);
    {
        let st = state.clone();
        memory.set(
            "read_bytes_as_array",
            lua.create_function(move |lua, (addr, len, dom): (i64, i64, Option<String>)| {
                let dom = domain(dom)?;
                let t = lua.create_table_with_capacity(len.max(0) as usize, 0)?;
                let mut s = st.borrow_mut();
                for i in 0..len.max(0) {
                    t.raw_set(i + 1, s.read(dom, addr + i) as i64)?;
                }
                Ok(t)
            })?,
        )?;
    }
    {
        let st = state.clone();
        memory.set(
            "write_bytes_as_array",
            lua.create_function(move |_, (addr, bytes, dom): (i64, Table, Option<String>)| {
                let dom = domain(dom)?;
                let mut s = st.borrow_mut();
                for i in 1..=bytes.raw_len() as i64 {
                    let v: i64 = bytes.raw_get(i)?;
                    s.write(dom, addr + i - 1, v as u8)?;
                }
                Ok(())
            })?,
        )?;
    }
    g.set("memory", memory)?;

    // -- joypad ---------------------------------------------------------------------------------
    let joypad = lua.create_table()?;
    {
        let st = state.clone();
        joypad.set(
            "set",
            lua.create_function(move |_, buttons: Table| {
                let mut s = st.borrow_mut();
                // Quirk (m5_probe.lua: "set() puts the stylus back at the centre").
                s.machine.touch = TOUCH_CENTRE;
                for pair in buttons.pairs::<String, Value>() {
                    let (name, value) = pair?;
                    let on = matches!(value, Value::Boolean(true));
                    if name == "Touch" {
                        s.machine.touching = on;
                    } else if let Some(k) = Keys::from_name(&name) {
                        if on {
                            s.machine.keys |= k;
                        } else {
                            s.machine.keys = Keys(s.machine.keys.0 & !k.0);
                        }
                    } else if !matches!(
                        name.as_str(),
                        "Power" | "LidOpen" | "LidClose" | "Mic Volume"
                    ) {
                        return Err(mlua::Error::runtime(format!(
                            "joypad.set: unknown button \"{name}\""
                        )));
                    }
                }
                Ok(())
            })?,
        )?;
    }
    {
        let st = state.clone();
        joypad.set(
            "setanalog",
            lua.create_function(move |_, axes: Table| {
                let mut s = st.borrow_mut();
                if let Some(x) = axes.get::<Option<i64>>("Touch X")? {
                    s.machine.touch.0 = x.clamp(0, 255) as u16;
                }
                if let Some(y) = axes.get::<Option<i64>>("Touch Y")? {
                    s.machine.touch.1 = y.clamp(0, 191) as u16;
                }
                Ok(())
            })?,
        )?;
    }
    {
        let st = state.clone();
        joypad.set(
            "get",
            lua.create_function(move |lua, ()| {
                let s = st.borrow();
                let t = lua.create_table()?;
                for name in [
                    "A", "B", "X", "Y", "L", "R", "Start", "Select", "Up", "Down", "Left", "Right",
                ] {
                    t.set(
                        name,
                        s.machine.keys.contains(Keys::from_name(name).unwrap()),
                    )?;
                }
                t.set("Touch", s.machine.touching)?;
                t.set("Touch X", s.machine.touch.0 as i64)?;
                t.set("Touch Y", s.machine.touch.1 as i64)?;
                Ok(t)
            })?,
        )?;
    }
    g.set("joypad", joypad)?;

    // -- emu ------------------------------------------------------------------------------------
    let emu = lua.create_table()?;
    {
        let st = state.clone();
        emu.set(
            "framecount",
            lua.create_function(move |_, ()| Ok(st.borrow().machine.nds.frame_count() as i64))?,
        )?;
    }
    let registers = |s: &Shared| {
        let mut all = Vec::with_capacity(34);
        for cpu in [9u8, 7] {
            let r = s.machine.nds.registers(cpu);
            for (i, v) in r.iter().enumerate() {
                let name = if i == 16 {
                    format!("ARM{cpu} CPSR")
                } else {
                    format!("ARM{cpu} r{i}")
                };
                all.push((name, *v as i64));
            }
        }
        all
    };
    {
        let st = state.clone();
        emu.set(
            "getregisters",
            lua.create_function(move |lua, ()| {
                let t = lua.create_table()?;
                for (name, v) in registers(&st.borrow()) {
                    t.set(name, v)?;
                }
                Ok(t)
            })?,
        )?;
    }
    {
        let st = state.clone();
        emu.set(
            "getregister",
            lua.create_function(move |_, name: String| {
                Ok(registers(&st.borrow())
                    .into_iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, v)| v))
            })?,
        )?;
    }
    g.set("emu", emu)?;

    // -- client ---------------------------------------------------------------------------------
    let client = lua.create_table()?;
    {
        let st = state.clone();
        client.set(
            "screenshot",
            lua.create_function(move |_, path: Option<String>| {
                let path = path.ok_or_else(|| {
                    mlua::Error::runtime("client.screenshot needs a path in the goldlink dev shim")
                })?;
                st.borrow()
                    .machine
                    .screenshot(Path::new(&path))
                    .map_err(mlua::Error::runtime)
            })?,
        )?;
    }
    {
        let st = state.clone();
        client.set(
            "speedmode",
            lua.create_function(move |_, pct: f64| {
                st.borrow_mut().machine.speed = Some(pct);
                Ok(())
            })?,
        )?;
    }
    client.set("SetSoundOn", lua.create_function(|_, _on: Value| Ok(()))?)?;
    {
        let st = state.clone();
        client.set(
            "saveram",
            lua.create_function(move |_, ()| {
                st.borrow()
                    .machine
                    .flush_save()
                    .map_err(mlua::Error::runtime)
            })?,
        )?;
    }
    {
        let st = state.clone();
        client.set(
            "reboot_core",
            lua.create_function(move |_, ()| {
                st.borrow_mut()
                    .machine
                    .power_on()
                    .map_err(mlua::Error::runtime)
            })?,
        )?;
    }
    {
        let st = state.clone();
        g.set(
            "__goldlink_exit",
            lua.create_function(move |_, ()| {
                st.borrow_mut().exit = true;
                Ok(())
            })?,
        )?;
    }
    g.set("client", client)?;

    // -- memorysavestate ------------------------------------------------------------------------
    let mss = lua.create_table()?;
    {
        let st = state.clone();
        mss.set(
            "savecorestate",
            lua.create_function(move |_, ()| {
                let mut s = st.borrow_mut();
                let data = s
                    .machine
                    .nds
                    .savestate()
                    .ok_or_else(|| mlua::Error::runtime("savestate failed"))?;
                s.next_state += 1;
                let id = format!("goldlink-state-{}", s.next_state);
                s.states.insert(id.clone(), data);
                Ok(id)
            })?,
        )?;
    }
    {
        let st = state.clone();
        mss.set(
            "loadcorestate",
            lua.create_function(move |_, id: String| {
                let mut s = st.borrow_mut();
                let data = s
                    .states
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| mlua::Error::runtime(format!("no savestate {id}")))?;
                s.machine
                    .nds
                    .load_savestate(&data)
                    .map_err(|e| mlua::Error::runtime(e.to_string()))
            })?,
        )?;
    }
    {
        let st = state.clone();
        mss.set(
            "removestate",
            lua.create_function(move |_, id: String| {
                st.borrow_mut().states.remove(&id);
                Ok(())
            })?,
        )?;
    }
    g.set("memorysavestate", mss)?;

    // -- console, and what yields --------------------------------------------------------------
    lua.load(
        r#"
        console = { log = function(...) print(...) end, clear = function() end }
        emu.frameadvance = function() coroutine.yield() end
        client.exit = function() __goldlink_exit() coroutine.yield() end
        for _, name in ipairs({ "gui", "event", "savestate", "forms", "bizstring", "comm", "movie" }) do
          _G[name] = setmetatable({}, { __index = function(_, k)
            error(name .. "." .. tostring(k) .. " is not provided by the goldlink dev shim", 2)
          end })
        end
        "#,
    )
    .set_name("goldlink prelude")
    .exec()?;
    Ok(())
}

/// How a script run ended.
pub enum Outcome {
    /// client.exit(), or the script returned.
    Exited,
    /// An uncaught Lua error (already printed).
    Error,
}

/// Runs `script` against `machine` until it calls client.exit() or ends.
pub fn run(script: &Path, machine: Machine) -> Result<(Outcome, Machine), String> {
    let source =
        std::fs::read_to_string(script).map_err(|e| format!("{}: {e}", script.display()))?;
    let lua = Lua::new();
    let state: State = Rc::new(RefCell::new(Shared {
        machine,
        exit: false,
        warned: HashSet::new(),
        states: HashMap::new(),
        next_state: 0,
    }));
    install(&lua, &state).map_err(|e| e.to_string())?;
    let func = lua
        .load(&source)
        .set_name(format!("@{}", script.display()))
        .into_function()
        .map_err(|e| e.to_string())?;
    let thread = lua.create_thread(func).map_err(|e| e.to_string())?;
    let outcome = loop {
        if let Err(e) = thread.resume::<mlua::MultiValue>(()) {
            eprintln!("Lua error: {e}");
            break Outcome::Error;
        }
        if state.borrow().exit {
            break Outcome::Exited;
        }
        if thread.status() != ThreadStatus::Resumable {
            eprintln!("warning: the script ended without client.exit()");
            break Outcome::Exited;
        }
        state.borrow_mut().machine.frame();
    };
    drop(thread);
    drop(lua);
    let shared = Rc::try_unwrap(state)
        .map_err(|_| "the Lua state is still in use".to_string())?
        .into_inner();
    Ok((outcome, shared.machine))
}
