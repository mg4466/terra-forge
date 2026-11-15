# Learning guide: how terra-forge works

This is a guided tour of the code for someone who wants to understand *why* it is
built this way, not just what it does. Every section points at real functions you can
open and read.

---

## 1. The shape of the thing

```
Terraria saves a character as an encrypted blob.
We want to edit it without breaking it.
```

That sentence produces the whole architecture:

| crate | job | key idea |
|---|---|---|
| `terra-core` | bytes ⇄ `PlayerFile` | the only code that knows the file format |
| `terra-assets` | item names/ids/stats | a generated database, not hand-written tables |
| `terra-gui` | the editor | mutates a `PlayerFile` in memory; never touches bytes |

The GUI holds one `PlayerFile`. Everything it draws is a read of that struct, and every
edit is a write to it. Bytes only appear at two moments: `terra_core::load` and
`terra_core::save_with_backup`.

Why this split matters: the file format is the risky part. By keeping it in one crate
with no UI dependencies, it can be tested headlessly (and it is — `cargo test`, plus the
`tforge` CLI against real saves).

---

## 2. The encryption, in ten lines

`crates/terra-core/src/crypto.rs`

```rust
pub const KEY_BYTES: [u8; 16] = [
    0x68, 0x00, 0x33, 0x00, ... // "h3y_gUyZ" encoded as UTF-16LE
];
```

Terraria encrypts the whole payload with AES-128-CBC and — this is the interesting part —
uses the **same 16 bytes for the key and the IV**. That is a crypto smell in general, but
here it is a gift: CBC with a fixed IV is deterministic, so

```
decrypt(file) → parse → serialise → encrypt
```

must reproduce the original file **byte for byte**. That single property is what lets us
claim "this editor does not corrupt saves", and it is what `tforge verify` checks on real
files.

`encrypt`/`decrypt` also handle PKCS#7 padding. Note `decrypt` rejects input that is not a
multiple of 16 bytes; a `.plr` always is, so a failure there means the file is not a
player file at all.

---

## 3. Reading bytes the way .NET does

`crates/terra-core/src/cursor.rs`

Terraria is a C# game, so the file was written by `BinaryWriter`. Our `Reader` mirrors
exactly those primitives: little-endian `i32`, `u8`, `f32`, and — the one people get wrong
— strings:

```rust
// .NET BinaryWriter.Write(string):
//   7-bit LEB128 byte length, then UTF-8 bytes
pub fn string(&mut self) -> Result<String, IoError> {
    let mut len = 0u64;
    let mut shift = 0;
    loop {
        let b = self.u8()?;
        len |= ((b & 0x7F) as u64) << shift;
        if b & 0x80 == 0 { break; }
        shift += 7;
    }
    ...
}
```

A player name is at most a few dozen bytes, so one length byte; a long research id needs
two. The unit test `dotnet_string_prefixes` pins both cases, including the exact prefix
bytes for a 300-character string (`0xAC 0x02`).

`Reader::take` returns `IoError::Eof { offset, needed }` rather than panicking. That
matters: while reverse-engineering, "EOF at 2755 when 18 bytes were requested" is the
diagnostic that tells you which field you got wrong.

---

## 4. The parser: one long function, and why

`crates/terra-core/src/plr.rs`, `PlayerFile::parse`

The format is a straight-line sequence of fields — there is no tree, no random access. So
the parser is a straight-line function:

```rust
let v = r.i32()?;                       // format version
p.magic_a = r.u32()?;                   // "relo"
p.magic_b = r.u32()?;                   // "gic" + file type 3
```

Two ideas make it survivable across game updates:

**(a) Version gates.** Every field added in some patch sits behind `if v >= N`:

```rust
if v >= 82 { p.hair_dye = r.u8()?; }
if v >= 315 { p.team = r.u8()?; }
```

The numbers come from real files and from the game's own build: 1.4.4.0 writes 269,
1.4.4.9 writes 279, 1.4.5.8 writes 326. Fields are appended as the number grows, so a gate
is usually "does this field exist yet".

**(b) Trailing bytes.** After the last field we understand, whatever is left is kept:

```rust
p.trailing = r.rest().to_vec();
```

and written back on save. So an unknown future field survives the round trip even though we
cannot edit it. That is the single most important safety net in the project.

### The slot encoding, and the bug that proved the model

An item slot is not the same everywhere. `SlotKind` encodes the difference:

```rust
struct SlotKind { multi: bool, fav_min: i32 }
```

* `multi` = the slot stores an i32 stack count.
* `fav_min` = the version from which a favourite byte follows.

Equipment slots (`multi: false`) store neither a stack nor — before version 325 — a
favourite flag. Bank slots store a stack but no favourite byte.

Getting this wrong shifts *everything downstream* by a whole number of bytes. During
development the parse ended exactly 10 bytes late on every file, which turned out to be one
extra slot's worth of data: the **active** loadout does not store its 10 hide-visual bytes
(those live in the player-level `hide_visual` u16); only the three stored loadouts do.

