using System;
using System.Collections.Generic;
using System.Drawing;
using System.IO;
using System.Net;
using System.Net.Sockets;
using System.Reflection;
using System.Security.Cryptography;
using System.Text;
using System.Text.RegularExpressions;
using System.Threading;
using System.Threading.Tasks;
using System.Windows.Forms;
using BizHawk.Client.Common;
using BizHawk.Client.EmuHawk;
using BizHawk.Emulation.Common;

namespace CoopBridge
{
    /// <summary>
    /// Offsets inside gCoopCommBlock. Must match hg-engine/include/coop_sync.h,
    /// where every offset is pinned by a _Static_assert.
    /// </summary>
    internal static class Mailbox
    {
        public const int Size = 512;
        public const uint Magic = 0x434F4F50;
        public const ushort Version = 7; // COOP_MAILBOX_VERSION

        public const uint MagicOff = 0x000;          // u32
        public const uint VersionOff = 0x004;        // u16
        public const uint AssignedPlayerId = 0x006;  // u8   bridge
        public const uint ActivePlayersMask = 0x007; // u8   bridge
        public const uint HeartbeatRom = 0x008;      // u32  rom (restarts at 0 when the game resets)
        public const uint ServerTick = 0x00C;        // u32  bridge
        public const uint LocalPlayer = 0x010;       // 64 B rom
        public const uint ServerPayload = 0x050;     // 344 B bridge (remote players, gates)
        public const int ServerPayloadSize = 344;
        public const uint Gates = 0x190;             // 24 B bridge
        public const uint IsConnectedToRoom = 0x1A8; // u8   bridge
        public const uint WasInActiveSession = 0x1AA; // u8  rom: a save is in play (startup gate passed)
        public const uint DisconnectState = 0x1AB;   // u8   rom
        public const uint BadgeMaskLocal = 0x1AC;    // u16  rom
        public const uint RoomRejection = 0x1AE;     // u8   bridge: relay status that turned this save away
        public const uint HeartbeatBridge = 0x1B0;   // u32  bridge
        public const uint LocalAvatar = 0x1BE;       // u8   bridge: this player's avatar ID (docs/CUSTOM_SPRITES_PLAN.md)
        public const uint LocalPlayerSeq = 0x1B4;    // u32  rom (odd while writing)
        public const uint LimitMapId = 0x1C0;        // u16  rom: map_id < limit
        public const uint LimitSpecies = 0x1C2;      // u16  rom: species <= limit
        public const uint Diag = 0x1C4;              // 24 B rom: CoopDiagnostics
        public const int DiagSize = 24;
        public const uint DebugCmd = 0x1DC;          // u8   bridge: CoopDebugCommand
        public const uint DebugSeq = 0x1DD;          // u8   bridge: bumped per command
        public const uint DebugArg = 0x1DE;          // u8   bridge: command argument
        public const uint DebugResult = 0x1DF;       // u8   rom: CoopDebugResult
        public const uint SaveCounter = 0x1E0;       // u32  rom: bumped after every completed save write
        public const uint SaveSlotReady = 0x1E4;     // u8   bridge: this room's save is in SRAM
        public const uint ResetRequest = 0x1E5;      // u8   bridge: restart at the title to load SRAM
        public const uint SessionOut = 0x1F0;        // 8 B  rom: latched session op (seq, op, kind, pad, u32 target)
        public const int SessionOutSize = 8;
        public const uint ExchangePtr = 0x1FC;       // u32  rom: address of the open CoopTradeExchange, 0 when none

        // CoopTradeExchange (hg-engine coop_sync.h): header, then the ROM's
        // `out` (forwarded to the relay) and `in` (written from the relay).
        public const uint ExchangeMagic = 0x31585443; // "CTX1"
        public const ushort ExchangeVersion = 1;
        public const int ExchangeSize = 0x1F8;
        public const int ExchangeOut = 0x008;
        public const int TradeOutSize = 244;
        public const int ExchangeIn = 0x0FC;
        public const int TradeInSize = 252;
        public const int TradeInStateMax = 4;
        public const int TradeInReasonMax = 3;
        public const int TradeInPeerSlot = 5;
        public const uint MainRamBase = 0x02000000;  // ARM9 address of Main RAM offset 0
        public const uint MainRamSize = 0x00400000;

        // Milestone 5 challenge sync (hg-engine coop_sync.h CoopChallengeSync,
        // docs/MILESTONE5_PLAN.md §4.4). It lives in Overlay 131, reached
        // through the overlay's pinned header, so it exists only while the
        // field runs: the header magic and the block header are checked
        // every frame before anything is read or written.
        public const uint FieldEntryHeader = 0x003C4000;  // Main RAM offset of COOP_FIELD_ENTRY_ADDR
        public const uint FieldEntryMagic = 0x31464F43;   // "COF1"
        public const int FieldEntryChallenge = 0x0C;      // u32 pointer to the CoopChallengeSync
        public const uint Overlay131Start = 0x023C4000;
        public const uint Overlay131End = 0x023D8000;   // Overlay 129 starts here
        public const uint ChallengeSyncMagic = 0x31534843; // "CHS1"
        public const ushort ChallengeSyncVersion = 1;
        public const int ChallengeSyncSize = 0x78;
        public const int ChallengeSyncOut = 0x08;          // the up trailer as the ROM fills it (48 B)
        public const int ChallengeSyncIn = 0x38;           // the relay's down trailer, verbatim (64 B)
        public const byte ChallengeFlagsMask = 0x01;       // player state 0x3F: bit 0 spirit

        // CoopDiagnostics field offsets (relative to Diag)
        public const int DgFieldEntryCalls = 0x00;   // u32
        public const int DgRenderReady = 0x04;
        public const int DgTexResident = 0x05;
        public const int DgTexDynamic = 0x06;
        public const int DgTexDynamicFree = 0x07;
        public const int DgModelDynamic = 0x08;
        public const int DgModelDynamicFree = 0x09;
        public const int DgSeqDynamic = 0x0A;
        public const int DgSeqDynamicFree = 0x0B;
        public const int DgMapObjects = 0x0C;
        public const int DgGhostState = 0x0D;        // 0 none, 1 idle, 2 moving
        public const int DgDebugAck = 0x0E;          // last DebugSeq processed
        public const int DgFieldEntryStatus = 0x0F;  // CoopFieldEntryStatus
        public const int DgGhostX = 0x10;            // s16
        public const int DgGhostZ = 0x12;            // s16
        public const int DgGhostY = 0x14;            // s16 render height
        public const int DgPlayerY = 0x16;           // s16 render height

        // CoopDebugCommand. DEVELOPMENT ONLY (docs/DEBUG_TOOLS.md).
        public const byte DebugSpawnGhost = 1;
        public const byte DebugStepGhost = 2;
        public const byte DebugEmoteGhost = 3;
        public const byte DebugRemoveGhost = 4;
        public const byte DebugWarp = 5;             // DebugArg = CoopWarpPreset
        public const byte DebugToggleBike = 6;
        public const byte DebugSurf = 7;
        public const byte DebugFish = 8;

        // CoopWarpPreset, in order (hg-engine/include/coop_sync.h).
        public static readonly string[] WarpPresets =
        {
            "New Bark Town (Route 29 border, water)",
            "Cherrygrove City (beach)",
            "Violet City (route borders)",
            "Goldenrod City (dense)",
            "Ecruteak City (dense)",
            "Olivine City (harbor)",
            "Route 40 (beach, sea)",
            "Cerulean City (Nugget Bridge north)",
            "Lavender Town (Silence Bridge south)",
            "Celadon City (dense)",
            "Violet Gym (gym gate, Falkner ahead)",
            "Lance's room (team boss by coord event)",
            "Elm's lab (below the starter machine)",
            "Route 29 (below the Potion ball)",
        };

        // CoopPlayerState field offsets (64 bytes, follower included)
        public const int PlayerStateSize = 64;
        public const int PsMapId = 0x00;             // u16
        public const int PsMatrixId = 0x02;
        public const int PsAvatarState = 0x03;       // pret PLAYER_STATE_*
        public const int PsX = 0x04;                 // s16
        public const int PsZ = 0x06;                 // s16
        public const int PsDirection = 0x0A;
        public const int PsGender = 0x0B;
        public const int PsMoveAction = 0x0E;
        public const int PsIsActive = 0x10;
        public const int PsClientReady = 0x11;
        public const int PsSaveStatus = 0x12;        // CoopSaveStatus
        public const int PsActivity = 0x13;          // CoopActivity
        public const int PsTrainerId = 0x14;         // u32
        public const int PsName = 0x18;              // u16[8] HG charset
        public const int PsFollowerSpecies = 0x28;   // u16
        public const int PsFollowerShiny = 0x2D;
        public const int PsFollowerGender = 0x2E;
        public const int PsFollowerVisible = 0x2F;
        public const int PsFollowerDirection = 0x36;
        public const int PsFollowerMoveAction = 0x37;
        public const int PsAvatarId = 0x3C;          // follower block ends here
        public const int PsAnim = 0x3D;              // pose animation index
        public const int PsFieldBusy = 0x3E;
        public const int PsChallengeFlags = 0x3F;    // Milestone 5: bit 0 spirit (Ghost Mode)         // field task or message running (0/1)
        public const int MaxAnim = 15;
        public const int MaxHeldMovement = 0x71;     // MapObject_SetHeldMovement limit
        public const byte MoveActionNone = 0xFF;     // no action since the map loaded
        public const int MaxAvatarState = 14;
        public const int MaxActivity = 5;             // CoopActivity: COOP_ACTIVITY_EVENT (Milestone 6 §6.1) is the last

        // CoopProgressionGates field offsets (relative to Gates)
        public const int GatesSize = 24;
        public const int GateSyncReady = 0x04;
        public const int GateCheckpoint = 0x05;
        public const int GateMonolocke = 0x06;
        public const int GateChallengeRoomState = 0x07; // Milestone 5: bit 0 run lost
        public const int GateCampaignStarted = 0x08;
        public const int GateExpectedParty = 0x09;
        public const int GateOnlineParty = 0x0A;
        public const int GateReadyParty = 0x0B;
        public const int GateRoomUid = 0x0C;         // u32
        public const int GateTeamPartner = 0x10;     // remote slot or 0xFF
        public const int GateTeamedMask = 0x11;
        public const int GateSessionState = 0x12;
        public const int GateSessionKind = 0x13;
        public const int GateSessionPeer = 0x14;     // remote slot or 0xFF
        public const int GateSessionSeq = 0x15;
        public const int GateSessionReason = 0x16;
        public const int GateSessionSize = 7;
        public const int GateTradePending = 0x17;
        public const int MaxSessionState = 8; // 8: notice (a teammate left)
        public const int MaxSessionKind = 6;  // 5: team battle (2v2), 6: revive (Milestone 5 Ghost Mode)
        public const int MaxSessionReason = 6; // 6: a teammate is away (a team battle's teammate: the opponent's remote slot)

        // Layout of the 344-byte server payload (relative to ServerPayload)
        public const int RemoteSlots = 5;
        public const int PayloadGates = RemoteSlots * PlayerStateSize; // 320

        public const byte PlayerIdNone = 0xFF;
        public const int MaxPlayers = 6;
    }

    /// <summary>
    /// Milestone 5 room rules (docs/MILESTONE5_PLAN.md §1, §4.2). Bits must
    /// match coop-relay/src/protocol.rs CHALLENGE_RULE_* and hg-engine's
    /// coop_challenge.h.
    /// </summary>
    internal static class ChallengeRules
    {
        public const ushort RandomMask = 0x0003;     // 0 none, 1 semi-random, 2 preserve difficulty, 3 fully random
        public const ushort LegendsWild = 0x0004;
        public const ushort RandomAbilities = 0x0008;
        public const ushort Nuzlocke = 0x0010;
        public const int SoulLinkShift = 5;           // 0 off, 1 room-wide, 2 duo
        public const ushort SoulLinkMask = 0x0060;
        public const ushort Monolocke = 0x0080;
        public const ushort LevelCap = 0x0100;
        public const ushort RandomItems = 0x0200;
        public const byte MonoNone = 18;

        public static readonly string[] RandomModes = { "None", "Semi-random", "Preserve difficulty", "Fully random" };
        public static readonly string[] SoulLinkModes = { "Off", "World-wide", "Duo" };

        // hg-engine type order (constants/battle_constants.h): Fairy took the "???" slot 9.
        public static readonly string[] TypeNames =
        {
            "Normal", "Fighting", "Flying", "Poison", "Ground", "Rock", "Bug", "Ghost", "Steel",
            "Fairy", "Fire", "Water", "Grass", "Electric", "Psychic", "Ice", "Dragon", "Dark",
        };

        /// <summary>Same rule as the relay: drop unknown bits and rules whose prerequisite is off.</summary>
        public static ushort Sanitize(ushort raw)
        {
            ushort rules = (ushort)(raw & Wire.ChallengeRulesValid);
            int soulLink = (rules & SoulLinkMask) >> SoulLinkShift;
            if (soulLink > 2 || (rules & Nuzlocke) == 0) rules = (ushort)(rules & ~SoulLinkMask);
            if ((rules & RandomMask) == 0) rules = (ushort)(rules & ~(LegendsWild | RandomAbilities));
            return rules;
        }

        public static ushort Checksum(byte[] buf, int length, int start = 0)
        {
            ushort sum = 0;
            for (int i = 0; i < length; i++) sum = (ushort)(sum + buf[start + i]);
            return sum;
        }

        public static string Describe(ushort rules, byte monoType)
        {
            var parts = new List<string>();
            int random = rules & RandomMask;
            if (random != 0)
            {
                string r = "Random encounters: " + RandomModes[random];
                if ((rules & LegendsWild) != 0) r += ", legends in the wild";
                if ((rules & RandomAbilities) != 0) r += ", random abilities";
                parts.Add(r);
            }
            if ((rules & Nuzlocke) != 0)
            {
                int soulLink = (rules & SoulLinkMask) >> SoulLinkShift;
                parts.Add(soulLink == 0 ? "Nuzlocke" : $"Nuzlocke, Soul-Link {SoulLinkModes[soulLink].ToLowerInvariant()}");
            }
            if ((rules & Monolocke) != 0)
                parts.Add(monoType < TypeNames.Length ? $"Monolocke ({TypeNames[monoType]})" : "Monolocke");
            if ((rules & LevelCap) != 0) parts.Add("Level cap");
            if ((rules & RandomItems) != 0) parts.Add("Random items");
            return parts.Count == 0 ? "None" : string.Join("; ", parts);
        }
    }

    /// <summary>UDP wire format. Must match coop-relay/src/protocol.rs.</summary>
    internal static class Wire
    {
        public const ushort ProtocolVersion = 8;
        public const int ClientPacketSize = 120;   // + Mailbox.TradeOutSize while the ROM has a trade open, + a link trailer (CoopLink), + the challenge trailer (always last)
        public const int ServerPacketSize = 360;
        public const int ErrorPacketSize = 16;
        public const int ChallengePacketSize = 24; // header + cookie (u64 @16)
        public const int ServerPayloadOffset = 16; // packet[16..360] -> mailbox 0x050..0x1A7

        // Milestone 5 challenge trailers (protocol.rs ChallengeUp / ChallengeDown):
        // fixed size, always the last bytes of a packet, ending in a checksum.
        public const uint ChallengeMagic = 0x314C4843;   // "CHL1"
        public const byte ChallengeVersion = 1;
        public const int ChallengeUpSize = 48;
        public const int ChallengeDownSize = 64;
        public const ushort ChallengeRulesValid = 0x03FF;

        public const byte StatusOk = 0;
        public const byte StatusChallenge = 7;   // not an error: echo the cookie
        public const byte StatusRateLimited = 8;
        public const byte StatusMidgameSave = 6;   // forming room, save already in progress
        public const byte StatusSaveOtherRoom = 9; // trainer belongs to another locked campaign
        public const byte StatusRunLost = 10;      // Milestone 5: every player of this Nuzlocke run is a spirit
        // The relay speaks another protocol version: its own is at bytes 4..6 of this reply (the
        // 16-byte error header, frozen across versions; docs/RELEASE_PLAN.md §5).
        public const byte StatusProtocol = 11;

        public const byte ActionCreate = 1;
        public const byte ActionJoin = 2;
        public const byte ActionSync = 3;
        public const byte ActionLeave = 4;
    }

