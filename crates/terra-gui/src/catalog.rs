//! Turning the flat item database into a browsable, grouped list.
//!
//! `terra-assets` knows about tags (`Melee`, `Tile`, `Accessory`, …) but not
//! about presentation. This module is the presentation layer: it decides which
//! section header a row belongs under, in what order rows appear, and what
//! happens when the user types in the search box.
//!
//! Two modes:
//!
//! * **browsing** (empty query) — items are bucketed into sections, each section
//!   sorted by rarity then name. That is what makes the browser feel organised
//!   instead of being one 6,000-row alphabet.
//! * **searching** (non-empty query) — the ranking from `terra_assets::ItemDb`
//!   is kept verbatim, with no headers, because "why is this first" matters more
//!   than "which section is this in".
//!
//! The result is a flat `Vec<Row>` so the UI can virtualise it: only the rows
//! actually on screen are ever built into widgets.

use std::sync::OnceLock;

use terra_assets::{tag_bit, Item, ItemDb};

/// One line in the browser list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    /// A section heading, e.g. "Weapons".
    Header(&'static str),
    /// An item, by numeric id.
    Item(i32),
}

/// A ready-to-render list of rows.
#[derive(Default, Debug)]
pub struct Catalog {
    pub rows: Vec<Row>,
    /// Number of items, excluding headers — what the UI reports as "results".
    pub item_count: usize,
}

impl Catalog {
    /// Build the list for the current query and category filter.
    ///
    /// `cat` is a tag mask; `0` means "no filter / All".
    pub fn build(db: &ItemDb, query: &str, cat: u32) -> Self {
        if query.trim().is_empty() {
            Self::browse(db, cat)
        } else {
            Self::search(db, query.trim(), cat)
        }
    }

    fn search(db: &ItemDb, query: &str, cat: u32) -> Self {
        let mut item_count = 0;
        let rows = db
            .search(query, cat)
            .into_iter()
            .map(|it| {
                item_count += 1;
                Row::Item(it.id)
            })
            .collect();
        Self { rows, item_count }
    }

    fn browse(db: &ItemDb, cat: u32) -> Self {
        let sections: Vec<(&'static str, u32)> = if cat == 0 {
            supersections().to_vec()
        } else {
            subsections(cat)
        };

        // Bucket items per section, "first matching mask wins" so an item that is
        // both Tile and Material shows up once. The final section always has
        // mask 0 and acts as the catch-all.
        let mut buckets: Vec<Vec<&Item>> = vec![Vec::new(); sections.len()];
        for it in &db.items {
            if it.id == 0 || it.name.is_empty() {
                continue;
            }
            if cat != 0 && it.tags & cat == 0 {
                continue;
            }
            let idx = sections
                .iter()
                .position(|(_, mask)| *mask != 0 && it.tags & *mask != 0)
                .unwrap_or(sections.len() - 1);
            buckets[idx].push(it);
        }

        let mut rows = Vec::new();
        let mut item_count = 0;
        for (i, (name, _)) in sections.iter().enumerate() {
            let bucket = &mut buckets[i];
            if bucket.is_empty() {
                continue;
            }
            bucket.sort_by(|a, b| b.rarity.cmp(&a.rarity).then_with(|| a.name.cmp(&b.name)));
            rows.push(Row::Header(name));
            for it in bucket.iter() {
                rows.push(Row::Item(it.id));
                item_count += 1;
            }
        }
        Self { rows, item_count }
    }
}

// ---------------------------------------------------------------- tag masks
//
// These are functions rather than `const`s because `tag_bit` looks the name up
// in the database's tag list at runtime, and because building them once and
// caching avoids recomputing masks on every frame.

fn cached(slot: &'static OnceLock<u32>, f: impl FnOnce() -> u32) -> u32 {
    *slot.get_or_init(f)
}

macro_rules! mask {
    ($name:ident, $($tag:expr),+ $(,)?) => {
        pub fn $name() -> u32 {
            static CACHE: OnceLock<u32> = OnceLock::new();
            cached(&CACHE, || 0 $(| tag_bit($tag))+)
        }
    };
}

mask!(mask_weapons, "Melee", "Ranged", "Magic", "Summon", "Sentry");
mask!(mask_armour, "HeadSlot", "BodySlot", "LegsSlot");
mask!(mask_accessory, "Accessory");
mask!(mask_blocks, "Tile", "Wall");
mask!(mask_furniture, "Painting");
mask!(mask_consumable, "Consumable");
mask!(mask_material, "Material");
mask!(
    mask_vanity,
    "Vanity",
    "Mount",
    "Minecart",
    "VanityPet",
    "LightPet",
    "Hook",
    "Kite"
);