**How that was found**, and the technique worth stealing:

1. Decrypt a real file and walk the layout, printing the byte offset after each section.
2. Compare against the payload length.
3. Sections are mostly zero bytes, so a wrong boundary is invisible… until the **server
   list terminator** (`i32 -1`) lands at the wrong offset. That sentinel is the anchor that
   makes misalignment obvious.

If you ever extend this parser, keep that tracing habit: `/tmp` scripts aside, the fastest
debug is "print the offset and the next four bytes after each block".

---

## 5. Proving correctness, not hoping for it

`crates/terra-core/src/file.rs`

`save_with_backup` is deliberately paranoid:

```rust
let plain = player.serialize();
let encrypted = crypto::encrypt(&plain)?;   // 1. build what we intend to write
fs::write(&bak, &existing)?;                // 2. copy the old file to .bak
fs::write(&tmp, &encrypted)?;               // 3. write somewhere else
let check = fs::read(&tmp)?;                // 4. read it back
if crypto::decrypt(&check)? != plain {      // 5. prove it round-trips
    let _ = fs::remove_file(&tmp);
    return Err(FileError::RoundTripMismatch(0));  // 6. refuse
}
fs::rename(&tmp, path)?;                    // 7. only now replace the original
```

Step 5 is cheap and catches an entire class of bugs (bad padding, off-by-one in a gate,
wrong slot width). The cost is one extra decrypt per save — microseconds.

The CLI exposes this to you:

```sh
tforge verify   <file.plr>   # decrypt → parse → serialise → encrypt == original bytes
tforge selftest <file.plr>   # edit, save, reload, assert every edit stuck
```

`verify` is the regression test you should run after touching `plr.rs`. `selftest` is the
one that proves editing works, not just round-tripping.

---

## 6. Where the item data comes from

`crates/terra-assets`, generated by `tools/gen_items.py`

Two sources, deliberately separate:

1. **From the installed game** (`tools/extract_from_install.py`): the `ItemID` constants are
   `const` fields in the .NET metadata, and the display names sit in an embedded resource
   `Terraria.Localization.Content.en-US.Items.json`. Reading those means the id↔name mapping
   always matches the version the player actually runs.
2. **From a stats snapshot** (`tools/snapshot_1.4.4.9.json`): max stack, rarity, tags,
   research goals, damage, defence. These live inside `Item.SetDefaults` in compiled IL and
   exist in no data file anywhere, so they cannot be read out.

Items newer than the snapshot get a real name and `estimated = true`, which the browser shows
as a note. That is an honest degradation, not a silent lie.

The DB is a flat JSON array, parsed once into a `OnceLock<ItemDb>`:

```
[id, pid, name, maxStack, rarity, tagsBitmask, researchGoal, damage, defense, estimated]
```

Compact arrays instead of objects keep the embedded file at ~400 KB and parse in ~10 ms.

---

## 7. The GUI: immediate mode, and the two traps

`crates/terra-gui/src/lib.rs`

egui has no retained widget tree. `App::ui()` runs top to bottom every frame; the only state
is in `App`. Two traps that cost real time here:

**Panel order.** Panels must be added before the central panel or they get no space:

```rust
self.top_bar(ui);
self.status_bar(ui);   // bottom panel: must come before the centre
self.side_left(ui);
self.side_right(ui);
self.central(ui);      // fills whatever is left
```

**Transient state must be stored.** A drag in progress is not remembered by egui between
frames, so `App::drag: Option<Drag>` holds it.

**Order of input handling matters.** Clearing a drag when the pointer is released *at the top
of the frame* looks harmless and silently breaks every drop: the release is consumed before
the slots are drawn, so no slot ever sees a drag on its release frame. The cancel-on-release
check therefore runs at the *end* of `ui()`, after the slots had their chance. Escape and
right-click cancels run at the top, because those are unambiguous.

### Drag and drop, as a state machine

```
drag_started()  → App::drag = Some(Drag { side, from: Some(slot), item })
pointer released over slot X
                → operations::drop_item / transfer_item
                → App::drag = None
pointer released over nothing
                → App::drag = None   (cancel)
```

`Drag::from` is `None` when the item came from the browser, which turns a drop into a copy
instead of a swap. Painting follows the pointer through a tooltip-layer painter
(`App::drag_ghost`), which is how the game does it too.

### One model, two characters

Stash mode is the same drag machine with one more dimension. A `SlotRef` only says
*container + index*, so `Side::{Primary, Stash}` is carried alongside it — in `Drag` and in
`App::selected_side`. `App::player_of(side)` / `player_mut(side)` are the only places that
decide which `PlayerFile` a reference means; every drawing and editing routine takes a
`Side` instead of reading `self.player` directly.

Cross-character moves go through `operations::transfer_item(src, dst, ...)`, which takes two
players because Rust will not let you borrow one `PlayerFile` mutably twice. Same-character
moves keep using `drop_item`. Both implement the same swap-or-copy rule, and both are unit
tested, so the "drop onto the stash" path is not special-cased UI code.

