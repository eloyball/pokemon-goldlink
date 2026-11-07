using System;
using System.Collections.Generic;
using BizHawk.Client.Common;

namespace CoopBridge
{
    /// <summary>
    /// Virtual link carrier (docs/MILESTONE3_PLAN.md §15). While the ROM runs a
    /// local-wireless session it publishes a link block (hg-engine coop_sync.h
    /// CoopLinkBlock, magic "CLK1") through the mailbox exchange pointer. Each
    /// frame this drains the block's out ring into a reliable stream to the
    /// relay, and writes the relay's routed messages into its in ring and the
    /// room's beacons into its "seen" slots.
    ///
    /// Streams (both directions, as coop-relay/src/lobby/link.rs): consecutive
    /// u16 sequence numbers, a cumulative ack (the next one expected), unacked
    /// messages resent after <see cref="ResendMs"/>, out-of-order ones dropped.
    /// The epoch changes for every new link block, which resets both streams.
    /// </summary>
    internal sealed class CoopLink
    {
        // CoopLinkBlock layout.
        private const uint BlockMagic = 0x314B4C43; // "CLK1"
        private const ushort BlockVersion = 1;
        private const int BlockSize = 0xDB0;
        private const int OffRole = 0x08;
        private const int OffOutHead = 0x0C, OffOutTail = 0x0D, OffInHead = 0x0E, OffInTail = 0x0F;
        private const int OffMine = 0x010, OffSeen = 0x088, OffOut = 0x2E0, OffIn = 0x5F0;
        private const int BeaconSize = 0x78, BeaconMax = 0x70, MsgSize = 0xC4, DataMax = 0xC0;
        private const int OutSlots = 4, InSlots = 10, SeenSlots = 5;
        private const int MsgKindMax = 5;

        // Trailer (coop-relay/src/protocol.rs LinkUp / LinkDown).
        private const uint TrailerMagic = 0x314B4E4C; // "LNK1"
        private const int MaxMsgsPerTrailer = 16;
        private const int SendBudget = 1000;
        private const int ResendMs = 150;

        private sealed class Pending
        {
            public ushort Seq;
            public byte Kind, Aid;
            public byte[] Data = Array.Empty<byte>();
            public long SentTicks; // 0: not sent yet
        }

        private readonly object _lock = new object();
        private bool _active;
        private uint _blockOffset;
        private byte _epoch;
        private byte _role;
        private byte[]? _beacon;     // encoded: ggid u32, len, entry, seq, pad, data
        private ushort _outNext;
        private readonly List<Pending> _out = new List<Pending>();
        private ushort _inNext;
        private readonly Queue<byte[]> _inbox = new Queue<byte[]>(); // kind, aid, data...
        private readonly Queue<byte[]> _received = new Queue<byte[]>(); // raw trailers from the network thread

        /// <summary>Main RAM offset of a valid link block, or null.</summary>
        public static uint? Find(IMemoryApi mem, string domain, uint ptr)
        {
            if (ptr < 0x02000000 || ptr >= 0x02400000 - BlockSize || (ptr & 3) != 0) return null;
            uint off = ptr - 0x02000000;
            if (mem.ReadU32(off, domain) != BlockMagic || mem.ReadU16(off + 4, domain) != BlockVersion
                || mem.ReadU16(off + 6, domain) != BlockSize)
                return null;
            return off;
        }

        /// <summary>Network thread: keeps a server packet's link trailer for the next frame.</summary>
        public void OnServerPacket(byte[] pkt, int serverPacketSize, int tradeInSize)
        {
            int at = serverPacketSize;
            if (!HasMagic(pkt, at))
            {
                at += tradeInSize;
                if (!HasMagic(pkt, at)) return;
            }
            byte[] trailer = new byte[pkt.Length - at];
            Buffer.BlockCopy(pkt, at, trailer, 0, trailer.Length);
            lock (_lock)
            {
                if (_received.Count < 64) _received.Enqueue(trailer);
            }
        }

        /// <summary>True when a link trailer starts at <paramref name="at"/> (so no trade trailer is there).</summary>
        public static bool HasMagic(byte[] buf, int at)
        {
            return buf.Length >= at + 4 && BitConverter.ToUInt32(buf, at) == TrailerMagic;
        }

