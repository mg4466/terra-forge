//! User interface settings, persisted as JSON.
//!
//! Kept deliberately tiny and dependency-free: a handful of scalars written to
//! `~/.config/terra-forge/settings.json`. Loading never fails hard — a missing
//! or malformed file just yields defaults, because a settings file must never
//! be the reason someone cannot open their character.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UiSettings {
    /// Global zoom, applied through `egui::Context::set_zoom_factor`.
    /// 1.0 = the default size; useful range is roughly 0.8 .. 1.8.
    pub zoom: f32,
    /// Slot cell size in points, before zoom.
    pub cell_size: f32,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            zoom: 1.15,
            cell_size: 46.0,
        }
    }
}

impl UiSettings {
    fn path() -> Option<PathBuf> {
        let dirs = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(dirs.join("terra-forge/settings.json"))
    }

    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        serde_json::from_str(&text).unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(path) = Self::path() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, text);
        }
    }

    pub fn clamped(mut self) -> Self {
        self.zoom = self.zoom.clamp(0.7, 2.5);
        self.cell_size = self.cell_size.clamp(34.0, 72.0);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let d = UiSettings::default();
        assert!(d.zoom >= 1.0 && d.zoom <= 2.0);
        assert!(d.cell_size >= 40.0);
    }

    #[test]
    fn out_of_range_values_are_clamped() {
        let s = UiSettings {
            zoom: 99.0,
            cell_size: 1.0,
        }
        .clamped();
        assert_eq!(s.zoom, 2.5);
        assert_eq!(s.cell_size, 34.0);
    }

    #[test]
    fn partial_json_keeps_other_defaults() {
        let s: UiSettings = serde_json::from_str(r#"{"zoom":1.3}"#).unwrap();
        assert_eq!(s.zoom, 1.3);
        assert_eq!(s.cell_size, UiSettings::default().cell_size);
    }
}
