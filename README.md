# terra-forge

Native Linux Terraria `.plr` character and inventory editor. Rust + `eframe`/`egui`.
Built to be safe first: it will not write a file it cannot reproduce byte-for-byte.

```
crates/terra-core     AES-128-CBC framing, C# BinaryReader primitives, .plr parse/serialise, backup
crates/terra-assets   6,196-item database + buffs/prefixes + fuzzy search (embedded JSON)
crates/terra-gui      the editor (lib + `terra-forge` binary)
crates/tforge-cli     headless `tforge` harness: info, verify, inventory, copy, selftest
tools/                item-data pipeline (read the installed game, not the internet)
```

## Build & run

```sh
cargo build --release
./target/release/terra-forge                 # opens the character picker
./target/release/terra-forge ~/.local/share/Terraria/Players/Testguy.plr
```

If a Wayland session matters (`eframe` picks the backend itself):

```sh
WAYLAND_DISPLAY=wayland-1 ./target/release/terra-forge
```

## Headless verification

```sh
tforge info      <file.plr>     # decode and summarise
tforge verify    <file.plr>     # decrypt -> parse -> serialise -> encrypt == original bytes
tforge inventory <file.plr>     # list non-empty slots with real item names
tforge copy      <file.plr>     # save through the GUI save path, then compare to source
tforge selftest  <file.plr>     # apply edits, save, reload, assert every edit landed
```

Results on this machine (Terraria 1.4.5.8, four real characters):

```
Testguy.plr            326    identical:   3840 bytes
Bayaz.plr              326    identical: 103056 bytes   (5,389 research entries)
Harmony_Driftmark.plr  279    identical:  41552 bytes
Michael.plr            279    identical:   3744 bytes
```

`selftest` on Bayaz and Testguy: place a Zenith (Mythical, favourited), put 9999 dirt in
the piggy bank, add a dye, raise max life to 500, add a research entry — all re-read
correctly after save, every untouched slot unchanged, trailing bytes preserved.

A `version = 194` (pre-1.4) file in the same folder is rejected with a clear error instead
of being guessed at.

## Format

See `../tforge.md` for the field-by-field layout, the per-container encoding table, and a
list of corrections to the original spec draft. Short version:

* AES-128-CBC, key == IV == UTF-16LE of `h3y_gUyZ`, PKCS#7 padding, one stream.
* Little-endian, .NET `BinaryWriter` strings (7-bit length + UTF-8).
* Every field is gated on the `version` i32: 1.4.4.0 = 269, 1.4.4.9 = 279, 1.4.5.0 = 315,
  1.4.5.7 = 325, 1.4.5.8 = 326.
* Item slots store a stack count only in "multi" containers, and a favourite byte only
  from the version that introduced it. Equipment slots store neither.
* Anything after the last known field is preserved verbatim in `trailing`.

## Saving

`terra_core::save_with_backup` copies the original to `<name>.plr.bak`, writes a temp file,
reads it back, decrypts it and compares it with the payload before renaming over the
original. If the comparison fails, the original is untouched.

## Item data

`crates/terra-assets/data/items.json` is generated, not hand-written:

```sh
python3 -m pip install dnfile
python3 tools/extract_from_install.py ~/.local/share/Steam/steamapps/common/Terraria/Terraria.exe /tmp/ex.json
python3 tools/gen_items.py /tmp/ex.json tools/snapshot_1.4.4.9.json
```

* `extract_from_install.py` reads `ItemID`/`BuffID`/`PrefixID` constants out of the CLR
  metadata and the `Terraria.Localization.Content.en-US.Items.json` embedded resource, so
  ids and names come from **whatever game version is installed**. On this box: 1.4.5.8 →
  6,144 ids, 6,184 names.
* `snapshot_1.4.4.9.json` supplies max stack, rarity, tags, research goals, damage and
  defense per id. It came from terrasavr's public dataset and is vendored here so the
  build does not depend on that site. Items newer than the snapshot keep their real name
  and get `estimated = true` with a permissive stack cap (740 of 6,196 today).

Stats cannot be derived from any data file — they are compiled into `Item.SetDefaults`.
That is why they come from a snapshot rather than from the install; see tforge.md §0.

## Not implemented

* `.tplr` / tModLoader support (dropped from scope; tModLoader is still on 1.4.4.9).
* Sprite decoding: `Content/Images/Item_<id>.xnb` are LZX-compressed; slots currently show
  rarity-coloured tiles with item initials instead.
* Dual-character transfer pane, sprite tooltips, restoring a `.bak` from inside the app,
  editing Journey creative powers (they are preserved verbatim, not edited), editing the
  research dictionary beyond appending.
* Drag-and-drop by mouse-drag: pick-and-place (click item in browser → click slot),
  right-click "pick up", and middle-click duplicate cover the workflow.

## Keyboard / mouse

| action | effect |
|---|---|
| Ctrl+S | save (with backup) |
| click slot | select it |
| click slot while holding an item | place it (moving if it was picked up) |
| right-click slot | pick up / duplicate / max stack / favourite / clear |
| middle-click slot | duplicate that item onto the cursor |
| double-click slot | pick it up |
| wheel over slot | ±1 stack (Shift: ±100) |
| click item in browser | place in the selected slot, or hold it if nothing is selected |