# Technical Specification v2: `terra-forge` (Native Linux Terraria Character & Inventory Editor)

This supersedes the original draft. Everything below marked **(verified)** was checked
against real files or the installed game on this machine, not assumed.

Build status: `crates/terra-core`, `crates/terra-assets` and `crates/tforge-cli` are
implemented and tested; `crates/terra-gui` runs as a working editor (see README).

---

## 0. Corrections to the original draft

| Original claim | Reality (verified) |
|---|---|
| `version = 279` for 1.4.4.9, `280+` for 1.4.5 | 1.4.4.9 = **279**, 1.4.5.0 = **315**, 1.4.5.7 = **325**, 1.4.5.8 = **326**. The number advances every patch. |
| "5,500+ items" | 1.4.5.8 has IDs **0..6195** (6,196 entries), of which 6,126 carry a display name. |
| Magic = `[u8;7]` + type byte, then `revision` + `is_favorite` | `"relogic"` + `0x03` = two `u32`s (`0x6F6C6572`, `0x03636967`), then **three** `u32` metadata fields (`metaVersion`, `metaFlags1`, `metaFlags2`). No `revision`/`is_favorite` header fields exist. |
| Slot = `{net_id: i32, stack: i32, prefix: u8, favorited: bool}` | Order is netID, **then stack only in "multi" containers**, then prefix, then favorited only when the format version reaches that container's gate. Armour/vanity/dyes and the five misc slots store **no stack at all**; bank/safe/forge store **no favourite byte**. |
| "3 `EquipmentLoadout` structs" | Four loadouts exist; the **active** one is written inline with the equipment and has **no** hide-visual bytes (those are the player-level `hideVisual` u16). The three stored loadouts each carry 10 extra hide bytes. |
| `trailing_unparsed_bytes` | Kept, and used: the reader consumes exactly to the end (trailing = 0 on all four sample players) but still round-trips unknown tail data verbatim. |
| Steam XNB sprite extraction | Not implemented. `Content/Images/Item_<id>.xnb` are LZX-compressed (`XNBw`, flags `0x81`); 6,134 item sprites exist. Slots currently render rarity-coloured tiles with item initials. |
| tModLoader `.tplr` support | **Dropped** (agreed out of scope). tModLoader is also still on 1.4.4.9 (5,443 names, IDs ≤5455) while vanilla is 1.4.5.8, so it would have been a second, older format. |
| `egui`/`eframe` 0.37 with `App::update` + `SidePanel`/`TopBottomPanel` | 0.37 replaced these: `eframe::App::ui(&mut self, ui: &mut Ui, frame)` and a unified `egui::Panel::{left,right,top,bottom}` taking a `Ui`. There is no `SidePanel`/`TopBottomPanel` any more. |

---

## 1. Goals

A fast native Linux Terraria `.plr` editor: immediate-mode UI, mouse-driven inventory
work, instant fuzzy item search, and **non-destructive** serialisation — a file that is
loaded and saved without edits must be byte-identical, or nothing is written.

## 2. Workspace

```
terra-forge/
├── crates/terra-core/     crypto, C# BinaryReader primitives, .plr parse/serialise, backup
├── crates/terra-assets/   embedded item/prefix/buff database + fuzzy search
├── crates/terra-gui/      eframe/egui editor (lib + `terra-forge` binary)
└── crates/tforge-cli/     headless `tforge` harness (info/verify/inventory/copy/selftest)
```

Dependencies actually used: `aes 0.8`, `cbc 0.1`, `cipher 0.4`, `thiserror 2`, `serde 1`,
`serde_json 1`, `eframe 0.37`. No `binrw`, no `image`, no `rfd` (file picking is in-app to
avoid GTK/portal dependencies on wlroots boxes).

## 3. Binary format (verified against 1.4.4.9 and 1.4.5.8 files)

### 3.1 Framing
AES-128-CBC, key == IV == UTF-16LE of `h3y_gUyZ`, PKCS#7 padding over the whole payload.
Re-encrypting identical plaintext reproduces the original file exactly (fixed IV).

### 3.2 Field order for a 326 file (differences for 279 are noted)

```
i32 version                                       (>=225 supported)
u32 magic_a 0x6F6C6572 ("relo")
u32 magic_b 0x03636967 ("gic" + file type 3)
u32 meta_version, u32 meta_flags1, u32 meta_flags2
string name                     (C# 7-bit length prefix + UTF-8)
u8 difficulty
u32 play_time_low, u32 play_time_high             (>=145)
i32 hair_style
u8 hair_dye                     (>=82)
u8 team                         (>=315)
u8 hide_visual, u8 hide_visual_hi                 (>=83, second byte >=145)
u8 hide_misc, u8 gender                           (>=145)
i32 life, i32 max_life, i32 mana, i32 max_mana
u8 extra_accessory                                (>=145)
u8 unlocked_biome_torches, u8 using_biome_torches (>=230)
u8 ate_artisan_bread                              (>=269)
u8 mystery_flag_1470                              (>=325)
u8[6] extra_using_flags                           (>=269)
u8 finished_dd2_event                             (>=184)
i32 tax_money
i32 pve_deaths, i32 pvp_deaths                     (>=269)
7x RGB  hair, skin, eye, shirt, undershirt, pants, shoes
active loadout: 10 items, 10 social, 10 dyes       (NO hide bytes)
50 inventory slots
4 coins, 4 ammo
5x (equipment item, equipment dye)                (interleaved)
40 bank, 40 safe, 40 forge, 40 void bag
u8 void_vault_byte                                (>=200)
44x (i32 buff id, i32 duration)
server list: (i32 spawnX, i32 spawnY, i32 address, string name)*  terminated by i32 -1
u8 hotbar_locked
u8[13] hide_info                                  (>=145)
i32 fishing_quests                                (>=98)
i32[4] dpad_bindings                              (>=168)
i32[12] builder_acc_status                        (>=230; 11 at >=200, 10 below)
i32 bartender_quests                              (>=184)
u8 is_dead [+ i32 respawn_timer if dead]          (>=200)
i32 last_time_saved_1, i32 last_time_saved_2, i32 golfer_score   (>=200)
u8 research_mystery_byte                          (>=315)
i32 research_count, then (string persistent_id, i32 count)*
u8 temp_item_mask, then one slot per set bit
creative powers: while u8 tag != 0 { i16 power_id, payload }
u8 super_cart_byte                                (>=253)
i32 current_loadout                               (>=269)
3 stored loadouts: 10 items, 10 social, 10 dyes, 10 hide bytes
u8 voice_variant, i32 voice_pitch_offset          (>=315)
i32 refund_count, then (i32 item, i32 count, u8 prefix)*      (>=315)
i32 dialogue_count, then string*                              (>=315)
trailing bytes (preserved verbatim)
```

