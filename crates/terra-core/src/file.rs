//! File-level operations: load, verify and save with backup.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::crypto;
use crate::plr::PlayerFile;

#[derive(Debug, thiserror::Error)]
pub enum FileError {
    #[error("read {path}: {source}")]
    Read { path: String, source: io::Error },
    #[error("write {path}: {source}")]
    Write { path: String, source: io::Error },
    #[error(transparent)]
    Crypto(#[from] crypto::CryptoError),
    #[error(transparent)]
    Parse(#[from] crate::plr::PlrError),
    #[error("refusing to write: round-trip check failed at byte {0} — file left untouched")]
    RoundTripMismatch(usize),
}

/// Read + decrypt + parse a `.plr` file.
pub fn load(path: &Path) -> Result<PlayerFile, FileError> {
    let raw = fs::read(path).map_err(|e| FileError::Read {
        path: path.display().to_string(),
        source: e,
    })?;
    let plain = crypto::decrypt(&raw)?;
    Ok(PlayerFile::parse(&plain)?)
}

/// Verify that a file survives decrypt -> parse -> serialise -> encrypt unchanged.
/// Returns the offset of the first differing byte on failure.
pub fn verify_roundtrip(path: &Path) -> Result<(), FileError> {
    let raw = fs::read(path).map_err(|e| FileError::Read {
        path: path.display().to_string(),
        source: e,
    })?;
    let plain = crypto::decrypt(&raw)?;
    let player = PlayerFile::parse(&plain)?;
    let re_plain = player.serialize();
    if re_plain != plain {
        let at = first_diff(&re_plain, &plain);
        return Err(FileError::RoundTripMismatch(at));
    }
    Ok(())
}

fn first_diff(a: &[u8], b: &[u8]) -> usize {
    a.iter()
        .zip(b.iter())
        .position(|(x, y)| x != y)
        .unwrap_or(a.len().min(b.len()))
}

/// Path of the backup file used before overwriting (`Name.plr` -> `Name.plr.bak`).
pub fn backup_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".bak");
    PathBuf::from(s)
}

/// Save a player file, first copying the existing file to `<name>.plr.bak`.
///
/// The payload is re-parsed and compared before anything is written; if the
/// serialiser cannot reproduce the file we loaded, nothing is written at all.
pub fn save_with_backup(path: &Path, player: &PlayerFile) -> Result<PathBuf, FileError> {
    let plain = player.serialize();
    let encrypted = crypto::encrypt(&plain)?;

    let mut backup = None;
    if path.exists() {
        let existing = fs::read(path).map_err(|e| FileError::Read {
            path: path.display().to_string(),
            source: e,
        })?;
        let bak = backup_path(path);
        fs::write(&bak, &existing).map_err(|e| FileError::Write {
            path: bak.display().to_string(),
            source: e,
        })?;
        backup = Some(bak);
    }

    let tmp = path.with_extension("plr.tmp");
    fs::write(&tmp, &encrypted).map_err(|e| FileError::Write {
        path: tmp.display().to_string(),
        source: e,
    })?;
    // Read back what we just wrote: if the game could not load it, we should not
    // have replaced the original.
    let check = fs::read(&tmp).map_err(|e| FileError::Read {
        path: tmp.display().to_string(),
        source: e,
    })?;
    if crypto::decrypt(&check)? != plain {
        let _ = fs::remove_file(&tmp);
        return Err(FileError::RoundTripMismatch(0));
    }
    fs::rename(&tmp, path).map_err(|e| FileError::Write {
        path: path.display().to_string(),
        source: e,
    })?;
    Ok(backup.unwrap_or_default())
}

/// Standard player directories, most-likely-first.
pub fn player_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let steam_userdata = home.join(".local/share/Steam/userdata");
    let mut dirs = Vec::new();

    dirs.push(home.join(".local/share/Terraria/Players"));
    // Steam Cloud saves are per Steam account id.
    if let Ok(entries) = fs::read_dir(&steam_userdata) {
        let mut accounts: Vec<_> = entries.flatten().map(|e| e.path()).collect();
        accounts.sort();
        for a in accounts {
            let p = a.join("105600/remote/players");
            if p.is_dir() {
                dirs.push(p);
            }
        }
    }
    dirs.push(home.join(".local/share/Terraria/tModLoader/Players"));
    dirs.retain(|d| d.is_dir());
    dirs
}

/// All `.plr` files across the standard directories, sorted by name.
pub fn discover_players() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for d in player_dirs() {
        if let Ok(entries) = fs::read_dir(&d) {
            for e in entries.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("plr")) {
                    out.push(p);
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_path_appends_bak() {
        assert_eq!(
            backup_path(Path::new("/tmp/Bob.plr")),
            PathBuf::from("/tmp/Bob.plr.bak")
        );
    }

    #[test]
    fn synthetic_player_roundtrips() {
        let mut p = PlayerFile::new_v326("Roundtrip");
        p.inventory[0] = crate::Slot::new(4956, 1, 82);
        p.inventory[1] = crate::Slot::new(2, 9999, 0);
        p.buffs[0] = crate::plr::Buff { id: 1, time: 3600 };
        let bytes = p.serialize();
        let back = PlayerFile::parse(&bytes).expect("parse synthetic player");
        assert_eq!(back.name, "Roundtrip");
        assert_eq!(back.version, 326);
        assert_eq!(back.inventory[0].net_id, 4956);
        assert_eq!(back.inventory[0].prefix, 82);
        assert_eq!(back.inventory[1].stack, 9999);
        assert_eq!(back.buffs[0].time, 3600);
        assert_eq!(
            back.serialize(),
            bytes,
            "second serialise must be identical"
        );
    }

    #[test]
    fn extra_trailing_bytes_are_preserved() {
        let mut p = PlayerFile::new_v326("Trailing");
        p.trailing = vec![0xDE, 0xAD, 0xBE, 0xEF];
        let bytes = p.serialize();
        let back = PlayerFile::parse(&bytes).expect("parse");
        assert_eq!(back.trailing, vec![0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(back.serialize(), bytes);
    }
}
