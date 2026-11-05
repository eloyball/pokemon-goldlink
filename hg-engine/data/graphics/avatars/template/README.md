# Custom avatar template

One PNG per avatar, drawn on `avatar_template.png`. `avatar_template_guide.png` is the same
layout with Ethan's frames in every cell, at full opacity: use it as a reference layer, or start
from it and repaint Ethan. The build cuts the
finished sheet into the game's files using `avatar_layout.json`. Regenerate all three with
`python3 scripts/avatar_template.py`.

## Rules

- **Draw only inside the cells** (transparent, outlined in grey) and keep the canvas size. The
  grey header, gutters and the 2 px gaps between frames are ignored, so notes there are fine.
- **In an editor** (Aseprite, Photoshop, GIMP, Krita…), either:
  - open `avatar_template.png`, add the guide as a layer underneath, draw on a layer on top, and
    hide the guide before exporting; or
  - open the guide itself and paint over Ethan. Anything of Ethan left in a cell ends up in the
    game.
- **Transparency:** whatever isn't drawn stays transparent (alpha 0). No background colour.
- **Colours:** at most **15 colours plus transparency per column**. Each overworld state and each
  battle picture gets its own palette, so different columns may use different colours. No
  anti-aliasing or half-transparent pixels: each pixel is fully opaque or fully transparent.
- **Same poses, same order:** each frame must show the same pose and facing as Ethan's frame in
  that cell. The game reuses Ethan's animation tables, so frame 7 of "Walk / run" is shown
  exactly when Ethan's frame 7 would be.
- **Position:** keep the character where Ethan stands in each cell, with the feet at the same
  height. The shadow and the tile position are anchored to the frame.
  - Overworld frames are 32x32.
  - Battle front and throw frames are 80x80, each a whole picture. The game draws some throw
    frames shifted (frames 3-7 move up to 16 px right as the arm reaches out); keep the body where
    Ethan's is in each cell and the shift is applied for you (`offsets` in the layout).
  - The VS face is one 96x64 picture.

## Columns

Red label = must, black = should / optional. A column left empty falls back to the vanilla sprite
(Ethan's or Lyra's).

| Column | Frames | Shown when |
| :--- | :--- | :--- |
| Walk / run | 24 | walking and running (must) |
| Cycling | 22 | on the bike (must) |
| Surfing | 4 | surfing, Waterfall, Whirlpool (must) |
| HM use | 4 | Cut, Rock Smash, Strength, Rock Climb |
| Fishing | 16 | casting, waiting, reeling in |
| Saving | 2 | while saving |
| Healing | 6 | at a Pokemon Center counter |
| Pokegear | 12 | when a phone call comes in |
| Ladder | 6 | climbing ladders |
| Watering | 5 | the SquirtBottle |
| Apricorn shake | 8 | shaking Apricorn trees |
| Pokeathlon | 25 | in the Pokeathlon Dome |
| Rocket disguise / healing / saving | 26 / 6 / 2 | in the Team Rocket uniform (story) |
| Battle front | 1 | what your opponent sees in a battle against you (must) |
| Throw | 8 | your back picture throwing a Poke Ball (must) |
| Throw (link) | 8 | the same in link battles (PvP, team battles); empty = uses "Throw" |
| VS face | 1 | the battle intro's face close-up. Vanilla only shows the player's in the Elite Four and Champion intros; PvP has no intro yet, so showing it there is new work |
