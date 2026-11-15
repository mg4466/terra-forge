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
///
/// `side` records *which character* the source slot belongs to, so the same
/// `Drag` can carry an item from the stash to the hero or the other way round.
#[derive(Clone, Copy)]
struct Drag {
    side: Side,
    from: Option<SlotRef>,
    item: Slot,
}

/// Which character a slot belongs to. The stash is a second, fully editable
/// `PlayerFile`; without this tag a slot reference would be ambiguous.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    Primary,
    Stash,
}

/// A second character kept open purely as a source/sink for items.
struct Stash {
    path: PathBuf,
    player: terra_core::PlayerFile,
    dirty: bool,
}

/// A texture drawn on screen, plus the part of it that is one animation frame.
#[derive(Clone)]
struct SpriteTex {
    handle: egui::TextureHandle,
    /// Full texture for still art; the top frame for a vertical strip.
    uv: egui::Rect,
}

impl SpriteTex {
    /// Size of one frame in pixels (not the whole strip).
    fn frame_size(&self) -> Vec2 {
        let size = self.handle.size_vec2();
        Vec2::new(size.x, size.y * self.uv.height())
    }
}

/// Lazily decoded item sprites, held as GPU textures in a small LRU cache.
///
/// Decoding a single `.xnb` costs about 0.05 ms (see `terra_assets::sprites`), so
/// there is no pre-pass: a sprite is decoded the first time a slot showing it is
/// drawn on screen. Only a few hundred are ever live at once, and misses are
/// cached too so a missing file is not re-read every frame.
///
/// The eviction order is true LRU: hits are promoted to the back, so browsing
/// the 6,196-item catalogue cannot evict the sprites of the gear currently on
/// screen. A plain insertion-order queue would do exactly that.
struct SpriteCache {
    /// `None` means "looked for it, there is no sprite for this id".
    textures: std::collections::HashMap<i32, Option<SpriteTex>>,
    order: std::collections::VecDeque<i32>,
    cap: usize,
}

impl SpriteCache {
    fn new(cap: usize) -> Self {
        Self {
            textures: Default::default(),
            order: Default::default(),
            cap,
        }
    }

