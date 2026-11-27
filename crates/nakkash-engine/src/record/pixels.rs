//! A frame's pixels and masks in a record, one file each.
//!
//! Every file starts with a header: the magic `nakkash.Pixels.1`, what the
//! file holds (one byte: 1 for our pixels, 2 for a mask), the frame's width
//! and height (`u32` each) and the content hash of the payload (`u128`),
//! all little-endian. Then comes the payload:
//!
//! - our pixels: their RGBA bytes in the lz4 frame format;
//! - a mask: one bit a pixel, row by row, the first pixel in a byte's
//!   lowest bit, and bits past the last pixel zero.
//!
//! Reading checks the payload against its hash before anything else, so a
//! file cut short, damaged, of another kind or of another frame's size is
//! refused, never used. A mask's payload is its packed bits, so its hash is
//! [`mask_hash`], the one the frame's [`FrameRecord`](super::FrameRecord)
//! names; our pixels are hashed as compressed, and the record names them by
//! [`pixels_hash`], of their RGBA bytes.

use std::fs::{self, File};
use std::io::{self, BufWriter, Cursor, Read, Write};
use std::path::{Path, PathBuf};

use binrw::{BinRead, BinWrite, binrw};
use lz4_flex::frame::{FrameDecoder, FrameEncoder};
use nakkash_format::{Diagnostic, Kind, Place, Result};
use nakkash_raster::{Image, content_hash};

use super::{Drawn, Hex128};

/// The tag of [`pixels_hash`].
const PIXELS: &str = "nakkash.record.pixels";
/// The tag of [`mask_hash`], and of a mask file's payload.
const MASK: &str = "nakkash.record.mask";
/// The tag of the payload of a file of our pixels: compressed.
const COMPRESSED: &str = "nakkash.record.pixels.lz4";

/// The file name endings of our pixels, the coverage and the shown mask.
const OURS: &str = "ours.lz4";
const COVERAGE: &str = "cov";
const SHOWN: &str = "shown";

/// The accepted form of every pixel file.
const EXPECTED: &str = "a pixel file whole, as the record wrote it; record the sample again";

/// What a pixel file holds.
#[binrw]
#[brw(repr = u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::record) enum Content {
    /// Our pixels, compressed.
    Pixels = 1,
    /// A mask, one bit a pixel.
    Mask = 2,
}

/// The start of every pixel file.
#[binrw]
#[brw(little, magic = b"nakkash.Pixels.1")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Header {
    /// What the file holds.
    content: Content,
    /// The frame's width in pixels.
    width: u32,
    /// The frame's height in pixels.
    height: u32,
    /// The content hash of the payload, tagged by the content.
    hash: u128,
}

impl Content {
    /// The tag of the content hash of a payload of this content.
    fn tag(self) -> &'static str {
        match self {
            Content::Pixels => COMPRESSED,
            Content::Mask => MASK,
        }
    }
}

/// The length of [`Header`] in bytes: the magic, the content, the size and
/// the hash.
const HEADER: usize = 16 + 1 + 4 + 4 + 16;

/// Why a pixel file is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reason {
    /// No header, or not one of ours.
    NotPixels,
    /// It holds the other kind of content.
    Holds(Content),
    /// It holds a frame of another size, `(width, height)`.
    Size {
        found: (u32, u32),
        frame: (u32, u32),
    },
    /// Its payload is cut short, too long or not what its kind holds.
    Damaged,
    /// Its payload's hash is not the one its header gives.
    Hash { found: u128, header: u128 },
}