/// Top-level groups used when browsing "All", in display order.
fn supersections() -> &'static [(&'static str, u32)] {
    static CACHE: OnceLock<Vec<(&'static str, u32)>> = OnceLock::new();
    CACHE.get_or_init(|| {
        vec![
            ("Weapons", mask_weapons()),
            ("Armor", mask_armour()),
            ("Accessories", mask_accessory()),
            ("Vanity, pets & mounts", mask_vanity()),
            ("Blocks & furniture", mask_blocks() | mask_furniture()),
            ("Consumables & ammo", mask_consumable() | tag_bit("Ammo")),
            ("Materials", mask_material()),
            ("Other", 0),
        ]
    })
}

/// Sub-sections shown when a specific category pill is selected.
fn subsections(cat: u32) -> Vec<(&'static str, u32)> {
    if cat == mask_weapons() {
        vec![
            ("Melee", tag_bit("Melee")),
            ("Ranged", tag_bit("Ranged")),
            ("Magic", tag_bit("Magic")),
            ("Summon", tag_bit("Summon")),
            ("Sentry", tag_bit("Sentry")),
            ("Other weapons", 0),
        ]
    } else if cat == mask_armour() {
        vec![
            ("Helmets", tag_bit("HeadSlot")),
            ("Body armor", tag_bit("BodySlot")),
            ("Leg armor", tag_bit("LegsSlot")),
        ]
    } else if cat == mask_blocks() | mask_furniture() {
        vec![
            ("Blocks", tag_bit("Tile")),
            ("Walls", tag_bit("Wall")),
            ("Paintings", tag_bit("Painting")),
        ]
    } else if cat == mask_consumable() {
        vec![
            ("Consumables", tag_bit("Consumable")),
            ("Ammo", tag_bit("Ammo")),
            ("Other", 0),
        ]
    } else if cat == mask_vanity() {
        vec![
            ("Vanity armor", tag_bit("Vanity")),
            ("Mounts & minecarts", tag_bit("Mount") | tag_bit("Minecart")),
            ("Pets", tag_bit("VanityPet") | tag_bit("LightPet")),
            ("Hooks & kites", tag_bit("Hook") | tag_bit("Kite")),
        ]
    } else {
        vec![("Results", 0)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> &'static ItemDb {
        ItemDb::embedded()
    }

    fn headers(cat: &Catalog) -> Vec<&'static str> {
        cat.rows
            .iter()
            .filter_map(|r| match r {
                Row::Header(h) => Some(*h),
                _ => None,
            })
            .collect()
    }

    fn ids(cat: &Catalog) -> Vec<i32> {
        cat.rows
            .iter()
            .filter_map(|r| match r {
                Row::Item(id) => Some(*id),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn browsing_puts_every_item_in_exactly_one_section() {
        let cat = Catalog::build(db(), "", 0);
        let items = ids(&cat);
        let mut unique = items.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), items.len(), "an item appeared twice");
        assert_eq!(items.len(), cat.item_count);
        assert!(items.contains(&4956), "Zenith should be present");
        assert!(!headers(&cat).is_empty());
    }

    #[test]
    fn weapons_are_listed_before_materials() {
        let cat = Catalog::build(db(), "", 0);
        let h = headers(&cat);
        let wi = h.iter().position(|x| *x == "Weapons").unwrap();
        let mi = h.iter().position(|x| *x == "Materials").unwrap();
        assert!(wi < mi, "headers: {h:?}");
    }

    #[test]
    fn armour_filter_only_contains_armour_and_has_subsections() {
        let cat = Catalog::build(db(), "", mask_armour());
        let mask = mask_armour();
        for id in ids(&cat) {
            assert!(
                db().by_id(id).unwrap().tags & mask != 0,
                "{id} is not armor"
            );
        }
        assert!(headers(&cat).contains(&"Helmets"), "{:?}", headers(&cat));
    }

    #[test]
    fn weapons_filter_splits_into_damage_classes() {
        let cat = Catalog::build(db(), "", mask_weapons());
        let h = headers(&cat);
        assert!(h.contains(&"Melee") && h.contains(&"Ranged"), "{h:?}");
    }

    #[test]
    fn search_has_no_headers_and_ranks_exact_first() {
        let cat = Catalog::build(db(), "terra blade", 0);
        assert!(headers(&cat).is_empty());
        match cat.rows.first() {
            Some(Row::Item(id)) => assert_eq!(db().by_id(*id).unwrap().name, "Terra Blade"),
            other => panic!("unexpected first row {other:?}"),
        }
    }

    #[test]
    fn every_section_list_ends_with_a_catch_all() {
        assert_eq!(supersections().last().unwrap().1, 0);
        assert_eq!(subsections(mask_weapons()).last().unwrap().1, 0);
        assert_eq!(subsections(mask_consumable()).last().unwrap().1, 0);
    }
}
