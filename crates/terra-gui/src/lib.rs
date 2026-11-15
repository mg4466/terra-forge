//! terra-forge — a native Linux Terraria character editor.
//!
//! # How the app is put together
//!
//! ```text
//! terra-core    the file format      (bytes in -> PlayerFile -> bytes out)
//! terra-assets  the item catalogue   (embedded JSON + fuzzy search)
//! terra-gui     this crate: state + egui rendering
//! ```
//!
//! The GUI never touches raw bytes. It holds one [`PlayerFile`] in memory and
//! mutates fields on it; saving goes through `terra_core::save_with_backup`,
//! which refuses to write anything it cannot reproduce byte-for-byte.
//!
//! ## The one structural idea worth knowing
//!
//! Everything the user can click on is a [`SlotRef`] — a *container* plus an
//! *index*. Containers are enumerated in [`containers`], which is also where the
//! "does this container store a stack / favourite byte" rules live. Adding a new
//! grid to the UI is therefore: pick a `Ct`, call `section()` with it, done.
//!
//! ## Rendering model
//!
//! Immediate mode: `ui()` runs top to bottom every frame. There is no widget
//! tree to keep in sync — only `App`'s fields are state. Anything transient
//! (a drag in progress) lives in `App` too, because egui forgets it between
//! frames.
//!
//! Panels must be added *before* the central panel or they get no space; that
//! ordering is why `ui()` reads top bar → status bar → side panels → centre.

use std::path::PathBuf;

use eframe::egui::{self, Color32, RichText, Sense, Stroke, Vec2};

use terra_core::Slot;

pub mod catalog;
pub mod containers;
pub mod operations;
pub mod settings;
pub mod theme;

use catalog::{Catalog, Row};
use containers::{container, is_multi, label as container_label, Bag, Ct, Group, SlotRef};
use settings::UiSettings;
use theme::rarity_color;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Inventory,
    Banks,
    Buffs,
    Journey,
    Raw,
}

/// One character found on disk, with the header already decoded so the picker
/// can show the in-game name instead of a file path.
struct PlayerEntry {
    path: PathBuf,
    /// In-game name from the file, or the file stem if it could not be parsed.
    name: String,
    detail: String,
    error: Option<String>,
}

impl PlayerEntry {
    fn probe(path: PathBuf) -> Self {
        let fallback = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        match terra_core::load(&path) {
            Ok(p) => {
                let used = p.inventory.iter().filter(|s| !s.is_empty()).count();
                Self {
                    path,
                    name: p.name.clone(),
                    detail: format!("v{} · {} · {used} items", p.version, p.format_name()),
                    error: None,
                }
            }
            Err(e) => Self {
                path,
                name: fallback,
                detail: String::new(),
                error: Some(e.to_string()),
            },
        }
    }
}

/// An item in flight: either picked from a slot (`from` set) or dragged out of
/// the item browser (`from` is `None`, which turns a drop into a copy).
#[derive(Clone, Copy)]
struct Drag {
    from: Option<SlotRef>,
    item: Slot,
}

pub struct App {
    db: &'static terra_assets::ItemDb,
    settings: UiSettings,

    player: Option<terra_core::PlayerFile>,
    path: Option<PathBuf>,
    dirty: bool,
    status: String,
    status_is_error: bool,

    tab: Tab,
    selected_loadout: u8,
    selected: Option<SlotRef>,

    /// In-progress drag-and-drop, if any.
    drag: Option<Drag>,

    search: String,
    cat: u32,
    catalog: Catalog,

    open_picker: bool,
    picker_filter: String,
    manual_path: String,
    players: Vec<PlayerEntry>,

    show_settings: bool,
    hovered_item: Option<i32>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings = UiSettings::load().clamped();
        apply_theme(&cc.egui_ctx, settings.zoom);

        let mut app = Self {
            db: terra_assets::ItemDb::embedded(),
            settings,
            player: None,
            path: None,
            dirty: false,
            status: "Open a character to begin.".to_owned(),
            status_is_error: false,
            tab: Tab::Inventory,
            selected_loadout: 0,
            selected: None,
            drag: None,
            search: String::new(),
            cat: 0,
            catalog: Catalog::default(),
            open_picker: true,
            picker_filter: String::new(),
            manual_path: String::new(),
            players: Vec::new(),
            show_settings: false,
            hovered_item: None,
        };
        app.rebuild_catalog();
        app.rescan();

