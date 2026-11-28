//! A frame's pixels and masks in a record, one file each.
//!
//! Every file starts with a header, little-endian:
//!
//! - the magic `nakkash.Pixels.1`;
//! - what the file holds, one byte: 1 for our pixels, 2 for a mask;
//! - the frame's width and height, a `u32` each;
//! - the content hash of the payload as stored (`u128`), which catches a
//!   file cut short or damaged;
//! - the content hash of what it holds, decoded (`u128`): [`pixels_hash`]
//!   or [`mask_hash`], the hash the frame's [`FrameRecord`] names.
//!
//! Then comes the payload: our pixels as their RGBA bytes in the lz4 frame
//! format, or a mask at one bit a pixel, row by row, the first pixel in a
//! byte's lowest bit, and bits past the last pixel zero.
//!
//! A file is read only against the record that names it ([`read_frame`]),
//! and refused unless it holds what the record names: a file of another
//! run, cut short, damaged, of another kind or of another frame's size is
//! never used. Each file is written whole ([`write_whole`]).

use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};

use binrw::{BinRead, BinWrite, binrw};
use lz4_flex::frame::{FrameDecoder, FrameEncoder};
use nakkash_format::grammar::parse_whole;
use nakkash_format::{Diagnostic, Kind, Place, Result};
use nakkash_hash::{ContentHasher, content_hash};
use nakkash_raster::Image;
use winnow::ascii::digit1;
use winnow::combinator::alt;

use super::files::write_whole;
use super::{Drawn, FrameRecord, Hex128};

/// The tag of [`pixels_hash`].
const PIXELS: &str = "nakkash.record.pixels";
/// The tag of [`mask_hash`].
const MASK: &str = "nakkash.record.mask";
/// The tag of the hash of a payload as stored.
const PAYLOAD: &str = "nakkash.record.payload";

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
    /// The content hash of the payload as stored.
    payload: u128,
    /// The content hash of what the file holds, decoded.
    hash: u128,
}

/// The length of [`Header`] in bytes: the magic, the content, the size and
/// the two hashes.
const HEADER: usize = 16 + 1 + 4 + 4 + 16 + 16;

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
    /// It holds other pixels than its record names: a file of another run.
    Record { found: u128, record: u128 },
    /// Its payload's hash is not the one its header gives: cut short or
    /// damaged.
    Payload { found: u128, header: u128 },
    /// Its payload does not decode to a frame of its size.
    Damaged,
    /// What it decodes to is not what its header names.
    Decoded { found: u128, header: u128 },
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
            Reason::Record { found, record } => format!(
                "holds a frame hashed {}, not the {} its record names",
                Hex128(found),
                Hex128(record)
            ),
            Reason::Payload { found, header } => format!(
                "its payload hashes to {}, not to the {} its header gives",
                Hex128(found),
                Hex128(header)
            ),
            Reason::Damaged => "its payload does not decode to the frame".to_owned(),
            Reason::Decoded { found, header } => format!(
                "it decodes to a frame hashed {}, not the {} its header gives",
                Hex128(found),
                Hex128(header)
            ),
        };
        Diagnostic::new(Kind::Invalid, message)
            .at(Place::Other(path.display().to_string()))
            .expected(EXPECTED)
    }
}

/// The content hash of our pixels: of their size and RGBA bytes.
pub fn pixels_hash(image: &Image) -> Hex128 {
    Hex128(sized(PIXELS, image.width(), image.height(), image.pixels()))
}

/// The content hash of a mask of a `width` by `height` frame: of the size
/// and of its bits, packed as its file holds them.
pub fn mask_hash(mask: &[bool], width: u32, height: u32) -> Hex128 {
    Hex128(sized(MASK, width, height, &pack(mask)))
}

/// The content hash, tagged `tag`, of `bytes` of a `width` by `height`
/// frame.
fn sized(tag: &str, width: u32, height: u32, bytes: &[u8]) -> u128 {
    (ContentHasher::new(tag).u32(width).u32(height))
        .bytes(bytes)
        .finish()
}

/// Writes frame `index` of `drawn` into `dir`: `f0000.ours.lz4`,
/// `f0000.cov` and, when there is a shown mask, `f0000.shown`, each whole.
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
    let path = file(dir, index, OURS);
    let mut encoder = FrameEncoder::new(Vec::new());
    let compressed = (encoder.write_all(drawn.ours.pixels()))
        .and_then(|()| Ok(encoder.finish()?))
        .map_err(|e| Diagnostic::io("compress", &path, &e))?;
    let header = Header {
        content: Content::Pixels,
        width,
        height,
        payload: content_hash(PAYLOAD, &compressed),
        hash: pixels_hash(&drawn.ours).0,
    };
    write_file(&path, header, &compressed)?;
    for (_, ending, mask) in masks {
        if let Some(mask) = mask {
            let bits = pack(mask);
            let header = Header {
                content: Content::Mask,
                width,
                height,
                payload: content_hash(PAYLOAD, &bits),
                hash: sized(MASK, width, height, &bits),
            };
            write_file(&file(dir, index, ending), header, &bits)?;
        }
    }
    Ok(())
}

/// Reads the frame `record` names from `dir`, a `width` by `height` frame,
/// with its shown mask when the record names one: [`read_ours`] and
/// [`read_masks`].
///
/// # Errors
/// As [`read_ours`] and [`read_masks`].
pub fn read_frame(dir: &Path, record: &FrameRecord, width: u32, height: u32) -> Result<Drawn> {
    let ours = read_ours(dir, record, width, height)?;
    let (coverage, shown) = read_masks(dir, record, width, height)?;
    Ok(Drawn {
        ours,
        coverage,
        shown,
    })
}

