using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text;
using System.Text.RegularExpressions;

namespace CoopBridge
{
    /// <summary>One world this install joined or created (docs/MILESTONE6_PLAN.md §4.2).</summary>
    internal sealed class WorldEntry
    {
        public string Code = string.Empty;   // 6 letters or digits, upper case
        public string Server = string.Empty; // host:port it was played on ("" = unknown: a save from before the list)
        public bool Created;                 // this install created it (else joined)
        public long LastPlayed;              // Unix seconds (0 = unknown)

        /// <summary>What the drop-down shows; picking an entry puts only the code in the field.</summary>
        public override string ToString()
        {
            string how = Created ? "created" : "joined";
            string where = Server.Length > 0 ? Server : "server unknown";
            return $"{Code}  ·  {how}  ·  {where}";
        }
    }

    /// <summary>
    /// The worlds this install played, most recent first, in coop_worlds.json next to
    /// coop_config.json (its own file: the config is rewritten by several paths that know
    /// nothing of the list). Seeded once from coop_saves/CODE-UID.sav, so worlds played before
    /// the list existed appear too. Forgetting a world drops the entry, never its save.
    /// </summary>
    internal sealed class WorldList
    {
        public const string FileName = "coop_worlds.json";
        private const int MaxEntries = 50;

        private readonly string _path;
        private readonly List<WorldEntry> _entries = new List<WorldEntry>();

        public WorldList(string configDir, string savesDir)
        {
            _path = Path.Combine(configDir, FileName);
            if (!Load())
            {
                SeedFromSaves(savesDir);
                Save();
            }
        }

        public IReadOnlyList<WorldEntry> Entries => _entries;

        public WorldEntry? Find(string code) => _entries.FirstOrDefault(e => e.Code == code);

        /// <summary>A world was joined or created on this server: to the top of the list.</summary>
        public void Record(string code, string server, bool created)
        {
            WorldEntry? entry = Find(code);
            if (entry == null)
            {
                entry = new WorldEntry { Code = code };
                _entries.Add(entry);
            }
            entry.Server = server;
            entry.Created |= created;
            entry.LastPlayed = DateTimeOffset.UtcNow.ToUnixTimeSeconds();
            Sort();
            while (_entries.Count > MaxEntries) _entries.RemoveAt(_entries.Count - 1);
            Save();
        }

        public void Forget(string code)
        {
            if (_entries.RemoveAll(e => e.Code == code) > 0) Save();
        }

        private void Sort() => _entries.Sort((a, b) => b.LastPlayed.CompareTo(a.LastPlayed));

        private void SeedFromSaves(string savesDir)
        {
            try
            {
                if (!Directory.Exists(savesDir)) return;
                foreach (string file in Directory.GetFiles(savesDir, "*.sav"))
                {
                    Match m = Regex.Match(Path.GetFileName(file), "^([A-Z0-9]{6})-[0-9A-F]{8}\\.sav$");
                    if (!m.Success || Find(m.Groups[1].Value) != null) continue;
                    _entries.Add(new WorldEntry
                    {
                        Code = m.Groups[1].Value,
                        LastPlayed = new DateTimeOffset(File.GetLastWriteTimeUtc(file)).ToUnixTimeSeconds()
                    });
                }
                Sort();
            }
            catch (IOException) { }
            catch (UnauthorizedAccessException) { }
        }

        // The file is a flat JSON list written below; read back with the same shape only.
        private bool Load()
        {
            try
            {
                if (!File.Exists(_path)) return false;
                string json = File.ReadAllText(_path);
                foreach (Match m in Regex.Matches(json, "\\{[^{}]*\\}"))
                {
                    string obj = m.Value;
                    string code = Str(obj, "code");
                    if (!Regex.IsMatch(code, "^[A-Z0-9]{6}$") || Find(code) != null) continue;
                    _entries.Add(new WorldEntry
                    {
                        Code = code,
                        Server = Str(obj, "server"),
                        Created = Regex.IsMatch(obj, "\"created\"\\s*:\\s*true"),
                        LastPlayed = long.TryParse(Regex.Match(obj, "\"last_played\"\\s*:\\s*(\\d+)").Groups[1].Value, out long t) ? t : 0
                    });
                }
                Sort();
                return true;
            }
            catch (IOException) { return false; }
            catch (UnauthorizedAccessException) { return false; }
        }

        private static string Str(string obj, string key)
        {
            Match m = Regex.Match(obj, "\"" + key + "\"\\s*:\\s*\"((?:[^\"\\\\]|\\\\.)*)\"");
            return m.Success ? m.Groups[1].Value.Replace("\\\"", "\"").Replace("\\\\", "\\") : string.Empty;
        }

        private static string Esc(string s) => s.Replace("\\", "\\\\").Replace("\"", "\\\"");

        private void Save()
        {
            var sb = new StringBuilder("{\n  \"worlds\": [\n");
            for (int i = 0; i < _entries.Count; i++)
            {
                WorldEntry e = _entries[i];
                sb.Append($"    {{ \"code\": \"{e.Code}\", \"server\": \"{Esc(e.Server)}\", \"created\": {(e.Created ? "true" : "false")}, \"last_played\": {e.LastPlayed} }}");
                sb.Append(i + 1 < _entries.Count ? ",\n" : "\n");
            }
            sb.Append("  ]\n}\n");
            try { File.WriteAllText(_path, sb.ToString()); }
            catch (IOException) { }
            catch (UnauthorizedAccessException) { }
        }
    }
}
