#!/usr/bin/env python3
"""Build the embedded item database for terra-assets.

Two inputs:
  1. An extraction from the installed game (tools/extract_from_install.py). Always required:
     it supplies the authoritative id <-> persistent-id mapping and display names for the
     exact game version the player runs.
  2. Optionally a metadata snapshot (max stack, rarity, tags, research goal, damage,
     defense) keyed by numeric id. The snapshot used here came from a 1.4.4.9 dataset;
     items newer than it get estimated stats and keep their real names.

    python3 tools/gen_items.py /tmp/ex1458.json [snapshot.json]

Writes crates/terra-assets/data/items.json.
"""
from __future__ import annotations

import json
import pathlib
import sys

TAGS = [
    "Melee", "Ranged", "Magic", "Sentry", "Summon", "HeadSlot", "BodySlot",
    "LegsSlot", "Accessory", "Vanity", "Hook", "Mount", "Minecart", "LightPet",
    "VanityPet", "Kite", "Drill", "Fish", "FishingCrate", "Ammo", "Tile", "Wall",
    "Painting", "Consumable", "Material",
]
TAG_BIT = {t: 1 << i for i, t in enumerate(TAGS)}
MAX_ID = 6200  # ignore sentinel constants above the real item range

PREFIX_NAMES = {
    1: "Large", 2: "Massive", 3: "Dangerous", 4: "Savage", 5: "Sharp", 6: "Pointy",
    7: "Tiny", 8: "Terrible", 9: "Small", 10: "Dull", 11: "Unhappy", 12: "Bulky",
    13: "Shameful", 14: "Heavy", 15: "Light", 16: "Sighted", 17: "Rapid", 18: "Hasty",
    19: "Intrepid", 20: "Deadly", 21: "Staunch", 22: "Awful", 23: "Lethargic",
    24: "Awkward", 25: "Powerful", 26: "Mystic", 27: "Adept", 28: "Masterful",
    29: "Inept", 30: "Ignorant", 31: "Deranged", 32: "Intense", 33: "Taboo",
    34: "Celestial", 35: "Furious", 36: "Keen", 37: "Superior", 38: "Forceful",
    39: "Broken", 40: "Damaged", 41: "Shoddy", 42: "Quick", 43: "Deadly",
    44: "Agile", 45: "Nimble", 46: "Murderous", 47: "Slow", 48: "Sluggish",
    49: "Lazy", 50: "Annoying", 51: "Nasty", 52: "Manic", 53: "Hurtful",
    54: "Strong", 55: "Unpleasant", 56: "Weak", 57: "Ruthless", 58: "Frenzying",
    59: "Godly", 60: "Demonic", 61: "Zealous", 62: "Hard", 63: "Guardian",
    64: "Armored", 65: "Warding", 66: "Arcane", 67: "Precise", 68: "Lucky",
    69: "Jagged", 70: "Spiked", 71: "Angry", 72: "Menacing", 73: "Brisk",
    74: "Fleeting", 75: "Hasty", 76: "Quick", 77: "Wild", 78: "Rash",
    79: "Intrepid", 80: "Violent", 81: "Legendary", 82: "Unreal", 83: "Mythical",
    84: "Legendary",
}


def split_pid(pid: str) -> str:
    """`FirstFractal` -> `First Fractal`; leaves all-lowercase ids alone."""
    import re
    words = re.sub(r"(?<=[a-z])(?=[A-Z])|(?<=[A-Z])(?=[A-Z][a-z])", " ", pid)
    return words.replace("_", " ").strip()


def main() -> None:
    if len(sys.argv) < 2:
        print(__doc__)
        raise SystemExit(2)
    extract = json.load(open(sys.argv[1]))
    snapshot = json.load(open(sys.argv[2])) if len(sys.argv) > 2 else None

    items_by_id: dict[int, dict] = {}
    for pid, iid in extract["items"].items():
        if pid == "None" or not (0 <= iid <= MAX_ID):
            continue
        items_by_id.setdefault(iid, pid)
    names = extract.get("names", {})

    snap_items = snapshot["items"] if snapshot else []
    max_id = max(max(items_by_id), len(snap_items) - 1, 0)

    rows = []
    estimated = 0
    unnamed = 0
    for iid in range(max_id + 1):
        src = snap_items[iid] if iid < len(snap_items) else {}
        pid = items_by_id.get(iid) or src.get("pid") or ""
        name = names.get(pid) or src.get("name") or ""
        if not name:
            # A few ids have no entry in the game's own Items.json (unused or
            # debug items). Fall back to the persistent id read as words:
            # "FirstFractal" -> "First Fractal", which is usually the real name.
            unnamed += 1
            name = split_pid(pid) if pid else f"Item #{iid}"
        est = iid >= len(snap_items)
        if est:
            estimated += 1
        tags = 0
        for t in src.get("tags", []):
            tags |= TAG_BIT.get(t, 0)
        rows.append([
            iid,
            pid,
            name,
            int(src.get("maxStack", 1)) if not est else 9999,
            int(src.get("rarity", 0)),
            tags,
            int(src.get("researchGoal", 0)),
            int(src.get("damage", -1)),
            int(src.get("defense", 0)),
            1 if est else 0,
        ])

    buffs = []
    if snapshot:
        for i, b in enumerate(snapshot.get("buffs", [])):
            if i == 0:
                continue
            buffs.append([i, b.get("name", "") or f"Buff #{i}", b.get("desc", ""), bool(b.get("debuff", False))])

    doc = {
        "dataset": "terra-forge item db",
        "game_version": extract.get("game_version", "installed"),
        "baseline_version": snapshot.get("version") if snapshot else None,
        "tags": TAGS,
        "items": rows,
        "prefixes": [[k, v] for k, v in sorted(PREFIX_NAMES.items())],
        "buffs": buffs,
    }

    dst = pathlib.Path(__file__).resolve().parent.parent / "crates/terra-assets/data/items.json"
    dst.parent.mkdir(parents=True, exist_ok=True)
    dst.write_text(json.dumps(doc, separators=(",", ":")))
    print(f"items {len(rows)} (estimated stats {estimated}, unnamed {unnamed}), buffs {len(buffs)}")
    print(f"max id {max_id}, wrote {dst} ({dst.stat().st_size} bytes)")


if __name__ == "__main__":
    main()