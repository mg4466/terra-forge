//! Pure edit operations on a loaded player.
//!
//! These live outside the `App` struct on purpose: they take `&mut PlayerFile`,
//! touch no UI state, and can therefore be unit-tested without an egui context.
//! The UI is a thin skin over this module — when a drag ends, it calls
//! [`drop_item`]; when a stack is scrolled, it calls [`bump_stack`].

use terra_assets::ItemDb;
use terra_core::{PlayerFile, Slot};

use crate::containers::{container, container_mut, is_multi, Ct, SlotRef};

/// Force an item to fit its destination container.
///
/// Equipment slots have no stack field on disk, so anything dropped there
/// becomes a single item — otherwise the game would read a stack out of the
/// next field and the whole file would be garbage.
pub fn fit(db: &ItemDb, mut item: Slot, to: Ct) -> Slot {
    if item.net_id <= 0 {
        return Slot::empty();
    }
    if is_multi(to) {
        item.stack = item.stack.clamp(1, db.max_stack(item.net_id).max(1));
    } else {
        item.stack = 1;
    }
    item
}

/// Move or copy an item onto `to`.
///
/// * `from = Some(src)` — a move: whatever occupied `to` is placed into `src`,
///   i.e. the two slots swap. This matches Terraria's own inventory drag.
/// * `from = None` — a copy: the item is simply written into `to`.
pub fn drop_item(db: &ItemDb, p: &mut PlayerFile, to: SlotRef, from: Option<SlotRef>, item: Slot) {
    // A drag carrying nothing is not an instruction to erase the destination.
    if item.net_id <= 0 {
        return;
    }
    let incoming = fit(db, item, to.c);
    let displaced = container(p, to.c).get(to.i).copied().unwrap_or_default();
    if let Some(slot) = container_mut(p, to.c).get_mut(to.i) {
        *slot = incoming;
    }
    if let Some(src) = from {
        if src != to {
            let back = if displaced.net_id != 0 {
                fit(db, displaced, src.c)
            } else {
                Slot::empty()
            };
            if let Some(slot) = container_mut(p, src.c).get_mut(src.i) {
                *slot = back;
            }
        }
    }
}

/// Clear a slot.
pub fn clear(p: &mut PlayerFile, r: SlotRef) {
    if let Some(slot) = container_mut(p, r.c).get_mut(r.i) {
        *slot = Slot::empty();
    }
}

/// Add `delta` to a stack, clamped to `1..=max_stack`. No-op for containers that
/// do not store stacks.
pub fn bump_stack(db: &ItemDb, p: &mut PlayerFile, r: SlotRef, delta: i32) -> bool {
    if !is_multi(r.c) {
        return false;
    }
    let Some(current) = container(p, r.c).get(r.i).copied() else {
        return false;
    };
    if current.is_empty() {
        return false;
    }
    let max = db.max_stack(current.net_id).max(1);
    if let Some(slot) = container_mut(p, r.c).get_mut(r.i) {
        slot.stack = (slot.stack + delta).clamp(1, max);
        return true;
    }
    false
}

/// Set a stack to its maximum. Returns false if the slot is empty.
pub fn max_stack(db: &ItemDb, p: &mut PlayerFile, r: SlotRef) -> bool {
    let Some(current) = container(p, r.c).get(r.i).copied() else {
        return false;
    };
    if current.is_empty() || !is_multi(r.c) {
        return false;
    }
    let max = db.max_stack(current.net_id).max(1);
    if let Some(slot) = container_mut(p, r.c).get_mut(r.i) {
        slot.stack = max;
        return true;
    }
    false
}

/// Change an item's prefix (its random modifier: "Legendary", "Broken", …).
///
/// The prefix byte belongs to the item, not the slot, so an empty slot has
/// nothing to modify. `0` means "no prefix", which is a legal value — un-forging
/// an item is the same operation as reforging it.
pub fn set_prefix(p: &mut PlayerFile, r: SlotRef, prefix: u8) -> bool {
    match container_mut(p, r.c).get_mut(r.i) {
        Some(slot) if !slot.is_empty() => {
            slot.prefix = prefix;
            true
        }
        _ => false,
    }
}

