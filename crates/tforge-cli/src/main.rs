//! `tforge` — headless companion CLI for terra-forge.
//!
//! Used to inspect and verify player files without opening the GUI, and as the
//! reference harness for the round-trip guarantee: `tforge verify <file>` must
//! report "identical" for every file the GUI is willing to save.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use terra_core::{crypto, PlayerFile};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        usage();
        return ExitCode::from(2);
    }
    let cmd = args[0].as_str();
    let paths: Vec<PathBuf> = args[1..].iter().map(PathBuf::from).collect();

    match cmd {
        "info" => run(paths, cmd_info),
        "verify" => run(paths, cmd_verify),
        "inventory" => run(paths, cmd_inventory),
        "copy" => run(paths, cmd_copy),
        "selftest" => run(paths, cmd_selftest),
        "-h" | "--help" | "help" => {
            usage();
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("unknown command: {other}");
            usage();
            ExitCode::from(2)
        }
    }
}

fn usage() {
    eprintln!(
        "usage: tforge <command> <player.plr>...\n\
         \n\
         commands:\n\
         \x20 info       decode and summarise a player file\n\
         \x20 verify     prove decrypt->parse->serialise->encrypt is byte-identical\n\
         \x20 inventory  list non-empty inventory/equipment slots\n\
         \x20 copy       load and save to <file>.roundtrip via the GUI save path\n\
         \x20 selftest   apply a set of edits, save, reload and assert the edits stuck"
    );
}