        if let Some(arg) = std::env::args().nth(1) {
            app.open(PathBuf::from(arg));
            // Opening straight into a character should not leave the picker on top.
            app.open_picker = app.player.is_none();
        }
        // Handy for screenshots and manual checking without a mouse.
        if let Ok(tab) = std::env::var("TF_TAB") {
            app.tab = match tab.as_str() {
                "banks" => Tab::Banks,
                "buffs" => Tab::Buffs,
                "journey" => Tab::Journey,
                "raw" => Tab::Raw,
                _ => Tab::Inventory,
            };
        }
        app
    }

    // ------------------------------------------------------------ file actions

    fn rescan(&mut self) {
        self.players = terra_core::discover_players()
            .into_iter()
            .map(PlayerEntry::probe)
            .collect();
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
                self.path = Some(path);
                self.player = Some(p);
                self.dirty = false;
                self.selected = Some(SlotRef {
                    c: Ct::Bag(Bag::Inv),
                    i: 0,
                });
                self.tab = Tab::Inventory;
            }
            Err(e) => self.set_status(format!("Could not open: {e}"), true),
        }
    }

    fn save(&mut self) {
        let Some(player) = &self.player else { return };
        let Some(path) = self.path.clone() else {
            self.set_status("No file path to save to.", true);
            return;
        };
        match terra_core::save_with_backup(&path, player) {
            Ok(bak) => {
                let mut msg = format!("Saved {}", path.display());
                if bak.as_os_str().is_empty() {
                    msg.push_str(" (new file, nothing to back up)");
                } else {
                    msg.push_str(&format!(
                        " (backup: {})",
                        bak.file_name().unwrap_or_default().to_string_lossy()
                    ));
                }
                self.set_status(msg, false);
                self.dirty = false;
                self.rescan();
            }
            Err(e) => self.set_status(format!("Save failed: {e}"), true),
        }
    }

    fn set_status(&mut self, msg: impl Into<String>, is_error: bool) {
        self.status = msg.into();
        self.status_is_error = is_error;
    }

    // -------------------------------------------------------------- catalogue

    fn rebuild_catalog(&mut self) {
        self.catalog = Catalog::build(self.db, &self.search, self.cat);
    }

    /// Drop `drag` onto `to`: dragging from a slot swaps the two, dragging from
    /// the browser copies. All the actual byte-level rules live in `operations`.
    fn drop_on(&mut self, to: SlotRef, drag: Drag) {
        if let Some(mut p) = self.player.take() {
            operations::drop_item(self.db, &mut p, to, drag.from, drag.item);
            self.player = Some(p);
            self.dirty = true;
        }
        self.selected = Some(to);
    }

    /// Put an item into a slot without a source slot (browser click / duplicate).
    fn place(&mut self, to: SlotRef, item: Slot) {
        self.drop_on(to, Drag { from: None, item });
    }

    fn clear_slot(&mut self, r: SlotRef) {
        if let Some(p) = &mut self.player {
            operations::clear(p, r);
            self.dirty = true;
        }
    }

    fn describe(&self, slot: &Slot) -> String {
        if slot.is_empty() {
            return "empty".into();
        }
        let mut s = format!("#{} {}", slot.net_id, self.db.display_name(slot.net_id));
        if let Some(p) = self.db.prefix_name(slot.prefix) {
            s.push_str(&format!(" · {p}"));
        }
        if slot.stack > 1 && is_multi(Ct::Bag(Bag::Inv)) {
            s.push_str(&format!(" ×{}", slot.stack));
        }
        s
    }
}

// -------------------------------------------------------------------- theme

/// Dark palette plus the user's zoom factor. Zoom is applied through egui's own
/// zoom factor so every widget, glyph and padding scales together.
fn apply_theme(ctx: &egui::Context, zoom: f32) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = Color32::from_rgb(24, 26, 32);
    visuals.window_fill = Color32::from_rgb(28, 30, 36);
    ctx.set_visuals(visuals);
    ctx.all_styles_mut(|style| {
        use egui::{FontFamily::Proportional, FontId, TextStyle};
        style.spacing.item_spacing = Vec2::new(6.0, 6.0);
        style.spacing.button_padding = Vec2::new(8.0, 4.0);
        // Slightly larger than egui's defaults: the item grid is dense and a
        // 10 px "small" style is unreadable at arm's length.
        style.text_styles = [
            (TextStyle::Heading, FontId::new(21.0, Proportional)),
            (TextStyle::Body, FontId::new(15.0, Proportional)),
            (TextStyle::Button, FontId::new(15.0, Proportional)),
            (TextStyle::Small, FontId::new(12.5, Proportional)),
            (
                TextStyle::Monospace,
                FontId::new(13.0, egui::FontFamily::Monospace),
            ),
        ]
        .into();
    });
    ctx.set_zoom_factor(zoom);
}