impl Reason {
    /// The diagnostic of the file at `path` refused for this reason.
    fn at(self, path: &Path) -> Diagnostic {
        let message = match self {
            Reason::NotPixels => "not a pixel file of a fidelity record".to_owned(),
            Reason::Holds(Content::Mask) => "holds a mask, not our pixels".to_owned(),
            Reason::Holds(Content::Pixels) => "holds our pixels, not a mask".to_owned(),
            Reason::Size { found, frame } => format!(
                "holds a {}x{} frame, not {}x{}",
                found.0, found.1, frame.0, frame.1
            ),
            Reason::Damaged => "its payload is cut short or damaged".to_owned(),
            Reason::Hash { found, header } => format!(
                "its payload hashes to {}, not to the {} its header gives",
                Hex128(found),
                Hex128(header)
            ),
        };
        Diagnostic::new(Kind::Invalid, message)
            .at(Place::Other(path.display().to_string()))
            .expected(EXPECTED)
    }
}

/// The content hash of our pixels: of their RGBA bytes.
pub fn pixels_hash(image: &Image) -> Hex128 {
    Hex128(content_hash(PIXELS, image.pixels()))
}

/// The content hash of a mask: of its bits, packed as its file holds them.
/// It does not count the pixels: masks are compared within one frame size.
pub fn mask_hash(mask: &[bool]) -> Hex128 {
    Hex128(content_hash(MASK, &pack(mask)))
}

/// Writes frame `index` of `drawn` into `dir`, which it makes if needed:
/// `f0000.ours.lz4`, `f0000.cov` and, when there is a shown mask,
/// `f0000.shown`.
///
/// # Errors
/// An invalid diagnostic when a mask does not have a value for each pixel
/// of our frame, or an I/O one when a file cannot be written.
pub fn write_frame(dir: &Path, index: usize, drawn: &Drawn) -> Result<()> {
    let (width, height) = (drawn.ours.width(), drawn.ours.height());
    let masks = [
        ("coverage", COVERAGE, Some(&drawn.coverage)),
        ("shown", SHOWN, drawn.shown.as_ref()),
    ];
    for (name, _, mask) in masks {
        if let Some(mask) = mask
            && mask.len() != width as usize * height as usize
        {
            return Err(Diagnostic::new(
                Kind::Invalid,
                format!(
                    "the {name} mask has {} pixels, not the {width}x{height} of our frame",
                    mask.len()
                ),
            )
            .expected("a mask with one value for each pixel of our frame"));
        }
    }
    fs::create_dir_all(dir).map_err(|e| Diagnostic::io("create", dir, &e))?;
    let size = (width, height);
    let path = file(dir, index, OURS);
    let compress = || {
        let mut encoder = FrameEncoder::new(Vec::new());
        encoder.write_all(drawn.ours.pixels())?;
        Ok::<_, io::Error>(encoder.finish()?)
    };
    let compressed = compress().map_err(|e| Diagnostic::io("compress", &path, &e))?;
    write_file(&path, Content::Pixels, size, &compressed)?;
    for (_, ending, mask) in masks {
        if let Some(mask) = mask {
            write_file(&file(dir, index, ending), Content::Mask, size, &pack(mask))?;
        }
    }
    Ok(())
}

/// Reads frame `index` of `dir`, a `width` by `height` frame, with its
/// shown mask when `shown`.
///
/// # Errors
/// An I/O diagnostic when a file cannot be read, or an invalid one, with
/// the file's path, when it is not a pixel file, holds the other kind or
/// another frame's size, is cut short or damaged, or is not what its
/// content hash names.
pub fn read_frame(dir: &Path, index: usize, width: u32, height: u32, shown: bool) -> Result<Drawn> {
    let pixels = read_file(&file(dir, index, OURS), |b| pixels_of(b, width, height))?;
    let mask = |ending| read_file(&file(dir, index, ending), |b| mask_of(b, width, height));
    Ok(Drawn {
        ours: Image::new(width, height, pixels)?,
        coverage: mask(COVERAGE)?,
        shown: shown.then(|| mask(SHOWN)).transpose()?,
    })
}

/// The file of frame `index` in `dir` with `ending`, such as `f0000.cov`.
fn file(dir: &Path, index: usize, ending: &str) -> PathBuf {
    dir.join(format!("f{index:04}.{ending}"))
}