fn run(paths: Vec<PathBuf>, f: fn(&Path) -> Result<(), String>) -> ExitCode {
    if paths.is_empty() {
        usage();
        return ExitCode::from(2);
    }
    let mut failed = 0;
    for p in &paths {
        println!("== {} ==", p.display());
        if let Err(e) = f(p) {
            println!("   ERROR: {e}");
            failed += 1;
        }
    }
    if failed > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn load(path: &Path) -> Result<PlayerFile, String> {
    let raw = std::fs::read(path).map_err(|e| format!("read: {e}"))?;
    let plain = crypto::decrypt(&raw).map_err(|e| format!("decrypt: {e}"))?;
    PlayerFile::parse(&plain).map_err(|e| format!("parse: {e}"))
}

fn cmd_info(path: &Path) -> Result<(), String> {
    let p = load(path)?;
    let raw_len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    println!(
        "   version {} ({})   file {} bytes   payload {} bytes",
        p.version,
        p.format_name(),
        raw_len,
        p.serialize().len()
    );
    println!(
        "   name {:?}   difficulty {}   life {}/{}   mana {}/{}",
        p.name, p.difficulty, p.health_now, p.health_max, p.mana_now, p.mana_max
    );
    println!(
        "   hair {} dye {} team {} gender {}   playtime {}s",
        p.hair_style,
        p.hair_dye,
        p.team,
        p.gender,
        (p.play_time_l as u64 / 10_000_000) + (p.play_time_h as u64 * 429)
    );
    let used = |v: &Vec<terra_core::Slot>| v.iter().filter(|s| !s.is_empty()).count();
    println!(
        "   slots used: inv {}/{}  coins {}  ammo {}  equip {}  bank {}/40  safe {}/40  forge {}/40  void {}/40",
        used(&p.inventory),
        p.inventory.len(),
        used(&p.coins),
        used(&p.ammo),
        used(&p.equipment_items),
        used(&p.bank),
        used(&p.safe),
        used(&p.forge),
        used(&p.void_bag)
    );
    println!(
        "   buffs active {}   research entries {}   servers {}   loadout {}",
        p.buffs.iter().filter(|b| b.id > 0).count(),
        p.research.len(),
        p.servers.len(),
        p.current_loadout
    );
    println!("   trailing unparsed bytes: {}", p.trailing.len());
    Ok(())
}

fn cmd_verify(path: &Path) -> Result<(), String> {
    let raw = std::fs::read(path).map_err(|e| format!("read: {e}"))?;
    let plain = crypto::decrypt(&raw).map_err(|e| format!("decrypt: {e}"))?;
    let p = PlayerFile::parse(&plain).map_err(|e| format!("parse: {e}"))?;
    let out_plain = p.serialize();
    if out_plain != plain {
        let at = out_plain
            .iter()
            .zip(plain.iter())
            .position(|(a, b)| a != b)
            .unwrap_or(plain.len().min(out_plain.len()));
        return Err(format!(
            "PLAINTEXT MISMATCH at byte {at} (in {} bytes, out {} bytes, version {})",
            plain.len(),
            out_plain.len(),
            p.version
        ));
    }
    let out = crypto::encrypt(&out_plain).map_err(|e| format!("encrypt: {e}"))?;
    if out != raw {
        let at = out
            .iter()
            .zip(raw.iter())
            .position(|(a, b)| a != b)
            .unwrap_or(raw.len().min(out.len()));
        return Err(format!("CIPHERTEXT MISMATCH at byte {at}"));
    }
    println!(
        "   identical: {} bytes, version {}, trailing {} bytes",
        raw.len(),
        p.version,
        p.trailing.len()
    );
    Ok(())
}

fn cmd_inventory(path: &Path) -> Result<(), String> {
    let p = load(path)?;
    let db = terra_assets::ItemDb::embedded();
    fn collect<'a>(
        rows: &mut Vec<(String, usize, &'a terra_core::Slot)>,
        label: &str,
        slots: &'a [terra_core::Slot],
    ) {
        for (i, s) in slots.iter().enumerate() {
            if !s.is_empty() {
                rows.push((label.to_string(), i, s));
            }
        }
    }
    let mut rows: Vec<(String, usize, &terra_core::Slot)> = Vec::new();
    collect(&mut rows, "inv", &p.inventory);
    collect(&mut rows, "coins", &p.coins);
    collect(&mut rows, "ammo", &p.ammo);
    collect(&mut rows, "equip", &p.equipment_items);
    collect(&mut rows, "dye", &p.equipment_dyes);
    collect(&mut rows, "bank", &p.bank);
    collect(&mut rows, "safe", &p.safe);
    collect(&mut rows, "forge", &p.forge);
    collect(&mut rows, "void", &p.void_bag);
    for l in 0..3 {
        collect(
            &mut rows,
            &format!("loadout{}", l + 1),
            &p.stored_loadouts[l].items,
        );
    }
    for (label, i, s) in rows {
        let name = db
            .by_id(s.net_id)
            .map(|it| it.name.clone())
            .unwrap_or_else(|| format!("<item #{}>", s.net_id));
        let prefix = if s.prefix != 0 {
            format!(" [prefix {}]", s.prefix)
        } else {
            String::new()
        };
        println!(
            "   {label}[{i:2}] {:<32} x{:<6}{prefix}{}",
            name,
            s.stack,
            if s.favorited { " *fav" } else { "" }
        );
    }
    Ok(())
}

/// Exercise the exact save path the GUI uses (backup + temp file + read-back
/// verification) by writing a copy next to the source and verifying the result.
fn cmd_copy(path: &Path) -> Result<(), String> {
    let player = terra_core::load(path).map_err(|e| e.to_string())?;
    let mut out = path.to_path_buf();
    let stem = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    out.set_file_name(format!("{stem}.roundtrip.plr"));
    let _ = std::fs::remove_file(terra_core::backup_path(&out));
    terra_core::save_with_backup(&out, &player).map_err(|e| format!("save: {e}"))?;
    let written = std::fs::read(&out).map_err(|e| e.to_string())?;
    let original = std::fs::read(path).map_err(|e| e.to_string())?;
    if written != original {
        return Err(format!(
            "saved copy differs from source ({} vs {} bytes)",
            written.len(),
            original.len()
        ));
    }
    println!(
        "   wrote {} ({} bytes, byte-identical to source)",
        out.display(),
        written.len()
    );
    Ok(())
}

/// Functional edit test: mutate the player in memory the same way the GUI does,
/// save through the backup path, reload from disk and assert every edit landed
/// while every untouched slot stayed identical.
fn cmd_selftest(path: &Path) -> Result<(), String> {
    let mut player = terra_core::load(path).map_err(|e| e.to_string())?;
    let before = player.clone();

    // pick a rich character or fall back to slot 0
    let idx = player
        .inventory
        .iter()
        .position(|s| !s.is_empty())
        .unwrap_or(0);
    player.inventory[idx] = terra_core::Slot {
        net_id: 4956,
        stack: 1,
        prefix: 82,
        favorited: true,
    };
    player.bank[3] = terra_core::Slot::new(2, 9999, 0);
    player.equipment_dyes[0] = terra_core::Slot::new(1050, 1, 0);
    player.health_max = 500;
    player.research.push(("Zenith".to_string(), 1));

    let out = {
        let mut o = path.to_path_buf();
        let stem = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        o.set_file_name(format!("{stem}.edited.plr"));
        o
    };
    terra_core::save_with_backup(&out, &player).map_err(|e| format!("save: {e}"))?;

    let reloaded = terra_core::load(&out).map_err(|e| format!("reload: {e}"))?;

    let mut checks: Vec<(String, bool)> = Vec::new();
    checks.push((
        format!("inventory[{idx}] is Zenith/Mythical/favourite"),
        reloaded.inventory[idx]
            == terra_core::Slot {
                net_id: 4956,
                stack: 1,
                prefix: 82,
                favorited: true,
            },
    ));
    checks.push((
        "piggy bank slot 3 is 9999 dirt".to_string(),
        reloaded.bank[3].net_id == 2 && reloaded.bank[3].stack == 9999,
    ));
    checks.push((
        "dye slot holds item 1050".to_string(),
        reloaded.equipment_dyes[0].net_id == 1050,
    ));
    checks.push(("max life is 500".to_string(), reloaded.health_max == 500));
    checks.push((
        "research entry added".to_string(),
        reloaded
            .research
            .last()
            .map(|(p, c)| p == "Zenith" && *c == 1)
            .unwrap_or(false),
    ));

    // untouched regions must be byte-identical
    let mut untouched_ok = true;
    for i in reloaded
        .inventory
        .iter()
        .zip(before.inventory.iter())
        .enumerate()
    {
        let (n, (a, b)) = i;
        if n != idx && a != b {
            untouched_ok = false;
        }
    }
    checks.push((
        "all other inventory slots unchanged".to_string(),
        untouched_ok,
    ));
    checks.push((
        format!("trailing bytes preserved ({})", before.trailing.len()),
        reloaded.trailing == before.trailing,
    ));

    let mut failed = 0;
    for (label, ok) in checks {
        println!("   [{}] {label}", if ok { "ok" } else { "FAIL" });
        if !ok {
            failed += 1;
        }
    }
    println!("   wrote {}", out.display());
    if failed > 0 {
        return Err(format!("{failed} checks failed"));
    }
    Ok(())
}
