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
}
