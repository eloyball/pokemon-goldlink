#!/usr/bin/env python3
"""The players' BizHawk defaults (docs/RELEASE_PLAN.md §4.4).

Makes coop-launcher/defaults/config.ini from the developer's BizHawk/config.ini with a whitelist:
the NDS controls and hotkeys, the NDS core settings (screen layout, no real BIOS), display, sound,
speed and window size. Everything else (paths, recent files and Lua scripts, trusted tools, window
position, RetroAchievements tokens, the OS-specific display and sound methods) is left to BizHawk's
own defaults, and the launcher fills in this machine's paths and the bridge's trust.

    scripts/bizhawk_defaults.py [--config BizHawk/config.ini] [--out coop-launcher/defaults/config.ini]
    scripts/bizhawk_defaults.py --check     fails if the committed defaults are out of date

Run it again after changing the controls or layout in your own BizHawk, and commit the result.
"""
import argparse
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# Kept as they are in the developer's config.
KEEP = [
    # Input
    "HotkeyBindings", "InputHotkeyOverrideOptions", "NoMixedInputHokeyOverride", "ModifierKeys",
    "MergeLAndRModifierKeys", "OpposingDirPolicy", "AnalogLargeChange", "AnalogSmallChange",
    "AutofireOn", "AutofireOff", "AutofireLagFrames", "HandleAlternateKeyboardLayouts",
    # Co-op needs the game to keep running (and the bridge to keep syncing) behind other windows.
    "RunInBackground", "AcceptBackgroundInput", "AcceptBackgroundInputControllerOnly",
    "PauseWhenMenuActivated",
    # Display
    "TargetZoomFactors", "TargetScanlineFilterIntensity", "TargetDisplayFilter", "DispFinalFilter",
    "DispFixAspectRatio", "DispFixScaleInteger", "DispFullscreenHacks", "DispAutoPrescale",
    "DispSpeedupFeatures", "DispPrescale", "DispManagerAR", "DispCustomUserARWidth",
    "DispCustomUserARHeight", "DispCustomUserArx", "DispCustomUserAry", "DispCropLeft", "DispCropTop",
    "DispCropRight", "DispCropBottom", "DispChromeFrameWindowed", "DispChromeStatusBarWindowed",
    "DispChromeCaptionWindowed", "DispChromeMenuWindowed", "DispChromeStatusBarFullscreen",
    "DispChromeMenuFullscreen", "DispChromeFullscreenAutohideMouse",
    "DispChromeAllowDoubleClickFullscreen", "ResizeWithFramebuffer", "VSync", "DispAllowTearing",
    "DisplayFps", "DisplayFrameCounter", "DisplayLagCounter", "DisplayInput", "DisplayMessages",
    "StackOSDMessages", "OSDMessageDuration", "ScaleOSDWithSystemScale", "ShowContextMenu",
    # Window (its size; the position belongs to the developer's screen)
    "MainWindowSize", "MainWindowMaximized", "SaveWindowPosition", "StartFullscreen",
    "MainFormStayOnTop", "StartPaused",
    # Sound (the volume; the output method and device are the machine's)
    "SoundEnabled", "SoundEnabledNormal", "SoundEnabledRWFF", "MuteFrameAdvance", "SoundVolume",
    "SoundVolumeRWFF", "SoundThrottle", "SoundBufferSizeMs",
    # Speed
    "SpeedPercent", "SpeedPercentAlternate", "FrameSkip", "ClockThrottle", "Unthrottled",
    "AutoMinimizeSkipping", "VSyncThrottle",
    # SaveRAM
    "BackupSaveram", "AutosaveSaveRAM", "FlushSaveRamFrames",
    # The version the config was written by (an older or missing one makes BizHawk ask).
    "LastWrittenFrom", "LastWrittenFromDetailed",
]

# Per-system dictionaries: only the Nintendo DS entries.
NDS_CONTROLLER = "NDS Controller"
KEEP_NDS = {
    "AllTrollers": NDS_CONTROLLER,
    "AllTrollersAutoFire": NDS_CONTROLLER,
    "AllTrollersAnalog": NDS_CONTROLLER,
    "AllTrollersFeedbacks": NDS_CONTROLLER,
    "CoreSettings": "BizHawk.Emulation.Cores.Consoles.Nintendo.NDS.NDS",
    "CoreSyncSettings": "BizHawk.Emulation.Cores.Consoles.Nintendo.NDS.NDS",
}

# The same for every player whatever the developer's config says. The launcher enforces the first
# two on every start as well (coop-launcher/src/bizhawk.rs).
OVERRIDE = {
    "SingleInstanceMode": False,     # several BizHawks on one machine (tests, two players)
    "UpdateAutoCheckEnabled": False,  # the release pins BizHawk 2.11.1
    "FirstBoot": False,
}

# Something that looks like a path on someone's machine.
MACHINE_PATH = re.compile(r'"(/home/|/Users/|/tmp/|[A-Za-z]:[\\/]|\\\\)')


def build(config: dict) -> dict:
    out = {}
    for key in KEEP:
        if key in config:
            out[key] = config[key]
    for key, entry in KEEP_NDS.items():
        if entry in config.get(key, {}):
            out[key] = {entry: config[key][entry]}
    out.update(OVERRIDE)
    return out


def check(defaults: dict) -> list:
    problems = []
    text = json.dumps(defaults)
    for m in MACHINE_PATH.finditer(text):
        problems.append(f"a machine path: {text[m.start():m.start() + 60]}")
    if NDS_CONTROLLER not in defaults.get("AllTrollers", {}):
        problems.append("no NDS controls")
    sync = defaults.get("CoreSyncSettings", {}).get(KEEP_NDS["CoreSyncSettings"])
    if sync is None:
        problems.append("no NDS core sync settings")
    elif sync.get("UseRealBIOS") is not False or sync.get("SkipFirmware") is not True:
        problems.append("the NDS core must run without a BIOS (UseRealBIOS false, SkipFirmware true)")
    return problems


def render(defaults: dict) -> str:
    # BizHawk's own style (two-space JSON); "$type" stays first in each core settings object.
    return json.dumps(defaults, indent=2, ensure_ascii=False) + "\n"


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--config", type=Path, default=ROOT / "BizHawk" / "config.ini")
    ap.add_argument("--out", type=Path, default=ROOT / "coop-launcher" / "defaults" / "config.ini")
    ap.add_argument("--check", action="store_true", help="compare with --out instead of writing it")
    args = ap.parse_args()

    config = json.loads(args.config.read_text(encoding="utf-8-sig"))
    defaults = build(config)
    problems = check(defaults)
    if problems:
        for p in problems:
            print(f"[-] {p}", file=sys.stderr)
        return 1
    text = render(defaults)
    if args.check:
        if not args.out.is_file() or args.out.read_text(encoding="utf-8") != text:
            print(f"[-] {args.out} is out of date: run scripts/bizhawk_defaults.py", file=sys.stderr)
            return 1
        print(f"[*] {args.out} is up to date")
        return 0
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(text, encoding="utf-8")
    dropped = sorted(set(config) - set(defaults))
    print(f"[*] {args.out}: {len(defaults)} settings kept, {len(dropped)} left to BizHawk's defaults")
    return 0


if __name__ == "__main__":
    sys.exit(main())
