//! terra-forge — native Linux Terraria character/inventory editor.

use std::path::PathBuf;

use eframe::egui::{self, Color32, RichText, Sense, Stroke, Vec2};

use terra_assets::{tag_bit, ItemDb};
use terra_core::{PlayerFile, Slot};

pub mod theme;

use theme::rarity_color;

// ---------------------------------------------------------------- containers

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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Group {
    Items,
    Social,
    Dyes,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ct {
    Bag(Bag),
    ActiveL(Group),
    StoredL(u8, Group),
}

fn container<'a>(p: &'a PlayerFile, c: Ct) -> &'a Vec<Slot> {
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

fn container_mut<'a>(p: &'a mut PlayerFile, c: Ct) -> &'a mut Vec<Slot> {
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

/// Whether this container stores a stack count on disk (equipment slots do not).
fn is_multi(c: Ct) -> bool {
    match c {
        Ct::Bag(Bag::EquipItems) | Ct::Bag(Bag::EquipDyes) => false,
        Ct::ActiveL(_) => false,
        _ => true,
    }
}

fn container_label(c: Ct) -> &'static str {
    match c {
        Ct::Bag(Bag::Inv) => "Inventory",
        Ct::Bag(Bag::Coins) => "Coins",
        Ct::Bag(Bag::Ammo) => "Ammo",
        Ct::Bag(Bag::EquipItems) => "Equipment",
        Ct::Bag(Bag::EquipDyes) => "Equipment Dye",
        Ct::Bag(Bag::Bank) => "Piggy Bank",
        Ct::Bag(Bag::Safe) => "Safe",
        Ct::Bag(Bag::Forge) => "Defender's Forge",
        Ct::Bag(Bag::Void) => "Void Vault",
        Ct::Bag(Bag::Temp) => "Temp",
        Ct::ActiveL(_) => "Loadout",
        Ct::StoredL(n, _) => match n {
            0 => "Loadout 2",
            1 => "Loadout 3",
            _ => "Loadout 4",
        },
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SlotRef {
    pub c: Ct,
    pub i: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Character,
    Inventory,
    Banks,
    Buffs,
    Research,
}

// ---------------------------------------------------------------- app state

pub struct App {
    db: &'static ItemDb,
    player: Option<PlayerFile>,
    path: Option<PathBuf>,
    dirty: bool,
    status: String,
    status_is_error: bool,

    tab: Tab,
    selected_loadout: u8,
    selected: Option<SlotRef>,
    picked: Option<Slot>,
    picked_from: Option<SlotRef>,

    search: String,
    cat: u32,
    results: Vec<usize>,

    open_picker: bool,
    picker_filter: String,
    manual_path: String,
    discovered: Vec<PathBuf>,

    stack_edit: i32,
    prefix_edit: u8,
    hovered_item: Option<i32>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = Color32::from_rgb(24, 26, 32);
        visuals.window_fill = Color32::from_rgb(28, 30, 36);
        cc.egui_ctx.set_visuals(visuals);
        cc.egui_ctx.all_styles_mut(|style| {
            style.spacing.item_spacing = Vec2::new(6.0, 6.0);
        });

        let mut app = Self {
            db: ItemDb::embedded(),
            player: None,
            path: None,
            dirty: false,
            status: String::from("Open a character to begin."),
            status_is_error: false,
            tab: Tab::Inventory,
            selected_loadout: 0,
            selected: None,
            picked: None,
            picked_from: None,
            search: String::new(),
            cat: 0,
            results: Vec::new(),
            open_picker: true,
            picker_filter: String::new(),
            manual_path: String::new(),
            discovered: Vec::new(),
            stack_edit: 1,
            prefix_edit: 0,
            hovered_item: None,
        };
        app.refresh_discovered();
        app.refresh_results();
        if let Some(arg) = std::env::args().nth(1) {
            app.open(PathBuf::from(arg));
        }
        // handy for screenshots / visual checks without a mouse
        if let Ok(tab) = std::env::var("TF_TAB") {
            app.tab = match tab.as_str() {
                "banks" => Tab::Banks,
                "buffs" => Tab::Buffs,
                "journey" => Tab::Research,
                "raw" => Tab::Character,
                _ => Tab::Inventory,
            };
        }
        app
    }

    fn refresh_discovered(&mut self) {
        self.discovered = terra_core::discover_players();
    }

    fn refresh_results(&mut self) {
        self.results = self
            .db
            .search(&self.search, self.cat)
            .iter()
            .map(|i| i.id as usize)
            .collect();
    }

    fn set_status(&mut self, msg: impl Into<String>, is_error: bool) {
        self.status = msg.into();
        self.status_is_error = is_error;
    }

    fn open(&mut self, path: PathBuf) {
        match terra_core::load(&path) {
            Ok(p) => {
                self.set_status(
                    format!(
                        "Loaded {} (v{}, {})",
                        path.file_name().unwrap_or_default().to_string_lossy(),
                        p.version,
                        p.format_name()
                    ),
                    false,
                );
                self.stack_edit = 1;
                self.selected = Some(SlotRef {
                    c: Ct::Bag(Bag::Inv),
                    i: 0,
                });
                self.path = Some(path);
                self.player = Some(p);
                self.dirty = false;
                self.open_picker = false;
                self.tab = Tab::Inventory;
            }
            Err(e) => self.set_status(format!("Could not open: {e}"), true),
        }
    }

    fn save(&mut self, as_path: Option<PathBuf>) {
        let Some(player) = &self.player else { return };
        let Some(path) = as_path.or_else(|| self.path.clone()) else {
            self.set_status("No file path to save to.", true);
            return;
        };
        match terra_core::save_with_backup(&path, player) {
            Ok(bak) => {
                let mut msg = format!("Saved {}", path.display());
                if bak.as_os_str().is_empty() {
                    msg.push_str(" (no previous file to back up)");
                } else {
                    msg.push_str(&format!(
                        " (backup: {})",
                        bak.file_name().unwrap_or_default().to_string_lossy()
                    ));
                }
                self.set_status(msg, false);
                self.path = Some(path);
                self.dirty = false;
                self.refresh_discovered();
            }
            Err(e) => self.set_status(format!("Save failed: {e}"), true),
        }
    }

    // ---------------------------------------------------------- item helpers

    fn max_stack_for(&self, net_id: i32) -> i32 {
        self.db.max_stack(net_id)
    }

    fn place(&mut self, to: SlotRef, mut item: Slot) {
        if item.net_id <= 0 {
            return;
        }
        if is_multi(to.c) {
            let max = self.max_stack_for(item.net_id).max(1);
            item.stack = item.stack.clamp(1, max);
        } else {
            item.stack = 1;
        }

        let source = self.picked_from;
        let mut player = self.player.take();
        if let Some(p) = &mut player {
            *container_mut(p, to.c).get_mut(to.i).unwrap() = item;
            if let Some(src) = source {
                if src != to {
                    *container_mut(p, src.c).get_mut(src.i).unwrap() = Slot::empty();
                }
            }
        }
        self.player = player;
        self.picked = None;
        self.picked_from = None;
        self.dirty = true;
    }

    fn describe(&self, slot: &Slot) -> String {
        if slot.is_empty() {
            return "empty".into();
        }
        let name = self.db.display_name(slot.net_id);
        let mut s = format!("#{} {name}", slot.net_id);
        if let Some(p) = self.db.prefix_name(slot.prefix) {
            s.push_str(&format!(" · {p}"));
        }
        s
    }
}

