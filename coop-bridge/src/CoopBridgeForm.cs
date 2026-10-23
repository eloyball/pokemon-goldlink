using System;
using System.Drawing;
using System.IO;
using System.Net;
using System.Net.Sockets;
using System.Security.Cryptography;
using System.Text;
using System.Text.RegularExpressions;
using System.Threading;
using System.Windows.Forms;
using BizHawk.Client.Common;
using BizHawk.Client.EmuHawk;

namespace CoopBridge
{
    [ExternalTool("HGSS Co-op Bridge")]
    public sealed class CoopBridgeForm : ToolFormBase, IExternalToolForm
    {
        protected override string WindowTitleStatic => "HGSS 6-Player Co-op Bridge";

        [RequiredApi]
        public IMemoryApi? _memoryApi { get; set; }

        private const string MemoryDomain = "Main RAM";
        private const string ConfigFilePath = "coop_config.json";
        private const uint CoopMagic = 0x434F4F50;
        private const ushort ProtocolVersion = 1;
        private const ushort BedroomMapId = 4;

        // Wire Action Constants
        private const byte RoomActionCreate = 1;
        private const byte RoomActionJoin = 2;
        private const byte RoomActionSync = 3;
        private const byte RoomActionLeave = 4;

        private uint _mailboxOffset = 0x1E6D40;
        private uint _sequence;
        private byte _assignedSlot = 0xFF;
        private readonly ushort _ephemeralPort;

        private bool _isConnected;
        private byte _activeAction;
        private uint _hostToken;
        private byte _serverStatus;
        private string _statusMessage = "DISCONNECTED — Enter a 6-char code above.";

        private readonly TextBox _serverInput;
        private readonly TextBox _roomInput;
        private readonly NumericUpDown _partySizeInput;
        private readonly Button _createButton;
        private readonly Button _joinButton;
        private readonly Button _disconnectButton;
        private readonly Label _infoLabel;
        private readonly Button _simBedroomButton;

        private readonly UdpClient _udpClient;
        private IPEndPoint _serverEndpoint;
        private byte[] _roomCodeBytes = new byte[6];
        private string _activeRoomString = "------";
        private Thread? _recvThread;
        private volatile bool _running;

        private readonly object _packetLock = new object();
        private byte[]? _latestServerPacket;

