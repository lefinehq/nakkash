//! Each frame's draws in a diff, on every thread a render draws on: the
//! coverage pass draws a frame's masks, the measure pass reads its
//! reference and draws our frame, each once.
//!
//! A run that records or reuses also computes the identities of the lists
//! each frame draws. A frame its reuse vouches for ([`Reusing::frame`]) is
//! read in place of its draws: its masks in the coverage pass, our frame in
//! the measure pass, each checked against its record. A run that records
//! writes each frame as it was drawn or read, with its record.

use std::path::Path;
use std::thread;
use std::time::Instant;

use nakkash_format::{Diagnostic, Kind, Result};
use nakkash_raster::{Frame, Image, RenderOptions, frame_identity};

use super::{DiffOptions, FrameDiff, Masks, Pair};
use crate::Session;
use crate::record::{
    Drawn, FrameRecord, Hex128, Identities, Times, mask_hash, pixels_hash, read_masks, read_ours,
    write_frame,
};

/// What a frame's coverage pass learnt of it, for its measure pass.
pub(crate) struct Origin<'a> {
    /// The identities of the lists the frame draws, when the run records
    /// or reuses.
    identities: Option<Identities>,
    /// Where to read our frame from in place of drawing it, when reused.
    reused: Option<Reused<'a>>,
    /// How long its draws took: the record's, when reused.
    ms: Times,
}

/// A frame read from a record in place of its draws.
#[derive(Clone, Copy)]
struct Reused<'a> {
    /// The record's folder.
    dir: &'a Path,
    /// The frame's record, which its files are checked against.
    record: &'a FrameRecord,
}

impl Session {
    /// Calls `each` on every frame's index and its own input from
    /// `inputs`, on as many threads as a render draws on (`--threads`,
    /// D90), and returns the results in frame order, or the first frame's
    /// error. Each thread enters the caller's tracing dispatcher, so its
    /// steps are timed and logged with the caller's.
    pub(crate) fn each_frame<I: Send, T: Send>(
        &self,
        inputs: Vec<I>,
        each: &(dyn Fn(usize, I) -> Result<T> + Sync),
    ) -> Result<Vec<T>> {
        let threads = crate::session::threads();
        let chunk = inputs.len().div_ceil(threads).max(1);
        let mut items = inputs.into_iter().enumerate().peekable();
        let mut parts: Vec<Vec<(usize, I)>> = Vec::new();
        while items.peek().is_some() {
            parts.push(items.by_ref().take(chunk).collect());
        }
        // The default dispatcher is per thread: a scoped one (a test's, or
        // the CLI's `--timings`) does not reach spawned threads by itself.
        let dispatch = &tracing::dispatcher::get_default(Clone::clone);
        let results: Vec<Vec<Result<T>>> = thread::scope(|scope| {
            let handles: Vec<_> = (parts.into_iter())
                .map(|part| {
                    scope.spawn(move || {
                        tracing::dispatcher::with_default(dispatch, || {
                            (part.into_iter())
                                .map(|(i, input)| each(i, input))
                                .collect::<Vec<_>>()
                        })
                    })
                })
                .collect();
            (handles.into_iter())
                .map(|handle| handle.join().expect("a diff thread does not panic"))
                .collect()
        });
        // The parts are runs of frames in order, joined in order.
        results.into_iter().flatten().collect()
    }

