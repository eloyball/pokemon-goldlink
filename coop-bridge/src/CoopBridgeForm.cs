using System;
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
        public const ushort Version = 3; // COOP_MAILBOX_VERSION

        public const uint MagicOff = 0x000;          // u32
        public const uint VersionOff = 0x004;        // u16
        public const uint AssignedPlayerId = 0x006;  // u8   bridge
        public const uint ActivePlayersMask = 0x007; // u8   bridge
        public const uint HeartbeatRom = 0x008;      // u32  rom
        public const uint ServerTick = 0x00C;        // u32  bridge
        public const uint LocalPlayer = 0x010;       // 32 B rom
        public const uint ServerPayload = 0x030;     // 304 B bridge (remote players, followers, gates)
        public const int ServerPayloadSize = 304;
        public const uint Gates = 0x148;             // 24 B bridge
        public const uint IsConnectedToRoom = 0x160; // u8   bridge
        public const uint DisconnectState = 0x163;   // u8   rom
        public const uint BadgeMaskLocal = 0x164;    // u16  rom
        public const uint HeartbeatBridge = 0x168;   // u32  bridge
        public const uint LocalPlayerSeq = 0x16C;    // u32  rom (odd while writing)
        public const uint LimitMapId = 0x178;        // u16  rom: map_id < limit
        public const uint LimitSpecies = 0x17A;      // u16  rom: species <= limit

        // CoopPlayerState field offsets
        public const int PlayerStateSize = 32;
        public const int PsMapId = 0x00;
        public const int PsX = 0x02;
        public const int PsY = 0x04;
        public const int PsDirection = 0x0A;
        public const int PsIsActive = 0x0F;
        public const int PsLeadSpecies = 0x10;
        public const int PsLeadShiny = 0x14;
        public const int PsLeadGender = 0x15;
        public const int PsClientReady = 0x17;
        public const int PsTrainerId = 0x18;

        // CoopFollowerState field offsets
        public const int FollowerStateSize = 24;
        public const int FsSpecies = 0x00;
        public const int FsShiny = 0x04;
        public const int FsGender = 0x05;
        public const int FsDirection = 0x0A;
        public const int FsVisible = 0x0B;

        // CoopProgressionGates field offsets (relative to Gates)
        public const int GatesSize = 24;
        public const int GateSyncReady = 0x04;
        public const int GateCheckpoint = 0x05;
        public const int GateMonolocke = 0x06;
        public const int GateNuzlockeFailed = 0x07;
        public const int GateCampaignStarted = 0x08;
        public const int GateExpectedParty = 0x09;
        public const int GateOnlineParty = 0x0A;
        public const int GateReadyParty = 0x0B;

        // Layout of the 304-byte server payload (relative to ServerPayload)
        public const int RemoteSlots = 5;
        public const int PayloadFollowers = RemoteSlots * PlayerStateSize;                  // 160
        public const int PayloadGates = PayloadFollowers + RemoteSlots * FollowerStateSize; // 280

        public const byte PlayerIdNone = 0xFF;
        public const int MaxPlayers = 6;
    }

    /// <summary>UDP wire format. Must match coop-relay/src/protocol.rs.</summary>
    internal static class Wire
    {
        public const ushort ProtocolVersion = 3;
        public const int ClientPacketSize = 80;
        public const int ServerPacketSize = 320;
        public const int ErrorPacketSize = 16;
        public const int ChallengePacketSize = 24; // header + cookie (u64 @16)
        public const int ServerPayloadOffset = 16; // packet[16..320] -> mailbox 0x030..0x15F

        public const byte StatusOk = 0;
        public const byte StatusChallenge = 7;   // not an error: echo the cookie
        public const byte StatusRateLimited = 8;

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
        private const string DefaultMessage = "Enter a room code to join your friends, or host a new room.";

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
        private readonly TextBox _roomInput;
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
        private readonly Label _gymValue;
        private readonly Label _locationValue;
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
            (string savedHost, int savedParty, ulong savedSecret) = LoadNetConfig();
            _playerSecret = savedSecret != 0 ? savedSecret : GenerateCryptoSecret();
            if (savedSecret == 0) SaveNetConfig(savedHost, savedParty, _playerSecret);

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
            _joinModeButton = Theme.MakeButton("Join a room", Theme.ButtonKind.SegmentOn);
            _hostModeButton = Theme.MakeButton("Host a room", Theme.ButtonKind.SegmentOff);
            _joinModeButton.Height = _hostModeButton.Height = 30;
            _joinModeButton.Margin = new Padding(0, 0, 2, 0);
            _hostModeButton.Margin = new Padding(2, 0, 0, 0);
            _joinModeButton.Click += (_, _) => SetHostMode(false);
            _hostModeButton.Click += (_, _) => SetHostMode(true);
            segments.Controls.Add(_joinModeButton, 0, 0);
            segments.Controls.Add(_hostModeButton, 1, 0);

            var roomCaption = Theme.MakeLabel("Room code", Theme.Caption, Theme.TextSecondary);
            _roomInput = new TextBox
            {
                MaxLength = 6,
                CharacterCasing = CharacterCasing.Upper,
                TextAlign = HorizontalAlignment.Center,
                Font = Theme.Code,
                BackColor = Theme.Input,
                ForeColor = Theme.TextPrimary,
                BorderStyle = BorderStyle.FixedSingle,
                Dock = DockStyle.Fill,
                Margin = new Padding(0, 0, 0, 4)
            };
            // Only letters and digits; everything else (except editing keys) is dropped.
            _roomInput.KeyPress += (_, e) =>
            {
                if (!char.IsControl(e.KeyChar) && !IsRoomCodeChar(e.KeyChar)) e.Handled = true;
            };
            _roomInput.TextChanged += (_, _) => UpdateControls();
            var roomHint = Theme.MakeLabel("6 letters or numbers, shared by everyone in the room.", Theme.Caption, Theme.TextSecondary);
            roomHint.Margin = new Padding(0, 0, 0, 12);

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
                "The adventure starts, and the room locks to these players, once everyone is ready in their bedroom.",
                Theme.Caption, Theme.TextSecondary);
            playersHint.MaximumSize = new Size(contentWidth - 28, 0);
            playersHint.Margin = new Padding(0);
            _hostOptions.Controls.Add(playersHint);

            _primaryButton = Theme.MakeButton("Join room", Theme.ButtonKind.Primary);
            _primaryButton.Click += (_, _) => StartConnection(_hostMode ? Wire.ActionCreate : Wire.ActionJoin);
            _leaveButton = Theme.MakeButton("Leave room", Theme.ButtonKind.Danger);
            _leaveButton.Visible = false;
            _leaveButton.Click += (_, _) => DisconnectFromServer("You left the room.", isError: false);

            roomCard.Controls.Add(segments);
            roomCard.Controls.Add(roomCaption);
            roomCard.Controls.Add(_roomInput);
            roomCard.Controls.Add(roomHint);
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
            _gymValue = AddRow("Gym gate");
            _locationValue = AddRow("Location");
            sessionCard.Controls.Add(grid);

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

            root.Controls.Add(header);
            root.Controls.Add(_messageLabel);
            root.Controls.Add(roomCard);
            root.Controls.Add(sessionCard);
            root.Controls.Add(serverCard);
            Controls.Add(root);
            AcceptButton = _primaryButton; // Enter in the room code submits
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
        }

        // ------------------------------------------------------------------
        // UI state (UI thread)
        // ------------------------------------------------------------------

        private static bool IsRoomCodeChar(char c) => (c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9');

        private bool RoomCodeIsValid => Regex.IsMatch(_roomInput.Text, "^[A-Z0-9]{6}$");

        private void SetHostMode(bool host)
        {
            if (_link != LinkState.Offline) return;
            _hostMode = host;
            UpdateControls();
        }

        private void SetStatus(string message, bool isError)
        {
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
            _serverInput.Enabled = offline;
            for (int i = 0; i < _partySizeButtons.Length; i++)
            {
                _partySizeButtons[i].Kind = i + 2 == _partySize ? Theme.ButtonKind.SegmentOn : Theme.ButtonKind.SegmentOff;
                _partySizeButtons[i].Enabled = offline;
            }
            _hostOptions.Visible = offline && _hostMode;

            _primaryButton.Visible = _link != LinkState.Connected;
            _leaveButton.Visible = _link == LinkState.Connected;
            _primaryButton.Text = _link switch
            {
                LinkState.Resolving or LinkState.Connecting => "Connecting…",
                _ => _hostMode ? "Create room" : "Join room"
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
            _activeAction = initialAction;
            _assignedSlot = Mailbox.PlayerIdNone;
            _lastServerPacketUtc = DateTime.UtcNow;
            BeginNewSession();

            // Generate a fresh cryptographic host token on every Create Room attempt
            _hostToken = initialAction == Wire.ActionCreate ? GenerateCryptoToken() : 0;
            _link = LinkState.Connecting;
            SetStatus(initialAction == Wire.ActionCreate
                ? $"Creating room {roomText}…"
                : $"Joining room {roomText}…", isError: false);

            SaveNetConfig(hostText, _partySize, _playerSecret);
            StartReceiveThread();
        }

        private void DisconnectFromServer(string reasonMessage, bool isError)
        {
            if (_link == LinkState.Connecting || _link == LinkState.Connected)
            {
                // One-shot LEAVE so the relay frees the slot immediately
                SendPacket(Wire.ActionLeave);
            }

            _link = LinkState.Offline;
            _assignedSlot = Mailbox.PlayerIdNone;
            _activeRoomString = string.Empty;
            _clearPayloadPending = true;
            BeginNewSession();
            SetStatus(reasonMessage, isError);
        }

        private void BeginNewSession()
        {
            Interlocked.Increment(ref _sessionId);
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
                    if (BitConverter.ToUInt16(data, 4) != Wire.ProtocolVersion) continue;

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
            tx[14] = 0; // reserved: readiness travels in local_player.client_ready
            tx[15] = (byte)_partySize; // honored by the relay only from the host
            Buffer.BlockCopy(BitConverter.GetBytes(_hostToken), 0, tx, 16, 4);
            Buffer.BlockCopy(BitConverter.GetBytes(_badgeMaskLocal), 0, tx, 20, 2);
            Buffer.BlockCopy(BitConverter.GetBytes(++_sequence), 0, tx, 22, 4);
            Buffer.BlockCopy(BitConverter.GetBytes(_heartbeatRom), 0, tx, 26, 4);
            Buffer.BlockCopy(_localPlayer, 0, tx, 32, Mailbox.PlayerStateSize);
            Buffer.BlockCopy(BitConverter.GetBytes(Interlocked.Read(ref _cookie)), 0, tx, 64, 8);
            Buffer.BlockCopy(BitConverter.GetBytes(_playerSecret), 0, tx, 72, 8);

            return tx;
        }

        private static string MapServerStatusToError(byte code, string room)
        {
            return code switch
            {
                1 => $"Room {room} doesn't exist. Check the code, or host it instead.",
                2 => $"Room {room} already exists. Join it, or pick another code to host.",
                3 => $"Room {room} is locked to its original players (or this trainer is registered to another install).",
                4 => $"Room {room} is full, or the server is at capacity.",
                5 => "Room codes are 6 letters or numbers.",
                6 => "A new room can't start from a mid-game save.",
                Wire.StatusRateLimited => "Too many failed attempts. Wait a few seconds and try again.",
                _ => $"The server rejected the connection (code {code})."
            };
        }

        // ------------------------------------------------------------------
        // Emulated memory (emulation thread only: UpdateValues)
        // ------------------------------------------------------------------

        public override void UpdateValues(ToolFormUpdateType type)
        {
            // BizHawk calls this before AND after every frame; run once.
            if (type != ToolFormUpdateType.PostFrame && type != ToolFormUpdateType.FastPostFrame) return;
            if (_closing) return;
            _lastFrameUtc = DateTime.UtcNow;

            try
            {
                if (!TryValidateMailbox(out uint mb, out string waitReason))
                {
                    RefreshSession(waitReason, mb: null);
                    return;
                }

                // Tell the ROM we are alive (drives its disconnect watchdog).
                _memoryApi!.WriteU32(mb + Mailbox.HeartbeatBridge, ++_bridgeHeartbeat, MemoryDomain);
                ReadLocalPlayerSnapshot(mb);

                if (_link != LinkState.Connecting && _link != LinkState.Connected)
                {
                    ClearRomConnectionState(mb, _clearPayloadPending);
                    RefreshSession(null, mb);
                    return;
                }

                ProcessLatestServerPacket(mb);
                if (_link == LinkState.Offline) return; // server rejected us

                DateTime now = DateTime.UtcNow;
                DateTime lastChallenge = new DateTime(Interlocked.Read(ref _lastChallengeUtcTicks), DateTimeKind.Utc);
                bool handshaking = (now - lastChallenge).TotalSeconds <= ServerTimeoutSeconds
                    && (now - _lastServerPacketUtc).TotalSeconds <= HandshakeTimeoutSeconds;

                if (!handshaking && (now - _lastServerPacketUtc).TotalSeconds > ServerTimeoutSeconds)
                {
                    DisconnectFromServer("Lost the connection to the server.", isError: true);
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
                // Server rejected the Create/Join/Sync request
                DisconnectFromServer(MapServerStatusToError(status, _activeRoomString), isError: true);
                ClearRomConnectionState(mb, clearPayload: true);
                return;
            }

            byte assigned = pkt[6];
            if (assigned >= Mailbox.MaxPlayers) return; // malformed: ignore the packet

            bool firstOk = _link != LinkState.Connected || _assignedSlot != assigned;
            _assignedSlot = assigned;
            _activeAction = Wire.ActionSync; // Handshake complete; switch to steady-state SYNC
            _lastServerPacketUtc = DateTime.UtcNow;
            if (firstOk)
            {
                _link = LinkState.Connected;
                SetStatus($"In room {_activeRoomString}. Start or continue your game.", isError: false);
            }

            byte[] payload = SanitizeServerPayload(pkt);
            _memoryApi!.WriteByteRange(mb + Mailbox.ServerPayload, payload, MemoryDomain);
            _memoryApi.WriteU8(mb + Mailbox.AssignedPlayerId, _assignedSlot, MemoryDomain);
            _memoryApi.WriteU8(mb + Mailbox.ActivePlayersMask, (byte)(pkt[7] & 0x3F), MemoryDomain);
            _memoryApi.WriteU32(mb + Mailbox.ServerTick, BitConverter.ToUInt32(pkt, 8), MemoryDomain);
            _memoryApi.WriteU8(mb + Mailbox.IsConnectedToRoom, 1, MemoryDomain);
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
                    && BitConverter.ToUInt16(p, o + Mailbox.PsLeadSpecies) <= _limitSpecies
                    && p[o + Mailbox.PsLeadShiny] <= 1
                    && p[o + Mailbox.PsLeadGender] <= 2
                    && p[o + Mailbox.PsClientReady] <= 1;
                if (!ok) Array.Clear(p, o, Mailbox.PlayerStateSize);
            }

            for (int i = 0; i < Mailbox.RemoteSlots; i++)
            {
                int o = Mailbox.PayloadFollowers + i * Mailbox.FollowerStateSize;
                bool ok = BitConverter.ToUInt16(p, o + Mailbox.FsSpecies) <= _limitSpecies
                    && p[o + Mailbox.FsShiny] <= 1
                    && p[o + Mailbox.FsGender] <= 2
                    && p[o + Mailbox.FsDirection] <= 3
                    && p[o + Mailbox.FsVisible] <= 1;
                if (!ok) Array.Clear(p, o, Mailbox.FollowerStateSize);
            }

            int g = Mailbox.PayloadGates;
            bool gatesOk = p[g + Mailbox.GateSyncReady] <= 1
                && p[g + Mailbox.GateCheckpoint] <= 16
                && p[g + Mailbox.GateMonolocke] <= 18
                && p[g + Mailbox.GateNuzlockeFailed] <= 1
                && p[g + Mailbox.GateCampaignStarted] <= 1
                && p[g + Mailbox.GateExpectedParty] <= Mailbox.MaxPlayers
                && p[g + Mailbox.GateOnlineParty] <= Mailbox.MaxPlayers
                && p[g + Mailbox.GateReadyParty] <= Mailbox.MaxPlayers;
            if (!gatesOk) Array.Clear(p, g, Mailbox.GatesSize);

            return p;
        }

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
                foreach (Label l in new[] { _trainerValue, _bedroomValue, _playersValue, _adventureValue, _gymValue, _locationValue })
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
            bool inField = _localPlayer[Mailbox.PsIsActive] == 1;
            ushort map = BitConverter.ToUInt16(_localPlayer, Mailbox.PsMapId);
            short x = BitConverter.ToInt16(_localPlayer, Mailbox.PsX);
            short y = BitConverter.ToInt16(_localPlayer, Mailbox.PsY);

            SetValue(_trainerValue, otid != 0 ? $"{otid & 0xFFFF:D5}  (0x{otid:X8})" : "No trainer yet", otid != 0 ? (Color?)null : Theme.TextSecondary);
            SetValue(_bedroomValue, ready ? "Yes" : "No", ready ? Theme.Connected : Theme.TextSecondary);
            SetValue(_locationValue, inField ? $"Map {map} · ({x}, {y})" : "Not in the overworld", inField ? (Color?)null : Theme.TextSecondary);

            if (_link != LinkState.Connected)
            {
                SetValue(_playersValue, none, Theme.TextSecondary);
                SetValue(_adventureValue, none, Theme.TextSecondary);
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
            SetValue(_adventureValue, started == 1 ? "Started · room locked" : "Waiting for everyone in the bedroom",
                started == 1 ? Theme.Connected : Theme.Connecting);
            SetValue(_gymValue, syncReady == 1 ? "Open" : "Closed · needs everyone, equal badges",
                syncReady == 1 ? Theme.Connected : Theme.TextSecondary);
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
        private static void SaveNetConfig(string host, int partySize, ulong playerSecret)
        {
            try
            {
                string json = "{\n" +
                    $"  \"server\": \"{JsonEscape(host)}\",\n" +
                    $"  \"party_size\": {partySize},\n" +
                    $"  \"player_secret\": \"{playerSecret:X16}\"\n" +
                    "}\n";
                File.WriteAllText(ConfigFilePath, json);
            }
            catch (IOException) { }
            catch (UnauthorizedAccessException) { }
        }

        private static (string host, int party, ulong secret) LoadNetConfig()
        {
            try
            {
                if (File.Exists(ConfigFilePath))
                {
                    string json = File.ReadAllText(ConfigFilePath);
                    Match mH = Regex.Match(json, @"""server""\s*:\s*""([^""]+)""");
                    Match mP = Regex.Match(json, @"""party_size""\s*:\s*(\d+)");
                    Match mS = Regex.Match(json, @"""player_secret""\s*:\s*""([0-9A-Fa-f]{1,16})""");

                    string h = mH.Success ? mH.Groups[1].Value : "127.0.0.1:7777";
                    int p = mP.Success && int.TryParse(mP.Groups[1].Value, out int pv) ? pv : 2;
                    ulong secret = mS.Success ? Convert.ToUInt64(mS.Groups[1].Value, 16) : 0;
                    return (h, p, secret);
                }
            }
            catch (IOException) { }
            catch (UnauthorizedAccessException) { }
            return ("127.0.0.1:7777", 2, 0);
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
        private void LoadOffsetsFromJson()
        {
            string dllDir = Path.GetDirectoryName(Assembly.GetExecutingAssembly().Location) ?? ".";
            string[] searchPaths =
            {
                Path.Combine(dllDir, "..", "..", OffsetsFileName), // <repo>/BizHawk/ExternalTools -> <repo>
                Path.Combine(dllDir, OffsetsFileName),
                OffsetsFileName,
                Path.Combine("..", OffsetsFileName),
            };

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