        public CoopBridgeForm()
        {
            ClientSize = new Size(400, 420);
            FormBorderStyle = FormBorderStyle.FixedDialog;
            MaximizeBox = false;

            BackColor = Color.FromArgb(26, 28, 32);
            ForeColor = Color.FromArgb(235, 235, 240);

            LoadOffsetsFromJson();
            (string savedHost, int savedParty) = LoadNetConfig();

            Color panelBg = Color.FromArgb(34, 37, 42);
            Color inputBg = Color.FromArgb(44, 47, 54);
            Color textFg = Color.FromArgb(240, 240, 245);
            Color mutedFg = Color.FromArgb(170, 175, 185);

            // --- Section 1: Server & Room Code Inputs ---
            var serverLbl = new Label
            {
                Text = "Server (Host:Port):",
                Location = new Point(14, 15),
                Size = new Size(130, 22),
                ForeColor = textFg
            };
            _serverInput = new TextBox
            {
                Text = savedHost,
                Location = new Point(150, 12),
                Size = new Size(236, 24),
                BackColor = inputBg,
                ForeColor = textFg,
                BorderStyle = BorderStyle.FixedSingle
            };

            var roomLbl = new Label
            {
                Text = "Room Code (6 Chars):",
                Location = new Point(14, 47),
                Size = new Size(130, 22),
                ForeColor = textFg
            };
            _roomInput = new TextBox
            {
                Text = string.Empty, // Blank by default
                MaxLength = 6,
                CharacterCasing = CharacterCasing.Upper,
                Location = new Point(150, 44),
                Size = new Size(120, 24),
                BackColor = inputBg,
                ForeColor = textFg,
                Font = new Font(FontFamily.GenericMonospace, 10f, FontStyle.Bold),
                BorderStyle = BorderStyle.FixedSingle
            };

            var roomHintLbl = new Label
            {
                Text = "(A-Z, 0-9)",
                Location = new Point(278, 48),
                Size = new Size(100, 20),
                ForeColor = mutedFg
            };

            // --- Section 2A: Host New Room Panel ---
            var hostPanel = new Panel
            {
                Location = new Point(14, 80),
                Size = new Size(180, 100),
                BackColor = panelBg,
                BorderStyle = BorderStyle.FixedSingle
            };
            var hostTitle = new Label
            {
                Text = "HOST NEW ROOM",
                Location = new Point(10, 8),
                Size = new Size(158, 18),
                Font = new Font(FontFamily.GenericSansSerif, 8.5f, FontStyle.Bold),
                ForeColor = Color.FromArgb(130, 220, 150)
            };
            var sizeLbl = new Label
            {
                Text = "Party Size:",
                Location = new Point(10, 34),
                Size = new Size(85, 22),
                ForeColor = textFg
            };
            _partySizeInput = new NumericUpDown
            {
                Minimum = 2,
                Maximum = 6,
                Value = Math.Max(2, Math.Min(6, savedParty)),
                Location = new Point(100, 31),
                Size = new Size(64, 24),
                BackColor = inputBg,
                ForeColor = textFg
            };
            _createButton = new Button
            {
                Text = "Create Room",
                Location = new Point(10, 60),
                Size = new Size(158, 30),
                BackColor = Color.FromArgb(38, 105, 62),
                ForeColor = textFg,
                Font = new Font(FontFamily.GenericSansSerif, 9f, FontStyle.Bold),
                FlatStyle = FlatStyle.Flat
            };
            _createButton.FlatAppearance.BorderColor = Color.FromArgb(75, 155, 100);
            _createButton.Click += (_, _) => StartConnection(RoomActionCreate);
            hostPanel.Controls.AddRange(new Control[] { hostTitle, sizeLbl, _partySizeInput, _createButton });

            // --- Section 2B: Join Existing Room Panel ---
            var joinPanel = new Panel
            {
                Location = new Point(206, 80),
                Size = new Size(180, 100),
                BackColor = panelBg,
                BorderStyle = BorderStyle.FixedSingle
            };
            var joinTitle = new Label
            {
                Text = "JOIN EXISTING ROOM",
                Location = new Point(10, 8),
                Size = new Size(158, 18),
                Font = new Font(FontFamily.GenericSansSerif, 8.5f, FontStyle.Bold),
                ForeColor = Color.FromArgb(130, 190, 245)
            };
            var joinDesc = new Label
            {
                Text = "Connect to a friend's\n6-character room code.",
                Location = new Point(10, 28),
                Size = new Size(158, 30),
                ForeColor = mutedFg,
                Font = new Font(FontFamily.GenericSansSerif, 8f)
            };
            _joinButton = new Button
            {
                Text = "Join Room",
                Location = new Point(10, 60),
                Size = new Size(158, 30),
                BackColor = Color.FromArgb(40, 85, 135),
                ForeColor = textFg,
                Font = new Font(FontFamily.GenericSansSerif, 9f, FontStyle.Bold),
                FlatStyle = FlatStyle.Flat
            };
            _joinButton.FlatAppearance.BorderColor = Color.FromArgb(80, 135, 195);
            _joinButton.Click += (_, _) => StartConnection(RoomActionJoin);
            joinPanel.Controls.AddRange(new Control[] { joinTitle, joinDesc, _joinButton });

            // --- Section 3: Disconnect Button ---
            _disconnectButton = new Button
            {
                Text = "Disconnect / Leave Room",
                Location = new Point(14, 188),
                Size = new Size(372, 28),
                BackColor = Color.FromArgb(50, 52, 58),
                ForeColor = mutedFg,
                FlatStyle = FlatStyle.Flat,
                Enabled = false
            };
            _disconnectButton.FlatAppearance.BorderColor = Color.FromArgb(75, 80, 90);
            _disconnectButton.Click += (_, _) => DisconnectFromServer("DISCONNECTED — Left room.");

            // --- Section 4: Live Telemetry Console ---
            _infoLabel = new Label
            {
                Text = _statusMessage,
                Location = new Point(14, 224),
                Size = new Size(372, 144),
                BackColor = Color.FromArgb(18, 19, 22),
                ForeColor = Color.FromArgb(190, 225, 255),
                Font = new Font(FontFamily.GenericMonospace, 8.7f),
                BorderStyle = BorderStyle.FixedSingle,
                Padding = new Padding(6)
            };

            // --- Section 5: Synthetic Bedroom Test Button ---
            _simBedroomButton = new Button
            {
                Text = "Simulate Bedroom Spawn & Step (+1 X in Map 4)",
                Location = new Point(14, 376),
                Size = new Size(372, 32),
                BackColor = Color.FromArgb(52, 56, 66),
                ForeColor = textFg,
                FlatStyle = FlatStyle.Flat
            };
            _simBedroomButton.FlatAppearance.BorderColor = Color.FromArgb(80, 85, 95);
            _simBedroomButton.Click += (_, _) => SimulateBedroomStep();

            Controls.AddRange(new Control[]
            {
                serverLbl, _serverInput, roomLbl, _roomInput, roomHintLbl,
                hostPanel, joinPanel, _disconnectButton, _infoLabel, _simBedroomButton
            });

            _serverEndpoint = new IPEndPoint(IPAddress.Loopback, 7777);
            _udpClient = new UdpClient(0);
            _udpClient.Client.Blocking = false;

            if (_udpClient.Client.LocalEndPoint is IPEndPoint localEp)
            {
                _ephemeralPort = (ushort)localEp.Port;
            }
            else
            {
                _ephemeralPort = 50000;
            }
        }

