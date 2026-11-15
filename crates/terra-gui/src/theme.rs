//! Terraria item-rarity colours, used for slot fills until sprite decoding lands.

use eframe::egui::Color32;

/// Rarity index -> fill colour. Negative values are the special "expert/master/
/// quest" tiers Terraria uses for relics and quest items.
pub fn rarity_color(rarity: i32) -> Color32 {
    match rarity {
        -13 => Color32::from_rgb(120, 60, 160), // master mode relic
        -12 => Color32::from_rgb(90, 60, 140),
        -11 => Color32::from_rgb(150, 60, 60), // quest item
        -1 => Color32::from_rgb(90, 90, 96),   // grey (unobtainable)
        0 => Color32::from_rgb(200, 200, 205),
        1 => Color32::from_rgb(120, 160, 240),
        2 => Color32::from_rgb(110, 210, 120),
        3 => Color32::from_rgb(240, 150, 90),
        4 => Color32::from_rgb(240, 110, 110),
        5 => Color32::from_rgb(240, 140, 200),
        6 => Color32::from_rgb(180, 140, 240),
        7 => Color32::from_rgb(170, 220, 90),
        8 => Color32::from_rgb(240, 220, 110),
        9 => Color32::from_rgb(90, 210, 230),
        10 => Color32::from_rgb(235, 90, 90),
        11 => Color32::from_rgb(200, 120, 240),
        _ => Color32::from_rgb(180, 180, 190),
    }
}