        /// <summary>
        /// Emulation thread, once a frame. <paramref name="block"/> is the
        /// link block's Main RAM offset (null: no link session).
        /// </summary>
        public void Pump(IMemoryApi mem, string domain, uint? block, byte myPlayer)
        {
            lock (_lock)
            {
                if (block is not uint blk)
                {
                    if (_active) Reset();
                    _received.Clear();
                    return;
                }
                if (!_active || blk != _blockOffset)
                {
                    Reset();
                    _active = true;
                    _blockOffset = blk;
                    _epoch++;
                }

                // My messages and beacon.
                _role = (byte)mem.ReadU8(blk + OffRole, domain);
                byte head = (byte)mem.ReadU8(blk + OffOutHead, domain);
                byte tail = (byte)mem.ReadU8(blk + OffOutTail, domain);
                while (tail != head && _out.Count < 256)
                {
                    uint m = blk + OffOut + (uint)(tail % OutSlots) * MsgSize;
                    var hdr = mem.ReadByteRange(m, 4, domain);
                    int len = Math.Min(hdr[2] | hdr[3] << 8, DataMax);
                    var data = mem.ReadByteRange(m + 4, len, domain);
                    _out.Add(new Pending { Seq = _outNext++, Kind = hdr[0], Aid = hdr[1], Data = ToArray(data) });
                    tail++;
                }
                mem.WriteU8(blk + OffOutTail, tail, domain);
                var mine = mem.ReadByteRange(blk + OffMine, 8, domain);
                int beaconLen = mine[4];
                if (beaconLen > 0 && beaconLen <= BeaconMax)
                {
                    var data = mem.ReadByteRange(blk + OffMine + 8, beaconLen, domain);
                    _beacon = new byte[8 + beaconLen];
                    for (int i = 0; i < 4; i++) _beacon[i] = mine[i];
                    _beacon[4] = (byte)beaconLen;
                    _beacon[5] = mine[5];
                    _beacon[6] = mine[7];
                    for (int i = 0; i < beaconLen; i++) _beacon[8 + i] = data[i];
                }
                else
                {
                    _beacon = null;
                }

                DeliverIn(mem, domain, blk, myPlayer);
            }
        }

        /// <summary>
        /// Before a frame (link latency, docs/GAME_REBALANCE_UPDATE.md §25):
        /// what the relay sent since the last frame goes into the in ring
        /// now, so the game handles it in this frame instead of the next.
        /// Only the receiving half, on the block Pump validated after the
        /// last frame (the game has not run since).
        /// </summary>
        public void PumpIn(IMemoryApi mem, string domain, byte myPlayer)
        {
            lock (_lock)
            {
                if (_active && _received.Count > 0) DeliverIn(mem, domain, _blockOffset, myPlayer);
            }
        }

        // The relay's trailers since the last call, then the in ring. Under _lock.
        private void DeliverIn(IMemoryApi mem, string domain, uint blk, byte myPlayer)
        {
            {
                byte[]? lastBeacons = null;
                while (_received.Count > 0)
                    lastBeacons = Apply(_received.Dequeue()) ?? lastBeacons;
                if (lastBeacons != null) WriteSeen(mem, domain, blk, lastBeacons, myPlayer);

                // Into the in ring, as far as it has room.
                byte head = (byte)mem.ReadU8(blk + OffInHead, domain);
                byte tail = (byte)mem.ReadU8(blk + OffInTail, domain);
                while (_inbox.Count > 0 && (byte)(head - tail) < InSlots)
                {
                    byte[] msg = _inbox.Dequeue();
                    uint m = blk + OffIn + (uint)(head % InSlots) * MsgSize;
                    byte[] slot = new byte[4 + msg.Length - 2];
                    slot[0] = msg[0];
                    slot[1] = msg[1];
                    slot[2] = (byte)(msg.Length - 2);
                    slot[3] = (byte)((msg.Length - 2) >> 8);
                    Buffer.BlockCopy(msg, 2, slot, 4, msg.Length - 2);
                    mem.WriteByteRange(m, slot, domain);
                    head++;
                }
                mem.WriteU8(blk + OffInHead, head, domain); // last: the ROM reads up to in_head
            }
        }

        private void Reset()
        {
            _active = false;
            _out.Clear();
            _inbox.Clear();
            _outNext = 0;
            _inNext = 0;
            _beacon = null;
            _role = 0;
        }

        private static byte[] ToArray(IReadOnlyList<byte> list)
        {
            byte[] a = new byte[list.Count];
            for (int i = 0; i < a.Length; i++) a[i] = list[i];
            return a;
        }

        private static bool SeqBefore(ushort a, ushort b) => (short)(ushort)(b - a) > 0;

