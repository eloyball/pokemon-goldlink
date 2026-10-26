using System;
using System.IO;
using System.Security.Cryptography;
using System.Text;

namespace CoopBridge
{
    /// <summary>
    /// One save file per room (docs/MILESTONE2_PLAN.md 7.2). The cartridge
    /// save is the emulator's "SRAM" memory domain; the bridge swaps the room's
    /// image in before the game loads it and copies it back after every
    /// in-game save. Files live next to this instance's coop_config.json:
    /// coop_saves/ROOM-UID.sav (+ .bak), and sram_owner.txt records which room
    /// the image currently in SRAM belongs to and the hash of the last image
    /// the bridge wrote or copied, so progress made while the bridge was not
    /// watching (closed or crashed after a save) is never lost.
    /// No emulator access here: the form passes SRAM images in and out.
    /// </summary>
    internal sealed class SaveSlots
    {
        public const int SramSize = 524288;
        public const string SramDomain = "SRAM";
        private const string OwnerFileName = "sram_owner.txt";

        private readonly string _dir;
        private string _owner = string.Empty; // room key of the image in SRAM; "" = unknown
        private string _hash = string.Empty;  // hash of the last image written to / copied from SRAM

        public SaveSlots(string dir)
        {
            _dir = dir;
            try
            {
                string path = Path.Combine(_dir, OwnerFileName);
                if (File.Exists(path))
                {
                    string[] lines = File.ReadAllLines(path);
                    if (lines.Length >= 2)
                    {
                        _owner = lines[0].Trim();
                        _hash = lines[1].Trim();
                    }
                }
            }
            catch (IOException) { }
            catch (UnauthorizedAccessException) { }
        }

        /// <summary>Room the image currently in SRAM belongs to ("" = unknown).</summary>
        public string Owner => _owner;

        /// <summary>Last problem writing files, for the UI ("" = none).</summary>
        public string LastError { get; private set; } = string.Empty;

        public static string RoomKey(string roomCode, uint roomUid) => $"{roomCode}-{roomUid:X8}";

        public static string Hash(byte[] image)
        {
            using (var sha = SHA1.Create())
            {
                var sb = new StringBuilder(40);
                foreach (byte b in sha.ComputeHash(image)) sb.Append(b.ToString("x2"));
                return sb.ToString();
            }
        }

        public static byte[] Blank()
        {
            byte[] image = new byte[SramSize];
            for (int i = 0; i < image.Length; i++) image[i] = 0xFF; // erased flash: no save, New Game only
            return image;
        }

        public bool HasRoomSave(string key) => File.Exists(RoomFile(key));

        /// <summary>
        /// Picks the image for a room the player just joined (at the title
        /// screen, before a save is in play). Keeps the current SRAM first:
        /// for its owner room when it changed since the bridge last saw it,
        /// or as unassigned-*.sav when its owner is unknown. Returns the image
        /// to write, or null when SRAM already holds the room's save.
        /// </summary>
        public byte[]? PrepareJoin(string key, byte[] sram)
        {
            string current = Hash(sram);
            if (_owner.Length > 0 && current != _hash)
                Store(RoomFile(_owner), sram);
            else if (_owner.Length == 0 && !IsBlank(sram))
                Store(Path.Combine(_dir, $"unassigned-{DateTime.Now:yyyyMMdd-HHmmss}.sav"), sram);

            byte[] target = Blank();
            string file = RoomFile(key);
            if (File.Exists(file))
            {
                try
                {
                    byte[] saved = File.ReadAllBytes(file);
                    if (saved.Length == SramSize) target = saved;
                    else LastError = $"{Path.GetFileName(file)} has the wrong size; starting a new save.";
                }
                catch (IOException ex) { LastError = ex.Message; }
                catch (UnauthorizedAccessException ex) { LastError = ex.Message; }
            }

            string wanted = Hash(target);
            _owner = key;
            _hash = wanted;
            SaveOwner();
            return wanted == current ? null : target;
        }

        /// <summary>The game finished writing a save: keep it for the room that owns SRAM.</summary>
        public void Capture(byte[] sram)
        {
            if (_owner.Length == 0) return;
            Store(RoomFile(_owner), sram);
            _hash = Hash(sram);
            SaveOwner();
        }

        private string RoomFile(string key) => Path.Combine(_dir, key + ".sav");

        private static bool IsBlank(byte[] image)
        {
            byte first = image[0];
            if (first != 0xFF && first != 0x00) return false;
            foreach (byte b in image) if (b != first) return false;
            return true;
        }

        // Temp file + rename, keeping the previous copy as .bak.
        private void Store(string path, byte[] image)
        {
            try
            {
                Directory.CreateDirectory(_dir);
                string tmp = path + ".tmp";
                File.WriteAllBytes(tmp, image);
                if (File.Exists(path))
                {
                    File.Copy(path, path + ".bak", overwrite: true);
                    File.Delete(path);
                }
                File.Move(tmp, path);
                LastError = string.Empty;
            }
            catch (IOException ex) { LastError = ex.Message; }
            catch (UnauthorizedAccessException ex) { LastError = ex.Message; }
        }

        private void SaveOwner()
        {
            try
            {
                Directory.CreateDirectory(_dir);
                string path = Path.Combine(_dir, OwnerFileName);
                File.WriteAllText(path + ".tmp", _owner + "\n" + _hash + "\n");
                if (File.Exists(path)) File.Delete(path);
                File.Move(path + ".tmp", path);
            }
            catch (IOException ex) { LastError = ex.Message; }
            catch (UnauthorizedAccessException ex) { LastError = ex.Message; }
        }
    }
}
