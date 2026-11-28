//! Each frame's draws in a diff, on every thread a render draws on: the
//! coverage pass draws a frame's masks, the measure pass reads its
//! reference and draws our frame, each once.
//!
//! A run that records or reuses also computes the identities of the lists
//! each frame draws. A frame its reuse vouches for ([`Reusing::frame`]) is
//! read in place of its draws: its masks in the coverage pass, our frame in
//! the measure pass, each checked against its record. A run that records
//! writes each frame as it was drawn or read, with its record.
//!
//! A frame's draws are two: its masks ([`Session::draw_cover`]) and our
//! frame ([`Session::draw_ours`]). The diff draws them in its two passes,
//! and [`Replayer::draw_frame`] draws them one after the other, through the
//! same sessions, so `nakkash frames` draws a recorded frame as its run
//! drew it, bit for bit.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Instant;

use nakkash_format::{Diagnostic, Kind, Result};
use nakkash_raster::{Frame, Image, RenderOptions, frame_identity};

use super::{DiffOptions, FrameDiff, Masks, Pair};
use crate::Session;
use crate::record::{
    Drawn, FrameRecord, Hex128, Identities, Replay, Target, Times, mask_hash, pixels_hash,
    read_masks, read_ours, write_frame,
};

/// Frames drawn over the process: our frame and its masks.
static DRAWN: AtomicU64 = AtomicU64::new(0);

/// Frames read from a record in place of their draws, over the process.
static READ: AtomicU64 = AtomicU64::new(0);

/// How many frames this process drew, and how many it read from a record
/// in place of drawing them ([`frame_counts`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FrameCounts {
    /// Frames drawn: our frame and its masks.
    pub drawn: u64,
    /// Frames read from a record.
    pub read: u64,
}

/// How many frames fidelity diffs and replays drew over the process, and
/// how many they read from a record instead: what `--stats` says of a run
/// that reuses.
pub fn frame_counts() -> FrameCounts {
    FrameCounts {
        drawn: DRAWN.load(Ordering::Relaxed),
        read: READ.load(Ordering::Relaxed),
    }
}

/// A sample's scene opened to draw its frames as its fidelity run drew
/// them ([`Session::replayer`]): with its ignored layers hidden from what
/// is measured, and drawn as they are to show where they draw.
pub struct Replayer<'a> {
    /// The scene as it is.
    scene: &'a Session,
    /// The scene with the ignored layers hidden; none when none is ignored.
    hidden: Option<Session>,
    /// How the frames are drawn.
    replay: &'a Replay,
}

impl Session {
    /// This scene opened to draw the frames of `replay`, which it must have
    /// been recorded from.
    ///
    /// # Errors
    /// An invalid diagnostic when the scene's canvas is not the replay's;
    /// or as [`Session::graph`], or as an edit hiding a layer the scene
    /// lacks.
    pub fn replayer<'a>(&'a self, replay: &'a Replay) -> Result<Replayer<'a>> {
        let (width, height) = self.canvas()?;
        if (width, height) != (replay.width, replay.height) {
            return Err(Diagnostic::new(
                Kind::Invalid,
                format!(
                    "the sample was recorded on a {}x{} canvas, and its scene is {width}x{height}",
                    replay.width, replay.height
                ),
            )
            .expected("the scene the sample was recorded from"));
        }
        let hidden = match replay.ignore.is_empty() {
            true => None,
            false => Some(self.hiding(&replay.ignore)?),
        };
        Ok(Replayer {
            scene: self,
            hidden,
            replay,
        })
    }
}