    /// The coverage pass of frame `index`, at `time` seconds: where the
    /// scene draws then, and where `shown` draws, if given; each its bare
    /// list drawn once, or read from the record `options` reuse when it
    /// vouches for the frame.
    pub(crate) fn cover<'a>(
        &self,
        shown: Option<&Session>,
        options: &'a DiffOptions,
        index: usize,
        time: f64,
    ) -> Result<(Masks, Origin<'a>)> {
        let frame = time * self.scene().root.fps;
        let identity = self.identity()?;
        let list = self.display_list(frame)?;
        let identify = options.record.is_some() || options.reuse.is_some();
        let ours = identify.then(|| identity(&list));
        let measured = bare(list);
        let shown = (shown.map(|session| session.display_list(frame).map(bare))).transpose()?;
        let identities = ours.map(|ours| Identities {
            ours,
            bare: identity(&measured),
            shown: shown.as_ref().map(&identity),
        });
        let reused = match (&options.reuse, &identities) {
            (Some(reusing), Some(fresh)) => {
                (reusing.frame(index, fresh, time)?).map(|record| Reused {
                    dir: reusing.dir(),
                    record,
                })
            }
            _ => None,
        };
        let ((coverage, shown), ms) = match reused {
            Some(reused) => {
                let (width, height) = self.canvas()?;
                let masks = read_masks(reused.dir, reused.record, width, height)?;
                (masks, reused.record.ms)
            }
            None => {
                let (coverage, coverage_ms) = timed(|| self.coverage(&measured))?;
                let (shown, shown_ms) = match &shown {
                    Some(list) => timed(|| self.coverage(list).map(Some))?,
                    None => (None, 0.0),
                };
                let ms = Times {
                    draw: 0.0,
                    coverage: coverage_ms,
                    shown: shown_ms,
                };
                ((coverage, shown), ms)
            }
        };
        let origin = Origin {
            identities,
            reused,
            ms,
        };
        Ok((Masks { coverage, shown }, origin))
    }

    /// Reference frame `index`, read once, and the scene at `time` seconds,
    /// drawn once or read where `origin` says, with the frame's `masks`
    /// from the coverage pass.
    pub(crate) fn pair<'a>(
        &self,
        options: &DiffOptions,
        time: f64,
        index: usize,
        masks: Masks,
        origin: Origin<'a>,
    ) -> Result<(Pair, Origin<'a>)> {
        let path = &options.frames[index];
        let bytes = std::fs::read(path).map_err(|e| Diagnostic::io("read", path, &e))?;
        let reference = Image::from_png(&bytes).map_err(|d| d.at(path.display().to_string()))?;
        let (width, height) = self.canvas()?;
        if (reference.width(), reference.height()) != (width, height) {
            return Err(Diagnostic::new(
                Kind::Invalid,
                format!(
                    "reference frame {} is {}x{}, the scene {width}x{height}",
                    path.display(),
                    reference.width(),
                    reference.height()
                ),
            )
            .expected("reference frames of the scene's size"));
        }
        let (ours, origin) = match origin.reused {
            Some(reused) => (read_ours(reused.dir, reused.record, width, height)?, origin),
            None => {
                let list = self.display_list(time * self.scene().root.fps)?;
                let (ours, draw) = timed(|| self.draw_reference(&list, RenderOptions::default()))?;
                let ms = Times { draw, ..origin.ms };
                (ours, Origin { ms, ..origin })
            }
        };
        let Masks { coverage, shown } = masks;
        let pair = Pair {
            ours,
            coverage,
            shown,
            reference,
        };
        Ok((pair, origin))
    }

    /// The identity of a list drawn on this scene's canvas as the
    /// reference draws it: what [`Session::draw_reference`] draws with
    /// [`RenderOptions::default`].
    fn identity(&self) -> Result<impl Fn(&Frame) -> Hex128> {
        let (width, height) = self.canvas()?;
        Ok(move |list: &Frame| {
            Hex128(frame_identity(
                list,
                width,
                height,
                RenderOptions::default(),
            ))
        })
    }

    /// Whether the bare list `bare` draws each pixel of the canvas.
    fn coverage(&self, bare: &Frame) -> Result<Vec<bool>> {
        Ok(self
            .draw_reference(bare, RenderOptions::default())?
            .pixels()
            .chunks_exact(4)
            .map(|p| p[3] > 0)
            .collect())
    }
}

impl Origin<'_> {
    /// Records frame `index` into `dir`, if given: its pixels and masks of
    /// `pair`, as files, and its record, with its errors `diff`.
    ///
    /// # Errors
    /// As [`write_frame`].
    pub(crate) fn record(
        self,
        dir: Option<&Path>,
        index: usize,
        pair: Pair,
        diff: &FrameDiff,
    ) -> Result<Option<FrameRecord>> {
        // A run that records always knows its frames' identities.
        let (Some(dir), Some(identities)) = (dir, self.identities) else {
            return Ok(None);
        };
        let Pair {
            ours,
            coverage,
            shown,
            ..
        } = pair;
        let drawn = Drawn {
            ours,
            coverage,
            shown,
        };
        write_frame(dir, index, &drawn)?;
        let (width, height) = (drawn.ours.width(), drawn.ours.height());
        let Identities { ours, bare, shown } = identities;
        Ok(Some(FrameRecord {
            index,
            file: diff.file.clone(),
            time: diff.time,
            ours,
            bare,
            shown,
            ours_pixels: pixels_hash(&drawn.ours),
            coverage: mask_hash(&drawn.coverage, width, height),
            shown_mask: (drawn.shown.as_ref()).map(|mask| mask_hash(mask, width, height)),
            diff: diff.clone(),
            ms: self.ms,
        }))
    }
}

/// `list` without its background: what the coverage draws.
fn bare(list: Frame) -> Frame {
    Frame {
        background: None,
        ..list
    }
}

/// What `draw` gives, and how long it took in milliseconds.
fn timed<T>(draw: impl FnOnce() -> Result<T>) -> Result<(T, f64)> {
    let start = Instant::now();
    let drawn = draw()?;
    Ok((drawn, start.elapsed().as_secs_f64() * 1000.0))
}

#[cfg(test)]
#[path = "../../tests/unit/diff/frames.rs"]
mod tests;