        /// <summary>Applies one relay trailer; returns its beacon section (raw), or null if malformed.</summary>
        private byte[]? Apply(byte[] t)
        {
            if (t.Length < 12 || BitConverter.ToUInt32(t, 0) != TrailerMagic) return null;
            int beaconCount = t[4], count = t[5];
            ushort ack = BitConverter.ToUInt16(t, 6), firstSeq = BitConverter.ToUInt16(t, 8);
            if (beaconCount > 6 || count > MaxMsgsPerTrailer) return null;
            int pos = 12, beaconsStart = pos;
            for (int i = 0; i < beaconCount; i++)
            {
                if (pos + 8 > t.Length || t[pos + 1] > BeaconMax) return null;
                pos += 8 + t[pos + 1];
            }
            if (pos > t.Length) return null;
            byte[] beacons = new byte[1 + pos - beaconsStart];
            beacons[0] = (byte)beaconCount;
            Buffer.BlockCopy(t, beaconsStart, beacons, 1, pos - beaconsStart);

            // Milestone 6 (the PvP freeze over the VPS): every link block restarts its streams
            // at 0, so a late relay trailer of the previous block (the internet reorders) would
            // ack and deliver into this one: acked-away messages never arrive, and the battle
            // waits forever ("Communicating..."). The relay names the block it serves (bytes
            // 10-11: epoch, 1; older relays send 0, 0): another block's ack and messages are
            // skipped (its beacons are the room's, still fine). An ack past what was ever sent
            // is never from this block either.
            bool otherBlock = t[11] == 1 && t[10] != _epoch;
            if (otherBlock || SeqBefore(_outNext, ack)) return beacons;
            _out.RemoveAll(p => SeqBefore(p.Seq, ack));
            for (int i = 0; i < count; i++)
            {
                if (pos + 4 > t.Length) return null;
                byte kind = t[pos], aid = t[pos + 1];
                int len = t[pos + 2] | t[pos + 3] << 8;
                if (len > DataMax || pos + 4 + len > t.Length) return null;
                ushort seq = (ushort)(firstSeq + i);
                if (seq == _inNext && kind >= 1 && kind <= MsgKindMax && aid <= 15 && _inbox.Count < 256)
                {
                    byte[] msg = new byte[2 + len];
                    msg[0] = kind;
                    msg[1] = aid;
                    Buffer.BlockCopy(t, pos + 4, msg, 2, len);
                    _inbox.Enqueue(msg);
                    _inNext++;
                }
                pos += 4 + len;
            }
            return beacons;
        }

        /// <summary>The room's parents into the block's seen slots, by remote slot; the rest emptied.</summary>
        private static void WriteSeen(IMemoryApi mem, string domain, uint blk, byte[] beacons, byte myPlayer)
        {
            var seen = new byte[SeenSlots][];
            int pos = 1;
            for (int i = 0; i < beacons[0]; i++)
            {
                byte player = beacons[pos], len = beacons[pos + 1];
                if (player < 6 && player != myPlayer && myPlayer < 6)
                {
                    int remote = player < myPlayer ? player : player - 1;
                    byte[] b = new byte[8 + len];
                    Buffer.BlockCopy(beacons, pos + 4, b, 0, 4); // ggid
                    b[4] = len;
                    b[5] = beacons[pos + 2]; // entry
                    b[6] = player;
                    b[7] = beacons[pos + 3]; // seq
                    Buffer.BlockCopy(beacons, pos + 8, b, 8, len);
                    seen[remote] = b;
                }
                pos += 8 + len;
            }
            for (int k = 0; k < SeenSlots; k++)
            {
                uint at = blk + OffSeen + (uint)(k * BeaconSize);
                if (seen[k] != null) mem.WriteByteRange(at, seen[k], domain);
                else mem.WriteU8(at + 4, 0, domain); // no beacon
            }
        }

        /// <summary>Network or emulation thread: the link trailer for the next client packet, or null.</summary>
        public byte[]? BuildTrailer()
        {
            lock (_lock)
            {
                if (!_active) return null;
                long now = DateTime.UtcNow.Ticks;
                int start = _out.FindIndex(p => p.SentTicks == 0 || (now - p.SentTicks) / TimeSpan.TicksPerMillisecond >= ResendMs);
                var msgs = new List<Pending>();
                if (start >= 0)
                {
                    int budget = SendBudget;
                    for (int i = start; i < _out.Count && msgs.Count < MaxMsgsPerTrailer; i++)
                    {
                        int size = 4 + _out[i].Data.Length;
                        if (size > budget) break;
                        budget -= size;
                        _out[i].SentTicks = now;
                        msgs.Add(_out[i]);
                    }
                }
                var buf = new List<byte>(64);
                buf.AddRange(BitConverter.GetBytes(TrailerMagic));
                buf.Add(_epoch);
                buf.Add(_role);
                buf.Add((byte)(_beacon != null ? 1 : 0));
                buf.Add((byte)msgs.Count);
                buf.AddRange(BitConverter.GetBytes(_inNext));
                buf.AddRange(BitConverter.GetBytes(msgs.Count > 0 ? msgs[0].Seq : _outNext));
                if (_beacon != null) buf.AddRange(_beacon);
                foreach (var p in msgs)
                {
                    buf.Add(p.Kind);
                    buf.Add(p.Aid);
                    buf.AddRange(BitConverter.GetBytes((ushort)p.Data.Length));
                    buf.AddRange(p.Data);
                }
                return buf.ToArray();
            }
        }
    }
}
