//! `nakkash frames`: draw frames of a recorded sample again from its
//! record folder alone (D94), and print one JSON line a frame, in index
//! order: the content hashes of our pixels, the coverage and the shown
//! mask, as the sample's record names them, and how long each draw took.
//!
//! The scene and the replay are read only against the sample's record,
//! which names them by the content hashes of their bytes: files a later
//! run wrote over them, or ones beside no record, are refused.
//!
//! A frame is drawn as its fidelity run drew it, through the same draws
//! ([`Replayer::draw_frame`](nakkash_engine::Replayer::draw_frame)): on
//! the CPU at precise quality, so its hashes equal its record's bit for
//! bit. Asked to draw another way, it refuses rather than print hashes no
//! record can match.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::ExitCode;

use nakkash_engine::record::{Hex128, Replayed, SCENE, Times, mask_hash, open_replay, pixels_hash};
use nakkash_engine::{Diagnostic, Kind, Quality, Result, Session};
use serde::Serialize;

/// What `nakkash frames` takes besides the sample's folder.
pub struct Args<'a> {
    /// The frames to draw, by index; each once, in index order.
    pub frames: &'a [usize],
    /// The quality asked for: only precise is drawn.
    pub quality: Quality,
    /// Whether the GPU was asked for: it never draws a recorded frame.
    pub gpu: bool,
}

/// One frame's line.
#[derive(Debug, Serialize)]
struct Line {
    /// The frame's index among the sample's compared frames.
    index: usize,
    /// The content hash of our pixels.
    ours: Hex128,
    /// The content hash of the coverage.
    coverage: Hex128,
    /// The content hash of the shown mask; none when no layer is ignored.
    shown: Option<Hex128>,
    /// How long each draw took, in milliseconds.
    ms: Times,
}

/// Draws the frames `args` lists of the sample recorded in `sample`, and
/// prints a line for each. The folder's `sample.json` and `scene.json` are
/// read only against its record, and every index is checked, before any
/// frame is drawn.
///
/// # Errors
/// An invalid diagnostic when asked for the GPU or fast quality; an I/O one
/// when the folder holds no record or a file cannot be read; an invalid one
/// when `sample.json` or `scene.json` is not the file its record names
/// ([`open_replay`]); an out-of-range one for an index past the sample's
/// frames; or as [`Session::replayer`] and the draws.
pub fn run(sample: &Path, args: &Args<'_>) -> Result<ExitCode> {
    refuse_other_drawing(args.quality, args.gpu)?;
    let Replayed { replay, scene, .. } = open_replay(sample)?;
    let indices: BTreeSet<usize> = args.frames.iter().copied().collect();
    for &index in &indices {
        replay.time(index)?;
    }
    let session = Session::open_text(sample.join(SCENE), scene)?;
    let replayer = session.replayer(&replay)?;
    for index in indices {
        let (drawn, ms) = replayer.draw_frame(index)?;
        let (width, height) = (drawn.ours.width(), drawn.ours.height());
        let line = Line {
            index,
            ours: pixels_hash(&drawn.ours),
            coverage: mask_hash(&drawn.coverage, width, height),
            shown: (drawn.shown.as_ref()).map(|mask| mask_hash(mask, width, height)),
            ms,
        };
        let text = serde_json::to_string(&line).map_err(|e| {
            Diagnostic::new(Kind::Invalid, format!("cannot write a frame's line: {e}"))
        })?;
        println!("{text}");
    }
    Ok(ExitCode::SUCCESS)
}

/// Refuses to draw other than a fidelity run draws: on the GPU, or at fast
/// quality.
///
/// # Errors
/// An invalid diagnostic naming what was asked.
fn refuse_other_drawing(quality: Quality, gpu: bool) -> Result<()> {
    let asked = match (gpu, quality) {
        (true, _) => Some("`--gpu`"),
        (false, Quality::Fast) => Some("`--quality fast`"),
        (false, Quality::Precise) => None,
    };
    match asked {
        Some(asked) => Err(Diagnostic::new(
            Kind::Invalid,
            format!(
                "a recorded frame is drawn as its fidelity run drew it, on the CPU at precise quality, and {asked} draws it otherwise"
            ),
        )
        .expected("`nakkash frames` without `--gpu`, at `--quality precise`")),
        None => Ok(()),
    }
}
