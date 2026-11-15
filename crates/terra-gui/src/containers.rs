//! Container plumbing: which part of a player a slot index refers to.
//!
//! The on-disk format stores the same logical thing — an item slot — in a dozen
//! different places, and not all of them are encoded identically. A
//! [`Container`] (`Ct` below) names one such place, and [`container`] /
//! [`container_mut`] hand out the `Vec<Slot>` behind it.
//!
//! The encoding differences live in [`is_multi`] and in `terra_core::plr`:
//!
//! * `multi` containers store an explicit stack count, others implicitly hold 1.
//! * some containers store a "favourited" byte, some do not (version-gated).
//!
//! Keep those two facts in mind whenever you add a container: getting either
//! wrong shifts every following byte and the file stops parsing.

use terra_core::{PlayerFile, Slot};

/// Simple item bags, addressed as one flat vector each.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bag {
    Inv,
    Coins,
    Ammo,
    EquipItems,
    EquipDyes,
    Bank,
    Safe,
    Forge,
    Void,
    Temp,
}

/// The three parallel slot rows inside a loadout.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Group {
    Items,
    Social,
    Dyes,
}

/// One addressable container in a player file.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ct {
    Bag(Bag),
    /// Loadout 1, stored inline with the equipment (no hide-flags, no stacks).
    ActiveL(Group),
    /// Loadouts 2..4, stored after `current_loadout` (with hide-flags and stacks).
    StoredL(u8, Group),
}

/// Borrow the slots of a container.
pub fn container(p: &PlayerFile, c: Ct) -> &Vec<Slot> {
    match c {
        Ct::Bag(Bag::Inv) => &p.inventory,
        Ct::Bag(Bag::Coins) => &p.coins,
        Ct::Bag(Bag::Ammo) => &p.ammo,
        Ct::Bag(Bag::EquipItems) => &p.equipment_items,
        Ct::Bag(Bag::EquipDyes) => &p.equipment_dyes,
        Ct::Bag(Bag::Bank) => &p.bank,
        Ct::Bag(Bag::Safe) => &p.safe,
        Ct::Bag(Bag::Forge) => &p.forge,
        Ct::Bag(Bag::Void) => &p.void_bag,
        Ct::Bag(Bag::Temp) => &p.temp_items,
        Ct::ActiveL(Group::Items) => &p.active_loadout.items,
        Ct::ActiveL(Group::Social) => &p.active_loadout.social,
        Ct::ActiveL(Group::Dyes) => &p.active_loadout.dyes,
        Ct::StoredL(n, Group::Items) => &p.stored_loadouts[n as usize].items,
        Ct::StoredL(n, Group::Social) => &p.stored_loadouts[n as usize].social,
        Ct::StoredL(n, Group::Dyes) => &p.stored_loadouts[n as usize].dyes,
    }
}

/// Mutable counterpart of [`container`].
pub fn container_mut(p: &mut PlayerFile, c: Ct) -> &mut Vec<Slot> {
    match c {
        Ct::Bag(Bag::Inv) => &mut p.inventory,
        Ct::Bag(Bag::Coins) => &mut p.coins,
        Ct::Bag(Bag::Ammo) => &mut p.ammo,
        Ct::Bag(Bag::EquipItems) => &mut p.equipment_items,
        Ct::Bag(Bag::EquipDyes) => &mut p.equipment_dyes,
        Ct::Bag(Bag::Bank) => &mut p.bank,
        Ct::Bag(Bag::Safe) => &mut p.safe,
        Ct::Bag(Bag::Forge) => &mut p.forge,
        Ct::Bag(Bag::Void) => &mut p.void_bag,
        Ct::Bag(Bag::Temp) => &mut p.temp_items,
        Ct::ActiveL(Group::Items) => &mut p.active_loadout.items,
        Ct::ActiveL(Group::Social) => &mut p.active_loadout.social,
        Ct::ActiveL(Group::Dyes) => &mut p.active_loadout.dyes,
        Ct::StoredL(n, Group::Items) => &mut p.stored_loadouts[n as usize].items,
        Ct::StoredL(n, Group::Social) => &mut p.stored_loadouts[n as usize].social,
        Ct::StoredL(n, Group::Dyes) => &mut p.stored_loadouts[n as usize].dyes,
    }
}

/// Does this container store a stack count on disk?
///
/// Equipment and the active loadout do not: their slots are netID + prefix
/// (+ favourite). Writing a stack there would corrupt everything after it.
pub fn is_multi(c: Ct) -> bool {
    !matches!(
        c,
        Ct::Bag(Bag::EquipItems) | Ct::Bag(Bag::EquipDyes) | Ct::ActiveL(_)
    )
}

/// Human label, used in tooltips and section headers.
pub fn label(c: Ct) -> &'static str {
    match c {
        Ct::Bag(Bag::Inv) => "Inventory",
        Ct::Bag(Bag::Coins) => "Coins",
        Ct::Bag(Bag::Ammo) => "Ammo",
        Ct::Bag(Bag::EquipItems) => "Equipment",
        Ct::Bag(Bag::EquipDyes) => "Equipment dye",
        Ct::Bag(Bag::Bank) => "Piggy bank",
        Ct::Bag(Bag::Safe) => "Safe",
        Ct::Bag(Bag::Forge) => "Defender's forge",
        Ct::Bag(Bag::Void) => "Void vault",
        Ct::Bag(Bag::Temp) => "Temp",
        Ct::ActiveL(_) => "Loadout 1",
        Ct::StoredL(n, _) => match n {
            0 => "Loadout 2",
            1 => "Loadout 3",
            _ => "Loadout 4",
        },
    }
}

/// A single slot address: which container, which index.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SlotRef {
    pub c: Ct,
    pub i: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equipment_containers_do_not_store_stacks() {
        assert!(!is_multi(Ct::Bag(Bag::EquipItems)));
        assert!(!is_multi(Ct::Bag(Bag::EquipDyes)));
        assert!(!is_multi(Ct::ActiveL(Group::Items)));
        assert!(is_multi(Ct::StoredL(0, Group::Items)));
        assert!(is_multi(Ct::Bag(Bag::Bank)));
    }

    #[test]
    fn accessors_agree_on_shape() {
        let mut p = PlayerFile::new_v326("probe");
        p.inventory[7] = Slot::new(848, 3, 0);
        assert_eq!(container(&p, Ct::Bag(Bag::Inv))[7].net_id, 848);
        container_mut(&mut p, Ct::Bag(Bag::Inv))[7].stack = 42;
        assert_eq!(container(&p, Ct::Bag(Bag::Inv))[7].stack, 42);
        // the active loadout is 30 slots across three groups
        assert_eq!(container(&p, Ct::ActiveL(Group::Social)).len(), 10);
    }
}
