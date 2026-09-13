//! Audio playback through libmpv.
//!
//! mpv runs the whole network and decode path: ranged chunked reads, reconnection, seeking inside
//! a remote stream, Opus decode and gapless transitions. We only hand it URLs and read back state.
//!
//! State is *polled* rather than delivered through mpv's event context. We already repaint a few
//! times a second while playing, so a handful of property reads on those frames is cheaper and far
//! simpler than running an event thread and a second channel.

use std::sync::Arc;

use anyhow::{Context, Result};
use libmpv2::Mpv;

use crate::model::Track;

/// A snapshot of what mpv is doing, read once per frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerState {
    pub position: f64,
    pub duration: f64,
    pub paused: bool,
    /// True when mpv has nothing loaded — used to advance the queue.
    pub idle: bool,
}

pub struct Player {
    /// Built on the first `handle()`, not at startup. An idle mpv context still maps libav*
    /// and runs its own playloop/demuxer/ao threads (~10–15 MB); a session that only browses
    /// never needs any of it. `None` also means "nothing has ever been loaded", which is why
    /// `state()` reports idle in that case — the same as a fresh eager context did.
    mpv: Option<Arc<Mpv>>,
    volume: f64,
    /// Set if the one-time libmpv init failed. Surfaced exactly where an eager failure was —
    /// `App::player_error` and the sidebar's "audio unavailable" note — just at first play
    /// instead of at startup.
    init_error: Option<String>,
}

impl Default for Player {
    fn default() -> Self {
        Self::new()
    }
}

impl Player {
    pub fn new() -> Self {
        Self {
            mpv: None,
            volume: 70.0,
            init_error: None,
        }
    }

    fn init() -> Result<Mpv> {
        Mpv::with_initializer(|init| {
            // Audio only: never spend bandwidth or decode time on video.
            init.set_property("vid", "no")?;
            init.set_property("audio-display", "no")?;
            // Gapless needs the next file appended before the current one ends.
            init.set_property("gapless-audio", "yes")?;
            // Let mpv buffer ahead. It reads in ranged chunks by default, which YouTube
            // requires — a whole-file request is refused outright, and 1MiB ranges are too.
            init.set_property("cache", "yes")?;
            init.set_property("demuxer-max-bytes", "32MiB")?;
            // Already-played audio mpv keeps around for a backward seek. The default is
            // ~47 MiB held for the life of the file; a music player seeks back rarely and a
            // miss just re-reads from the network, which the forward cache already does past
            // its own edge. 4 MiB is a few minutes of Opus — plenty for a scrub-back.
            init.set_property("demuxer-max-back-bytes", "4MiB")?;
            // We resolve stream URLs ourselves via refryt-resolve; mpv must not invoke yt-dlp.
            init.set_property("ytdl", "no")?;
            init.set_property("terminal", "no")?;
            Ok(())
        })
        .context("could not initialise libmpv")
    }

    /// A handle safe to move into a background task, initialising libmpv on the first call.
    /// `Mpv` is `Send + Sync`, so playback can be started from whichever thread finished
    /// resolving. `None` (with [`Self::init_error`] set) if libmpv could not be brought up.
    pub fn handle(&mut self) -> Option<Arc<Mpv>> {
        if self.mpv.is_none() && self.init_error.is_none() {
            match Self::init() {
                Ok(mpv) => self.mpv = Some(Arc::new(mpv)),
                Err(e) => {
                    let msg = format!("{e:#}");
                    tracing::error!(error = %msg, "libmpv initialisation failed");
                    self.init_error = Some(msg);
                }
            }
        }
        self.mpv.clone()
    }

    /// The message from a failed one-time init, if any.
    pub fn init_error(&self) -> Option<&str> {
        self.init_error.as_deref()
    }

    pub fn state(&self) -> PlayerState {
        let Some(mpv) = &self.mpv else {
            // Nothing loaded — idle, so the queue-advance check behaves as it did before a
            // context was created eagerly.
            return PlayerState {
                idle: true,
                ..PlayerState::default()
            };
        };
        PlayerState {
            position: mpv.get_property("time-pos").unwrap_or(0.0),
            duration: mpv.get_property("duration").unwrap_or(0.0),
            paused: mpv.get_property("pause").unwrap_or(false),
            idle: mpv.get_property("idle-active").unwrap_or(true),
        }
    }

    pub fn set_paused(&self, paused: bool) {
        if let Some(mpv) = &self.mpv {
            let _ = mpv.set_property("pause", paused);
        }
    }

    pub fn toggle_pause(&self) {
        self.set_paused(!self.state().paused);
    }

    pub fn seek(&self, seconds: f64) {
        if let Some(mpv) = &self.mpv {
            let _ = mpv.command("seek", &[&format!("{seconds:.3}"), "absolute"]);
        }
    }

    pub fn volume(&self) -> f64 {
        self.volume
    }

    pub fn set_volume(&mut self, volume: f64) {
        self.volume = volume.clamp(0.0, 100.0);
        if let Some(mpv) = &self.mpv {
            let _ = mpv.set_property("volume", self.volume);
        }
    }
}

/// Hand a resolved URL to mpv and start playing it.
///
/// Separate from `Player` so it can be called from a background task that owns only a handle.
pub fn load(
    mpv: &Mpv,
    url: &str,
    volume: f64,
    loudness_db: Option<f32>,
    normalise: bool,
) -> Result<()> {
    // Apply YouTube's own loudness measurement so tracks don't jump in level. mpv's `volume`
    // is a percentage, so the correction is folded into it rather than set as a filter.
    let gain = match (normalise, loudness_db) {
        (true, Some(db)) => 10f64.powf(-f64::from(db) / 20.0).clamp(0.25, 2.0),
        _ => 1.0,
    };
    let _ = mpv.set_property("volume", (volume * gain).clamp(0.0, 130.0));
    let _ = mpv.set_property("pause", false);
    mpv.command("loadfile", &[url, "replace"])
        .context("mpv refused the stream URL")
}

/// What the player bar shows about the track currently loaded.
#[derive(Clone, Debug, Default)]
pub struct NowPlaying {
    pub track: Option<Track>,
    /// Set while we are resolving, so the UI can say so instead of looking frozen.
    pub resolving: bool,
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    /// Loudness normalisation must never mute a track or blow one out, whatever YouTube reports.
    #[test]
    fn loudness_gain_stays_within_sane_bounds() {
        for db in [-40.0f32, -12.0, -3.0, 0.0, 6.0, 40.0] {
            let gain = 10f64.powf(-f64::from(db) / 20.0).clamp(0.25, 2.0);
            assert!((0.25..=2.0).contains(&gain), "db {db} produced gain {gain}");
        }
    }

    #[test]
    fn a_quiet_track_is_boosted_and_a_loud_one_cut() {
        let quiet = 10f64.powf(6.0 / 20.0).clamp(0.25, 2.0); // loudness_db = -6
        let loud = 10f64.powf(-6.0 / 20.0).clamp(0.25, 2.0); // loudness_db = +6
        assert!(quiet > 1.0);
        assert!(loud < 1.0);
    }
}