    fn get(&mut self, ctx: &egui::Context, id: i32) -> Option<SpriteTex> {
        if let Some(hit) = self.textures.get(&id) {
            let hit = hit.clone();
            self.promote(id);
            return hit;
        }
        let entry = match terra_assets::sprites::load_item_sprite(id) {
            Ok(Some(sprite)) => {
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [sprite.width as usize, sprite.height as usize],
                    &sprite.rgba,
                );
                // Nearest filtering: pixel art must not be blurred.
                let handle =
                    ctx.load_texture(format!("item-{id}"), image, egui::TextureOptions::NEAREST);
                // Animated art is a vertical strip; draw only its first frame,
                // otherwise a four-frame soul is rendered at quarter size.
                let frame_frac = sprite.frame_height() as f32 / sprite.height.max(1) as f32;
                Some(SpriteTex {
                    handle,
                    uv: egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, frame_frac)),
                })
            }
            _ => None,
        };
        self.textures.insert(id, entry.clone());
        self.order.push_back(id);
        self.evict();
        entry
    }

    /// Move a used id to the back of the eviction queue.
    fn promote(&mut self, id: i32) {
        if let Some(pos) = self.order.iter().position(|&x| x == id) {
            self.order.remove(pos);
        }
        self.order.push_back(id);
    }

    fn evict(&mut self) {
        while self.order.len() > self.cap {
            if let Some(old) = self.order.pop_front() {
                self.textures.remove(&old);
            }
        }
    }
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
    /// Which character `selected` points into.
    selected_side: Side,

    /// In-progress drag-and-drop, if any.
    drag: Option<Drag>,

    search: String,
    cat: u32,
    catalog: Catalog,

    open_picker: bool,
    /// When set, the picker loads a stash character instead of replacing the hero.
    picker_for_stash: bool,
    picker_filter: String,
    manual_path: String,
    players: Vec<PlayerEntry>,

    /// Optional second character for transferring items between saves.
    stash: Option<Stash>,

    prefix_filter: String,
    journey_filter: String,

    show_settings: bool,
    hovered_item: Option<i32>,
    sprites: SpriteCache,
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
            selected_side: Side::Primary,
            drag: None,
            search: String::new(),
            cat: 0,
            catalog: Catalog::default(),
            open_picker: true,
            picker_for_stash: false,
            picker_filter: String::new(),
            manual_path: String::new(),
            players: Vec::new(),
            stash: None,
            prefix_filter: String::new(),
            journey_filter: String::new(),
            show_settings: false,
            hovered_item: None,
            sprites: SpriteCache::new(2500),
        };
        app.rebuild_catalog();
        app.rescan();

        if let Some(arg) = std::env::args().nth(1) {
            app.open(PathBuf::from(arg));
            // Opening straight into a character should not leave the picker on top.
            app.open_picker = app.player.is_none();
        }
        // Screenshot/manual-check hook: open a second character immediately so
        // the split stash view can be reached without a mouse.
        if let Some(stash) = std::env::var_os("TF_STASH") {
            app.open_stash(PathBuf::from(stash));
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
                self.selected_side = Side::Primary;
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

    /// Load the optional second character. It is deliberately not discovered
    /// automatically: silently opening somebody else's character as a stash
    /// would be a surprising thing for a file editor to do.
    fn open_stash(&mut self, path: PathBuf) {
        match terra_core::load(&path) {
            Ok(player) => {
                let name = player.name.clone();
                self.stash = Some(Stash {
                    path,
                    player,
                    dirty: false,
                });
                self.set_status(format!("Stash character: {name}"), false);
                self.tab = Tab::Inventory;
            }
            Err(e) => self.set_status(format!("Could not open stash: {e}"), true),
        }
    }

    fn save_stash(&mut self) {
        let Some(mut stash) = self.stash.take() else {
            return;
        };
        match terra_core::save_with_backup(&stash.path, &stash.player) {
            Ok(_) => {
                stash.dirty = false;
                let name = stash.player.name.clone();
                self.stash = Some(stash);
                self.set_status(format!("Stash saved: {name}"), false);
            }
            Err(e) => {
                self.stash = Some(stash);
                self.set_status(format!("Stash save failed: {e}"), true);
            }
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
    ///
    /// When the drag started on one character and ends on the other, the two
    /// models are handed to `operations::transfer_item` together.
    fn drop_on(&mut self, side: Side, to: SlotRef, drag: Drag) {
        let db = self.db;
        match (drag.side, side) {
            (Side::Primary, Side::Primary) => {
                if let Some(p) = self.player.as_mut() {
                    operations::drop_item(db, p, to, drag.from, drag.item);
                    self.dirty = true;
                }
            }
            (Side::Stash, Side::Stash) => {
                if let Some(s) = self.stash.as_mut() {
                    operations::drop_item(db, &mut s.player, to, drag.from, drag.item);
                    s.dirty = true;
                }
            }
            (Side::Primary, Side::Stash) => {
                if let (Some(p), Some(s)) = (self.player.as_mut(), self.stash.as_mut()) {
                    operations::transfer_item(db, p, &mut s.player, to, drag.from, drag.item);
                    s.dirty = true;
                }
            }
            (Side::Stash, Side::Primary) => {
                if let (Some(s), Some(p)) = (self.stash.as_mut(), self.player.as_mut()) {
                    operations::transfer_item(db, &mut s.player, p, to, drag.from, drag.item);
                    self.dirty = true;
                }
            }
        }
        self.selected = Some(to);
        self.selected_side = side;
    }

    /// Put an item into a slot without a source slot (browser click / duplicate).
    fn place(&mut self, to: SlotRef, item: Slot) {
        self.drop_on(
            self.selected_side,
            to,
            Drag {
                side: self.selected_side,
                from: None,
                item,
            },
        );
    }

    fn clear_slot(&mut self, side: Side, r: SlotRef) {
        if let Some(p) = self.player_mut(side) {
            operations::clear(p, r);
            self.mark_dirty(side);
        }
    }

    fn player_of(&self, side: Side) -> Option<&terra_core::PlayerFile> {
        match side {
            Side::Primary => self.player.as_ref(),
            Side::Stash => self.stash.as_ref().map(|s| &s.player),
        }
    }

    fn player_mut(&mut self, side: Side) -> Option<&mut terra_core::PlayerFile> {
        match side {
            Side::Primary => self.player.as_mut(),
            Side::Stash => self.stash.as_mut().map(|s| &mut s.player),
        }
    }

    fn is_dirty(&self, side: Side) -> bool {
        match side {
            Side::Primary => self.dirty,
            Side::Stash => self.stash.as_ref().map(|s| s.dirty).unwrap_or(false),
        }
    }

    fn mark_dirty(&mut self, side: Side) {
        match side {
            Side::Primary => self.dirty = true,
            Side::Stash => {
                if let Some(s) = self.stash.as_mut() {
                    s.dirty = true;
                }
            }
        }
    }

    fn describe(&self, slot: &Slot, ct: Option<Ct>) -> String {
        if slot.is_empty() {
            return "empty".into();
        }
        let mut s = format!("#{} {}", slot.net_id, self.db.display_name(slot.net_id));
        if let Some(p) = self.db.prefix_name(slot.prefix) {
            s.push_str(&format!(" · {p}"));
        }
        // Only containers that actually store a stack get a ×N suffix; a piece
        // of armour dragged from the bank into an equipment slot must not claim
        // to be a stack of twelve.
        if slot.stack > 1 && ct.map(is_multi).unwrap_or(true) {
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
        let ctrl = ctx.input(|i| i.modifiers.ctrl || i.modifiers.command);

        if ctrl && ctx.input(|i| i.key_pressed(egui::Key::S)) {
            self.save();
        }
        // Ctrl+O re-scans before opening: the character may have been created
        // since the last scan, and that is exactly when someone reaches for it.
        if ctrl && ctx.input(|i| i.key_pressed(egui::Key::O)) {
            self.rescan();
            self.picker_for_stash = false;
            self.open_picker = true;
        }
        // Ctrl+F focuses the item search, but only when no text box already has
        // focus — otherwise Ctrl+F in the path field would steal the caret.
        if ctrl
            && ctx.input(|i| i.key_pressed(egui::Key::F))
            && ctx.memory(|m| m.focused()).is_none()
        {
            ctx.memory_mut(|m| m.request_focus(egui::Id::unique("item-search")));
        }
        // Escape and right-click cancel a drag, matching the footer's promise.
        // This happens before the slots are drawn so the release that ends a
        // real drop is not mistaken for a cancel (that is why the generic
        // "released anywhere" cleanup runs at the *end* of the frame).
        if self.drag.is_some()
            && (ctx.input(|i| i.key_pressed(egui::Key::Escape))
                || ctx.input(|i| i.pointer.secondary_clicked()))
        {
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

        // A drag that ends over nothing is simply cancelled. Deliberately last:
        // slots get first refusal on the release, then whatever is left over is
        // a drop on empty space.
        if self.drag.is_some() && ctx.input(|i| i.pointer.any_released()) {
            self.drag = None;
        }
    }
}

impl App {
    fn top_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("top").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.heading(RichText::new("terra-forge").strong());
                ui.separator();
                if ui
                    .button("Open…")
                    .on_hover_text("Choose a character (Ctrl+O)")
                    .clicked()
                {
                    self.rescan();
                    self.picker_for_stash = false;
                    self.open_picker = true;
                }
                if ui
                    .button("Stash…")
                    .on_hover_text(
                        "Open a second character to move items between saves.\n\
                         Only you choose the file; nothing is opened automatically.",
                    )
                    .clicked()
                {
                    self.rescan();
                    self.picker_for_stash = true;
                    self.open_picker = true;
                }
                if let Some((stash_name, stash_dirty)) = self
                    .stash
                    .as_ref()
                    .map(|s| (s.player.name.clone(), s.dirty))
                {
                    let label = if stash_dirty {
                        "Save stash *"
                    } else {
                        "Save stash"
                    };
                    let save_clicked = ui
                        .add(egui::Button::new(label))
                        .on_hover_text("Write the stash character back to its own file")
                        .clicked();
                    let close_clicked = ui.button("Close stash").clicked();
                    ui.label(RichText::new(format!("stash: {stash_name}")).weak().small());
                    if save_clicked {
                        self.save_stash();
                    }
                    if close_clicked {
                        self.stash = None;
                        if self.selected_side == Side::Stash {
                            self.selected_side = Side::Primary;
                            self.selected = Some(SlotRef {
                                c: Ct::Bag(Bag::Inv),
                                i: 0,
                            });
                        }
                    }
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

                if let Some(p) = &self.player {
                    ui.separator();
                    ui.label(
                        RichText::new(format!(
                            "{} · items {}",
                            p.format_name(),
                            self.db.game_version
                        ))
                        .weak(),
                    );
                    if p.version > terra_core::plr::VER_1_4_5_8 {
                        ui.label(
                            RichText::new(format!(
                                "newer save (v{}) · unknown fields kept as-is",
                                p.version
                            ))
                            .color(Color32::from_rgb(232, 184, 96)),
                        )
                        .on_hover_text(
                            "This file comes from a newer game build than terra-forge was \
                             written for. Everything it does not understand is preserved \
                             verbatim on save, but new fields will not be editable here.",
                        );
                    }
                }
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
                    // The six 1.4.4 permanent boosters, in the order the file
                    // stores them (see `PlayerFile::extra_using_flags`). The
                    // first one's internal name is `AegisCrystal` but the item
                    // the player actually consumes is the Vital Crystal.
                    if p.version >= terra_core::plr::VER_1_4_4_0 {
                        for (flag, name) in p.extra_using_flags.iter_mut().zip([
                            "Vital Crystal (life regen)",
                            "Aegis Fruit (defence)",
                            "Arcane Crystal (mana regen)",
                            "Galaxy Pearl (luck)",
                            "Gummy Worm (fishing)",
                            "Ambrosia (mining speed)",
                        ]) {
                            changed |= ui.checkbox(flag, name).changed();
                        }
                    }

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
                            .id(egui::Id::unique("item-search"))
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

                // The inspector and footer are a fixed strip at the bottom; the
                // catalogue scrolls in whatever space is left. Declaring the
                // bottom panel first is what makes that reservation work.
                egui::Panel::bottom("browser-bottom").show(ui, |ui| {
                    self.inspector(ui);
                    self.browser_footer(ui);
                });

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
            });
    }

    /// Editable view of the currently selected slot: prefix, stack, favourite.
    ///
    /// This exists because a slot cell can only show a picture and a number.
    /// Prefixes have 84 legal values, and choosing one from a grid would be
    /// absurd; the inspector is the honest place for it.
    fn inspector(&mut self, ui: &mut egui::Ui) {
        let Some(r) = self.selected else { return };
        let side = self.selected_side;
        let Some(slot) = self
            .player_of(side)
            .and_then(|p| container(p, r.c).get(r.i).copied())
        else {
            return;
        };
        if slot.is_empty() {
            return;
        }
        let db = self.db;
        ui.separator();
        ui.label(RichText::new("Selected slot").strong());
        ui.label(
            RichText::new(format!(
                "{}  [#{} · {} #{}]",
                db.display_name(slot.net_id),
                slot.net_id,
                container_label(r.c),
                r.i
            ))
            .weak()
            .small(),
        );

        // Prefix: a filtered combo box. The filter field lives in `App` because
        // egui keeps no widget state of its own between frames.
        let current = if slot.prefix == 0 {
            "None (0)".to_string()
        } else {
            match db.prefix_name(slot.prefix) {
                Some(name) => format!("#{} · {name}", slot.prefix),
                None => format!("#{} · unknown", slot.prefix),
            }
        };
        ui.horizontal(|ui| {
            ui.label("Prefix");
            egui::ComboBox::from_id_salt("prefix-combo")
                .selected_text(current)
                .width(190.0)
                .show_ui(ui, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.prefix_filter)
                            .hint_text("filter prefixes…")
                            .desired_width(180.0),
                    );
                    let filter = self.prefix_filter.to_lowercase();
                    let mut chosen: Option<u8> = None;
                    if ui.selectable_label(slot.prefix == 0, "None (0)").clicked() {
                        chosen = Some(0);
                    }
                    for prefix in &self.db.prefixes {
                        if !filter.is_empty() && !prefix.name.to_lowercase().contains(&filter) {
                            continue;
                        }
                        if ui
                            .selectable_label(
                                slot.prefix == prefix.id,
                                format!("#{} · {}", prefix.id, prefix.name),
                            )
                            .clicked()
                        {
                            chosen = Some(prefix.id);
                        }
                    }
                    if let Some(prefix) = chosen {
                        if let Some(p) = self.player_mut(side) {
                            if operations::set_prefix(p, r, prefix) {
                                self.mark_dirty(side);
                            }
                        }
                    }
                });
        });

        if is_multi(r.c) {
            ui.horizontal(|ui| {
                ui.label("Stack");
                let max = db.max_stack(slot.net_id).max(1);
                let mut stack = slot.stack;
                let response = ui.add(
                    egui::DragValue::new(&mut stack)
                        .speed(1)
                        .range(1..=max)
                        .suffix(format!(" / {max}")),
                );
                if response.changed() {
                    if let Some(p) = self.player_mut(side) {
                        if operations::set_stack(db, p, r, stack) {
                            self.mark_dirty(side);
                        }
                    }
                }
            });
        }

        ui.horizontal(|ui| {
            let mut favourite = slot.favorited;
            if ui
                .checkbox(&mut favourite, "Favourite")
                .on_hover_text("The star keeps the item in place when you quick-stack in game")
                .changed()
            {
                if let Some(p) = self.player_mut(side) {
                    if operations::toggle_favourite(p, r) {
                        self.mark_dirty(side);
                    }
                }
            }
            if ui.button("Clear slot").clicked() {
                self.clear_slot(side, r);
            }
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
        let icon = egui::Rect::from_min_size(rect.min + Vec2::new(2.0, 1.0), Vec2::splat(22.0));
        ui.painter()
            .rect_filled(icon, 2.0, Color32::from_rgb(34, 37, 46));
        let sprite = if self.settings.sprites {
            self.sprites.get(ui.ctx(), item.id)
        } else {
            None
        };
        match sprite {
            Some(texture) => {
                // Use one animation frame, not the whole strip, so a 4-frame
                // soul is not shrunk to a quarter of its size.
                let native = texture.frame_size();
                let max_side = native.x.max(native.y);
                let fit = (icon.width() - 2.0) / max_side;
                let scale = if fit >= 2.0 { 2.0 } else { fit.min(1.0) };
                let target = egui::Rect::from_center_size(icon.center(), native * scale);
                ui.painter()
                    .image(texture.handle.id(), target, texture.uv, Color32::WHITE);
            }
            None => {
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
            }
        }
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
                side: self.selected_side,
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
                        side: self.selected_side,
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
                RichText::new(format!("Holding: {}", self.describe(&d.item, None)))
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

            // Journey owns its scroll area: the list is virtualised over 5,000+
            // rows and must fill the remaining height, which a nested ScrollArea
            // inside the generic "main" one cannot do (the outer one gives its
            // children unbounded height).
            if self.tab == Tab::Journey {
                self.tab_journey(ui);
            } else {
                egui::ScrollArea::vertical()
                    .id_salt("main")
                    .show(ui, |ui| match self.tab {
                        Tab::Inventory => self.tab_inventory(ui),
                        Tab::Banks => self.tab_banks(ui),
                        Tab::Buffs => self.tab_buffs(ui),
                        Tab::Journey => unreachable!("handled above"),
                        Tab::Raw => self.tab_raw(ui),
                    });
            }
        });
    }

    fn tab_inventory(&mut self, ui: &mut egui::Ui) {
        if self.stash.is_some() {
            // Two characters, side by side: the whole point is to drag across.
            let width = ((ui.available_width() - 24.0) / 2.0).max(180.0);
            let height = ui.available_height();
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    Vec2::new(width, height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        self.pane_heading(ui, Side::Primary);
                        self.pane_primary(ui);
                    },
                );
                ui.separator();
                ui.allocate_ui_with_layout(
                    Vec2::new(width, height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        self.pane_heading(ui, Side::Stash);
                        self.pane_stash(ui);
                    },
                );
            });
            return;
        }
        self.pane_primary(ui);
    }

    fn pane_heading(&self, ui: &mut egui::Ui, side: Side) {
        let name = self
            .player_of(side)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "—".into());
        let star = if self.is_dirty(side) { " *" } else { "" };
        // Both characters can share a name (Terraria allows it), and a narrow
        // pane truncates whatever is longest. Label the stash pane with its role
        // rather than a name that may not fit.
        let text = if side == Side::Stash {
            format!("Stash{star}")
        } else {
            format!("{name}{star}")
        };
        ui.add(egui::Label::new(RichText::new(text).heading()).truncate());
        ui.separator();
    }

    fn pane_primary(&mut self, ui: &mut egui::Ui) {
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
        self.section(ui, "Armor & accessories", items);
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

    /// The stash pane: inventory, coins/ammo and all four bank containers. Its
    /// equipped armour is deliberately not shown — a stash is somewhere items
    /// go, not a second character being played.
    fn pane_stash(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Drag between panes.").weak().small());
        self.section_side(ui, Side::Stash, "Stash inventory", Ct::Bag(Bag::Inv));
        ui.horizontal(|ui| {
            ui.vertical(|ui| self.section_side(ui, Side::Stash, "Coins", Ct::Bag(Bag::Coins)));
            ui.vertical(|ui| self.section_side(ui, Side::Stash, "Ammo", Ct::Bag(Bag::Ammo)));
        });
        for (label, bag) in [
            ("Piggy bank", Bag::Bank),
            ("Safe", Bag::Safe),
            ("Defender's forge", Bag::Forge),
            ("Void vault", Bag::Void),
        ] {
            self.section_side(ui, Side::Stash, label, Ct::Bag(bag));
        }
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
        let researched = p.research.len();
        let with_goal = self
            .db
            .items
            .iter()
            .filter(|i| !i.pid.is_empty() && i.research_goal > 0)
            .count();
        ui.horizontal_wrapped(|ui| {
            ui.label(format!(
                "{researched} entries researched · {with_goal} items have a research goal"
            ));
            if ui
                .button("Research all")
                .on_hover_text(
                    "Set every item with a known goal to fully researched.\n\
                     Counts already higher than the goal are left alone. Revert undoes it.",
                )
                .clicked()
            {
                if let Some(p) = &mut self.player {
                    let changed = operations::research_all(self.db, p);
                    self.dirty = true;
                    self.set_status(format!("Research all: {changed} entries updated"), false);
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("Filter");
            ui.add(
                egui::TextEdit::singleline(&mut self.journey_filter)
                    .hint_text("name or id")
                    .desired_width(220.0),
            );
        });

        // The catalogue is ~6,200 items; only the visible rows are drawn, so
        // rebuilding this id list each frame keeps the list responsive without
        // a parallel cache to invalidate.
        let filter = self.journey_filter.trim().to_lowercase();
        let ids: Vec<i32> = self
            .db
            .items
            .iter()
            .filter(|i| !i.pid.is_empty() && i.research_goal > 0)
            .filter(|i| {
                filter.is_empty()
                    || i.name.to_lowercase().contains(&filter)
                    || i.id.to_string() == filter
            })
            .map(|i| i.id)
            .collect();
        let total = ids.len();
        // Any edit, from the DragValue or the Complete button, is applied after
        // the list is drawn: the closure below only has immutable access to the
        // player, which keeps the borrow checker happy and the update atomic.
        let mut edit: Option<(String, i32)> = None;
        egui::ScrollArea::vertical()
            .id_salt("journey")
            // Fill the panel instead of shrinking to the widest row: a 300 px
            // list in a 1,200 px pane looks broken and wastes the row space.
            .auto_shrink([false, false])
            .show_rows(ui, 22.0, total, |ui, range| {
                for idx in range {
                    let Some(item) = self.db.by_id(ids[idx]) else {
                        continue;
                    };
                    let current = self
                        .player
                        .as_ref()
                        .and_then(|p| p.research.iter().find(|(k, _)| *k == item.pid))
                        .map(|(_, n)| *n)
                        .unwrap_or(0);
                    ui.horizontal(|ui| {
                        ui.add_sized([200.0, 18.0], egui::Label::new(&item.name).truncate());
                        ui.label(RichText::new(format!("#{}", item.id)).weak().small());
                        // Editable exact progress: 0..=goal. Typing 0 removes the
                        // entry entirely (see operations::research_item).
                        let mut value = current;
                        let edited = ui
                            .add_sized(
                                [64.0, 18.0],
                                egui::DragValue::new(&mut value)
                                    .speed(1)
                                    .range(0..=item.research_goal),
                            )
                            .changed();
                        ui.label(RichText::new(format!("/ {}", item.research_goal)).weak());
                        if edited {
                            edit = Some((item.pid.clone(), value));
                        }
                        if current >= item.research_goal {
                            ui.label(
                                RichText::new("✓ done").color(Color32::from_rgb(130, 200, 130)),
                            );
                        } else if ui
                            .small_button("Complete")
                            .on_hover_text("Research this item to its goal")
                            .clicked()
                        {
                            edit = Some((item.pid.clone(), item.research_goal));
                        }
                    });
                }
            });
        if let Some((pid, count)) = edit {
            if let Some(p) = &mut self.player {
                operations::research_item(p, &pid, count);
                self.dirty = true;
            }
        }
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

    /// Draw a titled block of slots for the primary character.
    fn section(&mut self, ui: &mut egui::Ui, title: &str, ct: Ct) {
        self.section_side(ui, Side::Primary, title, ct);
    }

    /// Draw a titled block of slots for either character.
    fn section_side(&mut self, ui: &mut egui::Ui, side: Side, title: &str, ct: Ct) {
        let n = self
            .player_of(side)
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
                    self.slot_side(ui, side, SlotRef { c: ct, i }, cell);
                    i += 1;
                }
            });
        }
    }

    /// One inventory cell: paints the item, handles select/drag/scroll/context menu.
    fn slot_side(&mut self, ui: &mut egui::Ui, side: Side, r: SlotRef, cell: f32) {
        let Some(p) = self.player_of(side) else {
            return;
        };
        let slot = *container(p, r.c).get(r.i).unwrap_or(&Slot::empty());
        let multi = is_multi(r.c);
        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(cell), Sense::click_and_drag());
        let painter = ui.painter_at(rect);

        let selected = self.selected == Some(r) && self.selected_side == side;
        let border = if selected {
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
            let sprite = if self.settings.sprites {
                self.sprites.get(ui.ctx(), slot.net_id)
            } else {
                None
            };

            match sprite {
                Some(texture) => {
                    // Draw at native size: upscaling a 16 px coin to fill a 48 px
                    // slot is what makes sprite grids look wrong. Only oversized
                    // art is scaled down, keeping pixel alignment.
                    //
                    // `frame_size` is one animation frame, not the whole strip:
                    // a soul's texture is 22x88, and drawing that whole thing
                    // would shrink it to a quarter of its proper size.
                    let native = texture.frame_size();
                    let room = cell - 8.0;
                    let max_side = native.x.max(native.y);
                    // Large art shrinks to fit; tiny art (coins, blocks, torches)
                    // gets an integer 2x so it does not vanish, which keeps every
                    // pixel on the grid. Anything in between stays native size.
                    let fit = room / max_side;
                    let scale = if fit >= 2.0 { 2.0 } else { fit.min(1.0) };
                    let target = egui::Rect::from_center_size(rect.center(), native * scale);
                    painter.image(texture.handle.id(), target, texture.uv, Color32::WHITE);
                    // Rarity stays visible, but as a quiet 2 px strip instead of a
                    // background wash that would fight the artwork.
                    painter.rect_filled(
                        egui::Rect::from_min_max(
                            egui::pos2(rect.left() + 3.0, rect.bottom() - 5.0),
                            egui::pos2(rect.right() - 3.0, rect.bottom() - 3.0),
                        ),
                        1.0,
                        rarity_color(rarity),
                    );
                }
                None => {
                    // Fallback: rarity tile with initials, as before.
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
                }
            }

            if slot.stack > 1 && multi {
                // Shadow first, then the number: stacks stay readable on bright art.
                let text = slot.stack.to_string();
                let pos = rect.max - Vec2::new(3.0, 2.0);
                let font = egui::FontId::proportional(cell * 0.24);
                painter.text(
                    pos + Vec2::new(1.0, 1.0),
                    egui::Align2::RIGHT_BOTTOM,
                    &text,
                    font.clone(),
                    Color32::from_black_alpha(220),
                );
                painter.text(pos, egui::Align2::RIGHT_BOTTOM, &text, font, Color32::WHITE);
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
                self.describe(&slot, Some(r.c)),
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
                let db = self.db;
                let changed = match self.player_mut(side) {
                    Some(p) => operations::bump_stack(db, p, r, delta),
                    None => false,
                };
                if changed {
                    self.mark_dirty(side);
                }
            }
        }

        if resp.drag_started() && !slot.is_empty() {
            self.drag = Some(Drag {
                side,
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
                self.drop_on(side, r, drag);
                self.drag = None;
            }
        }

        if resp.clicked() {
            self.selected = Some(r);
            self.selected_side = side;
            if let Some(drag) = self.drag.take() {
                self.drop_on(side, r, drag);
            }
        }

        // Right-click: the explicit route, for people who prefer menus.
        let mut action: Option<MenuAction> = None;
        resp.context_menu(|ui| {
            ui.label(RichText::new(self.describe(&slot, Some(r.c))).strong());
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
                    side,
                    from: Some(r),
                    item: slot,
                })
            }
            Some(MenuAction::Duplicate) => {
                self.drag = Some(Drag {
                    side,
                    from: None,
                    item: slot,
                })
            }
            Some(MenuAction::MaxStack) => {
                let db = self.db;
                let changed = match self.player_mut(side) {
                    Some(p) => operations::max_stack(db, p, r),
                    None => false,
                };
                if changed {
                    self.mark_dirty(side);
                }
            }
            Some(MenuAction::Favourite) => {
                let changed = match self.player_mut(side) {
                    Some(p) => operations::toggle_favourite(p, r),
                    None => false,
                };
                if changed {
                    self.mark_dirty(side);
                }
            }
            Some(MenuAction::Clear) => self.clear_slot(side, r),
            None => {}
        }

        if resp.middle_clicked() && !slot.is_empty() {
            self.drag = Some(Drag {
                side,
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
        let title = if self.picker_for_stash {
            "Open stash character"
        } else {
            "Open character"
        };

        egui::Window::new(title)
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
            if self.picker_for_stash {
                self.open_stash(path);
            } else {
                self.open(path);
            }
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
                ui.add_space(6.0);
                changed |= ui
                    .checkbox(&mut self.settings.sprites, "Item sprites")
                    .on_hover_text(
                        "Draw real item artwork decoded from your Terraria installation.\n                         Turn this off for the plain rarity tiles.",
                    )
                    .changed();
                match terra_assets::sprites::item_dir() {
                    Some(dir) => {
                        ui.label(
                            RichText::new(format!("art from {}", dir.display()))
                                .weak()
                                .small(),
                        );
                    }
                    None => {
                        ui.label(
                            RichText::new(
                                "no Terraria install found — using rarity tiles.\n\
                                 Set TFORGE_CONTENT to a Content/Images directory to override.",
                            )
                            .weak()
                            .small(),
                        );
                    }
                }
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