    /// <summary>Dark theme tokens. Text/background pairs meet WCAG AA contrast.</summary>
    internal static class Theme
    {
        public static readonly Color Window = Color.FromArgb(0x16, 0x18, 0x1D);
        public static readonly Color Card = Color.FromArgb(0x21, 0x24, 0x2B);
        public static readonly Color Input = Color.FromArgb(0x2B, 0x2F, 0x38);
        public static readonly Color TextPrimary = Color.FromArgb(0xE8, 0xEA, 0xED);
        public static readonly Color TextSecondary = Color.FromArgb(0xA0, 0xA6, 0xB0);
        public static readonly Color TextDisabled = Color.FromArgb(0x6B, 0x72, 0x80);

        public static readonly Color Primary = Color.FromArgb(0x25, 0x63, 0xEB);
        public static readonly Color PrimaryHover = Color.FromArgb(0x1D, 0x4E, 0xD8);
        public static readonly Color Danger = Color.FromArgb(0xDC, 0x26, 0x26);
        public static readonly Color DangerHover = Color.FromArgb(0xB9, 0x1C, 0x1C);
        public static readonly Color Neutral = Color.FromArgb(0x2B, 0x2F, 0x38);
        public static readonly Color NeutralHover = Color.FromArgb(0x37, 0x3C, 0x47);
        public static readonly Color Disabled = Color.FromArgb(0x26, 0x29, 0x30);

        public static readonly Color Offline = Color.FromArgb(0x9C, 0xA3, 0xAF);
        public static readonly Color Connecting = Color.FromArgb(0xF5, 0x9E, 0x0B);
        public static readonly Color Connected = Color.FromArgb(0x22, 0xC5, 0x5E);
        public static readonly Color Error = Color.FromArgb(0xF8, 0x71, 0x71);

        public static readonly Font Body = new Font(FontFamily.GenericSansSerif, 9f);
        public static readonly Font BodyBold = new Font(FontFamily.GenericSansSerif, 9f, FontStyle.Bold);
        public static readonly Font Caption = new Font(FontFamily.GenericSansSerif, 8f);
        public static readonly Font Title = new Font(FontFamily.GenericSansSerif, 12f, FontStyle.Bold);
        public static readonly Font Code = new Font(FontFamily.GenericMonospace, 16f, FontStyle.Bold);

        public static readonly Color FocusRing = Color.FromArgb(0x93, 0xC5, 0xFD);

        public enum ButtonKind { Primary, Danger, Neutral, SegmentOn, SegmentOff }

        /// <summary>Background / text colors for a button kind in a given state.</summary>
        public static (Color bg, Color fg) ButtonColors(ButtonKind kind, bool enabled, bool hot)
        {
            if (!enabled) return (Disabled, TextDisabled);
            return kind switch
            {
                ButtonKind.Primary => (hot ? PrimaryHover : Primary, Color.White),
                ButtonKind.Danger => (hot ? DangerHover : Danger, Color.White),
                ButtonKind.SegmentOn => (Primary, Color.White),
                ButtonKind.SegmentOff => (hot ? NeutralHover : Neutral, TextSecondary),
                _ => (hot ? NeutralHover : Neutral, TextPrimary)
            };
        }

        // Small preferred width: buttons are docked to fill their cell, and a
        // large default width would make auto-sized rows overflow the card.
        public static ThemedButton MakeButton(string text, ButtonKind kind) => new ThemedButton
        {
            Text = text,
            Kind = kind,
            Font = BodyBold,
            Size = new Size(40, 34),
            Dock = DockStyle.Fill,
            Margin = new Padding(0)
        };

        public static Label MakeLabel(string text, Font font, Color color) => new Label
        {
            Text = text,
            Font = font,
            ForeColor = color,
            AutoSize = true,
            Margin = new Padding(0, 0, 0, 4)
        };

        /// <summary>Single-column auto-sizing stack with card styling.</summary>
        public static TableLayoutPanel MakeCard() => new TableLayoutPanel
        {
            ColumnCount = 1,
            AutoSize = true,
            AutoSizeMode = AutoSizeMode.GrowAndShrink,
            Dock = DockStyle.Fill,
            BackColor = Card,
            Padding = new Padding(14, 12, 14, 14),
            Margin = new Padding(0, 0, 0, 12)
        };
    }

    /// <summary>
    /// Owner-drawn flat button. Mono's stock disabled-button rendering
    /// (engraved grey text) is illegible on dark backgrounds, so every state
    /// is painted from the theme instead.
    /// </summary>
    internal sealed class ThemedButton : Button
    {
        private Theme.ButtonKind _kind;
        private bool _hot;

        public ThemedButton()
        {
            SetStyle(ControlStyles.UserPaint | ControlStyles.AllPaintingInWmPaint | ControlStyles.OptimizedDoubleBuffer, true);
            FlatStyle = FlatStyle.Flat;
            FlatAppearance.BorderSize = 0;
            UseVisualStyleBackColor = false;
            Cursor = Cursors.Hand;
        }

        public Theme.ButtonKind Kind
        {
            get => _kind;
            set { if (_kind != value) { _kind = value; Invalidate(); } }
        }

        protected override void OnMouseEnter(EventArgs e) { _hot = true; Invalidate(); base.OnMouseEnter(e); }
        protected override void OnMouseLeave(EventArgs e) { _hot = false; Invalidate(); base.OnMouseLeave(e); }
        protected override void OnEnabledChanged(EventArgs e)
        {
            Cursor = Enabled ? Cursors.Hand : Cursors.Default;
            Invalidate();
            base.OnEnabledChanged(e);
        }
        protected override void OnGotFocus(EventArgs e) { Invalidate(); base.OnGotFocus(e); }
        protected override void OnLostFocus(EventArgs e) { Invalidate(); base.OnLostFocus(e); }

        protected override void OnPaint(PaintEventArgs e)
        {
            (Color bg, Color fg) = Theme.ButtonColors(_kind, Enabled, _hot);
            using (var brush = new SolidBrush(bg))
            {
                e.Graphics.FillRectangle(brush, ClientRectangle);
            }
            TextRenderer.DrawText(e.Graphics, Text, Font, ClientRectangle, fg,
                TextFormatFlags.HorizontalCenter | TextFormatFlags.VerticalCenter | TextFormatFlags.SingleLine);
            if (Focused && Enabled)
            {
                // Visible keyboard focus (Tab navigation).
                using (var pen = new Pen(Theme.FocusRing, 2f))
                {
                    e.Graphics.DrawRectangle(pen, 1, 1, Width - 3, Height - 3);
                }
            }
        }
    }

    [ExternalTool("HGSS Co-op Bridge")]
    public sealed class CoopBridgeForm : ToolFormBase, IExternalToolForm
    {
        protected override string WindowTitleStatic =>
            InstanceName == null ? "HGSS Co-op Bridge" : $"HGSS Co-op Bridge ({InstanceName})";

        [RequiredApi]
        public IMemoryApi? _memoryApi { get; set; }

        // Emergency backups (Backups.cs): savestates taken and loaded on frame
        // boundaries. Optional: without it, the bridge works without backups.
        [OptionalApi]
        public ISaveStateApi? _saveStateApi { get; set; }

        // Used only to verify the domain exists: IMemoryApi silently falls back
        // to the "current" domain when a name is unknown.
        [OptionalService]
        public IMemoryDomains? MemoryDomains { get; set; }

        private enum LinkState { Offline, Resolving, Connecting, Connected }

        private const string MemoryDomain = "Main RAM";
        private const string ConfigFileName = "coop_config.json";
        // BizHawk config passed with --config (null for the default config.ini).
        private static readonly string? BizHawkConfigPath = FindBizHawkConfigArg();
        // Name of the instance directory when launched with --config
        // (scripts/run_bizhawk.sh <name>), shown in the window title.
        private static readonly string? InstanceName = BizHawkConfigPath == null
            ? null : Path.GetFileName(Path.GetDirectoryName(BizHawkConfigPath));
        // One coop_config.json per BizHawk config, because it holds the player
        // secret: two instances sharing one secret would be treated as the same
        // player by a locked campaign. Default instance: <BizHawk>/coop_config.json,
        // resolved from the DLL location (<BizHawk>/ExternalTools) so it does
        // not depend on the working directory.
        private static readonly string ConfigFilePath = BizHawkConfigPath != null
            ? Path.Combine(Path.GetDirectoryName(BizHawkConfigPath) ?? ".", ConfigFileName)
            : Path.GetFullPath(Path.Combine(
                Path.GetDirectoryName(Assembly.GetExecutingAssembly().Location) ?? ".", "..", ConfigFileName));
        private const string OffsetsFileName = "coop_offsets.json";
        private const double ServerTimeoutSeconds = 3.0;
        // Challenges prove the relay is reachable, but must lead to a real
        // OK packet within this long (e.g. a NAT that changes our port per
        // packet would otherwise keep us "connecting" forever).
        private const double HandshakeTimeoutSeconds = 10.0;
        private const int KeepAliveIntervalMs = 250;
        private const double PausedThresholdSeconds = 0.5;
        private const int InfoRefreshMs = 100;
        private const string DefaultMessage = "Enter a world code to join your friends, or create a new world.";

        private uint? _mailboxOffset; // null => coop_offsets.json missing/invalid; never touch RAM
        private string _offsetsError = string.Empty;
        private uint _sequence;
        private uint _bridgeHeartbeat;
        private byte _assignedSlot = Mailbox.PlayerIdNone;
        // Relay-issued return-routability cookie (0 = none yet). Written by the
        // receive thread, read by the sender.
        private long _cookie;
        // Random per bridge install, persisted in coop_config.json. Binds this
        // player's roster slot so a known OTID alone cannot take it over.
        private readonly ulong _playerSecret;

        private LinkState _link = LinkState.Offline;
        private bool _hostMode;
        // Set as soon as the window starts closing. On emulator exit BizHawk
        // disposes the core before closing tools, so from then on any call into
        // emulated memory would enter a dead waterbox host and crash Mono.
        private volatile bool _closing;
        private bool _clearPayloadPending = true;
        private byte _activeAction;
        private uint _hostToken;
        private DateTime _lastServerPacketUtc;
        private DateTime _lastFrameUtc;
        private DateTime _lastInfoRefreshUtc;
        private string _message = DefaultMessage;
        private bool _messageIsError;

        // Last consistent snapshot of the ROM's local player block.
        private readonly byte[] _localPlayer = new byte[Mailbox.PlayerStateSize];
        // The ROM's latched session request (trade / battle / team up), forwarded as is.
        private readonly byte[] _sessionOut = new byte[Mailbox.SessionOutSize];
        // The ROM's side of an open trade (CoopTradeOut), forwarded while
        // _tradeOutValid; read on the emulator thread, sent from either thread.
        private readonly object _tradeLock = new object();
        private readonly byte[] _tradeOut = new byte[Mailbox.TradeOutSize];

        // Milestone 5 challenge rules (docs/MILESTONE5_PLAN.md §4.2): the
        // host's choice (persisted), the ROM's report (up trailer) and the
        // relay's answer (down trailer, also shown in the session card).
        private ushort _challengeRules;
        private readonly object _challengeLock = new object();

        // Custom avatars (docs/CUSTOM_SPRITES_PLAN.md §1): picked before
        // creating or joining, saved by key, sent as the player state's
        // avatar_id (the ROM never writes that byte).
        private readonly List<AvatarChoice> _avatars;
        private readonly ComboBox _avatarPicker;
        private readonly PictureBox _avatarPreview;
        private byte _avatarId;
        private string _avatarKey;
        private readonly byte[] _challengeUp = new byte[Wire.ChallengeUpSize];
        private bool _challengeUpValid;
        private ushort _roomRules;
        private byte _roomMonoType = ChallengeRules.MonoNone;
        private bool _roomRulesKnown;
        private bool _tradeOutValid;
        // Virtual link (Milestone 3 Phase 3): carries the ROM's link block.
        private readonly CoopLink _linkCarrier = new CoopLink();
        private ushort _badgeMaskLocal;
        private uint _heartbeatRom;
        // Bounds published by the ROM (mailbox v3), used to sanitize relay data.
        private ushort _limitMapId;
        private ushort _limitSpecies;

        // --- UI ---
        private readonly Label _statusPill;
        private readonly Label _messageLabel;
        private readonly ThemedButton _joinModeButton;
        private readonly ThemedButton _hostModeButton;
        private readonly ComboBox _roomInput; // the world code, with the worlds played before (Worlds.cs)
        private readonly ThemedButton _forgetButton;
        private readonly WorldList _worlds = new WorldList(
            Path.GetDirectoryName(ConfigFilePath) ?? ".",
            Path.Combine(Path.GetDirectoryName(ConfigFilePath) ?? ".", "coop_saves"));
        private string _activeServerText = string.Empty;
        private readonly TableLayoutPanel _modeSegments;
        private readonly TableLayoutPanel _hostOptions;
        private readonly ThemedButton[] _partySizeButtons = new ThemedButton[Mailbox.MaxPlayers - 1];
        private int _partySize;
        private readonly ThemedButton _primaryButton;
        private readonly ThemedButton _leaveButton;
        private readonly TextBox _serverInput;
        private readonly Label _gameValue;
        private readonly Label _trainerValue;
        private readonly Label _bedroomValue;
        private readonly Label _playersValue;
        private readonly Label _adventureValue;
        private readonly Label _rulesValue;
        private readonly ComboBox _randomPicker;
        private readonly ComboBox _soulLinkPicker;
        private readonly CheckBox _legendsCheck;
        private readonly CheckBox _abilitiesCheck;
        private readonly CheckBox _nuzlockeCheck;
        private readonly CheckBox _monolockeCheck;
        private readonly CheckBox _levelCapCheck;
        private readonly CheckBox _randomItemsCheck;
        private readonly Label _gymValue;
        private readonly Label _saveValue;
        private readonly ThemedButton _allowAheadButton;
        // Gym gate: teammates may challenge bosses while this player is behind on badges.
        private bool _allowAhead;
        private readonly Label _locationValue;
        private readonly Label _othersValue;
#if !COOP_SHIP // the debug tools: compiled out of releases (docs/RELEASE_PLAN.md §11 D3)
        private readonly ThemedButton _diagToggle;
        private readonly TableLayoutPanel _diagCard;
        private readonly Label _diagFieldValue;
        private readonly Label _diagTexturesValue;
        private readonly Label _diagModelsValue;
        private readonly Label _diagSequencesValue;
        private readonly Label _diagObjectsValue;
        private readonly Label _diagGhostValue;
        private readonly Label _diagHeightValue;
        private readonly Label _diagPlayerStateValue;
        private readonly Label _diagCommandValue;
        private readonly ComboBox _warpPicker;
        private readonly ThemedButton[] _diagButtons;
#endif
        // One save per room (SaveSlots.cs). Prepared once per connection,
        // after the relay's first OK packet names the room (room_uid).
        private enum SlotState { None, Swapping, Ready }
        private readonly SaveSlots _saveSlots = new SaveSlots(
            Path.Combine(Path.GetDirectoryName(ConfigFilePath) ?? ".", "coop_saves"));
        private SlotState _slotState = SlotState.None;
        private readonly Backups _backups = new Backups(
            Path.Combine(Path.GetDirectoryName(ConfigFilePath) ?? ".", "coop_backups"));
        private DateTime _nextBackupUtc = DateTime.MaxValue; // set once the room's save is in place
        private string? _pendingRestore; // a backup to load on the next frame boundary
        private ThemedButton? _restoreButton;
        private string? _slotKey;
        private uint? _lastSaveCounter;
        private uint _lastRomHeartbeat;
        private DateTime _swapRequestedUtc;
        private const double SwapRetrySeconds = 10.0;
#if !COOP_SHIP // the debug tools: compiled out of releases (docs/RELEASE_PLAN.md §11 D3)
        private byte _pendingDebugCmd;
        private byte _pendingDebugArg;
        private string _sentDebugName = "";
        private byte _debugSeq;
        private uint _lastFieldEntryCalls;
        private DateTime _lastFieldEntryChangeUtc;
#endif
        private readonly System.Windows.Forms.Timer _keepAliveTimer;

        private readonly UdpClient _udpClient;
        private volatile IPEndPoint _serverEndpoint;
        private byte[] _roomCodeBytes = new byte[6];
        private string _activeRoomString = string.Empty;
        private Thread? _recvThread;
        private volatile bool _running;

