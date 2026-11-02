using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;

namespace CoopBridge
{
    /// <summary>
    /// Emergency backups (docs/GAME_REBALANCE_UPDATE.md §23): an emulator
    /// savestate every few minutes while the player walks the overworld in a
    /// room, kept per room in coop_backups/ROOM-UID/, the newest few only.
    /// They are for crashes and game-breaking bugs that the in-game save
    /// cannot fix: restoring one rolls the whole game back, so progress since
    /// then is lost, and a trade made since then can duplicate a Pokemon (the
    /// partner keeps it too). The form restores only outside a room.
    /// No emulator access here: the form takes and loads the states.
    /// </summary>
    internal sealed class Backups
    {
        public static readonly TimeSpan Interval = TimeSpan.FromMinutes(5);
        public const int Keep = 12; // one hour at the interval
        private const string Extension = ".State";

        private readonly string _root;

        public Backups(string root) => _root = root;

        /// <summary>Last problem writing or pruning files, for the UI ("" = none).</summary>
        public string LastError { get; private set; } = string.Empty;

        public string RoomDir(string roomKey) => Path.Combine(_root, roomKey);

        /// <summary>A new file name for the room's next backup (its folder is created).</summary>
        public string? NextPath(string roomKey)
        {
            try
            {
                string dir = RoomDir(roomKey);
                Directory.CreateDirectory(dir);
                return Path.Combine(dir, $"{DateTime.Now:yyyyMMdd-HHmmss}{Extension}");
            }
            catch (IOException ex) { LastError = ex.Message; }
            catch (UnauthorizedAccessException ex) { LastError = ex.Message; }
            return null;
        }

        /// <summary>The room's backups, newest first.</summary>
        public List<FileInfo> List(string roomKey)
        {
            try
            {
                var dir = new DirectoryInfo(RoomDir(roomKey));
                if (!dir.Exists) return new List<FileInfo>();
                return dir.GetFiles("*" + Extension)
                    .Where(f => f.Length > 0)
                    .OrderByDescending(f => f.LastWriteTimeUtc)
                    .ToList();
            }
            catch (IOException ex) { LastError = ex.Message; }
            catch (UnauthorizedAccessException ex) { LastError = ex.Message; }
            return new List<FileInfo>();
        }

        /// <summary>Deletes all but the newest Keep backups of the room.</summary>
        public void Prune(string roomKey)
        {
            foreach (FileInfo old in List(roomKey).Skip(Keep))
            {
                try { old.Delete(); }
                catch (IOException ex) { LastError = ex.Message; }
                catch (UnauthorizedAccessException ex) { LastError = ex.Message; }
            }
        }
    }
}