// ---------------------------------------------------------------------- app

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        if (ctx.input(|i| i.modifiers.ctrl || i.modifiers.command))
            && ctx.input(|i| i.key_pressed(egui::Key::S))
        {
            self.save();
        }
        // A drag that ends over nothing is simply cancelled.
        if self.drag.is_some() && ctx.input(|i| i.pointer.any_released()) {
            self.drag = None;
        }

        self.top_bar(ui);
        self.status_bar(ui);
        self.side_left(ui);
        self.side_right(ui);
        self.central(ui);
        self.picker_window(&ctx);
        self.settings_window(&ctx);
        self.drag_ghost(&ctx);
    }
}

impl App {
    fn top_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("top").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading(RichText::new("terra-forge").strong());
                ui.separator();
                if ui
                    .button("Open…")
                    .on_hover_text("Choose a character (Ctrl+O)")
                    .clicked()
                {
                    self.rescan();
                    self.open_picker = true;
                }
                let can_save = self.player.is_some();
                let save_label = if self.dirty { "Save *" } else { "Save" };
                if ui
                    .add_enabled(can_save, egui::Button::new(save_label))
                    .on_hover_text("Save over the current file, keeping a .bak (Ctrl+S)")
                    .clicked()
                {
                    self.save();
                }
                if ui
                    .add_enabled(can_save, egui::Button::new("Revert"))
                    .on_hover_text("Reload from disk, discarding unsaved edits")
                    .clicked()
                {
                    if let Some(p) = self.path.clone() {
                        self.open(p);
                    }
                }
                if ui
                    .button("Aa")
                    .on_hover_text("Text size and slot size")
                    .clicked()
                {
                    self.show_settings = true;
                }