### 3.3 Slot encoding

```
i32 net_id
i32 stack        only when the container is "multi"
u8  prefix
u8  favorited    only when version >= the container's gate
```

Container table (multi / favourite gate):

| container | multi | fav gate |
|---|---|---|
| inventory, coins, ammo | yes | 145 |
| bank, safe, forge | yes | — |
| void bag | yes | 269 |
| temp items | yes | — |
| misc equipment + dyes (5+5) | **no** | — |
| active loadout (30 slots) | **no** | 325 |
| stored loadouts (3 × 30) | yes | 325 |

### 3.4 Round-trip guarantee

`terra-core::file` never writes unless it can prove the result: it re-parses what it
produced, and `save_with_backup` writes a temp file, reads it back, decrypts and compares
before renaming over the original. The original is first copied to `<name>.plr.bak`.

Verified on this machine: Testguy (326), Bayaz (326, 103 KB, 5,389 research entries),
Harmony Driftmark (279, 41 KB), Michael (279). All four: decrypt → parse → serialise →
encrypt is byte-identical (`tforge verify`). Edit path also verified (`tforge selftest`):
place/modify items, change stats, add research, save, reload, assert.

Not supported: pre-1.4 files (e.g. a 1.4.0-era `version = 194` file). They are rejected
with a clear error rather than guessed at.

## 4. Item database

- 6,196 items, 354 buffs, 84 prefixes embedded as JSON (413 KB) in `terra-assets`.
- Generated by `tools/gen_items.py` (see repo) from a 1.4.4.9 metadata snapshot
  (max stack, rarity, tags, research goal, damage/defense) merged with the **1.4.5.8
  item IDs and display names extracted from the user's own `Terraria.exe`**.
- Items newer than the metadata snapshot keep real names and get `estimated = true`
  with a permissive stack cap; the UI can mark them.
- Future-proofing (designed, not yet wired): re-extract IDs + names from the installed
  `Terraria.exe` at first run and cache them, so a game update does not require a new
  build. The extraction recipe is proven: `ItemID`/`BuffID`/`PrefixID` constants live in
  the CLR metadata Constant table, and `Terraria.Localization.Content.en-US.Items.json` is
  an embedded manifest resource (uncompressed, but with trailing commas).

Known limit: max stack, rarity, damage, tags and research goals are compiled into
`Item.SetDefaults` IL. They cannot be derived from any data file; new items fall back to
defaults. This is cosmetic in an editor — the game re-clamps on load.

## 5. UI (implemented)

- Top bar: Open…, Save (Ctrl+S), Revert, current path, format + item-db version.
- Left panel: name, difficulty, life/mana sliders with Base / maxed presets, permanent
  upgrade checkboxes, seven colour pickers, Journey counters.
- Centre: tabs (Inventory & Equipment | Banks | Buffs | Journey | Raw).
  - Loadout switcher 1–4; armour/vanity/dye rows; 10×5 inventory; coins/ammo; misc gear+dyes.
  - Slots: click selects, click again with a held item places it, right-click menu
    (pick up / duplicate / max stack / favourite / clear), middle-click duplicates,
    double-click picks up, wheel adjusts stack (Shift = ±100).
  - Banks tab: four 10×4 grids. Buffs tab: per-slot id/duration editing. Journey tab:
    research list. Raw tab: decoded header dump.
- Right panel: search (fuzzy: exact > prefix > substring > persistent-id > subsequence),
  category pills, virtualised result list, hover stats, click-to-place.
- Status bar: last action, plus version / trailing bytes / research count.

Not implemented yet: dual-character transfer pane, sprite rendering from XNB, drag-and-drop
by mouse-drag (pick-and-place and middle-click copy cover the workflow), editing creative
powers (they are preserved, not edited), research add/remove, backups restore UI.

## 6. Milestones as executed

1. Crypto + primitives + workspace — done, unit-tested.
2. Full 326/279 parser + serializer + backup engine — done, byte-identical on 4 files.
3. Item database + fuzzy search — done (fuzzy search is hand-rolled; `nucleo` not needed
   at 6 k items).
4. egui shell, grids, inspector, save path — done.
5. Polish — partial (see §5 "not implemented").