/// Set a stack count explicitly, clamped to the item's maximum.
///
/// The inspector uses this rather than [`bump_stack`] because a typed value can
/// jump anywhere; equipment has no stack field to write, hence the `false`.
pub fn set_stack(db: &ItemDb, p: &mut PlayerFile, r: SlotRef, stack: i32) -> bool {
    if !is_multi(r.c) {
        return false;
    }
    let Some(current) = container(p, r.c).get(r.i).copied() else {
        return false;
    };
    if current.is_empty() {
        return false;
    }
    // A filled slot cannot hold zero items; emptying is `clear`'s job.
    if stack < 1 {
        return false;
    }
    let max = db.max_stack(current.net_id).max(1);
    if let Some(slot) = container_mut(p, r.c).get_mut(r.i) {
        slot.stack = stack.clamp(1, max);
        return true;
    }
    false
}

/// Move one item from `src` to `dst`, where the two may be different characters.
///
/// This is [`drop_item`] with the player split in two so the same code can move
/// gear between a character and the stash. Semantics are identical: with
/// `from = Some(src)` the destination's old item travels back to `src` (a swap);
/// with `from = None` the item is copied and the source slot is left alone.
pub fn transfer_item(
    db: &ItemDb,
    src: &mut PlayerFile,
    dst: &mut PlayerFile,
    to: SlotRef,
    from: Option<SlotRef>,
    item: Slot,
) {
    if item.net_id <= 0 {
        return;
    }
    let incoming = fit(db, item, to.c);
    let displaced = container(dst, to.c).get(to.i).copied().unwrap_or_default();
    if let Some(slot) = container_mut(dst, to.c).get_mut(to.i) {
        *slot = incoming;
    }
    if let Some(src_ref) = from {
        let back = if displaced.net_id != 0 {
            fit(db, displaced, src_ref.c)
        } else {
            Slot::empty()
        };
        if let Some(slot) = container_mut(src, src_ref.c).get_mut(src_ref.i) {
            *slot = back;
        }
    }
}

/// Journey-mode research bookkeeping.
///
/// Research lives in `PlayerFile::research` as `(persistent item id, count)`
/// pairs, independent of the item's net id, because the game identifies research
/// by its internal name (`Zenith`, `AegisCrystal`, …). Entries are only written
/// for items that have been researched at all, so adding one is normal — the
/// vector is the save file's own list, not a fixed-size table.
pub fn research_item(p: &mut PlayerFile, pid: &str, goal: i32) {
    if pid.is_empty() {
        return;
    }
    match p.research.iter_mut().find(|(k, _)| k == pid) {
        Some((_, count)) => *count = goal,
        None => p.research.push((pid.to_string(), goal)),
    }
}

/// Learn everything the catalogue knows a research goal for.
///
/// `preserve` is deliberate: a character that already researched past the goal
/// (possible with mods or a future balance change) must not be lowered. Returns
/// the number of entries actually changed, which the status bar reports.
pub fn research_all(db: &ItemDb, p: &mut PlayerFile) -> usize {
    let mut index: std::collections::HashMap<String, usize> = p
        .research
        .iter()
        .enumerate()
        .map(|(i, (k, _))| (k.clone(), i))
        .collect();
    let mut changed = 0usize;
    for item in &db.items {
        if item.pid.is_empty() || item.research_goal <= 0 {
            continue;
        }
        match index.get(&item.pid) {
            Some(&i) => {
                if p.research[i].1 < item.research_goal {
                    p.research[i].1 = item.research_goal;
                    changed += 1;
                }
            }
            None => {
                index.insert(item.pid.clone(), p.research.len());
                p.research.push((item.pid.clone(), item.research_goal));
                changed += 1;
            }
        }
    }
    changed
}