// ---------------------------------------------------------------- rendering

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let saves_now =
            ui.input(|i| (i.modifiers.ctrl || i.modifiers.command) && i.key_pressed(egui::Key::S));
        if saves_now && self.player.is_some() {
            self.save(None);
        }

        self.top_bar(ui);
        self.status_bar(ui);
        self.right_panel(ui);
        self.left_panel(ui);
        self.central(ui);
        self.picker_window(&ctx);
    }
}

impl App {
    fn top_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("top").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading(RichText::new("terra-forge").strong());
                ui.separator();
                if ui.button("Open…").clicked() {
                    self.refresh_discovered();
                    self.open_picker = true;
                }
                let can_save = self.player.is_some();
                if ui
                    .add_enabled(
                        can_save,
                        egui::Button::new(if self.dirty { "Save *" } else { "Save" }),
                    )
                    .clicked()
                {
                    self.save(None);
                }
                if ui
                    .add_enabled(can_save, egui::Button::new("Revert"))
                    .clicked()
                {
                    if let Some(p) = self.path.clone() {
                        self.open(p);
                    }
                }
                if let Some(path) = &self.path {
                    ui.separator();
                    let name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    ui.label(RichText::new(name).weak());
                    ui.label(
                        RichText::new(
                            path.parent()
                                .map(|p| p.display().to_string())
                                .unwrap_or_default(),
                        )
                        .weak()
                        .small(),
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(p) = &self.player {
                        let db = self.db.game_version.clone();
                        ui.label(
                            RichText::new(format!("{} · item db {}", p.format_name(), db)).weak(),
                        );
                    }
                });
            });
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                let color = if self.status_is_error {
                    Color32::from_rgb(240, 120, 120)
                } else {
                    Color32::from_rgb(150, 160, 175)
                };
                ui.label(RichText::new(&self.status).color(color));
                if let Some(p) = &self.player {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!(
                                "v{} · {} trailing bytes · {} research",
                                p.version,
                                p.trailing.len(),
                                p.research.len()
                            ))
                            .weak()
                            .small(),
                        );
                    });
                }
            });
        });
    }

    // ------------------------------------------------------------- left panel

    fn left_panel(&mut self, ui0: &mut egui::Ui) {
        egui::Panel::left("stats")
            .resizable(true)
            .default_size(268.0)
            .show(ui0, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let Some(p) = &mut self.player else {
                        ui.add_space(20.0);
                        ui.label("No character open.");
                        return;
                    };
                    ui.add_space(4.0);
                    ui.heading("Character");
                    ui.horizontal(|ui| {
                        ui.label("Name");
                        if ui.text_edit_singleline(&mut p.name).changed() {
                            self.dirty = true;
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Difficulty");
                        egui::ComboBox::from_id_salt("difficulty")
                            .selected_text(difficulty_name(p.difficulty))
                            .show_ui(ui, |ui| {
                                for (i, n) in ["Classic", "Expert", "Master", "Creative (Journey)"]
                                    .iter()
                                    .enumerate()
                                {
                                    if ui
                                        .selectable_value(&mut p.difficulty, i as u8, *n)
                                        .changed()
                                    {
                                        self.dirty = true;
                                    }
                                }
                            });
                    });

                    ui.add_space(6.0);
                    ui.label(RichText::new("Vitals").strong());
                    let mut changed = false;
                    changed |= ui
                        .add(
                            egui::Slider::new(&mut p.health_max, 100..=500)
                                .text("max life")
                                .clamping(egui::SliderClamping::Always),
                        )
                        .changed();
                    changed |= ui
                        .add(
                            egui::Slider::new(&mut p.health_now, 0..=p.health_max)
                                .text("life")
                                .clamping(egui::SliderClamping::Always),
                        )
                        .changed();
                    changed |= ui
                        .add(
                            egui::Slider::new(&mut p.mana_max, 20..=200)
                                .text("max mana")
                                .clamping(egui::SliderClamping::Always),
                        )
                        .changed();
                    changed |= ui
                        .add(
                            egui::Slider::new(&mut p.mana_now, 0..=p.mana_max)
                                .text("mana")
                                .clamping(egui::SliderClamping::Always),
                        )
                        .changed();
                    ui.horizontal(|ui| {
                        if ui.button("Base").clicked() {
                            p.health_max = 100;
                            p.mana_max = 20;
                            changed = true;
                        }
                        if ui.button("Life Fruit ×20 + Crystals ×9").clicked() {
                            p.health_max = 500;
                            p.mana_max = 200;
                            changed = true;
                        }
                    });
                    if changed {
                        p.health_now = p.health_now.clamp(0, p.health_max);
                        p.mana_now = p.mana_now.clamp(0, p.mana_max);
                        self.dirty = true;
                    }

                    ui.add_space(8.0);
                    ui.label(RichText::new("Permanent upgrades").strong());
                    let mut ch = false;
                    ch |= ui
                        .checkbox(&mut p.extra_accessory, "Demon Heart (+1 accessory slot)")
                        .changed();
                    ch |= ui
                        .checkbox(&mut p.ate_artisan_bread, "Artisan Loaf")
                        .changed();
                    ch |= ui
                        .checkbox(&mut p.unlocked_biome_torches, "Unlocked biome torches")
                        .changed();
                    ch |= ui
                        .checkbox(&mut p.using_biome_torches, "Using biome torches")
                        .changed();
                    ch |= ui
                        .checkbox(&mut p.finished_dd2_event, "Old One's Army completed")
                        .changed();
                    ch |= ui
                        .checkbox(&mut p.extra_using_flags[0], "Advanced Combat Targeting")
                        .changed();
                    if ch {
                        self.dirty = true;
                    }

                    ui.add_space(8.0);
                    ui.label(RichText::new("Colours").strong());
                    let mut ch = false;
                    for (label, c) in [
                        ("hair", &mut p.hair_color),
                        ("skin", &mut p.skin_color),
                        ("eyes", &mut p.eye_color),
                        ("shirt", &mut p.shirt_color),
                        ("undershirt", &mut p.under_color),
                        ("pants", &mut p.pants_color),
                        ("shoes", &mut p.shoes_color),
                    ] {
                        ui.horizontal(|ui| {
                            let mut col = [c.r, c.g, c.b];
                            if ui.color_edit_button_srgb(&mut col).changed() {
                                c.r = col[0];
                                c.g = col[1];
                                c.b = col[2];
                                ch = true;
                            }
                            ui.label(label);
                        });
                    }
                    if ch {
                        self.dirty = true;
                    }

                    ui.add_space(8.0);
                    ui.label(RichText::new("Journey").strong());
                    ui.label(
                        RichText::new(format!("{} items researched", p.research.len())).weak(),
                    );
                    ui.label(
                        RichText::new(format!(
                            "{} creative powers active",
                            p.creative_powers.len()
                        ))
                        .weak(),
                    );
                });
            });
    }

    // ------------------------------------------------------------ right panel

    fn right_panel(&mut self, ui0: &mut egui::Ui) {
        egui::Panel::right("browser")
            .resizable(true)
            .default_size(300.0)
            .show(ui0, |ui| {
                ui.add_space(4.0);
                ui.heading("Items");
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text("search items…  (name, phrase, id)")
                        .desired_width(f32::INFINITY),
                );
                if resp.changed() {
                    self.refresh_results();
                }
                ui.horizontal_wrapped(|ui| {
                    let cats = [
                        ("All", 0u32),
                        (
                            "Weapons",
                            tag_bit("Melee")
                                | tag_bit("Ranged")
                                | tag_bit("Magic")
                                | tag_bit("Summon"),
                        ),
                        (
                            "Armor",
                            tag_bit("HeadSlot") | tag_bit("BodySlot") | tag_bit("LegsSlot"),
                        ),
                        ("Accessories", tag_bit("Accessory")),
                        ("Blocks", tag_bit("Tile") | tag_bit("Wall")),
                        ("Consumables", tag_bit("Consumable")),
                        ("Materials", tag_bit("Material")),
                    ];
                    for (name, mask) in cats {
                        if ui.selectable_label(self.cat == mask, name).clicked() {
                            self.cat = mask;
                            self.refresh_results();
                        }
                    }
                });
                ui.separator();

                let target = self.selected;
                let row_h = 22.0;
                let total = self.results.len();
                ui.label(RichText::new(format!("{total} results")).weak().small());
                egui::ScrollArea::vertical()
                    .id_salt("itemscroll")
                    .show_rows(ui, row_h, total, |ui, range| {
                        for idx in range {
                            let id = self.results[idx] as i32;
                            let Some(item) = self.db.by_id(id) else {
                                continue;
                            };
                            let (rect, resp) = ui.allocate_exact_size(
                                Vec2::new(ui.available_width(), row_h),
                                Sense::click(),
                            );
                            let hovered = resp.hovered();
                            if hovered {
                                ui.painter()
                                    .rect_filled(rect, 3.0, Color32::from_rgb(40, 44, 54));
                            }
                            let icon = egui::Rect::from_min_size(
                                rect.min + Vec2::new(2.0, 2.0),
                                Vec2::splat(row_h - 4.0),
                            );
                            ui.painter()
                                .rect_filled(icon, 2.0, Color32::from_rgb(34, 37, 46));
                            let accent =
                                egui::Rect::from_min_size(icon.min, Vec2::new(2.0, icon.height()));
                            ui.painter()
                                .rect_filled(accent, 1.0, rarity_color(item.rarity));
                            let initials: String = item
                                .name
                                .split_whitespace()
                                .take(2)
                                .map(|w| w.chars().next().unwrap_or('?'))
                                .collect();
                            ui.painter().text(
                                icon.center(),
                                egui::Align2::CENTER_CENTER,
                                initials,
                                egui::FontId::proportional(10.0),
                                rarity_color(item.rarity),
                            );
                            ui.painter().text(
                                rect.min + Vec2::new(row_h + 6.0, row_h * 0.5),
                                egui::Align2::LEFT_CENTER,
                                &item.name,
                                egui::FontId::proportional(13.0),
                                Color32::from_gray(220),
                            );
                            ui.painter().text(
                                rect.max - Vec2::new(4.0, row_h * 0.5),
                                egui::Align2::RIGHT_CENTER,
                                format!("#{}", item.id),
                                egui::FontId::monospace(10.0),
                                Color32::from_gray(120),
                            );
                            if hovered {
                                self.hovered_item = Some(id);
                            }
                            if resp.clicked() {
                                let mut s = Slot::new(id, self.db.max_stack(id), 0);
                                if let Some(t) = target {
                                    if !is_multi(t.c) {
                                        s.stack = 1;
                                    }
                                    self.picked_from = None;
                                    self.place(t, s);
                                } else {
                                    self.picked = Some(s);
                                    self.picked_from = None;
                                    self.set_status(
                                        format!(
                                            "Picked up {}. Click a slot to place it.",
                                            self.db.display_name(id)
                                        ),
                                        false,
                                    );
                                }
                            }
                        }
                    });

                ui.separator();
                if let Some(id) = self.hovered_item {
                    if let Some(item) = self.db.by_id(id) {
                        ui.label(RichText::new(&item.name).strong());
                        let mut bits = Vec::new();
                        if item.damage >= 0 {
                            bits.push(format!("{} damage", item.damage));
                        }
                        if item.defense > 0 {
                            bits.push(format!("{} defense", item.defense));
                        }
                        bits.push(format!("stack {}", item.max_stack));
                        if item.estimated {
                            bits.push("stats estimated".into());
                        }
                        ui.label(RichText::new(bits.join(" · ")).weak().small());
                    }
                } else if self.picked.is_some() {
                    ui.label(
                        RichText::new(format!("Holding: {}", self.describe(&self.picked.unwrap())))
                            .color(Color32::from_rgb(200, 190, 120)),
                    );
                    if ui.button("Drop held item").clicked() {
                        self.picked = None;
                        self.picked_from = None;
                    }
                }
            });
    }

    // --------------------------------------------------------------- central

    fn central(&mut self, ui0: &mut egui::Ui) {
        egui::CentralPanel::default().show(ui0, |ui| {
            if self.player.is_none() {
                ui.add_space(24.0);
                ui.heading("Welcome");
                ui.label("Pick a character to edit:");
                ui.add_space(8.0);
                for p in self.discovered.clone() {
                    let label = p.display().to_string();
                    if ui.button(label).clicked() {
                        self.open(p);
                    }
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.label("Or a path:");
                    ui.text_edit_singleline(&mut self.manual_path);
                    if ui.button("Open").clicked() {
                        let p = PathBuf::from(self.manual_path.clone());
                        self.open(p);
                    }
                });
                return;
            }

            ui.horizontal_wrapped(|ui| {
                for (t, name) in [
                    (Tab::Inventory, "Inventory & Equipment"),
                    (Tab::Banks, "Banks"),
                    (Tab::Buffs, "Buffs"),
                    (Tab::Research, "Journey"),
                    (Tab::Character, "Raw"),
                ] {
                    if ui.selectable_label(self.tab == t, name).clicked() {
                        self.tab = t;
                    }
                }
            });
            ui.separator();

            egui::ScrollArea::vertical()
                .id_salt("main")
                .show(ui, |ui| match self.tab {
                    Tab::Inventory => self.tab_inventory(ui),
                    Tab::Banks => self.tab_banks(ui),
                    Tab::Buffs => self.tab_buffs(ui),
                    Tab::Research => self.tab_research(ui),
                    Tab::Character => self.tab_character(ui),
                });
        });
    }

    fn tab_inventory(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Loadout:");
            for (i, label) in ["1 (active)", "2", "3", "4"].iter().enumerate() {
                if ui
                    .selectable_label(self.selected_loadout == i as u8, *label)
                    .clicked()
                {
                    self.selected_loadout = i as u8;
                }
            }
        });

        let lo = self.selected_loadout;
        let (items_c, social_c, dyes_c) = if lo == 0 {
            (
                Ct::ActiveL(Group::Items),
                Ct::ActiveL(Group::Social),
                Ct::ActiveL(Group::Dyes),
            )
        } else {
            (
                Ct::StoredL(lo - 1, Group::Items),
                Ct::StoredL(lo - 1, Group::Social),
                Ct::StoredL(lo - 1, Group::Dyes),
            )
        };
        self.section(ui, "Armour & accessories", items_c);
        self.section(ui, "Vanity", social_c);
        self.section(ui, "Dyes", dyes_c);
        ui.add_space(6.0);
        self.section(ui, "Main inventory", Ct::Bag(Bag::Inv));
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                self.section(ui, "Coins", Ct::Bag(Bag::Coins));
            });
            ui.vertical(|ui| {
                self.section(ui, "Ammo", Ct::Bag(Bag::Ammo));
            });
        });
        self.section(
            ui,
            "Misc equipment (pet / light / cart / mount / hook)",
            Ct::Bag(Bag::EquipItems),
        );
        self.section(ui, "Misc dyes", Ct::Bag(Bag::EquipDyes));
    }

    fn tab_banks(&mut self, ui: &mut egui::Ui) {
        for (label, bag) in [
            ("Piggy Bank", Bag::Bank),
            ("Safe", Bag::Safe),
            ("Defender's Forge", Bag::Forge),
            ("Void Vault", Bag::Void),
        ] {
            self.section(ui, label, Ct::Bag(bag));
        }
    }

    fn tab_buffs(&mut self, ui: &mut egui::Ui) {
        ui.label("Active buffs (id / duration in ticks). 44 slots, as in the file format.");
        ui.add_space(4.0);
        let mut clear: Option<usize> = None;
        for i in 0..self.player.as_ref().map(|p| p.buffs.len()).unwrap_or(0) {
            let Some(p) = &mut self.player else { break };
            let b = &mut p.buffs[i];
            ui.horizontal(|ui| {
                ui.label(format!("#{i:02}"));
                let name = self
                    .db
                    .buffs
                    .iter()
                    .find(|x| x.id == b.id)
                    .map(|x| x.name.clone())
                    .unwrap_or_else(|| {
                        if b.id == 0 {
                            "—".into()
                        } else {
                            format!("buff {}", b.id)
                        }
                    });
                ui.label(RichText::new(name).weak());
                let mut id = b.id;
                if ui
                    .add(egui::DragValue::new(&mut id).speed(1.0).range(0..=400))
                    .changed()
                {
                    b.id = id;
                    self.dirty = true;
                }
                let mut t = b.time;
                if ui
                    .add(
                        egui::DragValue::new(&mut t)
                            .speed(60.0)
                            .range(0..=1_000_000),
                    )
                    .changed()
                {
                    b.time = t;
                    self.dirty = true;
                }
                if ui.small_button("clear").clicked() {
                    clear = Some(i);
                }
            });
        }
        if let (Some(i), Some(p)) = (clear, &mut self.player) {
            p.buffs[i] = terra_core::plr::Buff::default();
            self.dirty = true;
        }
    }

    fn tab_research(&mut self, ui: &mut egui::Ui) {
        let Some(p) = &self.player else { return };
        ui.label(format!("{} entries researched", p.research.len()));
        ui.separator();
        egui::ScrollArea::vertical()
            .max_height(420.0)
            .show(ui, |ui| {
                for (pid, count) in &p.research {
                    if let Some(item) = self.db.by_pid(pid) {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&item.name).strong());
                            ui.label(RichText::new(format!("#{}", item.id)).weak());
                            ui.label(format!("×{count}"));
                        });
                    } else {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(pid).weak());
                            ui.label(format!("×{count}"));
                        });
                    }
                }
            });
    }

    fn tab_character(&mut self, ui: &mut egui::Ui) {
        let Some(p) = &self.player else { return };
        ui.label(RichText::new("Decoded header").strong());
        egui::Grid::new("hdr").num_columns(2).show(ui, |ui| {
            for (k, v) in [
                ("version", p.version.to_string()),
                ("format", p.format_name().to_string()),
                ("meta version", p.meta_version.to_string()),
                (
                    "meta flags",
                    format!("{:#x}/{:#x}", p.meta_flags1, p.meta_flags2),
                ),
                (
                    "play time",
                    format!(
                        "{}s",
                        (p.play_time_l as u64 / 10_000_000) + (p.play_time_h as u64 * 429)
                    ),
                ),
                (
                    "hair style / dye",
                    format!("{} / {}", p.hair_style, p.hair_dye),
                ),
                ("team", p.team.to_string()),
                ("gender", p.gender.to_string()),
                ("hide visual", format!("{:#06x}", p.hide_visual)),
                ("tax money", p.tax_money.to_string()),
                (
                    "pve / pvp deaths",
                    format!("{} / {}", p.pve_deaths, p.pvp_deaths),
                ),
                ("golfer score", p.golfer_score.to_string()),
                ("angler quests", p.fishing_quests.to_string()),
                ("bartender quests", p.bartender_quests.to_string()),
                ("servers", p.servers.len().to_string()),
                ("trailing bytes", p.trailing.len().to_string()),
            ] {
                ui.label(RichText::new(k).weak());
                ui.label(v);
                ui.end_row();
            }
        });
    }

    // ------------------------------------------------------------- grid render

    fn section(&mut self, ui: &mut egui::Ui, title: &str, ct: Ct) {
        ui.add_space(6.0);
        ui.label(RichText::new(title).strong());
        let n = self
            .player
            .as_ref()
            .map(|p| container(p, ct).len())
            .unwrap_or(0);
        if n == 0 {
            return;
        }
        let per_row = 10.min(n);
        let cell = 44.0;
        let mut i = 0;
        while i < n {
            ui.horizontal(|ui| {
                for _ in 0..per_row {
                    if i >= n {
                        break;
                    }
                    self.slot_ui(ui, SlotRef { c: ct, i }, cell);
                    i += 1;
                }
            });
        }
    }

    fn slot_ui(&mut self, ui: &mut egui::Ui, r: SlotRef, cell: f32) {
        let Some(p) = &self.player else { return };
        let slot = *container(p, r.c).get(r.i).unwrap_or(&Slot::empty());
        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(cell), Sense::click());
        let painter = ui.painter_at(rect);
        let selected = self.selected == Some(r);
        let border = if selected {
            Stroke::new(2.0, Color32::from_rgb(240, 200, 90))
        } else {
            Stroke::new(1.0, Color32::from_rgb(64, 70, 84))
        };
        painter.rect_filled(rect.shrink(1.0), 3.0, Color32::from_rgb(34, 37, 46));
        painter.rect_stroke(rect.shrink(1.0), 3.0, border, egui::StrokeKind::Inside);

        if !slot.is_empty() {
            let item = self.db.by_id(slot.net_id);
            let rarity = item.map(|i| i.rarity).unwrap_or(0);
            let fill = rarity_color(rarity);
            painter.rect_filled(rect.shrink(4.0), 2.0, fill.linear_multiply(0.85));
            let initials: String = {
                let name = self.db.display_name(slot.net_id);
                name.split_whitespace()
                    .filter(|w| !w.is_empty())
                    .take(2)
                    .map(|w| w.chars().next().unwrap_or('?'))
                    .collect()
            };
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                initials,
                egui::FontId::proportional(13.0),
                Color32::from_black_alpha(200),
            );
            if slot.stack > 1 && is_multi(r.c) {
                painter.text(
                    rect.max - Vec2::new(3.0, 2.0),
                    egui::Align2::RIGHT_BOTTOM,
                    slot.stack.to_string(),
                    egui::FontId::proportional(11.0),
                    Color32::WHITE,
                );
            }
            if slot.favorited {
                painter.text(
                    rect.min + Vec2::new(3.0, 1.0),
                    egui::Align2::LEFT_TOP,
                    "★",
                    egui::FontId::proportional(10.0),
                    Color32::from_rgb(250, 220, 120),
                );
            }
        }

        let hovers = resp.hovered();
        if hovers {
            self.hovered_item = if slot.is_empty() {
                None
            } else {
                Some(slot.net_id)
            };
            let tip = format!(
                "{}  [{} {}]",
                self.describe(&slot),
                container_label(r.c),
                r.i
            );
            resp.clone().on_hover_text(tip);
        }

        // scroll to adjust stack
        if hovers {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.1 && is_multi(r.c) && !slot.is_empty() {
                let step = if ui.input(|i| i.modifiers.shift) {
                    100
                } else {
                    1
                };
                let delta = if scroll > 0.0 { step } else { -step };
                let max = self.max_stack_for(slot.net_id);
                if let Some(p) = &mut self.player {
                    let s = container_mut(p, r.c).get_mut(r.i).unwrap();
                    s.stack = (s.stack + delta).clamp(1, max);
                }
                self.dirty = true;
            }
        }

        if resp.clicked() {
            self.selected = Some(r);
            if let Some(item) = self.picked {
                self.place(r, item);
            }
            if let Some(s) = self
                .player
                .as_ref()
                .and_then(|p| container(p, r.c).get(r.i).copied())
            {
                self.stack_edit = s.stack.max(1);
                self.prefix_edit = s.prefix;
            }
        }

        let mut ctx_action: Option<CtxAction> = None;
        resp.context_menu(|ui| {
            ui.label(RichText::new(self.describe(&slot)).strong());
            ui.separator();
            let has = !slot.is_empty();
            if ui.add_enabled(has, egui::Button::new("Pick up")).clicked() {
                ctx_action = Some(CtxAction::Pick);
                ui.close();
            }
            if ui
                .add_enabled(has, egui::Button::new("Duplicate to cursor"))
                .clicked()
            {
                ctx_action = Some(CtxAction::Duplicate);
                ui.close();
            }
            if ui
                .add_enabled(has && is_multi(r.c), egui::Button::new("Max stack"))
                .clicked()
            {
                ctx_action = Some(CtxAction::MaxStack);
                ui.close();
            }
            if ui
                .add_enabled(has, egui::Button::new("Toggle favourite"))
                .clicked()
            {
                ctx_action = Some(CtxAction::Fav);
                ui.close();
            }
            if ui
                .add_enabled(has, egui::Button::new("Clear slot"))
                .clicked()
            {
                ctx_action = Some(CtxAction::Clear);
                ui.close();
            }
            ui.separator();
            if ui.button("Copy into brush (middle-click)").clicked() {
                ctx_action = Some(CtxAction::Duplicate);
                ui.close();
            }
        });

        if let Some(a) = ctx_action {
            self.apply_ctx(r, a, slot);
        }
        if resp.middle_clicked() && !slot.is_empty() {
            self.apply_ctx(r, CtxAction::Duplicate, slot);
        }
        if resp.double_clicked() && !slot.is_empty() {
            self.picked = Some(slot);
            self.picked_from = Some(r);
        }
    }

    fn apply_ctx(&mut self, r: SlotRef, a: CtxAction, slot: Slot) {
        match a {
            CtxAction::Pick => {
                self.picked = Some(slot);
                self.picked_from = Some(r);
            }
            CtxAction::Duplicate => {
                self.picked = Some(slot);
                self.picked_from = None;
            }
            CtxAction::MaxStack => {
                if let Some(p) = &mut self.player {
                    let max = self.db.max_stack(slot.net_id);
                    if let Some(s) = container_mut(p, r.c).get_mut(r.i) {
                        s.stack = max;
                    }
                    self.dirty = true;
                }
            }
            CtxAction::Fav => {
                if let Some(p) = &mut self.player {
                    if let Some(s) = container_mut(p, r.c).get_mut(r.i) {
                        s.favorited = !s.favorited;
                    }
                    self.dirty = true;
                }
            }
            CtxAction::Clear => {
                if let Some(p) = &mut self.player {
                    if let Some(s) = container_mut(p, r.c).get_mut(r.i) {
                        *s = Slot::empty();
                    }
                    self.dirty = true;
                }
            }
        }
    }

    // ---------------------------------------------------------------- dialogs

    fn picker_window(&mut self, ctx: &egui::Context) {
        if !self.open_picker {
            return;
        }
        let mut open = self.open_picker;
        egui::Window::new("Open character")
            .open(&mut open)
            .default_width(560.0)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(format!("{} player files found", self.discovered.len())).weak(),
                );
                ui.add(egui::TextEdit::singleline(&mut self.picker_filter).hint_text("filter…"));
                let filter = self.picker_filter.to_lowercase();
                let discovered = std::mem::take(&mut self.discovered);
                egui::ScrollArea::vertical()
                    .max_height(320.0)
                    .show(ui, |ui| {
                        for p in &discovered {
                            let label = p.display().to_string();
                            if !filter.is_empty() && !label.to_lowercase().contains(&filter) {
                                continue;
                            }
                            if ui.selectable_label(false, label).clicked() {
                                let path = p.clone();
                                self.open(path);
                            }
                        }
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Path:");
                    ui.text_edit_singleline(&mut self.manual_path);
                    if ui.button("Open").clicked() {
                        let p = PathBuf::from(self.manual_path.clone());
                        self.open(p);
                    }
                    if ui.button("Rescan").clicked() {
                        self.refresh_discovered();
                    }
                });
                self.discovered = discovered;
            });
        self.open_picker = open;
    }
}

#[derive(Clone, Copy)]
enum CtxAction {
    Pick,
    Duplicate,
    MaxStack,
    Fav,
    Clear,
}

fn difficulty_name(d: u8) -> &'static str {
    match d {
        0 => "Classic",
        1 => "Expert",
        2 => "Master",
        3 => "Creative (Journey)",
        _ => "Classic",
    }
}
