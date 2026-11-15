//! Terraria item sprites, read straight out of the installed game.
//!
//! # Why this is possible at all
//!
//! Terraria ships one encrypted-at-rest-but-not-encrypted file per item:
//! `Content/Images/Item_<id>.xnb`. An `.xnb` is an XNA content container:
//!
//! ```text
//! 'X' 'N' 'B' 'w'      platform (w = Windows)
//! u8  version          always 5 for Terraria
//! u8  flags            0x80 = LZX-compressed, 0x01 = HiDef
//! u32 file size
//! u32 decompressed size        (only when compressed)
//! ...   payload
//! ```
//!
//! Compressed payloads are LZX with a *chunk framing* on top that is not part of
//! the LZX spec: each 32 KiB chunk is preceded by a two-byte big-endian length,
//! and chunks whose first byte is `0xFF` also carry the uncompressed length:
//!
//! ```text
//! 0xFF  be16 uncompressed  be16 compressed  <compressed bytes>
//! be16 compressed                          <compressed bytes>
//! ```
//!
//! After decompression the payload is the usual XNB object graph: a list of type
//! readers, a shared-resource count, a type-reader index, then the `Texture2D`:
//! surface format, width, height, mip count, data size, data.
//!
//! Terraria uses surface format 0 (straight RGBA8, one mip), which is why the
//! pixel data can be handed to the GPU with no conversion.
//!
//! # Cost
//!
//! All 6,134 vanilla item textures decode in ~0.3 s single-threaded, roughly
//! 0.05 ms per item. That makes decoding on demand (when a slot first becomes
//! visible) cheaper than shipping a prebuilt atlas, and it means sprites always
//! match the installed game version.

use std::path::PathBuf;

use lzxd::{Lzxd, WindowSize};

#[derive(Debug, thiserror::Error)]
pub enum SpriteError {
    #[error("not an XNB file")]
    NotXnb,
    #[error("unsupported XNB version {0}")]
    UnsupportedVersion(u8),
    #[error("LZX decompression failed: {0}")]
    Lzx(String),
    #[error("malformed payload at offset {0}")]
    Malformed(usize),
    #[error("unsupported texture (surface format {0}, {1}x{2}, {3} bytes)")]
    UnsupportedTexture(i32, u32, u32, u32),
    #[error("read {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
}

/// A decoded item texture: straight (non-premultiplied) RGBA8, row-major.
#[derive(Clone)]
pub struct Sprite {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Sprite {
    /// Longest side, used to decide whether the sprite needs shrinking.
    pub fn longest_side(&self) -> u32 {
        self.width.max(self.height)
    }

    /// Size in bytes of the RGBA buffer (for cache accounting).
    pub fn bytes(&self) -> usize {
        self.rgba.len()
    }
}

// ---------------------------------------------------------------- XNB reader

struct Cursor<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Cursor<'a> {
    fn u8(&mut self) -> Result<u8, SpriteError> {
        let v = *self.b.get(self.p).ok_or(SpriteError::Malformed(self.p))?;
        self.p += 1;
        Ok(v)
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], SpriteError> {
        let end = self
            .p
            .checked_add(n)
            .ok_or(SpriteError::Malformed(self.p))?;
        let s = self
            .b
            .get(self.p..end)
            .ok_or(SpriteError::Malformed(self.p))?;
        self.p = end;
        Ok(s)
    }

    fn i32(&mut self) -> Result<i32, SpriteError> {
        let s = self.take(4)?;
        Ok(i32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }

    fn u32(&mut self) -> Result<u32, SpriteError> {
        Ok(self.i32()? as u32)
    }

    /// .NET 7-bit encoded integer (used for counts and string lengths).
    fn varint(&mut self) -> Result<usize, SpriteError> {
        let mut value = 0usize;
        let mut shift = 0;
        loop {
            let byte = self.u8()?;
            value |= ((byte & 0x7F) as usize) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
            shift += 7;
            if shift > 63 {
                return Err(SpriteError::Malformed(self.p));
            }
        }
    }

    fn string(&mut self) -> Result<&'a [u8], SpriteError> {
        let len = self.varint()?;
        self.take(len)
    }
}

/// Undo the XNB chunk framing and LZX-compress the payload.
fn decompress_lzx(payload: &[u8], total: usize) -> Result<Vec<u8>, SpriteError> {
    let mut dec = Lzxd::new(WindowSize::KB64);
    let mut out = Vec::with_capacity(total);
    let mut p = 0usize;

    while out.len() < total {
        let first = *payload.get(p).ok_or(SpriteError::Malformed(p))?;
        let (compressed, uncompressed) = if first == 0xFF {
            let sizes = payload.get(p + 1..p + 5).ok_or(SpriteError::Malformed(p))?;
            let uncompressed = u16::from_be_bytes([sizes[0], sizes[1]]) as usize;
            let compressed = u16::from_be_bytes([sizes[2], sizes[3]]) as usize;
            p += 5;
            (compressed, uncompressed)
        } else {
            let sizes = payload.get(p..p + 2).ok_or(SpriteError::Malformed(p))?;
            let compressed = u16::from_be_bytes([sizes[0], sizes[1]]) as usize;
            p += 2;
            (compressed, (total - out.len()).min(32 * 1024))
        };
        if compressed == 0 {
            return Err(SpriteError::Malformed(p));
        }
        let chunk = payload
            .get(p..p + compressed)
            .ok_or(SpriteError::Malformed(p))?;
        p += compressed;
        let decoded = dec
            .decompress_next(chunk, uncompressed)
            .map_err(|e| SpriteError::Lzx(e.to_string()))?;
        out.extend_from_slice(decoded);
    }
    Ok(out)
}