        // Incremented on every connect/disconnect; packets received for an
        // older session are dropped.
        private int _sessionId;
        private long _lastChallengeUtcTicks;
        private readonly object _packetLock = new object();
        private byte[]? _latestServerPacket;
        private int _latestServerPacketSession;

        public CoopBridgeForm()
        {
            LoadOffsetsFromJson();
            (string savedHost, int savedParty, ulong savedSecret, ushort savedRules, string savedAvatar) = LoadNetConfig();
            _playerSecret = savedSecret != 0 ? savedSecret : GenerateCryptoSecret();
            _challengeRules = ChallengeRules.Sanitize(savedRules);
            _avatars = Avatars.Load(RepoFileCandidates(Avatars.ListFileName));
            AvatarChoice avatar = _avatars.Find(a => a.Key == savedAvatar) ?? _avatars[0];
            _avatarId = avatar.Id;
            _avatarKey = avatar.Key;
            if (savedSecret == 0) SaveNetConfig(savedHost, savedParty, _playerSecret, _challengeRules, _avatarKey);

            BackColor = Theme.Window;
            ForeColor = Theme.TextPrimary;
            Font = Theme.Body;
            FormBorderStyle = FormBorderStyle.FixedDialog;
            MaximizeBox = false;
            AutoSize = true;
            AutoSizeMode = AutoSizeMode.GrowAndShrink;

            const int contentWidth = 400;
            var root = new TableLayoutPanel
            {
                ColumnCount = 1,
                AutoSize = true,
                AutoSizeMode = AutoSizeMode.GrowAndShrink,
                Padding = new Padding(16),
                MinimumSize = new Size(contentWidth + 32, 0),
                MaximumSize = new Size(contentWidth + 32, 0),
                BackColor = Theme.Window
            };
            root.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100f));

            // --- Header: title + status pill, then the message line ---
            var header = new TableLayoutPanel
            {
                ColumnCount = 2,
                AutoSize = true,
                Dock = DockStyle.Fill,
                Margin = new Padding(0, 0, 0, 6)
            };
            header.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100f));
            header.ColumnStyles.Add(new ColumnStyle(SizeType.AutoSize));
            var title = Theme.MakeLabel("Co-op Bridge", Theme.Title, Theme.TextPrimary);
            title.Margin = new Padding(0);
            title.Anchor = AnchorStyles.Left;
            _statusPill = new Label
            {
                AutoSize = true,
                Font = Theme.BodyBold,
                Padding = new Padding(10, 4, 10, 4),
                Anchor = AnchorStyles.Right,
                Margin = new Padding(0)
            };
            header.Controls.Add(title, 0, 0);
            header.Controls.Add(_statusPill, 1, 0);

            _messageLabel = new Label
            {
                AutoSize = true,
                MaximumSize = new Size(contentWidth, 0),
                Font = Theme.Body,
                Margin = new Padding(0, 0, 0, 12)
            };

            // --- Room card ---
            var roomCard = Theme.MakeCard();
            var segments = _modeSegments = new TableLayoutPanel
            {
                ColumnCount = 2,
                AutoSize = true,
                Dock = DockStyle.Fill,
                Margin = new Padding(0, 0, 0, 12)
            };
            segments.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 50f));
            segments.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 50f));
            _joinModeButton = Theme.MakeButton("Join a world", Theme.ButtonKind.SegmentOn);
            _hostModeButton = Theme.MakeButton("Create a world", Theme.ButtonKind.SegmentOff);
            _joinModeButton.Height = _hostModeButton.Height = 30;
            _joinModeButton.Margin = new Padding(0, 0, 2, 0);
            _hostModeButton.Margin = new Padding(2, 0, 0, 0);
            _joinModeButton.Click += (_, _) => SetHostMode(false);
            _hostModeButton.Click += (_, _) => SetHostMode(true);
            segments.Controls.Add(_joinModeButton, 0, 0);
            segments.Controls.Add(_hostModeButton, 1, 0);

            var roomCaption = Theme.MakeLabel("World code", Theme.Caption, Theme.TextSecondary);
            // Milestone 6 §4.2: type a code, or pick a world played before (most recent first).
            _roomInput = new ComboBox
            {
                DropDownStyle = ComboBoxStyle.DropDown,
                MaxLength = 6,
                Font = Theme.Code,
                BackColor = Theme.Input,
                ForeColor = Theme.TextPrimary,
                FlatStyle = FlatStyle.Flat,
                Dock = DockStyle.Fill,
                Margin = new Padding(0)
            };
            // Only letters and digits, upper case; everything else (except editing keys) is dropped.
            _roomInput.KeyPress += (_, e) =>
            {
                if (char.IsControl(e.KeyChar)) return;
                if (!IsRoomCodeChar(e.KeyChar)) e.Handled = true;
                else e.KeyChar = char.ToUpperInvariant(e.KeyChar);
            };
            _roomInput.TextChanged += (_, _) =>
            {
                string upper = _roomInput.Text.ToUpperInvariant();
                if (upper != _roomInput.Text && _roomInput.SelectedIndex < 0) // pasted lower case
                {
                    int caret = _roomInput.SelectionStart;
                    _roomInput.Text = upper;
                    _roomInput.SelectionStart = caret;
                    return;
                }
                UpdateControls();
            };
            _roomInput.SelectionChangeCommitted += (_, _) => OnWorldPicked();
            _forgetButton = Theme.MakeButton("Forget", Theme.ButtonKind.Neutral);
            _forgetButton.Dock = DockStyle.None;
            _forgetButton.Size = new Size(76, 28);
            _forgetButton.Margin = new Padding(6, 0, 0, 0);
            _forgetButton.Click += (_, _) =>
            {
                _worlds.Forget(_roomInput.Text);
                RefreshWorldList();
                UpdateControls();
            };
            var roomRow = new TableLayoutPanel
            {
                ColumnCount = 2,
                AutoSize = true,
                Dock = DockStyle.Fill,
                Margin = new Padding(0, 0, 0, 4)
            };
            roomRow.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100f));
            roomRow.ColumnStyles.Add(new ColumnStyle(SizeType.AutoSize));
            roomRow.Controls.Add(_roomInput, 0, 0);
            roomRow.Controls.Add(_forgetButton, 1, 0);
            var roomHint = Theme.MakeLabel("6 letters or numbers, shared by everyone in the world.", Theme.Caption, Theme.TextSecondary);
            roomHint.Margin = new Padding(0, 0, 0, 12);

            // The avatar: a list and its battle picture.
            var avatarCaption = Theme.MakeLabel("Avatar", Theme.Caption, Theme.TextSecondary);
            var avatarRow = new TableLayoutPanel
            {
                ColumnCount = 2,
                AutoSize = true,
                Dock = DockStyle.Fill,
                Margin = new Padding(0, 0, 0, 12)
            };
            avatarRow.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100f));
            avatarRow.ColumnStyles.Add(new ColumnStyle(SizeType.AutoSize));
            _avatarPicker = new ComboBox
            {
                DropDownStyle = ComboBoxStyle.DropDownList,
                FlatStyle = FlatStyle.Flat,
                BackColor = Theme.Input,
                ForeColor = Theme.TextPrimary,
                Font = Theme.Body,
                Dock = DockStyle.Top,
                Margin = new Padding(0, 0, 8, 4)
            };
            _avatarPicker.Items.AddRange(_avatars.ToArray());
            _avatarPicker.SelectedItem = avatar;
            _avatarPreview = new PictureBox
            {
                Size = new Size(80, 80),
                SizeMode = PictureBoxSizeMode.CenterImage,
                BackColor = Theme.Input,
                Margin = new Padding(0),
                Image = Avatars.LoadPreview(avatar)
            };
            var avatarHint = Theme.MakeLabel("How the others see you. Pick it before joining or creating a world.",
                Theme.Caption, Theme.TextSecondary);
            avatarHint.MaximumSize = new Size(contentWidth - 128, 0);
            avatarHint.Margin = new Padding(0);
            var avatarLeft = new TableLayoutPanel { ColumnCount = 1, AutoSize = true, Dock = DockStyle.Fill, Margin = new Padding(0) };
            avatarLeft.Controls.Add(_avatarPicker);
            avatarLeft.Controls.Add(avatarHint);
            avatarRow.Controls.Add(avatarLeft, 0, 0);
            avatarRow.Controls.Add(_avatarPreview, 1, 0);
            _avatarPicker.SelectedIndexChanged += (_, _) => OnAvatarPicked();

            _hostOptions = new TableLayoutPanel
            {
                ColumnCount = 1,
                AutoSize = true,
                Dock = DockStyle.Fill,
                Margin = new Padding(0, 0, 0, 12),
                Visible = false
            };
            _hostOptions.Controls.Add(Theme.MakeLabel("Players", Theme.Caption, Theme.TextSecondary));
            var sizePicker = new TableLayoutPanel
            {
                ColumnCount = _partySizeButtons.Length,
                AutoSize = true,
                Dock = DockStyle.Fill,
                Margin = new Padding(0, 0, 0, 4)
            };
            _partySize = Math.Max(2, Math.Min(Mailbox.MaxPlayers, savedParty));
            for (int i = 0; i < _partySizeButtons.Length; i++)
            {
                int size = i + 2;
                sizePicker.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100f / _partySizeButtons.Length));
                var b = Theme.MakeButton(size.ToString(), Theme.ButtonKind.SegmentOff);
                b.Height = 30;
                b.Margin = new Padding(i == 0 ? 0 : 2, 0, i == _partySizeButtons.Length - 1 ? 0 : 2, 0);
                b.Click += (_, _) => { _partySize = size; UpdateControls(); };
                _partySizeButtons[i] = b;
                sizePicker.Controls.Add(b, i, 0);
            }
            _hostOptions.Controls.Add(sizePicker);
            var playersHint = Theme.MakeLabel(
                "The adventure starts, and the world locks to these players, once everyone is ready in their bedroom.",
                Theme.Caption, Theme.TextSecondary);
            playersHint.MaximumSize = new Size(contentWidth - 28, 0);
            playersHint.Margin = new Padding(0);
            _hostOptions.Controls.Add(playersHint);

            // Milestone 5 challenge rules (docs/MILESTONE5_PLAN.md §1): set
            // here by the host, fixed once the adventure starts.
            var rulesCaption = Theme.MakeLabel("Challenge rules", Theme.Caption, Theme.TextSecondary);
            rulesCaption.Margin = new Padding(0, 12, 0, 2);
            _hostOptions.Controls.Add(rulesCaption);
            ComboBox MakePicker(string[] items)
            {
                var c = new ComboBox
                {
                    DropDownStyle = ComboBoxStyle.DropDownList,
                    FlatStyle = FlatStyle.Flat,
                    BackColor = Theme.Input,
                    ForeColor = Theme.TextPrimary,
                    Font = Theme.Body,
                    Dock = DockStyle.Fill,
                    Margin = new Padding(0, 0, 0, 4)
                };
                c.Items.AddRange(items);
                return c;
            }
            CheckBox MakeCheck(string text)
            {
                return new CheckBox
                {
                    Text = text,
                    AutoSize = true,
                    ForeColor = Theme.TextPrimary,
                    Font = Theme.Body,
                    Margin = new Padding(0, 0, 0, 2)
                };
            }
            _hostOptions.Controls.Add(Theme.MakeLabel("Random encounters", Theme.Caption, Theme.TextSecondary));
            _randomPicker = MakePicker(ChallengeRules.RandomModes);
            _hostOptions.Controls.Add(_randomPicker);
            _legendsCheck = MakeCheck("Legendaries in the wild");
            _abilitiesCheck = MakeCheck("Random abilities");
            _nuzlockeCheck = MakeCheck("Nuzlocke (with Ghost Mode)");
            _hostOptions.Controls.Add(_legendsCheck);
            _hostOptions.Controls.Add(_abilitiesCheck);
            _hostOptions.Controls.Add(_nuzlockeCheck);
            _hostOptions.Controls.Add(Theme.MakeLabel("Soul-Link", Theme.Caption, Theme.TextSecondary));
            _soulLinkPicker = MakePicker(ChallengeRules.SoulLinkModes);
            _hostOptions.Controls.Add(_soulLinkPicker);
            _monolockeCheck = MakeCheck("Monolocke");
            _levelCapCheck = MakeCheck("Level cap (next boss's ace)");
            _hostOptions.Controls.Add(_monolockeCheck);
            _hostOptions.Controls.Add(_levelCapCheck);
            _randomItemsCheck = MakeCheck("Random items (item balls)");
            _hostOptions.Controls.Add(_randomItemsCheck);
            var rulesHint = Theme.MakeLabel(
                "Everyone in the world plays with these rules. They can't change once the adventure starts.",
                Theme.Caption, Theme.TextSecondary);
            rulesHint.MaximumSize = new Size(contentWidth - 28, 0);
            rulesHint.Margin = new Padding(0);
            _hostOptions.Controls.Add(rulesHint);
            ShowRules(_challengeRules);
            _randomPicker.SelectedIndexChanged += (_, _) => OnRulesEdited();
            _soulLinkPicker.SelectedIndexChanged += (_, _) => OnRulesEdited();
            foreach (var c in new[] { _legendsCheck, _abilitiesCheck, _nuzlockeCheck, _monolockeCheck, _levelCapCheck, _randomItemsCheck })
                c.CheckedChanged += (_, _) => OnRulesEdited();

            _primaryButton = Theme.MakeButton("Join world", Theme.ButtonKind.Primary);
            _primaryButton.Click += (_, _) => StartConnection(_hostMode ? Wire.ActionCreate : Wire.ActionJoin);
            _leaveButton = Theme.MakeButton("Leave world", Theme.ButtonKind.Danger);
            _leaveButton.Visible = false;
            _leaveButton.Click += (_, _) => DisconnectFromServer("You left the world.", isError: false);

            roomCard.Controls.Add(segments);
            roomCard.Controls.Add(roomCaption);
            roomCard.Controls.Add(roomRow);
            roomCard.Controls.Add(roomHint);
            roomCard.Controls.Add(avatarCaption);
            roomCard.Controls.Add(avatarRow);
            roomCard.Controls.Add(_hostOptions);
            roomCard.Controls.Add(_primaryButton);
            roomCard.Controls.Add(_leaveButton);

            // --- Session card: labelled key/value grid ---
            var sessionCard = Theme.MakeCard();
            sessionCard.Controls.Add(Theme.MakeLabel("Session", Theme.BodyBold, Theme.TextPrimary));
            var grid = new TableLayoutPanel
            {
                ColumnCount = 2,
                AutoSize = true,
                Dock = DockStyle.Fill,
                Margin = new Padding(0, 4, 0, 0)
            };
            grid.ColumnStyles.Add(new ColumnStyle(SizeType.Absolute, 120f));
            grid.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100f));
            Label AddRow(string name)
            {
                var key = Theme.MakeLabel(name, Theme.Body, Theme.TextSecondary);
                var value = Theme.MakeLabel("—", Theme.Body, Theme.TextPrimary);
                value.MaximumSize = new Size(contentWidth - 150, 0);
                grid.Controls.Add(key);
                grid.Controls.Add(value);
                return value;
            }
            _gameValue = AddRow("Game");
            _trainerValue = AddRow("Trainer ID");
            _bedroomValue = AddRow("Ready in bedroom");
            _playersValue = AddRow("Players online");
            _adventureValue = AddRow("Adventure");
            _rulesValue = AddRow("Rules");
            _gymValue = AddRow("Gym gate");
            _saveValue = AddRow("Save");
            _locationValue = AddRow("You");
            _othersValue = AddRow("Players");
            sessionCard.Controls.Add(grid);
            _allowAheadButton = Theme.MakeButton("Let others go ahead: off", Theme.ButtonKind.Neutral);
            _allowAheadButton.Height = 28;
            _allowAheadButton.Margin = new Padding(0, 8, 0, 0);
            _allowAheadButton.Click += (_, _) =>
            {
                // Volatile: the receive thread builds packets with this flag.
                Volatile.Write(ref _allowAhead, !Volatile.Read(ref _allowAhead));
                _allowAheadButton.Text = Volatile.Read(ref _allowAhead) ? "Let others go ahead: on" : "Let others go ahead: off";
            };
            sessionCard.Controls.Add(_allowAheadButton);
            _restoreButton = Theme.MakeButton("Restore a backup…", Theme.ButtonKind.Neutral);
            _restoreButton.Height = 28;
            _restoreButton.Margin = new Padding(0, 8, 0, 0);
            _restoreButton.Click += (_, _) => ShowRestoreDialog();
            sessionCard.Controls.Add(_restoreButton);

            // --- Server (set once, so it sits at the bottom) ---
            var serverCard = Theme.MakeCard();
            serverCard.Margin = new Padding(0);
            serverCard.Controls.Add(Theme.MakeLabel("Server address (host:port)", Theme.Caption, Theme.TextSecondary));
            _serverInput = new TextBox
            {
                Text = savedHost,
                BackColor = Theme.Input,
                ForeColor = Theme.TextPrimary,
                BorderStyle = BorderStyle.FixedSingle,
                Dock = DockStyle.Fill,
                Margin = new Padding(0)
            };
            serverCard.Controls.Add(_serverInput);