/// Flip the favourite star.
pub fn toggle_favourite(p: &mut PlayerFile, r: SlotRef) -> bool {
    match container_mut(p, r.c).get_mut(r.i) {
        Some(slot) if !slot.is_empty() => {
            slot.favorited = !slot.favorited;
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::containers::{Bag, Group};

    fn db() -> &'static ItemDb {
        ItemDb::embedded()
    }

    fn inv(i: usize) -> SlotRef {
        SlotRef {
            c: Ct::Bag(Bag::Inv),
            i,
        }
    }

    #[test]
    fn move_between_inventory_slots_swaps_contents() {
        let mut p = PlayerFile::new_v326("t");
        p.inventory[0] = Slot::new(848, 5, 0);
        p.inventory[1] = Slot::new(849, 7, 0);
        let a = p.inventory[0];
        let b = p.inventory[1];
        drop_item(db(), &mut p, inv(1), Some(inv(0)), a);
        assert_eq!(p.inventory[1].net_id, a.net_id);
        assert_eq!(p.inventory[0].net_id, b.net_id, "displaced item moved back");
    }

    #[test]
    fn move_into_empty_slot_leaves_source_empty() {
        let mut p = PlayerFile::new_v326("t");
        p.inventory[0] = Slot::new(848, 5, 0);
        let a = p.inventory[0];
        drop_item(db(), &mut p, inv(9), Some(inv(0)), a);
        assert!(p.inventory[0].is_empty());
        assert_eq!(p.inventory[9].net_id, 848);
    }

    #[test]
    fn copy_from_browser_does_not_clear_source() {
        let mut p = PlayerFile::new_v326("t");
        p.inventory[0] = Slot::new(848, 5, 0);
        drop_item(db(), &mut p, inv(3), None, Slot::new(4956, 1, 82));
        assert_eq!(p.inventory[0].net_id, 848, "source untouched");
        assert_eq!(p.inventory[3].net_id, 4956);
        assert_eq!(p.inventory[3].prefix, 82);
    }

    #[test]
    fn equipment_destination_forces_stack_of_one() {
        let mut p = PlayerFile::new_v326("t");
        let armour = SlotRef {
            c: Ct::ActiveL(Group::Items),
            i: 0,
        };
        drop_item(db(), &mut p, armour, None, Slot::new(848, 999, 0));
        assert_eq!(p.active_loadout.items[0].stack, 1);
    }

    #[test]
    fn stacks_are_clamped_to_the_item_max() {
        let mut p = PlayerFile::new_v326("t");
        p.inventory[0] = Slot::new(2, 1, 0); // dirt, max 9999
        assert!(bump_stack(db(), &mut p, inv(0), 100_000));
        assert_eq!(p.inventory[0].stack, 9999);

        let mut p = PlayerFile::new_v326("t");
        p.inventory[0] = Slot::new(4956, 1, 0); // Zenith, max 1
        bump_stack(db(), &mut p, inv(0), 5);
        assert_eq!(p.inventory[0].stack, 1);
    }

    #[test]
    fn bump_stack_ignores_equipment() {
        let mut p = PlayerFile::new_v326("t");
        let armour = SlotRef {
            c: Ct::ActiveL(Group::Items),
            i: 0,
        };
        p.active_loadout.items[0] = Slot::new(848, 1, 0);
        assert!(!bump_stack(db(), &mut p, armour, 5));
        assert_eq!(p.active_loadout.items[0].stack, 1);
    }

    #[test]
    fn clearing_and_favouriting_only_touch_the_target() {
        let mut p = PlayerFile::new_v326("t");
        p.bank[4] = Slot::new(848, 2, 0);
        let r = SlotRef {
            c: Ct::Bag(Bag::Bank),
            i: 4,
        };
        assert!(toggle_favourite(&mut p, r));
        assert!(p.bank[4].favorited);
        clear(&mut p, r);
        assert!(p.bank[4].is_empty());
        assert!(p.bank[3].is_empty(), "neighbours untouched");
    }

    #[test]
    fn dropping_an_empty_item_is_a_no_op() {
        let mut p = PlayerFile::new_v326("t");
        p.inventory[0] = Slot::new(848, 5, 0);
        drop_item(db(), &mut p, inv(0), Some(inv(0)), Slot::empty());
        assert_eq!(p.inventory[0].net_id, 848, "empty drag does not erase");
    }

    #[test]
    fn prefix_can_be_set_and_cleared_but_not_on_empty_slots() {
        let mut p = PlayerFile::new_v326("t");
        p.inventory[0] = Slot::new(4956, 1, 0);
        assert!(set_prefix(&mut p, inv(0), 82));
        assert_eq!(p.inventory[0].prefix, 82);
        assert!(set_prefix(&mut p, inv(0), 0));
        assert_eq!(p.inventory[0].prefix, 0, "0 means un-forged, not 'refuse'");
        assert!(!set_prefix(&mut p, inv(1), 82), "empty slot has no item");
    }

    #[test]
    fn set_stack_clamps_and_respects_equipment() {
        let mut p = PlayerFile::new_v326("t");
        p.inventory[0] = Slot::new(2, 1, 0); // dirt, max 9999
        assert!(set_stack(db(), &mut p, inv(0), 250));
        assert_eq!(p.inventory[0].stack, 250);
        assert!(set_stack(db(), &mut p, inv(0), 10_000_000));
        assert_eq!(p.inventory[0].stack, 9999);
        assert!(!set_stack(db(), &mut p, inv(0), 0), "stacks start at 1");
        let armour = SlotRef {
            c: Ct::ActiveL(Group::Items),
            i: 0,
        };
        p.active_loadout.items[0] = Slot::new(848, 1, 0);
        assert!(!set_stack(db(), &mut p, armour, 5));
        assert_eq!(p.active_loadout.items[0].stack, 1);
    }

    #[test]
    fn transfer_swaps_between_two_characters() {
        let mut hero = PlayerFile::new_v326("hero");
        let mut stash = PlayerFile::new_v326("stash");
        hero.inventory[0] = Slot::new(4956, 1, 0); // Zenith
        stash.bank[7] = Slot::new(848, 3, 0); // Nightmare Pickaxe
        let from = inv(0);
        let to = SlotRef {
            c: Ct::Bag(Bag::Bank),
            i: 7,
        };
        let carried = hero.inventory[0];
        transfer_item(db(), &mut hero, &mut stash, to, Some(from), carried);
        assert_eq!(stash.bank[7].net_id, 4956);
        assert_eq!(hero.inventory[0].net_id, 848, "displaced item returned");
    }

    #[test]
    fn transfer_without_a_source_copies() {
        let mut hero = PlayerFile::new_v326("hero");
        let mut stash = PlayerFile::new_v326("stash");
        hero.inventory[0] = Slot::new(848, 5, 0);
        let to = SlotRef {
            c: Ct::Bag(Bag::Inv),
            i: 2,
        };
        transfer_item(
            db(),
            &mut hero,
            &mut stash,
            to,
            None,
            Slot::new(4956, 1, 82),
        );
        assert_eq!(hero.inventory[0].net_id, 848, "source untouched");
        assert_eq!(stash.inventory[2].net_id, 4956);
        assert_eq!(stash.inventory[2].prefix, 82);
    }

    #[test]
    fn research_updates_inserts_and_never_lowers() {
        let mut p = PlayerFile::new_v326("t");
        research_item(&mut p, "Zenith", 1);
        assert_eq!(p.research, vec![("Zenith".to_string(), 1)]);
        research_item(&mut p, "Zenith", 5);
        assert_eq!(p.research.len(), 1, "one entry per item");
        assert_eq!(p.research[0].1, 5);

        // A bogus-but-higher count must survive a full learn pass.
        p.research.push(("DirtBlock".into(), 99_999));
        let before = p.research.len();
        let changed = research_all(db(), &mut p);
        assert!(changed > 0);
        assert!(p.research.len() > before);
        assert_eq!(
            p.research
                .iter()
                .find(|(k, _)| k == "DirtBlock")
                .map(|x| x.1),
            Some(99_999),
            "already-finished research is preserved"
        );
        // Running it twice changes nothing the second time.
        assert_eq!(research_all(db(), &mut p), 0);
    }
}