### Why the edit operations are free functions

`crates/terra-gui/src/operations.rs` takes `&mut PlayerFile` and returns nothing. No `App`,
no egui, no context — which is exactly why it can have unit tests:

```rust
#[test]
fn move_between_inventory_slots_swaps_contents() { ... }
```

One of those tests (`dropping_an_empty_item_is_a_no_op`) caught a real bug: dropping an
"empty" drag used to erase the destination slot. That class of bug is invisible in manual
testing and trivial to catch with a test.

### The catalogue layer

`crates/terra-gui/src/catalog.rs` turns 6,196 items into a list of rows:

```rust
enum Row { Header(&'static str), Item(i32) }
```

Browsing → grouped by tag (Weapons / Armour / Accessories / …), each group sorted by rarity
then name, with "first matching group wins" so nothing appears twice. Searching → the
fuzzy ranking from `terra-assets`, no headers. The UI then virtualises the flat row list
with `show_rows`, so only visible rows cost anything.

---

## 8. Item sprites: what an XNB is

`crates/terra-assets/src/sprites.rs`

Terraria stores each item's artwork as `Content/Images/Item_<id>.xnb`, an XNA content file:

```
'X' 'N' 'B' 'w'  version(u8)  flags(u8)  fileSize(u32)
compressed?  decompressedSize(u32) then payload
```

`flags & 0x80` means LZX-compressed, and the payload has a framing that is not part of the
LZX spec: every 32 KiB chunk is preceded by a big-endian length, and chunks whose first byte
is `0xFF` also carry their uncompressed length. After decompression you get the ordinary XNB
object graph — a type-reader table, then a `Texture2D` with surface format, width, height,
mip count and pixel data. Terraria always writes surface format 0 (straight RGBA8, one mip),
which is why the buffer can go straight to the GPU (`ColorImage::from_rgba_unmultiplied`).

Two decisions in the GUI are worth understanding:

* **Decode on demand, not up front.** All 6,134 vanilla sprites decode in ~0.3 s total, about
  0.05 ms each, so a lazy per-slot decode beats any import step. `SpriteCache` in
  `crates/terra-gui/src/lib.rs` keeps the textures actually on screen and evicts the rest;
  misses are cached too, so a missing file is never re-read per frame. Eviction is true LRU —
  a cache hit is *promoted* to the back of the queue — because a plain insertion-ordered
  queue turns scrolling the 6,196-item catalogue into an eviction machine that throws away
  the sprites of the equipped gear on screen.
* **Never upscale, boost only integers.** Drawing 1:1 is what the game does and it keeps
  pixel art crisp; but a 12 px coin would vanish in a 48 px slot, so sprites smaller than
  half the slot get an integer 2×. That rule is why coins and dirt blocks stay readable
  without swords looking blurry.
* **One frame, not the whole sheet.** Animated items are vertical strips: a soul's texture is
  22×88, four frames. `Sprite::frames()` recognises a strip geometrically (height an exact
  multiple of width) and the cache stores a cropped UV rect for the first frame. Drawing the
  full strip would silently render every animated item at 1/frames size — a bug visible only
  if you happen to look at the right item.

## 9. Exercises, if you want to change something

Each of these is a real, contained task:

1. **Add a read-only field to the Header tab.** Find the field in `plr.rs`, add it to the
   struct, parse and serialise it, then show it in `tab_raw`. Run `tforge verify` on all
   four sample players — if the gate or the width is wrong, it fails immediately.
2. **Add a new container.** Add a variant to `Bag`, wire `container`/`container_mut`, add a
   `section()` call. Run `cargo test -p terra-gui` — `containers::tests` will tell you if the
   multi/favourite rules disagree with the parser.
3. **Add an item category.** Add a `mask_` function in `catalog.rs` and a pill in
   `side_right`. The tests assert that every item lands in exactly one group.
4. **Animate the sprite frames.** `Sprite::frame_height()` already finds the frame boundary
   and the cache stores the top frame's UV rect; cycling through frames on hover (advance the
   UV rect's `min.y`/`max.y`) is a contained extension, and it makes the animated items look
   alive instead of frozen on their first frame.

---

## 10. Debugging recipes

* **"string length 18446744073709551615"** — you read a 7-bit length where there was none;
  your cursor is misaligned. Print the offset.
* **Parse succeeds but a field looks wrong** — check the version gate upstream of it; one
  missing byte shifts the rest.
* **Save loads in the editor but not in Terraria** — run `tforge verify` on the *output*.
  If that passes, the payload is self-consistent, so you are probably writing a semantically
  invalid value (e.g. a stack larger than the item's max, a prefix on an item that has none).
  The game clamps those, so the usual symptom is a changed value rather than a broken file.
* **UI state confusion** — remember there is no widget tree; every value you see came from
  `App`'s fields this frame.