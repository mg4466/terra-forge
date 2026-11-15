//! Core of terra-forge: Terraria player-file cryptography, binary parsing and
//! non-destructive round-trip serialisation.

pub mod crypto;
pub mod cursor;
pub mod file;
pub mod plr;

pub use file::{
    backup_path, discover_players, load, player_dirs, save_with_backup, verify_roundtrip, FileError,
};
pub use plr::{CreativePower, CreativeValue, Loadout, PlayerFile, PlrError, Slot};