                if let Some(path) = &self.path {
                    ui.separator();
                    let name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    ui.label(RichText::new(name).strong());
                    ui.label(RichText::new("·").weak());
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
                        ui.label(
                            RichText::new(format!(
                                "{} · items {}",
                                p.format_name(),
                                self.db.game_version
                            ))
                            .weak(),
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
                                "v{} · {} trailing bytes · {} researched",
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

    // -------------------------------------------------------------- left side

    fn side_left(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("stats")
            .resizable(true)
            .default_size(240.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let Some(p) = &mut self.player else {
                        ui.add_space(12.0);
                        ui.label(RichText::new("No character open.").weak());
                        return;
                    };
                    let mut changed = false;

                    ui.add_space(4.0);
                    ui.heading("Character");
                    ui.horizontal(|ui| {
                        ui.label("Name");
                        changed |= ui.text_edit_singleline(&mut p.name).changed();
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
                                    changed |= ui
                                        .selectable_value(&mut p.difficulty, i as u8, *n)
                                        .changed();
                                }
                            });
                    });

                    // Vitals: plain numeric fields. DragValue is click-and-type,
                    // so it doubles as a text box without needing a separate mode.
                    ui.add_space(8.0);
                    ui.label(RichText::new("Vitals").strong());
                    egui::Grid::new("vitals")
                        .num_columns(3)
                        .spacing([6.0, 4.0])
                        .show(ui, |ui| {
                            ui.label("Life");
                            changed |= ui
                                .add(
                                    egui::DragValue::new(&mut p.health_now)
                                        .speed(5)
                                        .range(0..=p.health_max),
                                )
                                .changed();
                            changed |= ui
                                .add(
                                    egui::DragValue::new(&mut p.health_max)
                                        .speed(5)
                                        .range(100..=500),
                                )
                                .changed();
                            ui.end_row();

                            ui.label("Mana");
                            changed |= ui
                                .add(
                                    egui::DragValue::new(&mut p.mana_now)
                                        .speed(5)
                                        .range(0..=p.mana_max),
                                )
                                .changed();
                            changed |= ui
                                .add(
                                    egui::DragValue::new(&mut p.mana_max)
                                        .speed(5)
                                        .range(20..=200),
                                )
                                .changed();
                            ui.end_row();
                        });
                    ui.horizontal(|ui| {
                        if ui
                            .button("Fill to max")
                            .on_hover_text("Set current life and mana to their maximum")
                            .clicked()
                        {
                            p.health_now = p.health_max;
                            p.mana_now = p.mana_max;
                            changed = true;
                        }
                        if ui
                            .button("Max out")
                            .on_hover_text("Life Fruit ×20 + Mana Crystals ×9: life 500, mana 200")
                            .clicked()
                        {
                            p.health_max = 500;
                            p.health_now = 500;
                            p.mana_max = 200;
                            p.mana_now = 200;
                            changed = true;
                        }
                    });
                    if changed {
                        p.health_now = p.health_now.clamp(0, p.health_max.max(0));
                        p.mana_now = p.mana_now.clamp(0, p.mana_max.max(0));
                    }

                    ui.add_space(8.0);
                    ui.label(RichText::new("Permanent upgrades").strong());
                    changed |= ui
                        .checkbox(&mut p.extra_accessory, "Demon Heart (+1 accessory slot)")
                        .changed();
                    changed |= ui
                        .checkbox(&mut p.ate_artisan_bread, "Artisan Loaf")
                        .changed();
                    changed |= ui
                        .checkbox(&mut p.unlocked_biome_torches, "Unlocked biome torches")
                        .changed();
                    changed |= ui
                        .checkbox(&mut p.using_biome_torches, "Using biome torches")
                        .changed();
                    changed |= ui
                        .checkbox(&mut p.finished_dd2_event, "Old One's Army completed")
                        .changed();
                    changed |= ui
                        .checkbox(&mut p.extra_using_flags[0], "Advanced Combat Targeting")
                        .changed();

                    ui.add_space(8.0);
                    ui.label(RichText::new("Colours").strong());
                    for (name, c) in [
                        ("hair", &mut p.hair_color),
                        ("skin", &mut p.skin_color),
                        ("eyes", &mut p.eye_color),
                        ("shirt", &mut p.shirt_color),
                        ("undershirt", &mut p.under_color),
                        ("pants", &mut p.pants_color),
                        ("shoes", &mut p.shoes_color),
                    ] {
                        ui.horizontal(|ui| {
                            let mut rgb = [c.r, c.g, c.b];
                            if ui.color_edit_button_srgb(&mut rgb).changed() {
                                c.r = rgb[0];
                                c.g = rgb[1];
                                c.b = rgb[2];
                                changed = true;
                            }
                            ui.label(name);
                        });
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

                    if changed {
                        self.dirty = true;
                    }
                });
            });
    }

    // ------------------------------------------------------------- right side

    fn side_right(&mut self, ui: &mut egui::Ui) {
        egui::Panel::right("browser")
            .resizable(true)
            .default_size(290.0)
            .show(ui, |ui| {
                ui.add_space(4.0);
                ui.heading("Items");
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut self.search)
                            .hint_text("search…  name, phrase or id")
                            .desired_width(f32::INFINITY),
                    )
                    .changed()
                {
                    self.rebuild_catalog();
                }

                ui.horizontal_wrapped(|ui| {
                    for (name, mask) in [
                        ("All", 0u32),
                        ("Weapons", catalog::mask_weapons()),
                        ("Armor", catalog::mask_armour()),
                        ("Accessories", catalog::mask_accessory()),
                        ("Blocks", catalog::mask_blocks() | catalog::mask_furniture()),
                        ("Consumables", catalog::mask_consumable()),
                        ("Materials", catalog::mask_material()),
                    ] {
                        if ui.selectable_label(self.cat == mask, name).clicked() {
                            self.cat = mask;
                            self.rebuild_catalog();
                        }
                    }
                });
                ui.separator();

                let rows = std::mem::take(&mut self.catalog.rows);
                let total = rows.len();
                ui.label(
                    RichText::new(format!("{} items", self.catalog.item_count))
                        .weak()
                        .small(),
                );
                egui::ScrollArea::vertical()
                    .id_salt("itemscroll")
                    .show_rows(ui, 24.0, total, |ui, range| {
                        for idx in range {
                            match rows[idx] {
                                Row::Header(text) => self.header_row(ui, text),
                                Row::Item(id) => self.item_row(ui, id),
                            }
                        }
                    });
                self.catalog.rows = rows;
                ui.separator();
                self.browser_footer(ui);
            });
    }

    /// A section heading inside the item list.
    fn header_row(&self, ui: &mut egui::Ui, text: &str) {
        let (rect, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), Sense::hover());
        ui.painter()
            .rect_filled(rect, 0.0, Color32::from_rgb(36, 40, 48));
        ui.painter().text(
            rect.min + Vec2::new(6.0, 12.0),
            egui::Align2::LEFT_CENTER,
            text.to_uppercase(),
            egui::FontId::proportional(11.0),
            Color32::from_rgb(150, 165, 190),
        );
    }

    /// One item row: drag source and click-to-place target.
    fn item_row(&mut self, ui: &mut egui::Ui, id: i32) {
        let Some(item) = self.db.by_id(id) else {
            return;
        };
        let (rect, resp) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), 24.0),
            Sense::click_and_drag(),
        );
        let hovered = resp.hovered();
        if hovered {
            ui.painter()
                .rect_filled(rect, 2.0, Color32::from_rgb(42, 46, 56));
        }
        let icon = egui::Rect::from_min_size(rect.min + Vec2::new(2.0, 2.0), Vec2::splat(20.0));
        ui.painter()
            .rect_filled(icon, 2.0, Color32::from_rgb(34, 37, 46));
        ui.painter().rect_filled(
            egui::Rect::from_min_size(icon.min, Vec2::new(2.0, icon.height())),
            1.0,
            rarity_color(item.rarity),
        );
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
            egui::FontId::proportional(9.0),
            rarity_color(item.rarity),
        );
        ui.painter().text(
            rect.min + Vec2::new(26.0, 12.0),
            egui::Align2::LEFT_CENTER,
            &item.name,
            egui::FontId::proportional(13.0),
            Color32::from_gray(220),
        );
        ui.painter().text(
            rect.max - Vec2::new(4.0, 12.0),
            egui::Align2::RIGHT_CENTER,
            format!("#{}", item.id),
            egui::FontId::monospace(10.0),
            Color32::from_gray(110),
        );

        if hovered {
            self.hovered_item = Some(id);
        }

        if resp.drag_started() {
            let item_slot = Slot::new(id, self.db.max_stack(id), 0);
            self.drag = Some(Drag {
                from: None,
                item: item_slot,
            });
        }

        if resp.clicked() {
            let item_slot = Slot::new(id, self.db.max_stack(id), 0);
            match self.selected {
                Some(target) => self.place(target, item_slot),
                None => {
                    self.set_status(
                        format!(
                            "Holding {}. Click a slot to place it.",
                            self.db.display_name(id)
                        ),
                        false,
                    );
                    self.drag = Some(Drag {
                        from: None,
                        item: item_slot,
                    });
                }
            }
        }
    }

    fn browser_footer(&mut self, ui: &mut egui::Ui) {
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
                bits.push(format!("max stack {}", item.max_stack));
                if item.estimated {
                    bits.push("stats estimated (newer than the snapshot)".into());
                }
                ui.label(RichText::new(bits.join(" · ")).weak().small());
                return;
            }
        }
        if let Some(d) = &self.drag {
            ui.label(
                RichText::new(format!("Holding: {}", self.describe(&d.item)))
                    .color(Color32::from_rgb(205, 195, 130)),
            );
            ui.label(
                RichText::new("Drop it on a slot, or right-click to cancel.")
                    .weak()
                    .small(),
            );
        } else {
            ui.label(
                RichText::new(
                    "Drag an item onto a slot, or click a slot first and then click an item.",
                )
                .weak()
                .small(),
            );
        }
    }

    // ---------------------------------------------------------------- centre

    fn central(&mut self, ui0: &mut egui::Ui) {
        egui::CentralPanel::default().show(ui0, |ui| {
            if self.player.is_none() {
                ui.add_space(16.0);
                ui.label(RichText::new("No character open — use Open… in the top bar.").weak());
                return;
            }

            ui.horizontal_wrapped(|ui| {
                for (t, name) in [
                    (Tab::Inventory, "Inventory & equipment"),
                    (Tab::Banks, "Banks"),
                    (Tab::Buffs, "Buffs"),
                    (Tab::Journey, "Journey"),
                    (Tab::Raw, "Header"),
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
                    Tab::Journey => self.tab_journey(ui),
                    Tab::Raw => self.tab_raw(ui),
                });
        });
    }

    fn tab_inventory(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Loadout");
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
        let (items, social, dyes) = if lo == 0 {
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
        self.section(ui, "Armour & accessories", items);
        self.section(ui, "Vanity", social);
        self.section(ui, "Dyes", dyes);
        self.section(ui, "Main inventory", Ct::Bag(Bag::Inv));
        ui.horizontal(|ui| {
            ui.vertical(|ui| self.section(ui, "Coins", Ct::Bag(Bag::Coins)));
            ui.vertical(|ui| self.section(ui, "Ammo", Ct::Bag(Bag::Ammo)));
        });
        self.section(
            ui,
            "Pets · carts · mounts · hooks",
            Ct::Bag(Bag::EquipItems),
        );
        self.section(ui, "Misc dyes", Ct::Bag(Bag::EquipDyes));
    }

    fn tab_banks(&mut self, ui: &mut egui::Ui) {
        for (label, bag) in [
            ("Piggy bank", Bag::Bank),
            ("Safe", Bag::Safe),
            ("Defender's forge", Bag::Forge),
            ("Void vault", Bag::Void),
        ] {
            self.section(ui, label, Ct::Bag(bag));
        }
    }

    fn tab_buffs(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Buffs (id / duration in ticks). 44 slots on disk.").weak());
        let mut clear: Option<usize> = None;
        let count = self.player.as_ref().map(|p| p.buffs.len()).unwrap_or(0);
        for i in 0..count {
            let Some(p) = &mut self.player else { break };
            let b = &mut p.buffs[i];
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("#{i:02}")).weak());
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
                ui.add_sized(
                    [150.0, 18.0],
                    egui::Label::new(RichText::new(name).weak()).truncate(),
                );
                if ui
                    .add(egui::DragValue::new(&mut b.id).range(0..=400))
                    .changed()
                {
                    self.dirty = true;
                }
                if ui
                    .add(
                        egui::DragValue::new(&mut b.time)
                            .speed(60)
                            .range(0..=1_000_000),
                    )
                    .changed()
                {
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

    fn tab_journey(&mut self, ui: &mut egui::Ui) {
        let Some(p) = &self.player else { return };
        ui.label(format!("{} entries researched", p.research.len()));
        ui.separator();
        egui::ScrollArea::vertical()
            .max_height(460.0)
            .show(ui, |ui| {
                for (pid, count) in &p.research {
                    match self.db.by_pid(pid) {
                        Some(item) => {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&item.name).strong());
                                ui.label(RichText::new(format!("#{}", item.id)).weak());
                                ui.label(format!("×{count}"));
                            });
                        }
                        None => {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(pid).weak());
                                ui.label(format!("×{count}"));
                            });
                        }
                    }
                }
            });
    }

    fn tab_raw(&mut self, ui: &mut egui::Ui) {
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

    // ------------------------------------------------------------ slot grid

    /// Draw a titled block of slots.
    fn section(&mut self, ui: &mut egui::Ui, title: &str, ct: Ct) {
        let n = self
            .player
            .as_ref()
            .map(|p| container(p, ct).len())
            .unwrap_or(0);
        if n == 0 {
            return;
        }
        ui.add_space(6.0);
        ui.label(RichText::new(title).strong());
        // The game lays inventory out 10 wide. Keeping that shape matters, so the
        // first choice is to shrink the cells to fit; only if they would get too
        // small to read do we wrap into fewer columns instead.
        let gap = 6.0;
        let avail = ui.available_width();
        let wanted_cols = 10usize.min(n);
        let preferred = self.settings.cell_size;
        let fit_for_ten = ((avail + gap) / wanted_cols as f32 - gap).floor();
        let (cell, per_row) = if fit_for_ten >= 24.0 {
            (preferred.min(fit_for_ten), wanted_cols)
        } else {
            let cols = ((avail + gap) / (preferred + gap)).floor();
            (preferred, (cols as usize).clamp(1, wanted_cols))
        };
        let mut i = 0;
        while i < n {
            ui.horizontal(|ui| {
                for _ in 0..per_row {
                    if i >= n {
                        break;
                    }
                    self.slot(ui, SlotRef { c: ct, i }, cell);
                    i += 1;
                }
            });
        }
    }

    /// One inventory cell: paints the item, handles select/drag/scroll/context menu.
    fn slot(&mut self, ui: &mut egui::Ui, r: SlotRef, cell: f32) {
        let Some(p) = &self.player else { return };
        let slot = *container(p, r.c).get(r.i).unwrap_or(&Slot::empty());
        let multi = is_multi(r.c);
        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(cell), Sense::click_and_drag());
        let painter = ui.painter_at(rect);

        let border = if self.selected == Some(r) {
            Stroke::new(2.0, Color32::from_rgb(240, 200, 90))
        } else if resp.hovered() && self.drag.is_some() {
            Stroke::new(2.0, Color32::from_rgb(150, 200, 255))
        } else {
            Stroke::new(1.0, Color32::from_rgb(64, 70, 84))
        };
        painter.rect_filled(rect.shrink(1.0), 3.0, Color32::from_rgb(34, 37, 46));
        painter.rect_stroke(rect.shrink(1.0), 3.0, border, egui::StrokeKind::Inside);

        if !slot.is_empty() {
            let rarity = self.db.by_id(slot.net_id).map(|i| i.rarity).unwrap_or(0);
            painter.rect_filled(
                rect.shrink(4.0),
                2.0,
                rarity_color(rarity).linear_multiply(0.85),
            );
            let initials: String = self
                .db
                .display_name(slot.net_id)
                .split_whitespace()
                .take(2)
                .map(|w| w.chars().next().unwrap_or('?'))
                .collect();
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                initials,
                egui::FontId::proportional(cell * 0.30),
                Color32::from_black_alpha(200),
            );
            if slot.stack > 1 && multi {
                painter.text(
                    rect.max - Vec2::new(3.0, 2.0),
                    egui::Align2::RIGHT_BOTTOM,
                    slot.stack.to_string(),
                    egui::FontId::proportional(cell * 0.24),
                    Color32::WHITE,
                );
            }
            if slot.favorited {
                painter.text(
                    rect.min + Vec2::new(3.0, 1.0),
                    egui::Align2::LEFT_TOP,
                    "★",
                    egui::FontId::proportional(cell * 0.22),
                    Color32::from_rgb(250, 220, 120),
                );
            }
        }

        if resp.hovered() {
            self.hovered_item = if slot.is_empty() {
                None
            } else {
                Some(slot.net_id)
            };
            let tip = format!(
                "{}  [{} #{}]",
                self.describe(&slot),
                container_label(r.c),
                r.i
            );
            resp.clone().on_hover_text(tip);
        }

        // Wheel over a slot edits the stack, like the game does with right-click.
        if resp.hovered() && multi && !slot.is_empty() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.1 {
                let step = if ui.input(|i| i.modifiers.shift) {
                    100
                } else {
                    1
                };
                let delta = if scroll > 0.0 { step } else { -step };
                if let Some(p) = &mut self.player {
                    if operations::bump_stack(self.db, p, r, delta) {
                        self.dirty = true;
                    }
                }
            }
        }

        if resp.drag_started() && !slot.is_empty() {
            self.drag = Some(Drag {
                from: Some(r),
                item: slot,
            });
        }
        if resp.dragged() {
            // repaint the source as "in flight" so the user sees what moved
            painter.rect_stroke(
                rect.shrink(1.0),
                3.0,
                Stroke::new(1.0, Color32::from_rgb(120, 130, 150)),
                egui::StrokeKind::Inside,
            );
        }

        // Dropping: the slot under the cursor wins on release.
        if let Some(drag) = self.drag {
            if resp.hovered() && ui.input(|i| i.pointer.any_released()) {
                self.drop_on(r, drag);
                self.drag = None;
                self.selected = Some(r);
            }
        }

        if resp.clicked() {
            self.selected = Some(r);
            if let Some(drag) = self.drag.take() {
                self.drop_on(r, drag);
            }
        }

        // Right-click: the explicit route, for people who prefer menus.
        let mut action: Option<MenuAction> = None;
        resp.context_menu(|ui| {
            ui.label(RichText::new(self.describe(&slot)).strong());
            ui.separator();
            let has = !slot.is_empty();
            if ui
                .add_enabled(has, egui::Button::new("Pick up (drag)"))
                .clicked()
            {
                action = Some(MenuAction::Pick);
                ui.close();
            }
            if ui
                .add_enabled(has, egui::Button::new("Duplicate"))
                .clicked()
            {
                action = Some(MenuAction::Duplicate);
                ui.close();
            }
            if ui
                .add_enabled(has && multi, egui::Button::new("Max stack"))
                .clicked()
            {
                action = Some(MenuAction::MaxStack);
                ui.close();
            }
            if ui
                .add_enabled(has, egui::Button::new("Toggle favourite"))
                .clicked()
            {
                action = Some(MenuAction::Favourite);
                ui.close();
            }
            if ui
                .add_enabled(has, egui::Button::new("Clear slot"))
                .clicked()
            {
                action = Some(MenuAction::Clear);
                ui.close();
            }
        });

        match action {
            Some(MenuAction::Pick) => {
                self.drag = Some(Drag {
                    from: Some(r),
                    item: slot,
                })
            }
            Some(MenuAction::Duplicate) => {
                self.drag = Some(Drag {
                    from: None,
                    item: slot,
                })
            }
            Some(MenuAction::MaxStack) => {
                if let Some(p) = &mut self.player {
                    if operations::max_stack(self.db, p, r) {
                        self.dirty = true;
                    }
                }
            }
            Some(MenuAction::Favourite) => {
                if let Some(p) = &mut self.player {
                    if operations::toggle_favourite(p, r) {
                        self.dirty = true;
                    }
                }
            }
            Some(MenuAction::Clear) => self.clear_slot(r),
            None => {}
        }

        if resp.middle_clicked() && !slot.is_empty() {
            self.drag = Some(Drag {
                from: None,
                item: slot,
            });
        }
    }

    /// Paint the item being dragged under the pointer.
    fn drag_ghost(&self, ctx: &egui::Context) {
        let Some(drag) = &self.drag else { return };
        let Some(pos) = ctx.pointer_interact_pos() else {
            return;
        };
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            egui::Id::unique("tforge-drag-ghost"),
        ));
        let size = self.settings.cell_size;
        let rect = egui::Rect::from_center_size(pos + Vec2::new(12.0, 12.0), Vec2::splat(size));
        let rarity = self
            .db
            .by_id(drag.item.net_id)
            .map(|i| i.rarity)
            .unwrap_or(0);
        painter.rect_filled(rect, 4.0, rarity_color(rarity).linear_multiply(0.9));
        painter.rect_stroke(
            rect,
            4.0,
            Stroke::new(1.0, Color32::from_white_alpha(160)),
            egui::StrokeKind::Inside,
        );
        let initials: String = self
            .db
            .display_name(drag.item.net_id)
            .split_whitespace()
            .take(2)
            .map(|w| w.chars().next().unwrap_or('?'))
            .collect();
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            initials,
            egui::FontId::proportional(size * 0.3),
            Color32::from_black_alpha(210),
        );
    }

    // --------------------------------------------------------------- dialogs

    /// The "open character" dialog: names, grouped by where they live.
    fn picker_window(&mut self, ctx: &egui::Context) {
        if !self.open_picker {
            return;
        }
        let mut stay_open = self.open_picker;
        let mut pick: Option<PathBuf> = None;

        egui::Window::new("Open character")
            .open(&mut stay_open)
            .default_width(420.0)
            .min_width(360.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(format!("{} characters found", self.players.len()))
                        .weak()
                        .small(),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut self.picker_filter)
                        .hint_text("filter…")
                        .desired_width(f32::INFINITY),
                );
                let filter = self.picker_filter.to_lowercase();

                egui::ScrollArea::vertical()
                    .max_height(360.0)
                    .show(ui, |ui| {
                        let mut last_group: Option<String> = None;
                        for entry in &self.players {
                            let group = group_of(&entry.path);
                            if last_group.as_deref() != Some(group.as_str()) {
                                ui.add_space(4.0);
                                ui.label(RichText::new(group.to_uppercase()).weak().small());
                                last_group = Some(group);
                            }
                            if !filter.is_empty() && !entry.name.to_lowercase().contains(&filter) {
                                continue;
                            }
                            // Name only, with the path in the tooltip — no wrapping.
                            let resp = ui.add_sized(
                                [ui.available_width(), 22.0],
                                egui::Button::new(RichText::new(&entry.name).strong())
                                    .selected(false)
                                    .frame(false),
                            );
                            let resp = resp.on_hover_text(entry.path.display().to_string());
                            ui.horizontal(|ui| {
                                ui.add_space(4.0);
                                if let Some(err) = &entry.error {
                                    ui.label(
                                        RichText::new(err)
                                            .small()
                                            .color(Color32::from_rgb(220, 120, 120)),
                                    );
                                } else {
                                    ui.label(RichText::new(&entry.detail).weak().small());
                                }
                            });
                            if resp.clicked() {
                                pick = Some(entry.path.clone());
                            }
                        }
                    });

                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Path");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.manual_path)
                            .hint_text("/path/to/player.plr")
                            .desired_width(f32::INFINITY),
                    );
                });
                ui.horizontal(|ui| {
                    if ui.button("Open path").clicked() && !self.manual_path.trim().is_empty() {
                        pick = Some(PathBuf::from(self.manual_path.trim()));
                    }
                    if ui.button("Rescan").clicked() {
                        self.rescan();
                    }
                });
            });

        if let Some(path) = pick {
            self.open(path);
            stay_open = false;
        }
        self.open_picker = stay_open;
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let mut open = self.show_settings;
        let mut changed = false;
        egui::Window::new("Display")
            .open(&mut open)
            .default_width(320.0)
            .show(ctx, |ui| {
                ui.label("Text and slot size scale together.");
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label("Zoom");
                    changed |= ui
                        .add(egui::Slider::new(&mut self.settings.zoom, 0.8..=2.0).step_by(0.05))
                        .changed();
                });
                ui.horizontal(|ui| {
                    ui.label("Slot size");
                    changed |= ui
                        .add(
                            egui::Slider::new(&mut self.settings.cell_size, 36.0..=72.0)
                                .suffix(" px"),
                        )
                        .changed();
                });
                ui.horizontal(|ui| {
                    if ui.button("Reset").clicked() {
                        self.settings = UiSettings::default();
                        changed = true;
                    }
                    ui.label(
                        RichText::new("Saved to ~/.config/terra-forge/settings.json")
                            .weak()
                            .small(),
                    );
                });
            });
        if changed {
            self.settings = self.settings.clone().clamped();
            ctx.set_zoom_factor(self.settings.zoom);
            self.settings.save();
        }
        self.show_settings = open;
    }
}

#[derive(Clone, Copy)]
enum MenuAction {
    Pick,
    Duplicate,
    MaxStack,
    Favourite,
    Clear,
}

/// Friendly heading for a player's directory.
fn group_of(path: &std::path::Path) -> String {
    let dir = path
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    if dir.contains("tModLoader") {
        return "tModLoader".to_string();
    }
    if dir.contains("userdata") {
        // .../steam/userdata/<account>/105600/remote/players
        let account = dir
            .split("userdata/")
            .nth(1)
            .and_then(|rest| rest.split('/').next())
            .unwrap_or("?");
        return format!("Steam Cloud · account {account}");
    }
    "Local".to_string()
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