#if !COOP_SHIP // the debug tools: compiled out of releases (docs/RELEASE_PLAN.md §11 D3)
            _diagToggle = Theme.MakeButton("Show diagnostics", Theme.ButtonKind.Neutral);
            _diagToggle.Height = 28;
            _diagToggle.Margin = new Padding(0, 10, 0, 0);
            _diagToggle.Click += (_, _) =>
            {
                _diagCard!.Visible = !_diagCard.Visible;
                _diagToggle.Text = _diagCard.Visible ? "Hide diagnostics" : "Show diagnostics";
            };
            serverCard.Controls.Add(_diagToggle);

            // --- Diagnostics (DEVELOPMENT ONLY, docs/DEBUG_TOOLS.md) ---
            _diagCard = Theme.MakeCard();
            _diagCard.Margin = new Padding(0, 12, 0, 0);
            _diagCard.Visible = false;
            _diagCard.Controls.Add(Theme.MakeLabel("Diagnostics", Theme.BodyBold, Theme.TextPrimary));
            var diagGrid = new TableLayoutPanel
            {
                ColumnCount = 2,
                AutoSize = true,
                Dock = DockStyle.Fill,
                Margin = new Padding(0, 4, 0, 8)
            };
            diagGrid.ColumnStyles.Add(new ColumnStyle(SizeType.Absolute, 120f));
            diagGrid.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100f));
            Label AddDiagRow(string name)
            {
                var key = Theme.MakeLabel(name, Theme.Body, Theme.TextSecondary);
                var value = Theme.MakeLabel("—", Theme.Body, Theme.TextPrimary);
                value.MaximumSize = new Size(contentWidth - 150, 0);
                diagGrid.Controls.Add(key);
                diagGrid.Controls.Add(value);
                return value;
            }
            _diagFieldValue = AddDiagRow("Field code");
            _diagTexturesValue = AddDiagRow("Textures");
            _diagModelsValue = AddDiagRow("Models");
            _diagSequencesValue = AddDiagRow("Animations");
            _diagObjectsValue = AddDiagRow("Map objects");
            _diagGhostValue = AddDiagRow("Test ghost");
            _diagHeightValue = AddDiagRow("Ghost height");
            _diagPlayerStateValue = AddDiagRow("You are");
            _diagCommandValue = AddDiagRow("Last command");
            _diagCard.Controls.Add(diagGrid);

            var buttons = new List<ThemedButton>();
            TableLayoutPanel AddActionRow((string label, byte cmd)[] actions, int topMargin)
            {
                var row = new TableLayoutPanel
                {
                    ColumnCount = actions.Length,
                    AutoSize = true,
                    Dock = DockStyle.Fill,
                    Margin = new Padding(0, topMargin, 0, 0)
                };
                for (int i = 0; i < actions.Length; i++)
                {
                    byte cmd = actions[i].cmd;
                    string name = actions[i].label;
                    row.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100f / actions.Length));
                    var b = Theme.MakeButton(name, Theme.ButtonKind.Neutral);
                    b.Height = 30;
                    b.Margin = new Padding(i == 0 ? 0 : 2, 0, i == actions.Length - 1 ? 0 : 2, 0);
                    b.Click += (_, _) => QueueDebugCommand(cmd, 0, name);
                    buttons.Add(b);
                    row.Controls.Add(b, i, 0);
                }
                _diagCard.Controls.Add(row);
                return row;
            }
            AddActionRow(new[]
            {
                ("Spawn", Mailbox.DebugSpawnGhost),
                ("Step", Mailbox.DebugStepGhost),
                ("Emote", Mailbox.DebugEmoteGhost),
                ("Remove", Mailbox.DebugRemoveGhost),
            }, 0);
            AddActionRow(new[]
            {
                ("Bike", Mailbox.DebugToggleBike),
                ("Surf", Mailbox.DebugSurf),
                ("Fish", Mailbox.DebugFish),
            }, 4);

            var warpRow = new TableLayoutPanel
            {
                ColumnCount = 2,
                AutoSize = true,
                Dock = DockStyle.Fill,
                Margin = new Padding(0, 4, 0, 0)
            };
            warpRow.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100f));
            warpRow.ColumnStyles.Add(new ColumnStyle(SizeType.Absolute, 80f));
            _warpPicker = new ComboBox
            {
                DropDownStyle = ComboBoxStyle.DropDownList,
                FlatStyle = FlatStyle.Flat,
                BackColor = Theme.Input,
                ForeColor = Theme.TextPrimary,
                Font = Theme.Body,
                Dock = DockStyle.Fill,
                Margin = new Padding(0, 3, 4, 0)
            };
            _warpPicker.Items.AddRange(Mailbox.WarpPresets);
            _warpPicker.SelectedIndex = 0;
            warpRow.Controls.Add(_warpPicker, 0, 0);
            var warpButton = Theme.MakeButton("Warp", Theme.ButtonKind.Neutral);
            warpButton.Height = 30;
            warpButton.Dock = DockStyle.Fill;
            warpButton.Margin = new Padding(0);
            warpButton.Click += (_, _) =>
            {
                int preset = Math.Max(0, _warpPicker.SelectedIndex);
                QueueDebugCommand(Mailbox.DebugWarp, (byte)preset, "Warp to " + Mailbox.WarpPresets[preset]);
            };
            buttons.Add(warpButton);
            warpRow.Controls.Add(warpButton, 1, 0);
            _diagCard.Controls.Add(warpRow);
            _diagButtons = buttons.ToArray();

            var diagHint = Theme.MakeLabel(
                "Developer tools. The test ghost spawns in front of you with your sprite; Step walks it the way you face. "
                + "Surf and Fish need you to face water (no Pokemon needed to surf). Reeling in a bite starts a real battle. "
                + "Warping skips the bedroom lock and story events.",
                Theme.Caption, Theme.TextSecondary);
            diagHint.MaximumSize = new Size(contentWidth - 28, 0);
            diagHint.Margin = new Padding(0, 8, 0, 0);
            _diagCard.Controls.Add(diagHint);
#endif

            root.Controls.Add(header);
            root.Controls.Add(_messageLabel);
            root.Controls.Add(roomCard);
            root.Controls.Add(sessionCard);
            root.Controls.Add(serverCard);
#if !COOP_SHIP // the debug tools: compiled out of releases (docs/RELEASE_PLAN.md §11 D3)
            root.Controls.Add(_diagCard);
#endif
            Controls.Add(root);
            AcceptButton = _primaryButton; // Enter in the room code submits
            RefreshWorldList(); // after every control exists: it sets the text (TextChanged -> UpdateControls)
            ActiveControl = _roomInput;    // type the code straight away

            _serverEndpoint = new IPEndPoint(IPAddress.Loopback, 7777);
            _udpClient = new UdpClient(0);
            // Blocking receive with a timeout keeps the receive thread cheap
            // without a busy poll; UDP sends never block in practice.
            _udpClient.Client.ReceiveTimeout = 200;

            // Runs on the UI thread (same thread as UpdateValues). While the
            // emulator is paused no frames run, so this keeps the relay from
            // timing us out. It never touches emulated memory.
            _keepAliveTimer = new System.Windows.Forms.Timer { Interval = KeepAliveIntervalMs };
            _keepAliveTimer.Tick += (_, _) => SendKeepAliveIfPaused();
            _keepAliveTimer.Start();

            if (_mailboxOffset == null) SetStatus(_offsetsError, isError: true);
            UpdateControls();
#if !COOP_SHIP // the debug tools: compiled out of releases (docs/RELEASE_PLAN.md §11 D3)
            StartDevAutoJoin();
#endif
        }

#if !COOP_SHIP // the debug tools: compiled out of releases (docs/RELEASE_PLAN.md §11 D3)
        // DEVELOPMENT ONLY (docs/DEBUG_TOOLS.md): COOP_BRIDGE_AUTOJOIN="host:port|ROOM"
        // joins that room as soon as the tool is open, and again after a failed
        // attempt, until connected (dev/run_link_e2e.sh). Never set by users.
        // DEVELOPMENT ONLY (docs/DEBUG_TOOLS.md): COOP_BRIDGE_LEAVE_IN_BATTLE=seconds
        // leaves the room (as the Leave room button does) once the game has
        // been in a battle that long, once (docs/KNOWN_RISKS.md #52 repro).
        private double _devLeaveInBattle = -1;
        private DateTime? _devBattleSinceUtc;

        private void DevMaybeLeaveInBattle()
        {
            if (_devLeaveInBattle < 0) return;
            if (_localPlayer[Mailbox.PsActivity] != 2) { _devBattleSinceUtc = null; return; }
            _devBattleSinceUtc ??= DateTime.UtcNow;
            if ((DateTime.UtcNow - _devBattleSinceUtc.Value).TotalSeconds < _devLeaveInBattle) return;
            _devLeaveInBattle = -1;
            DisconnectFromServer("Left the world (COOP_BRIDGE_LEAVE_IN_BATTLE).", isError: false);
        }

        private void StartDevAutoJoin()
        {
            if (double.TryParse(Environment.GetEnvironmentVariable("COOP_BRIDGE_LEAVE_IN_BATTLE"),
                    System.Globalization.NumberStyles.Float, System.Globalization.CultureInfo.InvariantCulture, out double leave))
                _devLeaveInBattle = leave;
            string? autoJoin = Environment.GetEnvironmentVariable("COOP_BRIDGE_AUTOJOIN");
            string[] parts = (autoJoin ?? string.Empty).Split('|');
            if (parts.Length != 2) return;
            var timer = new System.Windows.Forms.Timer { Interval = 1000 };
            timer.Tick += (_, _) =>
            {
                if (_link == LinkState.Connected) { timer.Stop(); return; }
                if (_link != LinkState.Offline) return;
                _serverInput.Text = parts[0];
                _roomInput.Text = parts[1];
                StartConnection(Wire.ActionJoin);
            };
            timer.Start();
        }
