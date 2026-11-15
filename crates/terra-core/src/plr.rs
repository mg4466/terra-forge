//! Terraria player file (`.plr`) binary format.
//!
//! Layout reconstructed from real 1.4.4.9 (279) and 1.4.5.8 (326) files and
//! validated by byte-identical round-trip. Field order follows
//! `Player.SavePlayer`, with every field gated on the file's `version` int.
//!
//! Anything after the last field we understand is preserved verbatim in
//! [`PlayerFile::trailing`], so files written by a newer patch than this crate
//! knows about still round-trip losslessly.

use crate::cursor::{IoError, Reader, Writer};

pub const MAGIC_A: u32 = 1_869_374_834; // "relo"
pub const MAGIC_B: u32 = 56_846_695; // "gic" + file type 3

/// Format versions we have verified against real files.
pub const VER_1_4_0: i32 = 225;
pub const VER_1_4_3: i32 = 248;
pub const VER_1_4_4_0: i32 = 269;
pub const VER_1_4_4_9: i32 = 279;
pub const VER_1_4_5_0: i32 = 315;
pub const VER_1_4_5_7: i32 = 325;
pub const VER_1_4_5_8: i32 = 326;

#[derive(Debug, thiserror::Error)]
pub enum PlrError {
    #[error(transparent)]
    Io(#[from] IoError),
    #[error("not a Terraria player file (bad magic {0:#x}/{1:#x})")]
    BadMagic(u32, u32),
    #[error("unsupported pre-1.4 player format version {0}")]
    UnsupportedVersion(i32),
    #[error("player version {0} is newer than this build understands; trailing bytes were preserved but fields may be stale")]
    NewerVersion(i32),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Slot {
    /// Item netID, 0 = empty.
    pub net_id: i32,
    /// Stack size (only stored for stackable containers).
    pub stack: i32,
    pub prefix: u8,
    pub favorited: bool,
}

impl Slot {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.net_id == 0
    }

    pub fn new(net_id: i32, stack: i32, prefix: u8) -> Self {
        Self {
            net_id,
            stack,
            prefix,
            favorited: false,
        }
    }
}

/// How a given container encodes its slots on disk.
#[derive(Debug, Clone, Copy)]
struct SlotKind {
    /// Stack size is stored as an i32 after the netID.
    multi: bool,
    /// Favorited flag is stored after the prefix, once version >= fav_min (0 = never).
    fav_min: i32,
}

const K_INVENTORY: SlotKind = SlotKind {
    multi: true,
    fav_min: 145,
};
const K_COIN_AMMO: SlotKind = SlotKind {
    multi: true,
    fav_min: 145,
};
const K_EQUIP: SlotKind = SlotKind {
    multi: false,
    fav_min: 0,
};
const K_BANK: SlotKind = SlotKind {
    multi: true,
    fav_min: 0,
};
const K_VOID: SlotKind = SlotKind {
    multi: true,
    fav_min: 269,
};
const K_TEMP: SlotKind = SlotKind {
    multi: true,
    fav_min: 0,
};
const K_LOADOUT_ACTIVE: SlotKind = SlotKind {
    multi: false,
    fav_min: 325,
};
const K_LOADOUT_STORED: SlotKind = SlotKind {
    multi: true,
    fav_min: 325,
};

fn read_slot(r: &mut Reader<'_>, v: i32, k: SlotKind) -> Result<Slot, IoError> {
    let net_id = r.i32()?;
    let stack = if k.multi { r.i32()? } else { 1 };
    let prefix = r.u8()?;
    let favorited = if k.fav_min > 0 && v >= k.fav_min {
        r.bool()?
    } else {
        false
    };
    Ok(Slot {
        net_id,
        stack,
        prefix,
        favorited,
    })
}

fn write_slot(w: &mut Writer, v: i32, k: SlotKind, s: &Slot) {
    w.i32(s.net_id);
    if k.multi {
        w.i32(s.stack);
    }
    w.u8(s.prefix);
    if k.fav_min > 0 && v >= k.fav_min {
        w.bool(s.favorited);
    }
}

fn read_slots(r: &mut Reader<'_>, v: i32, k: SlotKind, n: usize) -> Result<Vec<Slot>, IoError> {
    (0..n).map(|_| read_slot(r, v, k)).collect()
}

fn write_slots(w: &mut Writer, v: i32, k: SlotKind, slots: &[Slot]) {
    for s in slots {
        write_slot(w, v, k, s);
    }
}

/// One of the four equipment loadouts (1.4.4+).
#[derive(Debug, Clone, Default)]
pub struct Loadout {
    /// 10 equipment slots: 3 armour + 7 accessories (the 8th is the demon-heart slot).
    pub items: Vec<Slot>,
    /// 10 vanity/social slots.
    pub social: Vec<Slot>,
    /// 10 dye slots.
    pub dyes: Vec<Slot>,
    /// 10 "hide visual" toggles.
    pub hide: Vec<bool>,
}

impl Loadout {
    fn new() -> Self {
        Self {
            items: vec![Slot::empty(); 10],
            social: vec![Slot::empty(); 10],
            dyes: vec![Slot::empty(); 10],
            hide: vec![false; 10],
        }
    }