/// Reads our pixels of the frame `record` names from `dir`, a `width` by
/// `height` frame.
///
/// # Errors
/// An I/O diagnostic when the file cannot be read, or an invalid one, with
/// the file's path, when it does not hold what the record names: it is not
/// a pixel file, holds the other kind, another frame's size or other
/// pixels, or is cut short or damaged.
pub fn read_ours(dir: &Path, record: &FrameRecord, width: u32, height: u32) -> Result<Image> {
    let ours = read_file(&file(dir, record.index, OURS), |bytes| {
        pixels_of(bytes, width, height, record.ours_pixels.0)
    })?;
    Image::new(width, height, ours)
}

/// Reads the coverage of the frame `record` names from `dir`, a `width` by
/// `height` frame, and its shown mask when the record names one.
///
/// # Errors
/// As [`read_ours`], for each mask file.
pub fn read_masks(
    dir: &Path,
    record: &FrameRecord,
    width: u32,
    height: u32,
) -> Result<(Vec<bool>, Option<Vec<bool>>)> {
    let mask = |ending, hash: Hex128| {
        read_file(&file(dir, record.index, ending), |bytes| {
            mask_of(bytes, width, height, hash.0)
        })
    };
    let coverage = mask(COVERAGE, record.coverage)?;
    let shown = (record.shown_mask)
        .map(|hash| mask(SHOWN, hash))
        .transpose()?;
    Ok((coverage, shown))
}

/// The file of frame `index` in `dir` with `ending`, such as `f0000.cov`.
fn file(dir: &Path, index: usize, ending: &str) -> PathBuf {
    dir.join(format!("f{index:04}.{ending}"))
}

/// Whether `name` is the name of a frame file [`file`] makes, of any frame:
/// `f`, the frame's index, then one of the endings.
pub(super) fn is_frame_file(name: &str) -> bool {
    let grammar = ("f", digit1, ".", alt((OURS, COVERAGE, SHOWN)));
    parse_whole(name, grammar, "frame file", "`f0000.cov`").is_ok()
}

/// Writes `header` then `payload` to the file at `path`, whole.
fn write_file(path: &Path, header: Header, payload: &[u8]) -> Result<()> {
    write_whole(path, |out| {
        header.write(out).map_err(std::io::Error::other)?;
        out.write_all(payload)
    })
}

/// Reads the file at `path` and decodes it with `decode`.
fn read_file<T>(
    path: &Path,
    decode: impl FnOnce(&[u8]) -> std::result::Result<T, Reason>,
) -> Result<T> {
    let bytes = std::fs::read(path).map_err(|e| Diagnostic::io("read", path, &e))?;
    decode(&bytes).map_err(|reason| reason.at(path))
}

/// A pixel file's header and its payload.
fn split(bytes: &[u8]) -> std::result::Result<(Header, &[u8]), Reason> {
    let header = Header::read(&mut Cursor::new(bytes)).map_err(|_| Reason::NotPixels)?;
    Ok((header, &bytes[HEADER..]))
}

/// The header and the payload of a pixel file of `content` for a `width`
/// by `height` frame, whose record names `record`: checked against the
/// record first, so a file of another run is refused before it is read
/// further, then against its own payload hash.
fn checked(
    bytes: &[u8],
    content: Content,
    (width, height): (u32, u32),
    record: u128,
) -> std::result::Result<(Header, &[u8]), Reason> {
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
    if header.hash != record {
        return Err(Reason::Record {
            found: header.hash,
            record,
        });
    }
    let found = content_hash(PAYLOAD, payload);
    match found == header.payload {
        true => Ok((header, payload)),
        false => Err(Reason::Payload {
            found,
            header: header.payload,
        }),
    }
}

/// Refuses what a file decodes to when its hash `found` is not the one its
/// header gives.
fn decoded(found: u128, header: &Header) -> std::result::Result<(), Reason> {
    match found == header.hash {
        true => Ok(()),
        false => Err(Reason::Decoded {
            found,
            header: header.hash,
        }),
    }
}

/// Our pixels from the bytes of a pixel file for a `width` by `height`
/// frame, whose record names the hash `record`.
fn pixels_of(
    bytes: &[u8],
    width: u32,
    height: u32,
    record: u128,
) -> std::result::Result<Vec<u8>, Reason> {
    let (header, payload) = checked(bytes, Content::Pixels, (width, height), record)?;
    let length = u64::from(width) * u64::from(height) * 4;
    // One byte more than a frame holds, so a payload too long is seen
    // without decoding all of it.
    let mut pixels = Vec::new();
    (FrameDecoder::new(payload).take(length + 1))
        .read_to_end(&mut pixels)
        .map_err(|_| Reason::Damaged)?;
    if pixels.len() as u64 != length {
        return Err(Reason::Damaged);
    }
    decoded(sized(PIXELS, width, height, &pixels), &header)?;
    Ok(pixels)
}

/// A mask from the bytes of a pixel file for a `width` by `height` frame,
/// whose record names the hash `record`.
fn mask_of(
    bytes: &[u8],
    width: u32,
    height: u32,
    record: u128,
) -> std::result::Result<Vec<bool>, Reason> {
    let (header, payload) = checked(bytes, Content::Mask, (width, height), record)?;
    let count = width as usize * height as usize;
    if payload.len() != count.div_ceil(8) {
        return Err(Reason::Damaged);
    }
    let mask = unpack(payload, count);
    // Bits past the last pixel are zero, so a mask has one spelling.
    if pack(&mask) != payload {
        return Err(Reason::Damaged);
    }
    decoded(sized(MASK, width, height, payload), &header)?;
    Ok(mask)
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
