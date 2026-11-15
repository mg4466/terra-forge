#!/usr/bin/env python3
"""Extract item/buff/prefix IDs and display names from an installed Terraria or tModLoader assembly.

This is the "no new build needed" half of the item database: everything here is read out
of the game the user actually has installed, so a game update only requires re-running it.

    python3 tools/extract_from_install.py ~/.local/share/Steam/steamapps/common/Terraria/Terraria.exe out.json

What it reads:
  * CLR metadata `Constant` table -> `Terraria.ID.ItemID` / `BuffID` / `PrefixID` constants
    (persistent name -> numeric id). Field ownership comes from each TypeDef's FieldList.
  * Embedded manifest resource `Terraria.Localization.Content.en-US.Items.json`
    (or `en_US` in tModLoader) -> display names keyed by persistent id.
    The JSON is stored uncompressed but with trailing commas, so it needs a tolerant parse.

Requires: pip install dnfile
"""
from __future__ import annotations

import json
import re
import struct
import sys

import dnfile


def load_assembly(path: str):
    pe = dnfile.dnPE(path)
    if pe.net is None or pe.net.mdtables is None:
        raise SystemExit(f"{path} is not a .NET assembly")
    return pe


def constants_for(pe, type_name: str) -> dict[str, int]:
    """Return {field name: constant} for a static class such as Terraria.ID.ItemID."""
    mdt = pe.net.mdtables
    typedefs = mdt.TypeDef.rows
    fields = mdt.Field.rows

    val_by_field: dict[int, int] = {}
    for c in mdt.Constant.rows:
        try:
            rid = c.Parent.row_index
            v = c.Value.value if hasattr(c.Value, "value") else c.Value
        except Exception:
            continue
        if isinstance(v, (bytes, bytearray)):
            v = int.from_bytes(v, "little", signed=True)
        if rid and isinstance(v, int):
            val_by_field[rid] = v

    def name_of(x):
        return x.value if hasattr(x, "value") else str(x)

    out: dict[str, int] = {}
    for i, t in enumerate(typedefs):
        if name_of(t.TypeName) != type_name:
            continue
        start = t.FieldList[0].row_index
        end = typedefs[i + 1].FieldList[0].row_index if i + 1 < len(typedefs) else len(fields) + 1
        for rid in range(start, min(end, len(fields) + 1)):
            if rid in val_by_field:
                out[name_of(fields[rid - 1].Name)] = val_by_field[rid]
        break
    return out


def item_names(pe) -> dict[str, str]:
    """Display names from the embedded localization resource, keyed by persistent id."""
    base = pe.get_offset_from_rva(pe.net.struct.ResourcesRva)
    data = pe.__data__
    for r in pe.net.mdtables.ManifestResource.rows:
        if not r.Offset:
            continue
        name = r.Name.value if hasattr(r.Name, "value") else str(r.Name)
        if "Items.json" not in name or "en-US" not in name and "en_US" not in name:
            continue
        off = base + (r.Offset & 0x7FFFFFFF)
        length = struct.unpack("<I", data[off:off + 4])[0]
        raw = data[off + 4:off + 4 + length]
        if raw[:2] == b"\x1f\x8b":
            import gzip
            raw = gzip.decompress(raw)
        text = raw.decode("utf-8-sig")
        # Terraria's own JSON parser tolerates trailing commas; json.loads does not.
        obj = json.loads(re.sub(r",(\s*[}\]])", r"\1", text))
        names: dict[str, str] = {}
        for pid, entry in obj.get("ItemName", {}).items():
            if isinstance(entry, str):
                names[pid] = entry
            elif isinstance(entry, dict):
                names[pid] = entry.get("Name", "")
        return names
    raise SystemExit("no en-US Items.json resource found in this assembly")


def game_version(assembly_path: str) -> str:
    """Best-effort: read the sibling changelog.txt the Steam build ships."""
    import os
    d = os.path.dirname(os.path.abspath(assembly_path))
    for candidate in (os.path.join(d, "changelog.txt"),):
        if os.path.exists(candidate):
            m = re.search(r"Version (\d+(?:\.\d+)+)", open(candidate, encoding="utf-8", errors="replace").read(4000))
            if m:
                return m.group(1)
    return "unknown"


def main() -> None:
    if len(sys.argv) < 2:
        print(__doc__)
        raise SystemExit(2)
    src = sys.argv[1]
    dst = sys.argv[2] if len(sys.argv) > 2 else "itemids.json"

    pe = load_assembly(src)
    items = {k: v for k, v in constants_for(pe, "ItemID").items() if 0 <= v <= 100_000}
    buffs = {k: v for k, v in constants_for(pe, "BuffID").items() if 0 <= v <= 1_000}
    prefixes = {k: v for k, v in constants_for(pe, "PrefixID").items() if 0 <= v <= 1_000}
    names = item_names(pe)

    doc = {
        "source": src,
        "game_version": game_version(src),
        "items": items,
        "buffs": buffs,
        "prefixes": prefixes,
        "names": names,
    }
    with open(dst, "w") as f:
        json.dump(doc, f, indent=0)
    print(f"{src}")
    print(f"  items {len(items)} (max id {max(items.values())}), buffs {len(buffs)}, prefixes {len(prefixes)}")
    print(f"  names {len(names)}")
    print(f"  wrote {dst}")


if __name__ == "__main__":
    main()