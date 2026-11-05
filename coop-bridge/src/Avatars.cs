using System;
using System.Collections.Generic;
using System.Drawing;
using System.IO;
using System.Text.RegularExpressions;

namespace CoopBridge
{
    /// <summary>
    /// The avatar choices (docs/CUSTOM_SPRITES_PLAN.md §1): coop_avatars.json,
    /// written by scripts/avatar_build.py beside coop_offsets.json. ID 0 is the
    /// save's own look, 1 Ethan, 2 Lyra, 3 + k the custom sheets. The choice is
    /// stored by key (the sheet's file name), so adding a sheet, which shifts
    /// the IDs, keeps everyone's pick. Without the file only the vanilla
    /// choices are offered.
    /// </summary>
    internal sealed class AvatarChoice
    {
        public byte Id;
        public string Key = string.Empty;   // "default", "ethan", "lyra" or the sheet's file name
        public string Name = string.Empty;
        public string? PreviewPath;
        public override string ToString() => Name;
    }

    internal static class Avatars
    {
        public const string ListFileName = "coop_avatars.json";

        public static List<AvatarChoice> Load(IEnumerable<string> searchPaths)
        {
            var list = new List<AvatarChoice>
            {
                new AvatarChoice { Id = 0, Key = "default", Name = "Default (by the save)" },
                new AvatarChoice { Id = 1, Key = "ethan", Name = "Ethan" },
                new AvatarChoice { Id = 2, Key = "lyra", Name = "Lyra" },
            };
            foreach (string path in searchPaths)
            {
                try
                {
                    if (!File.Exists(path)) continue;
                    string dir = Path.GetDirectoryName(Path.GetFullPath(path)) ?? ".";
                    string json = File.ReadAllText(path);
                    foreach (Match obj in Regex.Matches(json, @"\{[^{}\[\]]*\}"))
                    {
                        Match mId = Regex.Match(obj.Value, @"""id""\s*:\s*(\d+)");
                        Match mName = Regex.Match(obj.Value, @"""name""\s*:\s*""([^""]*)""");
                        Match mFile = Regex.Match(obj.Value, @"""file""\s*:\s*""([a-z0-9_]+)""");
                        Match mPreview = Regex.Match(obj.Value, @"""preview""\s*:\s*""([^""]+)""");
                        if (!mId.Success || !byte.TryParse(mId.Groups[1].Value, out byte id)) continue;
                        string? preview = mPreview.Success ? Path.Combine(dir, mPreview.Groups[1].Value) : null;
                        if (id < 3)
                        {
                            list[id].PreviewPath = preview;
                            continue;
                        }
                        if (!mFile.Success || !mName.Success) continue;
                        list.Add(new AvatarChoice { Id = id, Key = mFile.Groups[1].Value, Name = mName.Groups[1].Value, PreviewPath = preview });
                    }
                    break;
                }
                catch (IOException) { }
                catch (UnauthorizedAccessException) { }
            }
            return list;
        }

        /// <summary>The preview picture, or null. Copied so the file isn't kept locked.</summary>
        public static Image? LoadPreview(AvatarChoice choice)
        {
            if (choice.PreviewPath == null) return null;
            try
            {
                using (var stream = new MemoryStream(File.ReadAllBytes(choice.PreviewPath)))
                using (var image = Image.FromStream(stream))
                {
                    return new Bitmap(image);
                }
            }
            catch (IOException) { }
            catch (UnauthorizedAccessException) { }
            catch (ArgumentException) { }
            return null;
        }
    }
}