        private void StartConnection(byte initialAction)
        {
            string hostText = _serverInput.Text.Trim();
            string roomText = _roomInput.Text.Trim().ToUpperInvariant();
            _roomInput.Text = roomText;

            // Strict 6-character alphanumeric validation
            if (roomText.Length != 6 || !Regex.IsMatch(roomText, "^[A-Z0-9]{6}$"))
            {
                _statusMessage = "ERR: Room Code must be exactly 6 chars (A-Z, 0-9)!";
                return;
            }

            if (!TryResolveServerEndpoint(hostText, out IPEndPoint resolvedEp))
            {
                _statusMessage = "ERR: Invalid Server address or port!";
                return;
            }

            _serverEndpoint = resolvedEp;
            _roomCodeBytes = Encoding.ASCII.GetBytes(roomText);
            _activeRoomString = roomText;
            _activeAction = initialAction;
            _assignedSlot = 0xFF;
            _serverStatus = 0;

            // Generate a fresh cryptographic host token on every Create Room attempt
            if (initialAction == RoomActionCreate)
            {
                _hostToken = GenerateCryptoToken();
                _statusMessage = $"CREATING ROOM '{roomText}'...";
            }
            else
            {
                _hostToken = 0;
                _statusMessage = $"JOINING ROOM '{roomText}'...";
            }

            SaveNetConfig(hostText, (int)_partySizeInput.Value);
            SetConnectedUiState(true);
        }

        private void DisconnectFromServer(string reasonMessage)
        {
            if (_isConnected && _activeRoomString.Length == 6 && _activeRoomString != "------")
            {
                // Send one-shot LEAVE packet so the server frees the slot immediately
                try
                {
                    byte[] leavePkt = BuildClientPacket(RoomActionLeave, 0, 0, 0);
                    _udpClient.Send(leavePkt, leavePkt.Length, _serverEndpoint);
                }
                catch { }
            }

            _isConnected = false;
            _assignedSlot = 0xFF;
            _activeRoomString = "------";
            _statusMessage = reasonMessage;

            if (_memoryApi != null)
            {
                // Inform ROM that connection is closed
                _memoryApi.WriteU8(_mailboxOffset + 0x006, 0xFF, MemoryDomain);
                _memoryApi.WriteU8(_mailboxOffset + 0x007, 0, MemoryDomain);
                _memoryApi.WriteU8(_mailboxOffset + 0x150, 0, MemoryDomain);
            }

            SetConnectedUiState(false);
        }