#endif

        // ------------------------------------------------------------------
        // UI state (UI thread)
        // ------------------------------------------------------------------

        private static bool IsRoomCodeChar(char c) => (c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9');

        private bool RoomCodeIsValid => Regex.IsMatch(_roomInput.Text, "^[A-Z0-9]{6}$");

        // The drop-down's list from the saved worlds, keeping what is typed.
        private void RefreshWorldList()
        {
            string text = _roomInput.Text;
            _roomInput.BeginUpdate();
            _roomInput.Items.Clear();
            foreach (WorldEntry entry in _worlds.Entries) _roomInput.Items.Add(entry);
            _roomInput.EndUpdate();
            _roomInput.Text = text;
        }

        // A world from the list: its code (the entry's text is longer), its server, and Join:
        // a world that exists can't be created again.
        private void OnWorldPicked()
        {
            if (!(_roomInput.SelectedItem is WorldEntry entry) || _link != LinkState.Offline) return;
            BeginInvoke((Action)(() =>
            {
                _roomInput.SelectedIndex = -1;
                _roomInput.Text = entry.Code;
                _roomInput.SelectionStart = entry.Code.Length;
                if (entry.Server.Length > 0) _serverInput.Text = entry.Server;
                SetHostMode(false);
            }));
        }

        private void SetHostMode(bool host)
        {
            if (_link != LinkState.Offline) return;
            _hostMode = host;
            UpdateControls();
        }

        private void SetStatus(string message, bool isError)
        {
#if !COOP_SHIP // the debug tools: compiled out of releases (docs/RELEASE_PLAN.md §11 D3)
            // DEVELOPMENT ONLY (docs/DEBUG_TOOLS.md): COOP_BRIDGE_STATUS_LOG=<file> appends each
            // message the bridge shows, for probes (dev/test_protocol_mismatch.sh).
            string? statusLog = Environment.GetEnvironmentVariable("COOP_BRIDGE_STATUS_LOG");
            if (!string.IsNullOrEmpty(statusLog))
            {
                try { File.AppendAllText(statusLog, $"{(isError ? "error" : "info")}: {message}\n"); }
                catch (IOException) { }
            }
#endif
            _message = message;
            _messageIsError = isError;
            _lastInfoRefreshUtc = DateTime.MinValue; // show it on the next refresh
            UpdateControls();
        }

        /// <summary>Single place that maps the link state onto every control.</summary>
        private void UpdateControls()
        {
            bool offline = _link == LinkState.Offline;

            _joinModeButton.Kind = _hostMode ? Theme.ButtonKind.SegmentOff : Theme.ButtonKind.SegmentOn;
            _hostModeButton.Kind = _hostMode ? Theme.ButtonKind.SegmentOn : Theme.ButtonKind.SegmentOff;
            _joinModeButton.Enabled = _hostModeButton.Enabled = offline;
            // While connecting/connected only the room code and the action matter.
            _modeSegments.Visible = offline;
            _roomInput.Enabled = offline;
            _forgetButton.Visible = offline && _worlds.Find(_roomInput.Text) != null;
            _serverInput.Enabled = offline;
            if (_avatarPicker != null) _avatarPicker.Enabled = offline;
            for (int i = 0; i < _partySizeButtons.Length; i++)
            {
                _partySizeButtons[i].Kind = i + 2 == _partySize ? Theme.ButtonKind.SegmentOn : Theme.ButtonKind.SegmentOff;
                _partySizeButtons[i].Enabled = offline;
            }
            if (_randomPicker != null) // UpdateControls can run while the form is still being built
            {
                bool random = (_challengeRules & ChallengeRules.RandomMask) != 0;
                bool nuzlocke = (_challengeRules & ChallengeRules.Nuzlocke) != 0;
                _randomPicker.Enabled = _nuzlockeCheck.Enabled = _monolockeCheck.Enabled = _levelCapCheck.Enabled = _randomItemsCheck.Enabled = offline;
                _legendsCheck.Enabled = _abilitiesCheck.Enabled = offline && random;
                _soulLinkPicker.Enabled = offline && nuzlocke;
            }
            _hostOptions.Visible = offline && _hostMode;

            if (_restoreButton != null) _restoreButton.Visible = offline && _saveStateApi != null;
            _primaryButton.Visible = _link != LinkState.Connected;
            _leaveButton.Visible = _link == LinkState.Connected;
            _primaryButton.Text = _link switch
            {
                LinkState.Resolving or LinkState.Connecting => "Connecting…",
                _ => _hostMode ? "Create world" : "Join world"
            };
            _primaryButton.Enabled = offline && RoomCodeIsValid && _mailboxOffset != null;

            (string pillText, Color pillColor) = _link switch
            {
                LinkState.Connected => (_assignedSlot < Mailbox.MaxPlayers
                    ? $"●  Connected · Player {_assignedSlot + 1}" : "●  Connected", Theme.Connected),
                LinkState.Resolving or LinkState.Connecting => ("●  Connecting", Theme.Connecting),
                _ => ("●  Offline", Theme.Offline)
            };
            _statusPill.Text = pillText;
            _statusPill.ForeColor = pillColor;
            _statusPill.BackColor = Blend(pillColor, Theme.Window, 0.18f);

            _messageLabel.Text = _message;
            _messageLabel.ForeColor = _messageIsError ? Theme.Error : Theme.TextSecondary;
        }

        private static Color Blend(Color fg, Color bg, float amount) => Color.FromArgb(
            (int)(bg.R + (fg.R - bg.R) * amount),
            (int)(bg.G + (fg.G - bg.G) * amount),
            (int)(bg.B + (fg.B - bg.B) * amount));

        private static void SetValue(Label label, string text, Color? color = null)
        {
            if (label.Text != text) label.Text = text;
            Color c = color ?? Theme.TextPrimary;
            if (label.ForeColor != c) label.ForeColor = c;
        }

        // ------------------------------------------------------------------
        // Connection lifecycle (UI thread)
        // ------------------------------------------------------------------

        private async void StartConnection(byte initialAction)
        {
            if (_link != LinkState.Offline || !RoomCodeIsValid) return;

            string hostText = _serverInput.Text.Trim();
            string roomText = _roomInput.Text;
            if (hostText.Length == 0)
            {
                // No address is built in (Milestone 6 §4.3): the world's host shares theirs.
                SetStatus("Enter the server address (host:port) your world's host gave you, below.", isError: true);
                _serverInput.Focus();
                return;
            }

            if (_mailboxOffset == null)
            {
                SetStatus(_offsetsError, isError: true);
                return;
            }

            // DNS can take seconds; never block the emulator/UI thread on it.
            _link = LinkState.Resolving;
            SetStatus($"Looking up {hostText}…", isError: false);
            IPEndPoint? resolvedEp = await Task.Run(() => TryResolveServerEndpoint(hostText));
            if (_closing) return;

            if (resolvedEp == null)
            {
                _link = LinkState.Offline;
                SetStatus($"Couldn't find the server \"{hostText}\". Use host:port, e.g. 127.0.0.1:7777.", isError: true);
                return;
            }

            if (!resolvedEp.Equals(_serverEndpoint)) Interlocked.Exchange(ref _cookie, 0);
            _serverEndpoint = resolvedEp;
            _roomCodeBytes = Encoding.ASCII.GetBytes(roomText);
            _activeRoomString = roomText;
            _activeServerText = hostText;
            _activeAction = initialAction;
            _assignedSlot = Mailbox.PlayerIdNone;
            _lastServerPacketUtc = DateTime.UtcNow;
            BeginNewSession();

            // Generate a fresh cryptographic host token on every Create Room attempt
            _hostToken = initialAction == Wire.ActionCreate ? GenerateCryptoToken() : 0;
            _link = LinkState.Connecting;
            SetStatus(initialAction == Wire.ActionCreate
                ? $"Creating world {roomText}…"
                : $"Joining world {roomText}…", isError: false);

            SaveNetConfig(hostText, _partySize, _playerSecret, _challengeRules, _avatarKey);
            StartReceiveThread();
        }

        // ------------------------------------------------------------------
        // Emergency backups (Backups.cs, docs/GAME_REBALANCE_UPDATE.md §23)
        // ------------------------------------------------------------------

        // Every Backups.Interval in a room, only while the player stands in
        // the overworld with nothing running: no field task or message, no
        // trade or link session (exchange_ptr), past the bedroom.
        private void TryTakeBackup(uint mb)
        {
            if (_saveStateApi == null || _slotKey == null || _slotState != SlotState.Ready) return;
            DateTime now = DateTime.UtcNow;
            if (_nextBackupUtc == DateTime.MaxValue) _nextBackupUtc = now + Backups.Interval;
            if (now < _nextBackupUtc) return;
            if (_localPlayer[Mailbox.PsActivity] != 1 || _localPlayer[Mailbox.PsIsActive] != 1
                || _localPlayer[Mailbox.PsFieldBusy] != 0 || _localPlayer[Mailbox.PsSaveStatus] != 2
                || _memoryApi!.ReadU32(mb + Mailbox.ExchangePtr, MemoryDomain) != 0)
                return; // try again next frame
            _nextBackupUtc = now + Backups.Interval;
            string? path = _backups.NextPath(_slotKey);
            if (path == null) return;
            try
            {
                _saveStateApi.Save(path, suppressOSD: true);
                _backups.Prune(_slotKey);
            }
            catch (Exception ex) when (ex is IOException || ex is UnauthorizedAccessException)
            {
                SetStatus("Could not write a backup: " + ex.Message, isError: true);
            }
        }

        // Outside a room only: the restored game still believes it is in the
        // room, so the ROM's disconnect flow saves it and returns to the title
        // screen, and joining the room again keeps that save (SaveSlots).
        private void ShowRestoreDialog()
        {
            if (_link != LinkState.Offline || _saveStateApi == null) return;
            string room = _saveSlots.Owner;
            var files = room.Length > 0 ? _backups.List(room) : new List<FileInfo>();
            if (files.Count == 0)
            {
                MessageBox.Show(this, "There are no backups of your last world yet. One is taken every few minutes while you play in a world.",
                    "Restore a backup", MessageBoxButtons.OK, MessageBoxIcon.Information);
                return;
            }
            using var dialog = new Form
            {
                Text = $"Restore a backup ({room.Split('-')[0]})",
                FormBorderStyle = FormBorderStyle.FixedDialog,
                StartPosition = FormStartPosition.CenterParent,
                MinimizeBox = false,
                MaximizeBox = false,
                ClientSize = new Size(360, 300),
            };
            var list = new ListBox { Dock = DockStyle.Fill };
            foreach (FileInfo f in files) list.Items.Add(f.LastWriteTime.ToString("ddd d MMM, HH:mm:ss"));
            list.SelectedIndex = 0;
            var buttons = new FlowLayoutPanel { Dock = DockStyle.Bottom, FlowDirection = FlowDirection.RightToLeft, Height = 40 };
            var restore = new Button { Text = "Restore", DialogResult = DialogResult.OK };
            var cancel = new Button { Text = "Cancel", DialogResult = DialogResult.Cancel };
            buttons.Controls.Add(cancel);
            buttons.Controls.Add(restore);
            dialog.Controls.Add(list);
            dialog.Controls.Add(buttons);
            dialog.AcceptButton = restore;
            dialog.CancelButton = cancel;
            if (dialog.ShowDialog(this) != DialogResult.OK || list.SelectedIndex < 0) return;
            FileInfo chosen = files[list.SelectedIndex];
            string warning = $"Restore the backup from {chosen.LastWriteTime:HH:mm:ss}?\n\n"
                + "Use this only after a crash or a game-breaking bug that your save can't fix. "
                + "Your game goes back to that moment: everything since then is lost, and a trade "
                + "made since then can leave the traded Pokémon in both games.\n\n"
                + "The game then saves and returns to the title screen. Join your world again to continue.";
            if (MessageBox.Show(this, warning, "Restore a backup", MessageBoxButtons.YesNo, MessageBoxIcon.Warning) != DialogResult.Yes)
                return;
            _pendingRestore = chosen.FullName;
        }

        private void DisconnectFromServer(string reasonMessage, bool isError)
        {
            if (_link == LinkState.Connecting || _link == LinkState.Connected)
            {
                // One-shot LEAVE so the relay frees the slot immediately
                SendPacket(Wire.ActionLeave);
            }

            _link = LinkState.Offline;
            _nextBackupUtc = DateTime.MaxValue;
            _assignedSlot = Mailbox.PlayerIdNone;
            _activeRoomString = string.Empty;
            _clearPayloadPending = true;
            BeginNewSession();
            SetStatus(reasonMessage, isError);
        }

        private void BeginNewSession()
        {
            Interlocked.Increment(ref _sessionId);
            _slotKey = null;
            _slotState = SlotState.None;
            lock (_packetLock)
            {
                _latestServerPacket = null;
            }
        }

        public override void Restart()
        {
            base.Restart();
            _clearPayloadPending = true;
            StartReceiveThread();
        }

        protected override void OnFormClosing(FormClosingEventArgs e)
        {
            // Never touch emulated memory from here: on emulator exit the core
            // is already disposed (SIGSEGV in wbx_activate_host). The ROM's
            // bridge watchdog notices the missing heartbeat within ~2 seconds.
            _closing = true;
            _keepAliveTimer.Stop();
            if (_link == LinkState.Connecting || _link == LinkState.Connected)
            {
                SendPacket(Wire.ActionLeave);
            }
            _running = false;
            _udpClient.Close();
            base.OnFormClosing(e);
        }

        // ------------------------------------------------------------------
        // Networking
        // ------------------------------------------------------------------

        private void StartReceiveThread()
        {
            if (_running || _closing) return;
            _running = true;

            _recvThread = new Thread(ReceiveLoop)
            {
                IsBackground = true,
                Name = "CoopBridge UDP receive"
            };
            _recvThread.Start();
        }

        private void ReceiveLoop()
        {
            while (_running)
            {
                try
                {
                    var remoteEp = new IPEndPoint(IPAddress.Any, 0);
                    byte[] data = _udpClient.Receive(ref remoteEp);

                    // Only the relay we are talking to, only this protocol.
                    if (!remoteEp.Equals(_serverEndpoint)) continue;
                    if (data.Length < Wire.ErrorPacketSize) continue;
                    if (BitConverter.ToUInt32(data, 0) != Mailbox.Magic) continue;
                    // Only this protocol, except the relay's "other version" refusal, which
                    // carries its own version and is read as a rejection.
                    if (BitConverter.ToUInt16(data, 4) != Wire.ProtocolVersion && data[12] != Wire.StatusProtocol) continue;

                    byte status = data[12];
                    if (status == Wire.StatusChallenge)
                    {
                        // Handled here so a following broadcast can never
                        // overwrite it; the next send echoes the cookie.
                        if (data.Length >= Wire.ChallengePacketSize)
                        {
                            Interlocked.Exchange(ref _cookie, BitConverter.ToInt64(data, 16));
                            Interlocked.Exchange(ref _lastChallengeUtcTicks, DateTime.UtcNow.Ticks);
                        }
                        continue;
                    }
                    if (status == Wire.StatusOk && data.Length < Wire.ServerPacketSize) continue;

                    lock (_packetLock)
                    {
                        _latestServerPacket = data;
                        _latestServerPacketSession = Volatile.Read(ref _sessionId);
                    }
                    // Every link trailer counts (the latest packet alone could skip one).
                    if (status == Wire.StatusOk) _linkCarrier.OnServerPacket(data, Wire.ServerPacketSize, Mailbox.TradeInSize);
                }
                catch (SocketException)
                {
                    // Receive timeouts, and ICMP "port unreachable" (relay not
                    // up yet) surface here. Neither is fatal: keep listening.
                }
                catch (ObjectDisposedException)
                {
                    break;
                }
            }
        }

        private void SendPacket(byte action)
        {
            try
            {
                byte[] tx = BuildClientPacket(action);
                _udpClient.Send(tx, tx.Length, _serverEndpoint);
            }
            catch (SocketException) { }
            catch (ObjectDisposedException) { }
        }

        private void SendKeepAliveIfPaused()
        {
            if (_link != LinkState.Connecting && _link != LinkState.Connected) return;
            if ((DateTime.UtcNow - _lastFrameUtc).TotalSeconds < PausedThresholdSeconds) return;
            SendPacket(_activeAction);
        }

        private byte[] BuildClientPacket(byte action)
        {
            byte[] tx = new byte[Wire.ClientPacketSize];

            Buffer.BlockCopy(BitConverter.GetBytes(Mailbox.Magic), 0, tx, 0, 4);
            Buffer.BlockCopy(BitConverter.GetBytes(Wire.ProtocolVersion), 0, tx, 4, 2);
            Buffer.BlockCopy(_roomCodeBytes, 0, tx, 6, 6);
            tx[12] = action;
            tx[13] = _assignedSlot;
            tx[14] = (byte)(Volatile.Read(ref _allowAhead) ? 1 : 0); // gym gate: let teammates go ahead
            tx[15] = (byte)_partySize; // honored by the relay only from the host
            Buffer.BlockCopy(BitConverter.GetBytes(_hostToken), 0, tx, 16, 4);
            Buffer.BlockCopy(BitConverter.GetBytes(_badgeMaskLocal), 0, tx, 20, 2);
            Buffer.BlockCopy(BitConverter.GetBytes(++_sequence), 0, tx, 22, 4);
            Buffer.BlockCopy(BitConverter.GetBytes(_heartbeatRom), 0, tx, 26, 4);
            Buffer.BlockCopy(BitConverter.GetBytes(_challengeRules), 0, tx, 30, 2); // honored only from the host
            Buffer.BlockCopy(_localPlayer, 0, tx, 32, Mailbox.PlayerStateSize);
            tx[32 + Mailbox.PsAvatarId] = Volatile.Read(ref _avatarId); // the ROM leaves it 0
            Buffer.BlockCopy(BitConverter.GetBytes(Interlocked.Read(ref _cookie)), 0, tx, 96, 8);
            Buffer.BlockCopy(BitConverter.GetBytes(_playerSecret), 0, tx, 104, 8);
            Buffer.BlockCopy(_sessionOut, 0, tx, 112, Mailbox.SessionOutSize);

            byte[]? link = _linkCarrier.BuildTrailer();
            byte[]? challenge = null;
            lock (_challengeLock)
            {
                if (_challengeUpValid) challenge = (byte[])_challengeUp.Clone();
            }
            lock (_tradeLock)
            {
                if (!_tradeOutValid && link == null && challenge == null) return tx;
                int tradeSize = _tradeOutValid ? Mailbox.TradeOutSize : 0;
                int linkSize = link?.Length ?? 0;
                byte[] full = new byte[Wire.ClientPacketSize + tradeSize + linkSize + (challenge?.Length ?? 0)];
                Buffer.BlockCopy(tx, 0, full, 0, Wire.ClientPacketSize);
                if (_tradeOutValid)
                    Buffer.BlockCopy(_tradeOut, 0, full, Wire.ClientPacketSize, Mailbox.TradeOutSize);
                if (link != null)
                    Buffer.BlockCopy(link, 0, full, Wire.ClientPacketSize + tradeSize, linkSize);
                if (challenge != null) // always last: the relay finds it from the end
                    Buffer.BlockCopy(challenge, 0, full, Wire.ClientPacketSize + tradeSize + linkSize, challenge.Length);
                return full;
            }
        }

        /// <summary>
        /// Main RAM offset of the ROM's CoopChallengeSync (Milestone 5), or
        /// null outside the field. Overlay 131's pinned header holds the
        /// pointer; the header magic, the range and the block's own header are
        /// all checked, since that memory belongs to other overlays in battles
        /// and menus.
        /// </summary>
        private uint? FindChallengeSync()
        {
            if (_localPlayer[Mailbox.PsIsActive] != 1) return null;
            if (_memoryApi!.ReadU32(Mailbox.FieldEntryHeader, MemoryDomain) != Mailbox.FieldEntryMagic) return null;
            uint ptr = _memoryApi.ReadU32(Mailbox.FieldEntryHeader + Mailbox.FieldEntryChallenge, MemoryDomain);
            if (ptr < Mailbox.Overlay131Start || ptr > Mailbox.Overlay131End - Mailbox.ChallengeSyncSize || (ptr & 3) != 0)
                return null;
            uint offset = ptr - Mailbox.MainRamBase;
            if (_memoryApi.ReadU32(offset, MemoryDomain) != Mailbox.ChallengeSyncMagic
                || _memoryApi.ReadU16(offset + 4, MemoryDomain) != Mailbox.ChallengeSyncVersion
                || _memoryApi.ReadU16(offset + 6, MemoryDomain) != Mailbox.ChallengeSyncSize)
                return null;
            return offset;
        }

        /// <summary>
        /// The ROM's challenge report, re-framed by the bridge (magic, version,
        /// checksum), so a stale or half-written block never reaches the relay
        /// looking valid. The previous report stays in use outside the field.
        /// </summary>
        private void ReadChallengeOut()
        {
            if (FindChallengeSync() is not uint sync) return;
            var block = _memoryApi!.ReadByteRange(sync + Mailbox.ChallengeSyncOut, Wire.ChallengeUpSize, MemoryDomain);
            if (block.Count != Wire.ChallengeUpSize) return;
            byte[] up = new byte[Wire.ChallengeUpSize];
            for (int i = 0; i < up.Length; i++) up[i] = block[i];
            Buffer.BlockCopy(BitConverter.GetBytes(Wire.ChallengeMagic), 0, up, 0, 4);
            up[4] = Wire.ChallengeVersion;
            ushort sum = ChallengeRules.Checksum(up, 44);
            Buffer.BlockCopy(BitConverter.GetBytes(sum), 0, up, 44, 2);
            up[46] = up[47] = 0;
            lock (_challengeLock)
            {
                Buffer.BlockCopy(up, 0, _challengeUp, 0, up.Length);
                _challengeUpValid = true;
            }
        }

        /// <summary>
        /// Splits the relay's challenge trailer off the end of a server packet
        /// (it is always last), checks it and remembers it; returns the packet
        /// without it, so the trade and link trailers parse as before.
        /// </summary>
        private byte[] TakeChallengeDown(byte[] pkt, out byte[]? down)
        {
            down = null;
            if (pkt.Length < Wire.ServerPacketSize + Wire.ChallengeDownSize) return pkt;
            int at = pkt.Length - Wire.ChallengeDownSize;
            if (BitConverter.ToUInt32(pkt, at) != Wire.ChallengeMagic
                || pkt[at + 4] != Wire.ChallengeVersion
                || BitConverter.ToUInt16(pkt, at + 60) != ChallengeRules.Checksum(pkt, 60, at))
                return pkt;
            down = new byte[Wire.ChallengeDownSize];
            Buffer.BlockCopy(pkt, at, down, 0, down.Length);
            // Sanitize: unknown rule bits, a type out of range, unknown room state.
            ushort rules = (ushort)(BitConverter.ToUInt16(down, 8) & Wire.ChallengeRulesValid);
            if (down[5] > ChallengeRules.MonoNone) down[5] = ChallengeRules.MonoNone;
            down[6] &= 0x01;
            Buffer.BlockCopy(BitConverter.GetBytes(rules), 0, down, 8, 2);
            Buffer.BlockCopy(BitConverter.GetBytes(ChallengeRules.Checksum(down, 60)), 0, down, 60, 2);
            byte[] rest = new byte[at];
            Buffer.BlockCopy(pkt, 0, rest, 0, at);
            return rest;
        }

        private void WriteChallengeIn(byte[] down)
        {
            if (FindChallengeSync() is not uint sync) return;
            _memoryApi!.WriteByteRange(sync + Mailbox.ChallengeSyncIn, down, MemoryDomain);
        }

        private static string MapServerStatusToError(byte code, string room, ushort serverProtocol)
        {
            return code switch
            {
                Wire.StatusProtocol => ProtocolMismatchText(serverProtocol),
                1 => $"World {room} doesn't exist. Check the code, or create it instead.",
                2 => $"World {room} already exists. Join it, or pick another code to create a world.",
                3 => $"World {room} is locked to its original players (or this trainer is registered to another install).",
                4 => $"World {room} is full, or the server is at capacity.",
                5 => "World codes are 6 letters or numbers.",
                Wire.StatusMidgameSave => $"World {room} hasn't started yet, so it only takes a New Game (this save is already under way).",
                Wire.StatusSaveOtherRoom => $"This save belongs to another world. Load that world's save, or start a New Game for {room}.",
                Wire.StatusRateLimited => "Too many failed attempts. Wait a few seconds and try again.",
                Wire.StatusRunLost => $"The run in world {room} is over: every player's Pokémon have fallen. Create a new world for a new run.",
                _ => $"The server rejected the connection (code {code})."
            };
        }

        /// The relay and this release speak different protocol versions (docs/RELEASE_PLAN.md §5):
        /// which one is behind decides who updates.
        private static string ProtocolMismatchText(ushort serverProtocol)
        {
            ushort ours = Wire.ProtocolVersion;
            return serverProtocol > ours
                ? $"This server runs relay protocol {serverProtocol}, newer than this GoldLink release (protocol {ours}). "
                  + "Update GoldLink to the release your group uses."
                : $"This server runs relay protocol {serverProtocol}, older than this GoldLink release (protocol {ours}). "
                  + $"The server needs a coop-relay release for protocol {ours}: ask whoever runs it to update.";
        }

        // ------------------------------------------------------------------
        // Emulated memory (emulation thread only: UpdateValues)
        // ------------------------------------------------------------------

        public override void UpdateValues(ToolFormUpdateType type)
        {
            // BizHawk calls this before AND after every frame. Before: only the
            // link's incoming messages (they reach the game a frame sooner).
            if (type == ToolFormUpdateType.PreFrame || type == ToolFormUpdateType.FastPreFrame)
            {
                if (!_closing && _link == LinkState.Connected && _memoryApi != null)
                {
                    try { _linkCarrier.PumpIn(_memoryApi, MemoryDomain, _assignedSlot); }
                    catch (NotImplementedException) { } // core closing
                }
                return;
            }
            if (type != ToolFormUpdateType.PostFrame && type != ToolFormUpdateType.FastPostFrame) return;
            if (_closing) return;
            _lastFrameUtc = DateTime.UtcNow;

            try
            {
                if (_pendingRestore != null)
                {
                    string path = _pendingRestore;
                    _pendingRestore = null;
                    bool loaded = _saveStateApi != null && _saveStateApi.Load(path, suppressOSD: true);
                    SetStatus(loaded ? "Backup restored. The game saves it and returns to the title screen; then join your world again."
                                     : "Could not load that backup.", isError: !loaded);
                    return;
                }
                if (!TryValidateMailbox(out uint mb, out string waitReason))
                {
                    RefreshSession(waitReason, mb: null);
                    return;
                }

                // Tell the ROM we are alive (drives its disconnect watchdog).
                _memoryApi!.WriteU32(mb + Mailbox.HeartbeatBridge, ++_bridgeHeartbeat, MemoryDomain);
                // Our own look: the ROM draws the local player in it from the next sprite change.
                _memoryApi.WriteU8(mb + Mailbox.LocalAvatar, Volatile.Read(ref _avatarId), MemoryDomain);
                ReadLocalPlayerSnapshot(mb);
                UpdateSaveSlot(mb);
#if !COOP_SHIP // the debug tools: compiled out of releases (docs/RELEASE_PLAN.md §11 D3)
                SendPendingDebugCommand(mb);
                RefreshDiagnostics(mb);
#endif

                if (_link != LinkState.Connecting && _link != LinkState.Connected)
                {
                    _linkCarrier.Pump(_memoryApi, MemoryDomain, null, Mailbox.PlayerIdNone);
                    ClearRomConnectionState(mb, _clearPayloadPending);
                    RefreshSession(null, mb);
                    return;
                }

                ProcessLatestServerPacket(mb);
                if (_link == LinkState.Offline) return; // server rejected us
                uint? linkBlock = CoopLink.Find(_memoryApi, MemoryDomain, _memoryApi.ReadU32(mb + Mailbox.ExchangePtr, MemoryDomain));
                _linkCarrier.Pump(_memoryApi, MemoryDomain, linkBlock, _assignedSlot);
                TryTakeBackup(mb);
#if !COOP_SHIP // the debug tools: compiled out of releases (docs/RELEASE_PLAN.md §11 D3)
                DevMaybeLeaveInBattle();
#endif

                DateTime now = DateTime.UtcNow;
                DateTime lastChallenge = new DateTime(Interlocked.Read(ref _lastChallengeUtcTicks), DateTimeKind.Utc);
                bool handshaking = (now - lastChallenge).TotalSeconds <= ServerTimeoutSeconds
                    && (now - _lastServerPacketUtc).TotalSeconds <= HandshakeTimeoutSeconds;

                if (!handshaking && (now - _lastServerPacketUtc).TotalSeconds > ServerTimeoutSeconds)
                {
                    // Never connected: no answer at all. Besides a wrong address or a closed port,
                    // an old relay drops another version's packets without a word.
                    DisconnectFromServer(_link == LinkState.Connecting
                        ? $"The server didn't answer. Check the address and that its UDP port is open; it must also run a relay for protocol {Wire.ProtocolVersion}."
                        : "Lost the connection to the server.", isError: true);
                    ClearRomConnectionState(mb, clearPayload: true);
                    return;
                }

                SendPacket(_activeAction);
                RefreshSession(null, mb);
            }
            catch (NotImplementedException)
            {
                // Core is NullHawk during ROM close/shutdown
            }
            catch (Exception ex)
            {
                SetStatus($"Bridge error: {ex.GetType().Name}: {ex.Message}", isError: true);
            }
        }

        /// <summary>
        /// True only when the offsets are known, the domain exists and the
        /// mailbox in RAM carries the expected magic and version. Until then
        /// nothing is written: the overlay may not be loaded yet, a different
        /// ROM may be running, or coop_offsets.json may be stale.
        /// </summary>
        private bool TryValidateMailbox(out uint mailbox, out string reason)
        {
            mailbox = 0;
            if (_mailboxOffset is not uint offset)
            {
                reason = _offsetsError;
                return false;
            }
            if (_memoryApi == null || MemoryDomains == null || !MemoryDomains.Has(MemoryDomain))
            {
                reason = "Load the co-op ROM (melonDS core).";
                return false;
            }
            uint magic = _memoryApi.ReadU32(offset + Mailbox.MagicOff, MemoryDomain);
            ushort version = (ushort)_memoryApi.ReadU16(offset + Mailbox.VersionOff, MemoryDomain);
            if (magic != Mailbox.Magic)
            {
                reason = "Waiting for the co-op ROM to boot…";
                return false;
            }
            if (version != Mailbox.Version)
            {
                reason = $"ROM is mailbox v{version}, bridge expects v{Mailbox.Version}. Rebuild with build_all.sh.";
                return false;
            }
            _limitMapId = (ushort)_memoryApi.ReadU16(offset + Mailbox.LimitMapId, MemoryDomain);
            _limitSpecies = (ushort)_memoryApi.ReadU16(offset + Mailbox.LimitSpecies, MemoryDomain);
            mailbox = offset;
            reason = string.Empty;
            return true;
        }

        private void ReadLocalPlayerSnapshot(uint mb)
        {
            // Sequence lock: keep the previous snapshot if the ROM was
            // mid-write when the frame ended (seq odd or changed).
            uint seqBefore = _memoryApi!.ReadU32(mb + Mailbox.LocalPlayerSeq, MemoryDomain);
            var block = _memoryApi.ReadByteRange(mb + Mailbox.LocalPlayer, Mailbox.PlayerStateSize, MemoryDomain);
            ushort badges = (ushort)_memoryApi.ReadU16(mb + Mailbox.BadgeMaskLocal, MemoryDomain);
            uint seqAfter = _memoryApi.ReadU32(mb + Mailbox.LocalPlayerSeq, MemoryDomain);

            if ((seqBefore & 1) == 0 && seqBefore == seqAfter && block.Count == Mailbox.PlayerStateSize)
            {
                for (int i = 0; i < Mailbox.PlayerStateSize; i++) _localPlayer[i] = block[i];
                _badgeMaskLocal = badges;
            }
            _heartbeatRom = _memoryApi.ReadU32(mb + Mailbox.HeartbeatRom, MemoryDomain);
            // The ROM fills the block before bumping its seq byte, and only
            // between frames, so one read is consistent.
            var session = _memoryApi.ReadByteRange(mb + Mailbox.SessionOut, Mailbox.SessionOutSize, MemoryDomain);
            if (session.Count == Mailbox.SessionOutSize)
                for (int i = 0; i < Mailbox.SessionOutSize; i++) _sessionOut[i] = session[i];

            ReadChallengeOut();

            uint? exchange = FindExchange(mb);
            var tradeOut = exchange is uint ex
                ? _memoryApi.ReadByteRange(ex + Mailbox.ExchangeOut, Mailbox.TradeOutSize, MemoryDomain)
                : null;
            lock (_tradeLock)
            {
                _tradeOutValid = tradeOut != null && tradeOut.Count == Mailbox.TradeOutSize;
                if (_tradeOutValid)
                    for (int i = 0; i < Mailbox.TradeOutSize; i++) _tradeOut[i] = tradeOut![i];
            }
        }

        /// <summary>
        /// Main RAM offset of the ROM's open CoopTradeExchange, or null. The
        /// pointer comes from the game, so it is range-checked and the block's
        /// header verified before anything is read from or written to it.
        /// </summary>
        private uint? FindExchange(uint mb)
        {
            uint ptr = _memoryApi!.ReadU32(mb + Mailbox.ExchangePtr, MemoryDomain);
            if (ptr < Mailbox.MainRamBase || ptr >= Mailbox.MainRamBase + Mailbox.MainRamSize - Mailbox.ExchangeSize || (ptr & 3) != 0)
                return null;
            uint offset = ptr - Mailbox.MainRamBase;
            if (_memoryApi.ReadU32(offset, MemoryDomain) != Mailbox.ExchangeMagic
                || _memoryApi.ReadU16(offset + 4, MemoryDomain) != Mailbox.ExchangeVersion
                || _memoryApi.ReadU16(offset + 6, MemoryDomain) != Mailbox.ExchangeSize)
                return null;
            return offset;
        }

        /// <summary>
        /// Copies the relay's trade trailer (or "nothing": all zeros) into the
        /// ROM's open exchange block. Out-of-range header values drop the
        /// trailer; the Pokemon bytes are validated by the ROM itself.
        /// </summary>
        private void WriteTradeIn(uint mb, byte[] pkt)
        {
            if (FindExchange(mb) is not uint ex) return;
            byte[] tradeIn = new byte[Mailbox.TradeInSize];
            if (pkt.Length >= Wire.ServerPacketSize + Mailbox.TradeInSize && !CoopLink.HasMagic(pkt, Wire.ServerPacketSize))
            {
                Buffer.BlockCopy(pkt, Wire.ServerPacketSize, tradeIn, 0, Mailbox.TradeInSize);
                byte peer = tradeIn[Mailbox.TradeInPeerSlot];
                bool ok = tradeIn[0] <= Mailbox.TradeInStateMax && tradeIn[1] <= Mailbox.TradeInReasonMax
                    && (peer < Mailbox.RemoteSlots || peer == Mailbox.PlayerIdNone);
                if (!ok) Array.Clear(tradeIn, 0, tradeIn.Length);
            }
            _memoryApi!.WriteByteRange(ex + Mailbox.ExchangeIn, tradeIn, MemoryDomain);
        }

        private void ProcessLatestServerPacket(uint mb)
        {
            byte[]? pkt;
            lock (_packetLock)
            {
                pkt = _latestServerPacketSession == Volatile.Read(ref _sessionId) ? _latestServerPacket : null;
                _latestServerPacket = null;
            }
            if (pkt == null) return;

            byte status = pkt[12];
            if (status != Wire.StatusOk)
            {
                // Server rejected the Create/Join/Sync request. A rejected save
                // also tells the ROM why, so it shows the reason in game and
                // returns to the title without saving (written in the same
                // frame as the disconnect, so it wins over the disconnect save).
                if (status == Wire.StatusMidgameSave || status == Wire.StatusSaveOtherRoom || status == Wire.StatusRunLost)
                    _memoryApi!.WriteU8(mb + Mailbox.RoomRejection, status, MemoryDomain);
                DisconnectFromServer(MapServerStatusToError(status, _activeRoomString, BitConverter.ToUInt16(pkt, 4)), isError: true);
                ClearRomConnectionState(mb, clearPayload: true);
                return;
            }

            byte assigned = pkt[6];
            if (assigned >= Mailbox.MaxPlayers) return; // malformed: ignore the packet

            pkt = TakeChallengeDown(pkt, out byte[]? challengeDown);
            if (challengeDown != null)
            {
                _roomRules = BitConverter.ToUInt16(challengeDown, 8);
                _roomMonoType = challengeDown[5];
                _roomRulesKnown = true;
                WriteChallengeIn(challengeDown);
            }

            uint roomUid = BitConverter.ToUInt32(pkt, Wire.ServerPayloadOffset + Mailbox.PayloadGates + Mailbox.GateRoomUid);
            if (roomUid != 0 && _slotKey == null)
                _slotKey = SaveSlots.RoomKey(_activeRoomString, roomUid);

            bool firstOk = _link != LinkState.Connected || _assignedSlot != assigned;
            bool created = _activeAction == Wire.ActionCreate;
            _assignedSlot = assigned;
            _activeAction = Wire.ActionSync; // Handshake complete; switch to steady-state SYNC
            _lastServerPacketUtc = DateTime.UtcNow;
            if (firstOk)
            {
                _link = LinkState.Connected;
                _worlds.Record(_activeRoomString, _activeServerText, created); // Milestone 6 §4.2
                RefreshWorldList();
                SetStatus($"In world {_activeRoomString}. Start or continue your game.", isError: false);
            }

            byte[] payload = SanitizeServerPayload(pkt);
            _memoryApi!.WriteByteRange(mb + Mailbox.ServerPayload, payload, MemoryDomain);
            _memoryApi.WriteU8(mb + Mailbox.AssignedPlayerId, _assignedSlot, MemoryDomain);
            _memoryApi.WriteU8(mb + Mailbox.ActivePlayersMask, (byte)(pkt[7] & 0x3F), MemoryDomain);
            _memoryApi.WriteU32(mb + Mailbox.ServerTick, BitConverter.ToUInt32(pkt, 8), MemoryDomain);
            _memoryApi.WriteU8(mb + Mailbox.IsConnectedToRoom, 1, MemoryDomain);
            WriteTradeIn(mb, pkt);
            _clearPayloadPending = true; // clear remote state once we disconnect
        }

        /// <summary>
        /// Copies the relay payload and zeroes every remote player, follower or
        /// gates block holding out-of-range values, using the limits the ROM
        /// publishes. The ROM re-validates on read (Coop_ReadRemotePlayer);
        /// this keeps garbage from ever reaching emulated RAM.
        /// </summary>
        private byte[] SanitizeServerPayload(byte[] pkt)
        {
            byte[] p = new byte[Mailbox.ServerPayloadSize];
            Buffer.BlockCopy(pkt, Wire.ServerPayloadOffset, p, 0, Mailbox.ServerPayloadSize);

            for (int i = 0; i < Mailbox.RemoteSlots; i++)
            {
                int o = i * Mailbox.PlayerStateSize;
                ushort map = BitConverter.ToUInt16(p, o + Mailbox.PsMapId);
                byte active = p[o + Mailbox.PsIsActive];
                bool ok = active <= 1
                    && map < _limitMapId
                    && (active == 0 || map != 0)
                    && p[o + Mailbox.PsDirection] <= 3
                    && p[o + Mailbox.PsGender] <= 1
                    && p[o + Mailbox.PsAvatarState] <= Mailbox.MaxAvatarState
                    && IsValidMoveAction(p[o + Mailbox.PsMoveAction])
                    && p[o + Mailbox.PsClientReady] <= 1
                    && p[o + Mailbox.PsSaveStatus] <= 2
                    && p[o + Mailbox.PsActivity] <= Mailbox.MaxActivity;
                if (!ok)
                {
                    Array.Clear(p, o, Mailbox.PlayerStateSize);
                    continue;
                }
                if (p[o + Mailbox.PsAnim] > Mailbox.MaxAnim) p[o + Mailbox.PsAnim] = 0; // cosmetic only
                if (p[o + Mailbox.PsFieldBusy] > 1) p[o + Mailbox.PsFieldBusy] = 1;
                p[o + Mailbox.PsChallengeFlags] &= Mailbox.ChallengeFlagsMask;
                // A bad follower hides only the follower, not the player.
                bool followerOk = BitConverter.ToUInt16(p, o + Mailbox.PsFollowerSpecies) <= _limitSpecies
                    && p[o + Mailbox.PsFollowerShiny] <= 1
                    && p[o + Mailbox.PsFollowerGender] <= 2
                    && p[o + Mailbox.PsFollowerVisible] <= 1
                    && p[o + Mailbox.PsFollowerDirection] <= 3
                    && IsValidMoveAction(p[o + Mailbox.PsFollowerMoveAction]);
                if (!followerOk)
                    Array.Clear(p, o + Mailbox.PsFollowerSpecies, Mailbox.PsAvatarId - Mailbox.PsFollowerSpecies);
            }

            int g = Mailbox.PayloadGates;
            bool gatesOk = p[g + Mailbox.GateSyncReady] <= 1
                && p[g + Mailbox.GateCheckpoint] <= 16
                && p[g + Mailbox.GateMonolocke] <= 18
                && p[g + Mailbox.GateChallengeRoomState] <= 1
                && p[g + Mailbox.GateCampaignStarted] <= 1
                && p[g + Mailbox.GateExpectedParty] <= Mailbox.MaxPlayers
                && p[g + Mailbox.GateOnlineParty] <= Mailbox.MaxPlayers
                && p[g + Mailbox.GateReadyParty] <= Mailbox.MaxPlayers;
            if (!gatesOk) Array.Clear(p, g, Mailbox.GatesSize);
            // Session and team fields: a bad value drops only those (the ROM
            // reads state 0 as "no session" and 0xFF as "no slot").
            static bool SlotOk(byte b) => b < Mailbox.RemoteSlots || b == Mailbox.PlayerIdNone;
            bool sessionOk = SlotOk(p[g + Mailbox.GateTeamPartner])
                && (p[g + Mailbox.GateTeamedMask] & 0x60) == 0
                && p[g + Mailbox.GateSessionState] <= Mailbox.MaxSessionState
                && p[g + Mailbox.GateSessionKind] <= Mailbox.MaxSessionKind
                && SlotOk(p[g + Mailbox.GateSessionPeer])
                && p[g + Mailbox.GateSessionReason] <= Mailbox.MaxSessionReason;
            if (p[g + Mailbox.GateTradePending] > 1) p[g + Mailbox.GateTradePending] = 1;
            if (!sessionOk)
            {
                Array.Clear(p, g + Mailbox.GateTeamPartner, Mailbox.GateSessionSize);
                p[g + Mailbox.GateTeamPartner] = Mailbox.PlayerIdNone;
                p[g + Mailbox.GateSessionPeer] = Mailbox.PlayerIdNone;
            }

            return p;
        }

        private byte[] ReadSram()
        {
            var raw = _memoryApi!.ReadByteRange(0, SaveSlots.SramSize, SaveSlots.SramDomain);
            byte[] image = new byte[SaveSlots.SramSize];
            for (int i = 0; i < image.Length && i < raw.Count; i++) image[i] = raw[i];
            return image;
        }

        /// <summary>
        /// One save per room. Copies SRAM to the owning room's file after every
        /// completed in-game save (also while disconnected: the disconnect flow
        /// saves after the link drops). Once connected at the title screen,
        /// swaps the room's save (or a blank one for a new room) into SRAM and
        /// asks the ROM to restart so the game loads it. save_slot_ready lets
        /// the startup gate pass only when the right save is loaded.
        /// </summary>
        private void UpdateSaveSlot(uint mb)
        {
            uint romHeartbeat = _memoryApi!.ReadU32(mb + Mailbox.HeartbeatRom, MemoryDomain);
            bool romRestarted = romHeartbeat < _lastRomHeartbeat;
            _lastRomHeartbeat = romHeartbeat;

            // save_counter restarts at 0 with the game; only a change to a
            // non-zero value is a new save.
            uint counter = _memoryApi.ReadU32(mb + Mailbox.SaveCounter, MemoryDomain);
            if (_lastSaveCounter.HasValue && counter != _lastSaveCounter.Value && counter != 0 && _slotState != SlotState.Swapping)
                _saveSlots.Capture(ReadSram());
            _lastSaveCounter = counter;

            if (_link == LinkState.Connected && _slotKey != null)
            {
                switch (_slotState)
                {
                    case SlotState.None:
                        if (_memoryApi.ReadU8(mb + Mailbox.WasInActiveSession, MemoryDomain) == 1)
                        {
                            // A save is already in play: never swap under it.
                            _slotState = SlotState.Ready;
                            break;
                        }
                        byte[]? image = _saveSlots.PrepareJoin(_slotKey, ReadSram());
                        if (image == null)
                        {
                            _slotState = SlotState.Ready;
                            break;
                        }
                        _memoryApi.WriteByteRange(0, image, SaveSlots.SramDomain);
                        _memoryApi.WriteU8(mb + Mailbox.ResetRequest, 1, MemoryDomain);
                        _swapRequestedUtc = DateTime.UtcNow;
                        _slotState = SlotState.Swapping;
                        break;

                    case SlotState.Swapping:
                        // The restart re-initializes the mailbox (heartbeat_rom
                        // back near 0, reset_request cleared).
                        if (romRestarted)
                            _slotState = SlotState.Ready;
                        else if ((DateTime.UtcNow - _swapRequestedUtc).TotalSeconds > SwapRetrySeconds)
                        {
                            _memoryApi.WriteU8(mb + Mailbox.ResetRequest, 1, MemoryDomain);
                            _swapRequestedUtc = DateTime.UtcNow;
                        }
                        break;
                }
            }

            bool ready = _link == LinkState.Connected && _slotState == SlotState.Ready;
            _memoryApi.WriteU8(mb + Mailbox.SaveSlotReady, (byte)(ready ? 1 : 0), MemoryDomain);
        }

#if !COOP_SHIP // the debug tools: compiled out of releases (docs/RELEASE_PLAN.md §11 D3)
        /// <summary>
        /// Hands a diagnostics-card command to the ROM (handled by the
        /// Overlay 131 field entry): the command byte first, then a new
        /// sequence number the ROM acknowledges in diag.debug_ack.
        /// </summary>
        private void QueueDebugCommand(byte cmd, byte arg, string name)
        {
            _pendingDebugCmd = cmd;
            _pendingDebugArg = arg;
            _sentDebugName = name;
        }

        private void SendPendingDebugCommand(uint mb)
        {
            if (_pendingDebugCmd == 0) return;
            _memoryApi!.WriteU8(mb + Mailbox.DebugCmd, _pendingDebugCmd, MemoryDomain);
            _memoryApi.WriteU8(mb + Mailbox.DebugArg, _pendingDebugArg, MemoryDomain);
            if (++_debugSeq == 0) _debugSeq = 1;
            _memoryApi.WriteU8(mb + Mailbox.DebugSeq, _debugSeq, MemoryDomain);
            _pendingDebugCmd = 0;
        }
#endif

        private static bool IsValidMoveAction(byte action) =>
            action < Mailbox.MaxHeldMovement || action == Mailbox.MoveActionNone;

        /// <summary>Player name from a CoopPlayerState (HG charset: digits, letters, space).</summary>
        private static string DecodeName(byte[] state, int offset)
        {
            var sb = new StringBuilder();
            for (int i = 0; i < 8; i++)
            {
                ushort ch = BitConverter.ToUInt16(state, offset + Mailbox.PsName + i * 2);
                if (ch == 0xFFFF || ch == 0) break;
                if (ch >= 0x121 && ch <= 0x12A) sb.Append((char)('0' + ch - 0x121));
                else if (ch >= 0x12B && ch <= 0x144) sb.Append((char)('A' + ch - 0x12B));
                else if (ch >= 0x145 && ch <= 0x15E) sb.Append((char)('a' + ch - 0x145));
                else if (ch == 0x1DE) sb.Append(' ');
                else sb.Append('?');
            }
            return sb.Length > 0 ? sb.ToString() : "Trainer";
        }

        /// <summary>Where a player is and what they are doing, from a CoopPlayerState.</summary>
        private static string DescribePlayer(byte[] state, int offset)
        {
            ushort map = BitConverter.ToUInt16(state, offset + Mailbox.PsMapId);
            short x = BitConverter.ToInt16(state, offset + Mailbox.PsX);
            short z = BitConverter.ToInt16(state, offset + Mailbox.PsZ);
            string where = $"map {map} ({x}, {z})";
            switch (state[offset + Mailbox.PsActivity])
            {
                case 2: return $"in a battle · {where}";
                case 3: return $"in a menu · {where}";
                case 4: return $"changing maps · {where}";
                case 5: return $"busy (talking, a sign or a cutscene) · {where}";
            }
            if (state[offset + Mailbox.PsIsActive] != 1) return $"away · {where}";
            string text = $"{where} · {PlayerStateName(state[offset + Mailbox.PsAvatarState])}";
            ushort follower = BitConverter.ToUInt16(state, offset + Mailbox.PsFollowerSpecies);
            if (state[offset + Mailbox.PsFollowerVisible] == 1 && follower != 0) text += $" · follower #{follower}";
            return text;
        }

        private static string PlayerStateName(byte state) => state switch
        {
            0 => "walking",
            1 => "cycling",
            2 => "surfing",
            3 => "in Team Rocket disguise",
            4 => "using a field move",
            7 => "fishing",
            9 => "saving",
            _ => $"state {state}",
        };

#if !COOP_SHIP // the debug tools: compiled out of releases (docs/RELEASE_PLAN.md §11 D3)
        private static (string text, Color color) DebugResultText(byte result) => result switch
        {
            1 => ("done", Theme.Connected),
            2 => ("nothing to act on (spawn the ghost first)", Theme.Connecting),
            3 => ("not now (wrong riding state)", Theme.Connecting),
            4 => ("face surfable water first", Theme.Connecting),
            5 => ("unknown command or preset", Theme.Error),
            6 => ("failed (no free object or memory)", Theme.Error),
            7 => ("debug tools are compiled out of this ROM", Theme.Error),
            _ => ("—", Theme.TextSecondary),
        };

        private void RefreshDiagnostics(uint mb)
        {
            if (!_diagCard.Visible) return;
            if ((DateTime.UtcNow - _lastInfoRefreshUtc).TotalMilliseconds < InfoRefreshMs) return;

            var raw = _memoryApi!.ReadByteRange(mb + Mailbox.Diag, Mailbox.DiagSize, MemoryDomain);
            if (raw.Count != Mailbox.DiagSize) return;
            byte[] d = new byte[Mailbox.DiagSize];
            for (int i = 0; i < d.Length; i++) d[i] = raw[i];

            // The ROM reports why the Overlay 131 field code did or did not run
            // this frame (CoopFieldEntryStatus); the counter shows it is ticking.
            uint calls = BitConverter.ToUInt32(d, Mailbox.DgFieldEntryCalls);
            if (calls != _lastFieldEntryCalls)
            {
                _lastFieldEntryCalls = calls;
                _lastFieldEntryChangeUtc = DateTime.UtcNow;
            }
            bool ticking = (DateTime.UtcNow - _lastFieldEntryChangeUtc).TotalMilliseconds < 500;
            byte status = d[Mailbox.DgFieldEntryStatus];
            bool running = status == 3 && ticking;
            (string fieldText, Color fieldColor) = status switch
            {
                3 => (ticking ? $"running · {calls} frames" : $"stalled · {calls} frames", ticking ? Theme.Connected : Theme.Error),
                2 => ("error: Overlay 131 has no co-op header (rebuild)", Theme.Error),
                1 => ("error: Overlay 131 is not loaded", Theme.Error),
                _ => calls == 0
                    ? ("waiting for the overworld", Theme.TextSecondary)
                    : ($"paused (battle, menu or transition) · {calls} frames", Theme.Connecting),
            };
            SetValue(_diagFieldValue, fieldText, fieldColor);
            foreach (var b in _diagButtons) b.Enabled = running;
            _warpPicker.Enabled = running;

            byte playerState = _localPlayer[Mailbox.PsAvatarState];
            if (running) SetValue(_diagPlayerStateValue, PlayerStateName(playerState));
            else SetValue(_diagPlayerStateValue, "—", Theme.TextSecondary);

            if (_debugSeq == 0)
                SetValue(_diagCommandValue, "—", Theme.TextSecondary);
            else if (d[Mailbox.DgDebugAck] != _debugSeq || _pendingDebugCmd != 0)
                SetValue(_diagCommandValue, $"{_sentDebugName}: waiting (event, message or step in progress)", Theme.Connecting);
            else
            {
                var (resultText, resultColor) = DebugResultText((byte)_memoryApi.ReadU8(mb + Mailbox.DebugResult, MemoryDomain));
                SetValue(_diagCommandValue, $"{_sentDebugName}: {resultText}", resultColor);
            }

            if (d[Mailbox.DgRenderReady] == 1)
            {
                SetValue(_diagTexturesValue, $"{d[Mailbox.DgTexDynamicFree]} of {d[Mailbox.DgTexDynamic]} free · {d[Mailbox.DgTexResident]} resident");
                SetValue(_diagModelsValue, $"{d[Mailbox.DgModelDynamicFree]} of {d[Mailbox.DgModelDynamic]} free");
                SetValue(_diagSequencesValue, $"{d[Mailbox.DgSeqDynamicFree]} of {d[Mailbox.DgSeqDynamic]} free");
                SetValue(_diagObjectsValue, $"{d[Mailbox.DgMapObjects]} of 64");
            }
            else
            {
                foreach (Label l in new[] { _diagTexturesValue, _diagModelsValue, _diagSequencesValue, _diagObjectsValue })
                    SetValue(l, "—", Theme.TextSecondary);
            }

            byte ghost = d[Mailbox.DgGhostState];
            short gx = BitConverter.ToInt16(d, Mailbox.DgGhostX);
            short gz = BitConverter.ToInt16(d, Mailbox.DgGhostZ);
            short gy = BitConverter.ToInt16(d, Mailbox.DgGhostY);
            short py = BitConverter.ToInt16(d, Mailbox.DgPlayerY);
            if (ghost == 0)
            {
                SetValue(_diagGhostValue, "none", Theme.TextSecondary);
                SetValue(_diagHeightValue, "—", Theme.TextSecondary);
            }
            else
            {
                SetValue(_diagGhostValue, $"{(ghost == 2 ? "moving" : "idle")} at ({gx}, {gz})");
                SetValue(_diagHeightValue, $"ghost {gy} · you {py}", gy == py ? Theme.Connected : Theme.Connecting);
            }
        }
#endif

        /// <summary>
        /// Writes the "not connected" state. Called every disconnected frame so
        /// a savestate can never resurrect a stale connection.
        /// </summary>
        private void ClearRomConnectionState(uint mb, bool clearPayload)
        {
            _memoryApi!.WriteU8(mb + Mailbox.IsConnectedToRoom, 0, MemoryDomain);
            _memoryApi.WriteU8(mb + Mailbox.AssignedPlayerId, Mailbox.PlayerIdNone, MemoryDomain);
            _memoryApi.WriteU8(mb + Mailbox.ActivePlayersMask, 0, MemoryDomain);
            if (clearPayload)
            {
                _memoryApi.WriteByteRange(mb + Mailbox.ServerPayload, new byte[Mailbox.ServerPayloadSize], MemoryDomain);
                _clearPayloadPending = false;
            }
        }

        private void RefreshSession(string? waitReason, uint? mb)
        {
            if ((DateTime.UtcNow - _lastInfoRefreshUtc).TotalMilliseconds < InfoRefreshMs) return;
            _lastInfoRefreshUtc = DateTime.UtcNow;
            UpdateControls();

            const string none = "—";
            if (mb is not uint box)
            {
                SetValue(_gameValue, waitReason ?? none, Theme.Connecting);
                foreach (Label l in new[] { _trainerValue, _bedroomValue, _playersValue, _adventureValue, _rulesValue, _gymValue, _locationValue, _othersValue, _saveValue })
                    SetValue(l, none, Theme.TextSecondary);
                return;
            }

            byte disconnectState = (byte)_memoryApi!.ReadU8(box + Mailbox.DisconnectState, MemoryDomain);
            switch (disconnectState)
            {
                case 1: SetValue(_gameValue, "Disconnected · will save at the next safe moment", Theme.Connecting); break;
                case 2: SetValue(_gameValue, "Saving and returning to the title screen…", Theme.Connecting); break;
                default: SetValue(_gameValue, "Running", Theme.Connected); break;
            }

            uint otid = BitConverter.ToUInt32(_localPlayer, Mailbox.PsTrainerId);
            bool ready = _localPlayer[Mailbox.PsClientReady] == 1;

            SetValue(_trainerValue, otid != 0 ? $"{DecodeName(_localPlayer, 0)} · {otid & 0xFFFF:D5}  (0x{otid:X8})" : "No trainer yet", otid != 0 ? (Color?)null : Theme.TextSecondary);
            SetValue(_bedroomValue, ready ? "Yes" : "No", ready ? Theme.Connected : Theme.TextSecondary);
            SetValue(_locationValue, otid != 0 ? DescribePlayer(_localPlayer, 0) : "Not in the overworld", otid != 0 ? (Color?)null : Theme.TextSecondary);

            if (_saveSlots.LastError.Length > 0)
                SetValue(_saveValue, "Error: " + _saveSlots.LastError, Theme.Error);
            else if (_link != LinkState.Connected || _slotKey == null)
                SetValue(_saveValue, _saveSlots.Owner.Length > 0 ? $"Last world: {_saveSlots.Owner.Split('-')[0]}" : none, Theme.TextSecondary);
            else if (_slotState == SlotState.Swapping)
                SetValue(_saveValue, "Loading this world's save (the game restarts)…", Theme.Connecting);
            else if (_slotState == SlotState.Ready)
                SetValue(_saveValue, _saveSlots.HasRoomSave(_slotKey) ? "This world's save is loaded" : "New world: start a New Game", Theme.Connected);
            else
                SetValue(_saveValue, "Preparing…", Theme.Connecting);

            if (_link != LinkState.Connected)
            {
                SetValue(_othersValue, none, Theme.TextSecondary);
                SetValue(_playersValue, none, Theme.TextSecondary);
                SetValue(_adventureValue, none, Theme.TextSecondary);
                SetValue(_rulesValue, none, Theme.TextSecondary);
                SetValue(_gymValue, none, Theme.TextSecondary);
                return;
            }

            uint g = box + Mailbox.Gates;
            byte syncReady = (byte)_memoryApi.ReadU8(g + Mailbox.GateSyncReady, MemoryDomain);
            byte started = (byte)_memoryApi.ReadU8(g + Mailbox.GateCampaignStarted, MemoryDomain);
            byte expected = (byte)_memoryApi.ReadU8(g + Mailbox.GateExpectedParty, MemoryDomain);
            byte online = (byte)_memoryApi.ReadU8(g + Mailbox.GateOnlineParty, MemoryDomain);
            byte readyCount = (byte)_memoryApi.ReadU8(g + Mailbox.GateReadyParty, MemoryDomain);

            SetValue(_playersValue, started == 1
                ? $"{online} of {expected}"
                : $"{online} of {expected} · {readyCount} ready in bedroom");

            var remote = _memoryApi.ReadByteRange(box + Mailbox.ServerPayload, Mailbox.PayloadGates, MemoryDomain);
            byte[] rp = new byte[Mailbox.PayloadGates];
            for (int i = 0; i < rp.Length && i < remote.Count; i++) rp[i] = remote[i];
            byte partner = (byte)_memoryApi.ReadU8(g + Mailbox.GateTeamPartner, MemoryDomain);
            var lines = new List<string>();
            for (int i = 0; i < Mailbox.RemoteSlots; i++)
            {
                int o = i * Mailbox.PlayerStateSize;
                if (BitConverter.ToUInt32(rp, o + Mailbox.PsTrainerId) == 0) continue;
                string tag = i == partner ? " (teammate)" : "";
                lines.Add($"{DecodeName(rp, o)}{tag}: {DescribePlayer(rp, o)}");
            }
            SetValue(_othersValue, lines.Count > 0 ? string.Join("\n", lines) : "Nobody else yet",
                lines.Count > 0 ? (Color?)null : Theme.TextSecondary);
            SetValue(_adventureValue, started == 1 ? "Started · world locked" : "Waiting for everyone in the bedroom",
                started == 1 ? Theme.Connected : Theme.Connecting);
            SetValue(_gymValue, syncReady == 1 ? "Open for you" : "Closed · a teammate is behind on badges",
                syncReady == 1 ? Theme.Connected : Theme.TextSecondary);
            SetValue(_rulesValue, _roomRulesKnown ? ChallengeRules.Describe(_roomRules, _roomMonoType) : "Waiting for the server…",
                _roomRulesKnown ? (Color?)null : Theme.TextSecondary);
        }

        /// <summary>Puts a rules word into the host's pickers.</summary>
        private void ShowRules(ushort rules)
        {
            _randomPicker.SelectedIndex = rules & ChallengeRules.RandomMask;
            _soulLinkPicker.SelectedIndex = (rules & ChallengeRules.SoulLinkMask) >> ChallengeRules.SoulLinkShift;
            _legendsCheck.Checked = (rules & ChallengeRules.LegendsWild) != 0;
            _abilitiesCheck.Checked = (rules & ChallengeRules.RandomAbilities) != 0;
            _nuzlockeCheck.Checked = (rules & ChallengeRules.Nuzlocke) != 0;
            _monolockeCheck.Checked = (rules & ChallengeRules.Monolocke) != 0;
            _levelCapCheck.Checked = (rules & ChallengeRules.LevelCap) != 0;
            _randomItemsCheck.Checked = (rules & ChallengeRules.RandomItems) != 0;
        }

        private bool _showingRules;

        /// <summary>Reads the pickers, drops what a prerequisite rules out, and shows the result.</summary>
        private void OnRulesEdited()
        {
            if (_showingRules) return;
            int random = Math.Max(0, _randomPicker.SelectedIndex);
            int soulLink = Math.Max(0, _soulLinkPicker.SelectedIndex);
            ushort raw = (ushort)(random
                | (_legendsCheck.Checked ? ChallengeRules.LegendsWild : 0)
                | (_abilitiesCheck.Checked ? ChallengeRules.RandomAbilities : 0)
                | (_nuzlockeCheck.Checked ? ChallengeRules.Nuzlocke : 0)
                | (soulLink << ChallengeRules.SoulLinkShift)
                | (_monolockeCheck.Checked ? ChallengeRules.Monolocke : 0)
                | (_levelCapCheck.Checked ? ChallengeRules.LevelCap : 0)
                | (_randomItemsCheck.Checked ? ChallengeRules.RandomItems : 0));
            _challengeRules = ChallengeRules.Sanitize(raw);
            _showingRules = true;
            ShowRules(_challengeRules);
            _showingRules = false;
            UpdateControls();
        }

        // ------------------------------------------------------------------
        // Config & helpers
        // ------------------------------------------------------------------

        /// <summary>Full path given to BizHawk's --config (both "--config=x" and "--config x").</summary>
        private static string? FindBizHawkConfigArg()
        {
            string[] args = Environment.GetCommandLineArgs();
            for (int i = 0; i < args.Length; i++)
            {
                string? value = null;
                if (args[i].StartsWith("--config=", StringComparison.Ordinal)) value = args[i].Substring("--config=".Length);
                else if (args[i] == "--config" && i + 1 < args.Length) value = args[i + 1];
                if (!string.IsNullOrWhiteSpace(value))
                {
                    try { return Path.GetFullPath(value!.Trim('"')); }
                    catch (ArgumentException) { return null; }
                }
            }
            return null;
        }

        private static uint GenerateCryptoToken()
        {
            using (var rng = RandomNumberGenerator.Create())
            {
                byte[] buf = new byte[4];
                rng.GetBytes(buf);
                uint val = BitConverter.ToUInt32(buf, 0);
                return val == 0 ? 1U : val;
            }
        }

        private static ulong GenerateCryptoSecret()
        {
            using (var rng = RandomNumberGenerator.Create())
            {
                byte[] buf = new byte[8];
                rng.GetBytes(buf);
                ulong val = BitConverter.ToUInt64(buf, 0);
                return val == 0 ? 1UL : val;
            }
        }

        private static string JsonEscape(string s) => s.Replace("\\", "\\\\").Replace("\"", "\\\"");

        // player_secret identifies this install to locked campaigns: losing it
        // means the relay operator must reset that roster entry's secret to 0.
        private static void SaveNetConfig(string host, int partySize, ulong playerSecret, ushort challengeRules, string avatar)
        {
            try
            {
                string json = "{\n" +
                    $"  \"server\": \"{JsonEscape(host)}\",\n" +
                    $"  \"party_size\": {partySize},\n" +
                    $"  \"challenge_rules\": {challengeRules},\n" +
                    $"  \"avatar\": \"{JsonEscape(avatar)}\",\n" +
                    $"  \"player_secret\": \"{playerSecret:X16}\"\n" +
                    "}\n";
                File.WriteAllText(ConfigFilePath, json);
            }
            catch (IOException) { }
            catch (UnauthorizedAccessException) { }
        }

        private static (string host, int party, ulong secret, ushort rules, string avatar) LoadNetConfig()
        {
            try
            {
                if (File.Exists(ConfigFilePath))
                {
                    string json = File.ReadAllText(ConfigFilePath);
                    Match mH = Regex.Match(json, @"""server""\s*:\s*""([^""]+)""");
                    Match mP = Regex.Match(json, @"""party_size""\s*:\s*(\d+)");
                    Match mS = Regex.Match(json, @"""player_secret""\s*:\s*""([0-9A-Fa-f]{1,16})""");
                    Match mR = Regex.Match(json, @"""challenge_rules""\s*:\s*(\d+)");
                    Match mA = Regex.Match(json, @"""avatar""\s*:\s*""([a-z0-9_]+)""");

                    string h = mH.Success ? mH.Groups[1].Value : string.Empty; // no built-in server (Milestone 6 §4.3)
                    int p = mP.Success && int.TryParse(mP.Groups[1].Value, out int pv) ? pv : 2;
                    ulong secret = mS.Success ? Convert.ToUInt64(mS.Groups[1].Value, 16) : 0;
                    ushort rules = mR.Success && ushort.TryParse(mR.Groups[1].Value, out ushort rv) ? rv : (ushort)0;
                    return (h, p, secret, rules, mA.Success ? mA.Groups[1].Value : "default");
                }
            }
            catch (IOException) { }
            catch (UnauthorizedAccessException) { }
            return ("", 2, 0, 0, "default");
        }

        private static IPEndPoint? TryResolveServerEndpoint(string hostPort)
        {
            if (string.IsNullOrWhiteSpace(hostPort)) return null;

            try
            {
                string[] parts = hostPort.Split(':');
                string host = parts[0].Trim();
                int port = 7777;
                if (parts.Length > 1 && (!int.TryParse(parts[1], out port) || port < 1 || port > 65535))
                {
                    return null;
                }

                if (IPAddress.TryParse(host, out IPAddress? ip))
                {
                    return new IPEndPoint(ip, port);
                }

                foreach (IPAddress a in Dns.GetHostAddresses(host))
                {
                    if (a.AddressFamily == AddressFamily.InterNetwork)
                    {
                        return new IPEndPoint(a, port);
                    }
                }
            }
            catch (SocketException) { }
            catch (ArgumentException) { }
            return null;
        }

        /// <summary>
        /// Loads the mailbox offset written by scripts/extract_coop_sym.py.
        /// There is deliberately no fallback offset: without a valid file the
        /// bridge never touches emulated memory.
        /// </summary>
        // Where build outputs at the repo root are looked for.
        private static string[] RepoFileCandidates(string fileName)
        {
            string dllDir = Path.GetDirectoryName(Assembly.GetExecutingAssembly().Location) ?? ".";
            return new[]
            {
                Path.Combine(dllDir, "..", "..", fileName), // <repo>/BizHawk/ExternalTools -> <repo>
                Path.Combine(dllDir, fileName),
                fileName,
                Path.Combine("..", fileName),
            };
        }

        private void OnAvatarPicked()
        {
            if (!(_avatarPicker.SelectedItem is AvatarChoice choice) || _link != LinkState.Offline) return;
            Volatile.Write(ref _avatarId, choice.Id);
            _avatarKey = choice.Key;
            Image? old = _avatarPreview.Image;
            _avatarPreview.Image = Avatars.LoadPreview(choice);
            old?.Dispose();
            (string host, int party, ulong secret, ushort rules, _) = LoadNetConfig();
            SaveNetConfig(host, party, secret != 0 ? secret : _playerSecret, rules, _avatarKey);
        }

        private void LoadOffsetsFromJson()
        {
            string[] searchPaths = RepoFileCandidates(OffsetsFileName);

            foreach (string path in searchPaths)
            {
                try
                {
                    if (!File.Exists(path)) continue;
                    string json = File.ReadAllText(path);
                    Match mOff = Regex.Match(json, @"""main_ram_offset_int""\s*:\s*(\d+)");
                    Match mVer = Regex.Match(json, @"""mailbox_version""\s*:\s*(\d+)");
                    if (!mOff.Success || !uint.TryParse(mOff.Groups[1].Value, out uint parsed))
                    {
                        _offsetsError = $"{OffsetsFileName} is incomplete. Run build_all.sh.";
                        continue;
                    }
                    if (!mVer.Success || mVer.Groups[1].Value != Mailbox.Version.ToString())
                    {
                        _offsetsError = $"{OffsetsFileName} is from an older build. Run build_all.sh.";
                        continue;
                    }
                    if (parsed > 0x003FFFFF - Mailbox.Size)
                    {
                        _offsetsError = $"{OffsetsFileName} has an invalid offset (0x{parsed:X}). Run build_all.sh.";
                        continue;
                    }
                    _mailboxOffset = parsed;
                    _offsetsError = string.Empty;
                    return;
                }
                catch (IOException ex) { _offsetsError = ex.Message; }
                catch (UnauthorizedAccessException ex) { _offsetsError = ex.Message; }
            }

            if (_offsetsError.Length == 0)
            {
                _offsetsError = $"{OffsetsFileName} not found. Run build_all.sh.";
            }
        }
    }
}