impl Replayer<'_> {
    /// The scene measured: with the ignored layers hidden.
    pub(crate) fn measured(&self) -> &Session {
        self.hidden.as_ref().unwrap_or(self.scene)
    }

    /// The scene that shows where the ignored layers draw; none when none
    /// is ignored.
    pub(crate) fn shown(&self) -> Option<&Session> {
        self.hidden.as_ref().map(|_| self.scene)
    }

    /// Draws frame `index` as the fidelity run drew it, on the CPU at
    /// precise quality: its masks, then our frame, each once, through the
    /// draws the diff makes; and how long each took.
    ///
    /// # Errors
    /// An out-of-range diagnostic for an index past the sample's frames
    /// ([`Replay::time`]), or as [`Session::render_with`].
    pub fn draw_frame(&self, index: usize) -> Result<(Drawn, Times)> {
        let time = self.replay.time(index)?;
        let measured = self.measured();
        let frame = time * measured.scene().root.fps;
        let (Masks { coverage, shown }, ms) = measured.draw_cover(self.shown(), frame)?;
        let (ours, draw) = measured.draw_ours(time)?;
        let drawn = Drawn {
            ours,
            coverage,
            shown,
        };
        Ok((drawn, Times { draw, ..ms }))
    }
}

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
    /// vouches for the frame. A run that neither records nor reuses
    /// computes no identity, and builds and draws in the order it always
    /// has, so it fails as it always has.
    pub(crate) fn cover<'a>(
        &self,
        shown: Option<&Session>,
        options: &'a DiffOptions,
        index: usize,
        time: f64,
    ) -> Result<(Masks, Origin<'a>)> {
        let frame = time * self.scene().root.fps;
        match options.record.is_some() || options.reuse.is_some() {
            false => {
                let (masks, ms) = self.draw_cover(shown, frame)?;
                let origin = Origin {
                    identities: None,
                    reused: None,
                    ms,
                };
                Ok((masks, origin))
            }
            true => {
                let list = self.display_list(frame)?;
                let identity = self.identity()?;
                let ours = identity(&list);
                let measured = bare(list);
                let shown = bare_list(shown, frame)?;
                let identities = Identities {
                    ours,
                    bare: identity(&measured),
                    shown: shown.as_ref().map(&identity),
                };
                let reused = match &options.reuse {
                    Some(reusing) => {
                        (reusing.frame(index, &identities, time)?).map(|record| Reused {
                            dir: reusing.dir(),
                            record,
                        })
                    }
                    None => None,
                };
                let (masks, ms) = match reused {
                    Some(reused) => {
                        let (width, height) = self.canvas()?;
                        let (coverage, shown) =
                            read_masks(reused.dir, reused.record, width, height)?;
                        (Masks { coverage, shown }, reused.record.ms)
                    }
                    None => self.draw_masks(&measured, || Ok(shown))?,
                };
                let origin = Origin {
                    identities: Some(identities),
                    reused,
                    ms,
                };
                Ok((masks, origin))
            }
        }
    }

    /// The masks of the frame at `frame` drawn, and how long they took:
    /// where this scene draws then, and where `shown` draws, if given, each
    /// its bare list, built and drawn in turn.
    fn draw_cover(&self, shown: Option<&Session>, frame: f64) -> Result<(Masks, Times)> {
        let list = self.display_list(frame)?;
        self.draw_masks(&bare(list), || bare_list(shown, frame))
    }

    /// Our frame at `time` seconds drawn as the reference draws it, and how
    /// long it took, in milliseconds; counted as a frame drawn.
    fn draw_ours(&self, time: f64) -> Result<(Image, f64)> {
        let list = self.display_list(time * self.scene().root.fps)?;
        let drawn = timed(|| self.draw_reference(&list, RenderOptions::default()))?;
        DRAWN.fetch_add(1, Ordering::Relaxed);
        Ok(drawn)
    }

    /// The masks of a frame drawn, and how long they took: the coverage of
    /// the bare list `measured`, then the bare list `shown` gives, if any.
    fn draw_masks(
        &self,
        measured: &Frame,
        shown: impl FnOnce() -> Result<Option<Frame>>,
    ) -> Result<(Masks, Times)> {
        let (coverage, coverage_ms) = timed(|| self.coverage(measured))?;
        let (shown, shown_ms) = match shown()? {
            Some(list) => timed(|| self.coverage(&list).map(Some))?,
            None => (None, 0.0),
        };
        let ms = Times {
            draw: 0.0,
            coverage: coverage_ms,
            shown: shown_ms,
        };
        Ok((Masks { coverage, shown }, ms))
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
            Some(reused) => {
                let ours = read_ours(reused.dir, reused.record, width, height)?;
                READ.fetch_add(1, Ordering::Relaxed);
                (ours, origin)
            }
            None => {
                let (ours, draw) = self.draw_ours(time)?;
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
    /// Records frame `index` into `target`, if given: its pixels and masks
    /// of `pair`, as files within its quota, and its record, with its
    /// errors `diff`.
    ///
    /// # Errors
    /// As [`write_frame`].
    pub(crate) fn record(
        self,
        target: Option<&Target>,
        index: usize,
        pair: Pair,
        diff: &FrameDiff,
    ) -> Result<Option<FrameRecord>> {
        // A run that records always knows its frames' identities.
        let (Some(target), Some(identities)) = (target, self.identities) else {
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
        write_frame(&target.dir, index, &drawn, &target.quota)?;
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

/// The bare list `shown` draws at `frame`, if given: where the ignored
/// layers draw.
fn bare_list(shown: Option<&Session>, frame: f64) -> Result<Option<Frame>> {
    (shown.map(|session| session.display_list(frame).map(bare))).transpose()
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