        private void SetConnectedUiState(bool connected)
        {
            _isConnected = connected;
            _serverInput.Enabled = !connected;
            _roomInput.Enabled = !connected;
            _partySizeInput.Enabled = !connected;
            _createButton.Enabled = !connected;
            _joinButton.Enabled = !connected;

            _disconnectButton.Enabled = connected;
            _disconnectButton.BackColor = connected
                ? Color.FromArgb(140, 45, 45)
                : Color.FromArgb(50, 52, 58);
            _disconnectButton.ForeColor = connected
                ? Color.FromArgb(240, 240, 245)
                : Color.FromArgb(170, 175, 185);
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

        private static void SaveNetConfig(string host, int partySize)
        {
            try
            {
                string json = "{\n" +
                    $"  \"server\": \"{host}\",\n" +
                    $"  \"party_size\": {partySize}\n" +
                    "}\n";
                File.WriteAllText(ConfigFilePath, json);
            }
            catch { }
        }

        private static (string host, int party) LoadNetConfig()
        {
            try
            {
                if (File.Exists(ConfigFilePath))
                {
                    string json = File.ReadAllText(ConfigFilePath);
                    Match mH = Regex.Match(json, @"""server""\s*:\s*""([^""]+)""");
                    Match mP = Regex.Match(json, @"""party_size""\s*:\s*(\d+)");

                    string h = mH.Success ? mH.Groups[1].Value : "127.0.0.1:7777";
                    int p = mP.Success && int.TryParse(mP.Groups[1].Value, out int pv) ? pv : 2;
                    return (h, p);
                }
            }
            catch { }
            return ("127.0.0.1:7777", 2);
        }

        private static bool TryResolveServerEndpoint(string hostPort, out IPEndPoint endpoint)
        {
            endpoint = new IPEndPoint(IPAddress.Loopback, 7777);
            if (string.IsNullOrWhiteSpace(hostPort)) return false;

            try
            {
                string[] parts = hostPort.Split(':');
                string host = parts[0].Trim();
                int port = 7777;
                if (parts.Length > 1 && (!int.TryParse(parts[1], out port) || port < 1 || port > 65535))
                {
                    return false;
                }

                if (IPAddress.TryParse(host, out IPAddress? ip))
                {
                    endpoint = new IPEndPoint(ip, port);
                    return true;
                }

                IPAddress[] addrs = Dns.GetHostAddresses(host);
                foreach (IPAddress a in addrs)
                {
                    if (a.AddressFamily == AddressFamily.InterNetwork)
                    {
                        endpoint = new IPEndPoint(a, port);
                        return true;
                    }
                }
            }
            catch { }
            return false;
        }

        private void LoadOffsetsFromJson()
        {
            try
            {
                string[] searchPaths = { "coop_offsets.json", "../coop_offsets.json", "../../coop_offsets.json" };
                foreach (string path in searchPaths)
                {
                    if (File.Exists(path))
                    {
                        string json = File.ReadAllText(path);
                        Match m = Regex.Match(json, @"""main_ram_offset_int""\s*:\s*(\d+)");
                        if (m.Success && uint.TryParse(m.Groups[1].Value, out uint parsedOffset))
                        {
                            _mailboxOffset = parsedOffset & 0x003FFFFF;
                            break;
                        }
                    }
                }
            }
            catch { }
        }

        public override void Restart()
        {
            base.Restart();
            StartReceiveThread();
        }

        protected override void OnFormClosing(FormClosingEventArgs e)
        {
            DisconnectFromServer("Closing window.");
            _running = false;
            _udpClient.Close();
            base.OnFormClosing(e);
        }

        private void StartReceiveThread()
        {
            if (_running) return;
            _running = true;

            _recvThread = new Thread(() =>
            {
                var remoteEp = new IPEndPoint(IPAddress.Any, 0);
                while (_running)
                {
                    try
                    {
                        if (_udpClient.Available > 0)
                        {
                            byte[] data = _udpClient.Receive(ref remoteEp);
                            if (data.Length >= 320 && BitConverter.ToUInt32(data, 0) == CoopMagic)
                            {
                                lock (_packetLock)
                                {
                                    _latestServerPacket = data;
                                }
                            }
                        }
                        else
                        {
                            Thread.Sleep(1);
                        }
                    }
                    catch (SocketException) { break; }
                    catch (ObjectDisposedException) { break; }
                }
            })
            {
                IsBackground = true
            };
            _recvThread.Start();
        }

        private void SimulateBedroomStep()
        {
            if (_memoryApi == null) return;
            uint localBase = _mailboxOffset + 0x010;
            ushort mapId = (ushort)_memoryApi.ReadU16(localBase + 0x00, MemoryDomain);
            short curX = (short)_memoryApi.ReadS16(localBase + 0x02, MemoryDomain);
            short curY = (short)_memoryApi.ReadS16(localBase + 0x04, MemoryDomain);
            uint trainerId = _memoryApi.ReadU32(localBase + 0x18, MemoryDomain);

            if (mapId == 0) mapId = BedroomMapId;
            if (curY == 0) curY = 6;
            if (trainerId == 0) trainerId = 0xCAFE0000U | _ephemeralPort;

            _memoryApi.WriteU32(_mailboxOffset + 0x000, CoopMagic, MemoryDomain);
            _memoryApi.WriteU16(_mailboxOffset + 0x004, ProtocolVersion, MemoryDomain);
            _memoryApi.WriteU16(localBase + 0x00, mapId, MemoryDomain);
            _memoryApi.WriteS16(localBase + 0x06, curX, MemoryDomain);
            _memoryApi.WriteS16(localBase + 0x08, curY, MemoryDomain);
            _memoryApi.WriteS16(localBase + 0x02, (short)(curX + 1), MemoryDomain);
            _memoryApi.WriteS16(localBase + 0x04, curY, MemoryDomain);
            _memoryApi.WriteU16(localBase + 0x10, 155, MemoryDomain);
            _memoryApi.WriteU8(localBase + 0x17, 1, MemoryDomain); // Auto-ready in bedroom
            _memoryApi.WriteU32(localBase + 0x18, trainerId, MemoryDomain);
        }

        private byte[] BuildClientPacket(byte action, byte readyByte, ushort badgeMaskLocal, uint hbRom)
        {
            uint localBase = _mailboxOffset + 0x010;
            byte[] tx = new byte[64];

            Buffer.BlockCopy(BitConverter.GetBytes(CoopMagic), 0, tx, 0, 4);
            Buffer.BlockCopy(BitConverter.GetBytes(ProtocolVersion), 0, tx, 4, 2);
            Buffer.BlockCopy(_roomCodeBytes, 0, tx, 6, 6);
            tx[12] = action;
            tx[13] = _assignedSlot;
            tx[14] = readyByte;
            tx[15] = 0;
            Buffer.BlockCopy(BitConverter.GetBytes(_hostToken), 0, tx, 16, 4);
            Buffer.BlockCopy(BitConverter.GetBytes(badgeMaskLocal), 0, tx, 20, 2);
            Buffer.BlockCopy(BitConverter.GetBytes(++_sequence), 0, tx, 22, 4);
            Buffer.BlockCopy(BitConverter.GetBytes(hbRom), 0, tx, 26, 4);
            Buffer.BlockCopy(BitConverter.GetBytes((ushort)0), 0, tx, 30, 2);

            if (_memoryApi != null)
            {
                for (int i = 0; i < 32; i++)
                {
                    tx[32 + i] = (byte)_memoryApi.ReadU8(localBase + i, MemoryDomain);
                }
            }

            return tx;
        }

        private static string MapServerStatusToError(byte code, string room)
        {
            return code switch
            {
                1 => $"ERR: Room '{room}' does not exist! Click Create or check code.",
                2 => $"ERR: Room '{room}' already exists! Pick a new code or Join.",
                3 => $"ERR: Room '{room}' is locked to its original TrainerID roster!",
                4 => $"ERR: Room '{room}' is full!",
                5 => "ERR: Room Code must be 6 alphanumeric chars (A-Z, 0-9)!",
                6 => "ERR: Cannot start a new room from a mid-game save!",
                _ => $"ERR: Server rejected connection (Code {code})."
            };
        }

        public override void UpdateValues(ToolFormUpdateType type)
        {
            if (_memoryApi == null) return;

            uint localBase = _mailboxOffset + 0x010;
            ushort locMap = (ushort)_memoryApi.ReadU16(localBase + 0x00, MemoryDomain);
            short locX = (short)_memoryApi.ReadS16(localBase + 0x02, MemoryDomain);
            short locY = (short)_memoryApi.ReadS16(localBase + 0x04, MemoryDomain);
            uint otid = _memoryApi.ReadU32(localBase + 0x18, MemoryDomain);

            // Automatic Bedroom Ready check: Player is ready once spawned in Bedroom (Map 4) with a valid TrainerID
            byte autoReady = (byte)((locMap == BedroomMapId && otid != 0) ? 1 : 0);
            _memoryApi.WriteU8(localBase + 0x17, autoReady, MemoryDomain);
            _memoryApi.WriteU8(localBase + 0x1C, (byte)_partySizeInput.Value, MemoryDomain);

            if (!_isConnected)
            {
                _infoLabel.Text =
                    $"Status      : {_statusMessage}\n" +
                    $"Room        : ------ | Slot: None\n" +
                    $"Trainer ID  : 0x{otid:X8} | In Bedroom: {(autoReady == 1 ? "YES" : "NO")}\n" +
                    $"Start Gate  : LOCKED (Connect to a room first)\n" +
                    $"Gym Gate    : LOCKED\n" +
                    $"Local Pos   : Map {locMap} ({locX}, {locY})";
                return;
            }

            byte[]? serverPkt = null;
            lock (_packetLock)
            {
                if (_latestServerPacket != null)
                {
                    serverPkt = _latestServerPacket;
                    _latestServerPacket = null;
                }
            }

            if (serverPkt != null)
            {
                _serverStatus = serverPkt[12];
                if (_serverStatus == 0)
                {
                    _assignedSlot = serverPkt[6];
                    _activeAction = RoomActionSync; // Handshake complete; switch to steady-state SYNC
                    _statusMessage = "CONNECTED";

                    byte activeMask = serverPkt[7];
                    uint serverTick = BitConverter.ToUInt32(serverPkt, 8);

                    _memoryApi.WriteU8(_mailboxOffset + 0x006, _assignedSlot, MemoryDomain);
                    _memoryApi.WriteU8(_mailboxOffset + 0x007, activeMask, MemoryDomain);
                    _memoryApi.WriteU32(_mailboxOffset + 0x00C, serverTick, MemoryDomain);

                    for (int i = 0; i < 304; i++)
                    {
                        _memoryApi.WriteU8(_mailboxOffset + 0x030 + i, serverPkt[16 + i], MemoryDomain);
                    }
                }
                else
                {
                    // Server rejected the Create/Join/Sync request -> Disconnect immediately and display error
                    string errText = MapServerStatusToError(_serverStatus, _activeRoomString);
                    DisconnectFromServer(errText);
                    return;
                }
            }

            uint hbRom = _memoryApi.ReadU32(_mailboxOffset + 0x008, MemoryDomain);
            ushort badgeMaskLocal = (ushort)_memoryApi.ReadU16(_mailboxOffset + 0x148, MemoryDomain);

            byte[] tx = BuildClientPacket(_activeAction, autoReady, badgeMaskLocal, hbRom);
            try { _udpClient.Send(tx, tx.Length, _serverEndpoint); } catch { }

            short rem0X = (short)_memoryApi.ReadS16(_mailboxOffset + 0x032, MemoryDomain);
            short rem0Y = (short)_memoryApi.ReadS16(_mailboxOffset + 0x034, MemoryDomain);

            byte syncReady = (byte)_memoryApi.ReadU8(_mailboxOffset + 0x14C, MemoryDomain);
            byte campaignStarted = (byte)_memoryApi.ReadU8(_mailboxOffset + 0x150, MemoryDomain);
            byte expectedParty = (byte)_memoryApi.ReadU8(_mailboxOffset + 0x151, MemoryDomain);
            byte onlineParty = (byte)_memoryApi.ReadU8(_mailboxOffset + 0x152, MemoryDomain);
            byte readyParty = (byte)_memoryApi.ReadU8(_mailboxOffset + 0x153, MemoryDomain);

            _infoLabel.Text =
                $"Status      : {_statusMessage}\n" +
                $"Room        : {_activeRoomString} | Slot: P{(_assignedSlot == 0xFF ? "?" : _assignedSlot.ToString())}\n" +
                $"Trainer ID  : 0x{otid:X8} | Party: {onlineParty}/{expectedParty} ({readyParty} in Bedroom)\n" +
                $"Start Gate  : {(campaignStarted == 1 ? "UNLOCKED (Adventure Started)" : "WAITING FOR PARTY IN BEDROOM")}\n" +
                $"Gym Gate    : {(syncReady == 1 ? "UNLOCKED" : "LOCKED")}\n" +
                $"Local Pos   : Map {locMap} ({locX}, {locY}) | Remote[0]: ({rem0X}, {rem0Y})";
        }
    }
}