    /// The active loadout stored inline with the player does *not* carry its own
    /// hide-visual flags (those live in `PlayerFile::hide_visual`); each of the
    /// three stored loadouts carries 10 extra hide bytes.
    fn read(r: &mut Reader<'_>, v: i32, k: SlotKind) -> Result<Self, IoError> {
        let items = read_slots(r, v, k, 10)?;
        let social = read_slots(r, v, k, 10)?;
        let dyes = read_slots(r, v, k, 10)?;
        let hide = if k.multi {
            (0..10).map(|_| r.bool()).collect::<Result<_, _>>()?
        } else {
            vec![false; 10]
        };
        Ok(Self {
            items,
            social,
            dyes,
            hide,
        })
    }

    fn write(&self, w: &mut Writer, v: i32, k: SlotKind) {
        let kk = if k.multi {
            K_LOADOUT_STORED
        } else {
            K_LOADOUT_ACTIVE
        };
        write_slots(w, v, kk, &self.items);
        write_slots(w, v, kk, &self.social);
        write_slots(w, v, kk, &self.dyes);
        if k.multi {
            for h in &self.hide {
                w.bool(*h);
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ServerEntry {
    pub spawn_x: i32,
    pub spawn_y: i32,
    pub address: i32,
    pub name: String,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Buff {
    pub id: i32,
    pub time: i32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    /// Terraria stores colours as `r << 16 | g << 8 | b`.
    pub fn to_u32(self) -> u32 {
        (self.r as u32) << 16 | (self.g as u32) << 8 | self.b as u32
    }

    pub fn from_u32(v: u32) -> Self {
        Self {
            r: (v >> 16) as u8,
            g: (v >> 8) as u8,
            b: v as u8,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlayerFile {
    pub version: i32,
    pub meta_version: u32,
    pub meta_flags1: u32,
    pub meta_flags2: u32,

    pub name: String,
    pub difficulty: u8,
    pub play_time_l: u32,
    pub play_time_h: u32,
    pub hair_style: i32,
    pub hair_dye: u8,
    pub team: u8,
    pub hide_visual: u16,
    pub hide_misc: u8,
    pub gender: u8,

    pub health_now: i32,
    pub health_max: i32,
    pub mana_now: i32,
    pub mana_max: i32,

    pub extra_accessory: bool,
    pub unlocked_biome_torches: bool,
    pub using_biome_torches: bool,
    pub ate_artisan_bread: bool,
    pub mystery_flag_1470: bool,
    /// The six 1.4.4 permanent-booster flags, in on-disk order (v >= 269):
    /// `usedAegisCrystal` (Vital Crystal), `usedAegisFruit`,
    /// `usedArcaneCrystal`, `usedGalaxyPearl`, `usedGummyWorm`, `usedAmbrosia`.
    ///
    /// Verified against TEdit's reader/writer, which is the closest thing to a
    /// readable copy of `Player.SavePlayer` that exists. The internal name for
    /// Vital Crystal is `AegisCrystal`, which is why the field names and the
    /// in-game names disagree.
    pub extra_using_flags: [bool; 6],
    pub finished_dd2_event: bool,
    pub tax_money: i32,
    pub pve_deaths: i32,
    pub pvp_deaths: i32,

    pub hair_color: Color,
    pub skin_color: Color,
    pub eye_color: Color,
    pub shirt_color: Color,
    pub under_color: Color,
    pub pants_color: Color,
    pub shoes_color: Color,

    pub active_loadout: Loadout,
    pub inventory: Vec<Slot>,
    pub coins: Vec<Slot>,
    pub ammo: Vec<Slot>,
    pub equipment_items: Vec<Slot>,
    pub equipment_dyes: Vec<Slot>,
    pub bank: Vec<Slot>,
    pub safe: Vec<Slot>,
    pub forge: Vec<Slot>,
    pub void_bag: Vec<Slot>,
    pub void_vault_byte: u8,

    pub buffs: Vec<Buff>,
    pub servers: Vec<ServerEntry>,
    pub hotbar_locked: bool,
    pub hide_info: Vec<bool>,
    pub fishing_quests: i32,
    pub dpad_bindings: Vec<i32>,
    pub builder_acc: Vec<i32>,
    pub bartender_quests: i32,
    pub is_dead: bool,
    pub respawn_timer: i32,
    pub last_time_saved_1: i32,
    pub last_time_saved_2: i32,
    pub golfer_score: i32,
    pub research_mystery_byte: u8,
    /// Journey-mode research: persistent item id -> researched count.
    pub research: Vec<(String, i32)>,
    pub temp_items: Vec<Slot>,
    /// Journey-mode creative powers, in file order.
    pub creative_powers: Vec<CreativePower>,

    pub super_cart_byte: u8,
    pub current_loadout: i32,
    pub stored_loadouts: Vec<Loadout>,
    pub voice_variant: u8,
    pub voice_pitch_offset: i32,
    pub pending_refunds: Vec<(i32, i32, u8)>,
    pub dialogues_seen: Vec<String>,

    /// Bytes after the last known field, written back verbatim on save.
    pub trailing: Vec<u8>,
}

impl PlayerFile {
    pub fn new_v326(name: &str) -> Self {
        let mut p = Self {
            version: VER_1_4_5_8,
            meta_version: 1,
            meta_flags1: 0,
            meta_flags2: 0,
            name: name.to_string(),
            difficulty: 0,
            play_time_l: 0,
            play_time_h: 0,
            hair_style: 0,
            hair_dye: 0,
            team: 0,
            hide_visual: 0,
            hide_misc: 0,
            gender: 0,
            health_now: 100,
            health_max: 100,
            mana_now: 20,
            mana_max: 20,
            extra_accessory: false,
            unlocked_biome_torches: false,
            using_biome_torches: false,
            ate_artisan_bread: false,
            mystery_flag_1470: false,
            extra_using_flags: [false; 6],
            finished_dd2_event: false,
            tax_money: 0,
            pve_deaths: 0,
            pvp_deaths: 0,
            hair_color: Color::from_u32(14_113_335),
            skin_color: Color::from_u32(16_743_770),
            eye_color: Color::from_u32(6_904_395),
            shirt_color: Color::from_u32(11_511_180),
            under_color: Color::from_u32(10_532_055),
            pants_color: Color::from_u32(16_770_735),
            shoes_color: Color::from_u32(10_512_700),
            active_loadout: Loadout::new(),
            inventory: vec![Slot::empty(); 50],
            coins: vec![Slot::empty(); 4],
            ammo: vec![Slot::empty(); 4],
            equipment_items: vec![Slot::empty(); 5],
            equipment_dyes: vec![Slot::empty(); 5],
            bank: vec![Slot::empty(); 40],
            safe: vec![Slot::empty(); 40],
            forge: vec![Slot::empty(); 40],
            void_bag: vec![Slot::empty(); 40],
            void_vault_byte: 0,
            buffs: vec![Buff::default(); 44],
            servers: Vec::new(),
            hotbar_locked: false,
            hide_info: vec![false; 13],
            fishing_quests: 0,
            dpad_bindings: vec![0; 4],
            builder_acc: vec![0; 12],
            bartender_quests: 0,
            is_dead: false,
            respawn_timer: 0,
            last_time_saved_1: 0,
            last_time_saved_2: 0,
            golfer_score: 0,
            research_mystery_byte: 0,
            research: Vec::new(),
            temp_items: vec![Slot::empty(); 4],
            creative_powers: Vec::new(),
            super_cart_byte: 0,
            current_loadout: 0,
            stored_loadouts: vec![Loadout::new(); 3],
            voice_variant: 0,
            voice_pitch_offset: 0,
            pending_refunds: Vec::new(),
            dialogues_seen: Vec::new(),
            trailing: Vec::new(),
        };
        p.builder_acc[0] = 1;
        p
    }

    pub fn read_slot_like(&self) -> &[Slot] {
        &self.inventory
    }

    pub fn parse(plain: &[u8]) -> Result<Self, PlrError> {
        let mut r = Reader::new(plain);
        let mut p = Self::new_v326("");

        let v = r.i32()?;
        if v < VER_1_4_0 {
            return Err(PlrError::UnsupportedVersion(v));
        }
        p.version = v;

        let ma = r.u32()?;
        let mb = r.u32()?;
        if ma != MAGIC_A || mb != MAGIC_B {
            return Err(PlrError::BadMagic(ma, mb));
        }
        p.meta_version = r.u32()?;
        p.meta_flags1 = r.u32()?;
        p.meta_flags2 = r.u32()?;

        p.name = r.string()?;
        p.difficulty = r.u8()?;
        p.play_time_l = r.u32()?;
        p.play_time_h = r.u32()?;
        p.hair_style = r.i32()?;
        if v >= 82 {
            p.hair_dye = r.u8()?;
        }
        if v >= VER_1_4_5_0 {
            p.team = r.u8()?;
        }
        if v >= 83 {
            p.hide_visual = r.u8()? as u16;
            if v >= 145 {
                p.hide_visual |= (r.u8()? as u16) << 8;
            }
        }
        if v >= 145 {
            p.hide_misc = r.u8()?;
            p.gender = r.u8()?;
        }
        p.health_now = r.i32()?;
        p.health_max = r.i32()?;
        p.mana_now = r.i32()?;
        p.mana_max = r.i32()?;
        if v >= 145 {
            p.extra_accessory = r.bool()?;
            if v >= 230 {
                p.unlocked_biome_torches = r.bool()?;
                p.using_biome_torches = r.bool()?;
            }
            if v >= 269 {
                p.ate_artisan_bread = r.bool()?;
            }
            if v >= VER_1_4_5_7 {
                p.mystery_flag_1470 = r.bool()?;
            }
            if v >= 269 {
                for f in p.extra_using_flags.iter_mut() {
                    *f = r.bool()?;
                }
            }
            if v >= 184 {
                p.finished_dd2_event = r.bool()?;
            }
            p.tax_money = r.i32()?;
            if v >= 269 {
                p.pve_deaths = r.i32()?;
                p.pvp_deaths = r.i32()?;
            }
        }

        p.hair_color = Color {
            r: r.u8()?,
            g: r.u8()?,
            b: r.u8()?,
        };
        p.skin_color = Color {
            r: r.u8()?,
            g: r.u8()?,
            b: r.u8()?,
        };
        p.eye_color = Color {
            r: r.u8()?,
            g: r.u8()?,
            b: r.u8()?,
        };
        p.shirt_color = Color {
            r: r.u8()?,
            g: r.u8()?,
            b: r.u8()?,
        };
        p.under_color = Color {
            r: r.u8()?,
            g: r.u8()?,
            b: r.u8()?,
        };
        p.pants_color = Color {
            r: r.u8()?,
            g: r.u8()?,
            b: r.u8()?,
        };
        p.shoes_color = Color {
            r: r.u8()?,
            g: r.u8()?,
            b: r.u8()?,
        };

        p.active_loadout = Loadout::read(&mut r, v, K_LOADOUT_ACTIVE)?;
        p.inventory = read_slots(&mut r, v, K_INVENTORY, 50)?;
        p.coins = read_slots(&mut r, v, K_COIN_AMMO, 4)?;
        p.ammo = read_slots(&mut r, v, K_COIN_AMMO, 4)?;

        p.equipment_items = Vec::with_capacity(5);
        p.equipment_dyes = Vec::with_capacity(5);
        for _ in 0..5 {
            p.equipment_items.push(read_slot(&mut r, v, K_EQUIP)?);
            p.equipment_dyes.push(read_slot(&mut r, v, K_EQUIP)?);
        }

        p.bank = read_slots(&mut r, v, K_BANK, 40)?;
        p.safe = read_slots(&mut r, v, K_BANK, 40)?;
        p.forge = read_slots(&mut r, v, K_BANK, 40)?;
        p.void_bag = read_slots(&mut r, v, K_VOID, 40)?;
        if v >= 200 {
            p.void_vault_byte = r.u8()?;
        }

        let n_buffs = if v >= 269 {
            44
        } else if v >= 77 {
            22
        } else {
            10
        };
        p.buffs = (0..n_buffs)
            .map(|_| {
                Ok::<_, IoError>(Buff {
                    id: r.i32()?,
                    time: r.i32()?,
                })
            })
            .collect::<Result<_, _>>()?;

        loop {
            let spawn_x = r.i32()?;
            if spawn_x == -1 {
                break;
            }
            let e = ServerEntry {
                spawn_x,
                spawn_y: r.i32()?,
                address: r.i32()?,
                name: r.string()?,
            };
            p.servers.push(e);
        }

        p.hotbar_locked = r.bool()?;
        if v >= 145 {
            p.hide_info = (0..13).map(|_| r.bool()).collect::<Result<_, _>>()?;
        }
        if v >= 98 {
            p.fishing_quests = r.i32()?;
        }
        if v >= 168 {
            p.dpad_bindings = (0..4).map(|_| r.i32()).collect::<Result<_, _>>()?;
        }
        let n_acc = if v >= 230 {
            12
        } else if v >= 200 {
            11
        } else {
            10
        };
        p.builder_acc = (0..n_acc).map(|_| r.i32()).collect::<Result<_, _>>()?;
        if v >= 184 {
            p.bartender_quests = r.i32()?;
        }
        if v >= 200 {
            p.is_dead = r.bool()?;
            if p.is_dead {
                p.respawn_timer = r.i32()?;
            }
            p.last_time_saved_1 = r.i32()?;
            p.last_time_saved_2 = r.i32()?;
            p.golfer_score = r.i32()?;
        }

        if v >= 200 {
            if v >= VER_1_4_5_0 {
                p.research_mystery_byte = r.u8()?;
            }
            let n = r.i32()?;
            if n < 0 {
                return Err(PlrError::Io(IoError::BadStringLen(n as u64)));
            }
            for _ in 0..n {
                let pid = r.string()?;
                let count = r.i32()?;
                p.research.push((pid, count));
            }

            let mask = r.u8()?;
            p.temp_items = Vec::new();
            for i in 0..4u8 {
                if mask & (1 << i) != 0 {
                    p.temp_items.push(read_slot(&mut r, v, K_TEMP)?);
                } else {
                    p.temp_items.push(Slot::empty());
                }
            }
        }

        if v >= 220 {
            // Creative-power list: a tag byte 1 + power id (i16) + payload, until tag 0.
            loop {
                let tag = r.u8()?;
                if tag == 0 {
                    break;
                }
                let power = r.i16()?;
                let value = match creative_power_payload(power) {
                    CreativePayload::None => CreativeValue::None,
                    CreativePayload::Byte => CreativeValue::Byte(r.u8()?),
                    CreativePayload::Float => CreativeValue::Float(r.f32()?),
                    CreativePayload::Unknown => {
                        return Err(PlrError::Io(IoError::BadStringLen(power as u64)));
                    }
                };
                p.creative_powers.push(CreativePower { id: power, value });
            }
        }
        if v >= 253 {
            p.super_cart_byte = r.u8()?;
        }
        if v >= 269 {
            p.current_loadout = r.i32()?;
            p.stored_loadouts = (0..3)
                .map(|_| Loadout::read(&mut r, v, K_LOADOUT_STORED))
                .collect::<Result<_, _>>()?;
        }
        if v >= VER_1_4_5_0 {
            p.voice_variant = r.u8()?;
            p.voice_pitch_offset = r.i32()?;

            let n = r.i32()?;
            for _ in 0..n.max(0) {
                let id = r.i32()?;
                let count = r.i32()?;
                let prefix = r.u8()?;
                p.pending_refunds.push((id, count, prefix));
            }
            let n = r.i32()?;
            for _ in 0..n.max(0) {
                p.dialogues_seen.push(r.string()?);
            }
        }

        p.trailing = r.rest().to_vec();
        Ok(p)
    }

    pub fn serialize(&self) -> Vec<u8> {
        let v = self.version;
        let mut w = Writer::new();

        w.i32(v);
        w.u32(MAGIC_A);
        w.u32(MAGIC_B);
        w.u32(self.meta_version);
        w.u32(self.meta_flags1);
        w.u32(self.meta_flags2);

        w.string(&self.name);
        w.u8(self.difficulty);
        w.u32(self.play_time_l);
        w.u32(self.play_time_h);
        w.i32(self.hair_style);
        if v >= 82 {
            w.u8(self.hair_dye);
        }
        if v >= VER_1_4_5_0 {
            w.u8(self.team);
        }
        if v >= 83 {
            w.u8(self.hide_visual as u8);
            if v >= 145 {
                w.u8((self.hide_visual >> 8) as u8);
            }
        }
        if v >= 145 {
            w.u8(self.hide_misc);
            w.u8(self.gender);
        }
        w.i32(self.health_now);
        w.i32(self.health_max);
        w.i32(self.mana_now);
        w.i32(self.mana_max);
        if v >= 145 {
            w.bool(self.extra_accessory);
            if v >= 230 {
                w.bool(self.unlocked_biome_torches);
                w.bool(self.using_biome_torches);
            }
            if v >= 269 {
                w.bool(self.ate_artisan_bread);
            }
            if v >= VER_1_4_5_7 {
                w.bool(self.mystery_flag_1470);
            }
            if v >= 269 {
                for f in self.extra_using_flags {
                    w.bool(f);
                }
            }
            if v >= 184 {
                w.bool(self.finished_dd2_event);
            }
            w.i32(self.tax_money);
            if v >= 269 {
                w.i32(self.pve_deaths);
                w.i32(self.pvp_deaths);
            }
        }

        for c in [
            self.hair_color,
            self.skin_color,
            self.eye_color,
            self.shirt_color,
            self.under_color,
            self.pants_color,
            self.shoes_color,
        ] {
            w.u8(c.r);
            w.u8(c.g);
            w.u8(c.b);
        }

        self.active_loadout.write(&mut w, v, K_LOADOUT_ACTIVE);
        write_slots(&mut w, v, K_INVENTORY, &self.inventory);
        write_slots(&mut w, v, K_COIN_AMMO, &self.coins);
        write_slots(&mut w, v, K_COIN_AMMO, &self.ammo);
        for i in 0..5 {
            write_slot(&mut w, v, K_EQUIP, &self.equipment_items[i]);
            write_slot(&mut w, v, K_EQUIP, &self.equipment_dyes[i]);
        }
        write_slots(&mut w, v, K_BANK, &self.bank);
        write_slots(&mut w, v, K_BANK, &self.safe);
        write_slots(&mut w, v, K_BANK, &self.forge);
        write_slots(&mut w, v, K_VOID, &self.void_bag);
        if v >= 200 {
            w.u8(self.void_vault_byte);
        }

        for b in &self.buffs {
            w.i32(b.id);
            w.i32(b.time);
        }
        for s in &self.servers {
            w.i32(s.spawn_x);
            w.i32(s.spawn_y);
            w.i32(s.address);
            w.string(&s.name);
        }
        w.i32(-1);

        w.bool(self.hotbar_locked);
        if v >= 145 {
            for h in &self.hide_info {
                w.bool(*h);
            }
        }
        if v >= 98 {
            w.i32(self.fishing_quests);
        }
        if v >= 168 {
            for d in &self.dpad_bindings {
                w.i32(*d);
            }
        }
        for a in &self.builder_acc {
            w.i32(*a);
        }
        if v >= 184 {
            w.i32(self.bartender_quests);
        }
        if v >= 200 {
            w.bool(self.is_dead);
            if self.is_dead {
                w.i32(self.respawn_timer);
            }
            w.i32(self.last_time_saved_1);
            w.i32(self.last_time_saved_2);
            w.i32(self.golfer_score);
        }

        if v >= 200 {
            if v >= VER_1_4_5_0 {
                w.u8(self.research_mystery_byte);
            }
            w.i32(self.research.len() as i32);
            for (pid, count) in &self.research {
                w.string(pid);
                w.i32(*count);
            }
            let mut mask = 0u8;
            for (i, s) in self.temp_items.iter().enumerate().take(4) {
                if !s.is_empty() {
                    mask |= 1 << i;
                }
            }
            w.u8(mask);
            for (i, s) in self.temp_items.iter().enumerate().take(4) {
                if mask & (1 << i) != 0 {
                    write_slot(&mut w, v, K_TEMP, s);
                }
            }
        }

        if v >= 220 {
            for cp in &self.creative_powers {
                w.u8(1);
                w.i16(cp.id);
                match cp.value {
                    CreativeValue::None => {}
                    CreativeValue::Byte(b) => w.u8(b),
                    CreativeValue::Float(f) => w.f32(f),
                }
            }
            w.u8(0);
        }

        if v >= 253 {
            w.u8(self.super_cart_byte);
        }
        if v >= 269 {
            w.i32(self.current_loadout);
            for l in &self.stored_loadouts {
                l.write(&mut w, v, K_LOADOUT_STORED);
            }
        }
        if v >= VER_1_4_5_0 {
            w.u8(self.voice_variant);
            w.i32(self.voice_pitch_offset);
            w.i32(self.pending_refunds.len() as i32);
            for (id, count, prefix) in &self.pending_refunds {
                w.i32(*id);
                w.i32(*count);
                w.u8(*prefix);
            }
            w.i32(self.dialogues_seen.len() as i32);
            for s in &self.dialogues_seen {
                w.string(s);
            }
        }

        w.raw(&self.trailing);
        w.into_bytes()
    }

    pub fn format_name(&self) -> &'static str {
        match self.version {
            v if v >= VER_1_4_5_8 => "1.4.5.8+",
            v if v >= VER_1_4_5_7 => "1.4.5.7",
            v if v >= VER_1_4_5_0 => "1.4.5.x",
            v if v >= VER_1_4_4_9 => "1.4.4.9",
            v if v >= VER_1_4_4_0 => "1.4.4.x",
            _ => "1.4.x",
        }
    }
}

/// Payload of one Journey-mode creative power entry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CreativeValue {
    None,
    Byte(u8),
    Float(f32),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CreativePower {
    pub id: i16,
    pub value: CreativeValue,
}

enum CreativePayload {
    None,
    Byte,
    Float,
    Unknown,
}

/// Payload size of each creative (Journey) power id, per 1.4.4.9 / 1.4.5.x.
fn creative_power_payload(id: i16) -> CreativePayload {
    match id {
        0 => CreativePayload::Byte,
        1..=4 => CreativePayload::None,
        5 => CreativePayload::Byte,
        6..=7 => CreativePayload::None,
        8 => CreativePayload::Float,
        9..=11 => CreativePayload::Byte,
        12 => CreativePayload::Float,
        13 => CreativePayload::Byte,
        14 => CreativePayload::Float,
        _ => CreativePayload::Unknown,
    }
}