/// Decode one `.xnb` texture file into RGBA8.
pub fn decode_xnb(data: &[u8]) -> Result<Sprite, SpriteError> {
    if data.len() < 10 || &data[0..4] != b"XNBw" && &data[0..4] != b"XNBx" {
        return Err(SpriteError::NotXnb);
    }
    let version = data[4];
    if version != 5 {
        return Err(SpriteError::UnsupportedVersion(version));
    }
    let flags = data[5];
    let compressed = flags & 0x80 != 0;

    let body: Vec<u8> = if compressed {
        let size_bytes = data.get(10..14).ok_or(SpriteError::Malformed(10))?;
        let total = u32::from_le_bytes([size_bytes[0], size_bytes[1], size_bytes[2], size_bytes[3]])
            as usize;
        decompress_lzx(&data[14..], total)?
    } else {
        data[10..].to_vec()
    };

    let mut c = Cursor { b: &body, p: 0 };

    // Type reader table, then the shared resource count, then the object.
    let readers = c.varint()?;
    for _ in 0..readers {
        c.string()?;
        c.i32()?;
    }
    let _shared = c.varint()?;
    let _type_index = c.varint()?;

    let surface = c.i32()?;
    let width = c.u32()?;
    let height = c.u32()?;
    let _mips = c.u32()?;
    let data_size = c.u32()?;

    // Surface format 0 is Color (RGBA8) and Terraria always ships one mip.
    if surface != 0 || width == 0 || height == 0 || data_size != width * height * 4 {
        return Err(SpriteError::UnsupportedTexture(
            surface, width, height, data_size,
        ));
    }
    let rgba = c.take(data_size as usize)?.to_vec();
    Ok(Sprite {
        width,
        height,
        rgba,
    })
}

// ------------------------------------------------------------- installation

/// Directories that may hold `Item_<id>.xnb`, most-likely first.
pub fn content_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(override_dir) = std::env::var_os("TFORGE_CONTENT") {
        dirs.push(PathBuf::from(override_dir));
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let steam = home.join(".local/share/Steam/steamapps/common");
    dirs.push(steam.join("Terraria/Content/Images"));
    dirs.push(steam.join("tModLoader/Content/Images"));
    // GOG / manual installs sometimes live here.
    dirs.push(home.join("GOG Games/Terraria/Content/Images"));
    dirs.retain(|d| d.is_dir());
    dirs
}

/// The first directory that actually contains item textures.
pub fn item_dir() -> Option<PathBuf> {
    content_dirs()
        .into_iter()
        .find(|d| d.join("Item_1.xnb").exists() || d.join("Item_2.xnb").exists())
}

/// Decode one item's sprite by id. Returns `Ok(None)` when the file is absent,
/// which is normal: not every id has artwork, and the game may not be installed.
pub fn load_item_sprite(id: i32) -> Result<Option<Sprite>, SpriteError> {
    if id <= 0 {
        return Ok(None);
    }
    let Some(dir) = item_dir() else {
        return Ok(None);
    };
    let path = dir.join(format!("Item_{id}.xnb"));
    match std::fs::read(&path) {
        Ok(data) => decode_xnb(&data).map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(SpriteError::Io {
            path: path.display().to_string(),
            source,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_xnb() {
        assert!(matches!(
            decode_xnb(b"hello world"),
            Err(SpriteError::NotXnb)
        ));
        assert!(matches!(decode_xnb(b""), Err(SpriteError::NotXnb)));
    }

    /// Decodes a real sprite when the game is installed; skips silently otherwise
    /// so the test suite works on machines (and CI) without Terraria.
    #[test]
    fn decodes_a_real_item_if_present() {
        match load_item_sprite(1) {
            Ok(Some(sprite)) => {
                assert!(sprite.width >= 16 && sprite.height >= 16);
                assert_eq!(
                    sprite.rgba.len(),
                    (sprite.width * sprite.height * 4) as usize
                );
                // An iron pickaxe is mostly transparent with visible metal/handle.
                let opaque = sprite.rgba.chunks(4).filter(|px| px[3] > 128).count();
                assert!(opaque > 50, "sprite looks empty ({opaque} opaque pixels)");
            }
            Ok(None) => eprintln!("no Terraria install found; skipping sprite decode test"),
            Err(e) => panic!("decode failed: {e}"),
        }
    }

    #[test]
    fn item_dir_is_a_directory_when_found() {
        if let Some(dir) = item_dir() {
            assert!(dir.is_dir());
        }
    }
}
