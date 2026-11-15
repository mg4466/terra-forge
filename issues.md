## Status: all six addressed (build 2026-11-15)

1. **Picker shows paths / wraps** — now lists in-game character names only, grouped under
   `LOCAL` / `STEAM CLOUD · account …` / `TMODLOADER`, with `v326 · 1.4.5.8+ · 43 items`
   underneath and the full path in the tooltip. The window is wider and rows do not wrap.
   Unreadable/old files show why (`unsupported pre-1.4 player format version 194`).
2. **Dialog stayed open after selecting** — fixed (the "open" flag was being written back
   from a stale local). It also no longer pops up when you pass a file on the command line.
3. **Had to use right-click → "Pick up"** — real drag-and-drop now: press and drag a slot
   onto another slot to swap; drag an item out of the browser onto any slot to add it; a
   ghost tile follows the cursor; the drop target highlights. Right-click menu still exists,
   plus middle-click duplicate and double-click.
4. **Text too small** — `Aa` button in the top bar opens Display settings: zoom 0.8–2.0
   (default 1.25) and slot size 36–72 px, saved to `~/.config/terra-forge/settings.json`.
   Base font sizes were raised too, and the grids now shrink cells (or wrap) rather than
   clip when a panel is narrow.
5. **Sliders / unclear "Base" button** — Life and Mana are plain numeric fields (click to
   type, drag to nudge). The two buttons are now `Fill to max` (current → max) and
   `Max out` (life 500 / mana 200 + fill), each with a tooltip.
6. **One giant item list** — the browser is grouped into section headers. Browsing "All"
   gives `WEAPONS · ARMOUR · ACCESSORIES · VANITY, PETS & MOUNTS · BLOCKS & FURNITURE ·
   CONSUMABLES & AMMO · MATERIALS · OTHER`, each sorted by rarity then name. Selecting a
   pill subdivides further (Weapons → Melee / Ranged / Magic / Summon / Sentry;
   Armour → Helmets / Body / Legs; Blocks → Blocks / Walls / Paintings). Typing in the
   search box switches to a flat relevance-ranked list, because sections get in the way
   when you know what you want.

If the grouping still is not what you had in mind: the next natural step is grouping by
*progression tier* (pre-boss / pre-hardmode / hardmode / post-Moon Lord) instead of by type,
which needs a tier source we do not have yet.

---

## Original notes

1. the open character menu is weird. It shows the characters with the long path to the character, rather than just showing the character names that are saved on the system, and the little window is having to word-wrap with one of the players. that just doesn't look great.
2. When I select a player file from the Open Character menu, the character info populates in the editor below, but the menu stays open over top of the main editor.
3. I can't just drag an item to another slot, I have to right click and select "pick up" - that is an odd thing to require. We should definitely have a context menu, but the user shouldn't have to use it to drag and drop an item.
4. the text is a little too small. If you want to make it that small then you need to make it adjustable for the old farts.
5. I don't think we need sliders for max life, life, max mana, mana, etc. Having a editable text box is sufficient and takes up less space. I'm not sure what the little "base" button is supposed to be for? I can't tell that it does anything.
6. have the items grouped somehow instead of all in a giant list. Well I guess you have it separated by weapons armor etc. but it should have some category subdivisions or something, that isn't organized well. If we need to talk more about it you can ask me questions.