/// Writes `payload` of `content` for a frame of `size` to the file at
/// `path`, after its header.
fn write_file(path: &Path, content: Content, size: (u32, u32), payload: &[u8]) -> Result<()> {
    let header = Header {
        content,
        width: size.0,
        height: size.1,
        hash: content_hash(content.tag(), payload),
    };
    let write = || {
        let mut out = BufWriter::new(File::create(path)?);
        header.write(&mut out).map_err(io::Error::other)?;
        out.write_all(payload)?;
        out.flush()
    };
    write().map_err(|e| Diagnostic::io("write", path, &e))
}

/// Reads the file at `path` and decodes it with `decode`.
fn read_file<T>(
    path: &Path,
    decode: impl FnOnce(&[u8]) -> std::result::Result<T, Reason>,
) -> Result<T> {
    let bytes = fs::read(path).map_err(|e| Diagnostic::io("read", path, &e))?;
    decode(&bytes).map_err(|reason| reason.at(path))
}

/// A pixel file's header and its payload.
fn split(bytes: &[u8]) -> std::result::Result<(Header, &[u8]), Reason> {
    let header = Header::read(&mut Cursor::new(bytes)).map_err(|_| Reason::NotPixels)?;
    Ok((header, &bytes[HEADER..]))
}

/// The payload of a pixel file of `content` for a `width` by `height`
/// frame, checked against its hash.
fn checked(
    bytes: &[u8],
    content: Content,
    width: u32,
    height: u32,
) -> std::result::Result<&[u8], Reason> {
    let (header, payload) = split(bytes)?;
    if header.content != content {
        return Err(Reason::Holds(header.content));
    }
    if (header.width, header.height) != (width, height) {
        return Err(Reason::Size {
            found: (header.width, header.height),
            frame: (width, height),
        });
    }
    let found = content_hash(content.tag(), payload);
    match found == header.hash {
        true => Ok(payload),
        false => Err(Reason::Hash {
            found,
            header: header.hash,
        }),
    }
}

/// Our pixels from the bytes of a pixel file for a `width` by `height`
/// frame.
fn pixels_of(bytes: &[u8], width: u32, height: u32) -> std::result::Result<Vec<u8>, Reason> {
    let payload = checked(bytes, Content::Pixels, width, height)?;
    let length = u64::from(width) * u64::from(height) * 4;
    // One byte more than a frame holds, so a payload too long is seen
    // without decoding all of it.
    let mut pixels = Vec::new();
    (FrameDecoder::new(payload).take(length + 1))
        .read_to_end(&mut pixels)
        .map_err(|_| Reason::Damaged)?;
    match pixels.len() as u64 == length {
        true => Ok(pixels),
        false => Err(Reason::Damaged),
    }
}

/// A mask from the bytes of a pixel file for a `width` by `height` frame.
fn mask_of(bytes: &[u8], width: u32, height: u32) -> std::result::Result<Vec<bool>, Reason> {
    let payload = checked(bytes, Content::Mask, width, height)?;
    let count = width as usize * height as usize;
    if payload.len() != count.div_ceil(8) {
        return Err(Reason::Damaged);
    }
    let mask = unpack(payload, count);
    // Bits past the last pixel are zero, so a mask has one spelling.
    match pack(&mask) == payload {
        true => Ok(mask),
        false => Err(Reason::Damaged),
    }
}

/// `mask` at one bit a pixel, the first pixel in a byte's lowest bit.
fn pack(mask: &[bool]) -> Vec<u8> {
    (mask.chunks(8))
        .map(|bits| (bits.iter().enumerate()).fold(0, |byte, (i, &on)| byte | u8::from(on) << i))
        .collect()
}

/// The first `count` pixels of a mask packed by [`pack`].
fn unpack(bytes: &[u8], count: usize) -> Vec<bool> {
    (bytes.iter())
        .flat_map(|byte| (0..8).map(move |i| byte >> i & 1 == 1))
        .take(count)
        .collect()
}

#[cfg(test)]
#[path = "../../tests/unit/record/pixels.rs"]
pub(in crate::record) mod tests;
