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
            // We resolve stream URLs ourselves via refryt-resolve, so mpv must not invoke
            // yt-dlp for them — direct `googlevideo.com` URLs never match ytdl_hook's
            // `youtube.com` patterns, so enabling the hook costs nothing there. The in-app
            // music video *does* need it, and mpv reads `ytdl` and the hook's own path only
            // here, at init: setting them later is silently ignored, which is exactly why the
            // video view never started until the hook was armed once, up front.
            init.set_property("ytdl", "yes")?;
            init.set_property(
                "script-opts",
                format!("ytdl_hook-ytdl_path={}", crate::backend::yt_dlp_program()),
            )?;
            init.set_property("terminal", "no")?;
            // Diagnostic hatch for the video path, whose failures mpv reports only in its own
            // log: point `REFRYT_MPV_LOG` at a file to capture it.
            if let Ok(path) = std::env::var("REFRYT_MPV_LOG") {
                let _ = init.set_property("log-file", path);
                let _ = init.set_property("msg-level", "all=v");
            }
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
    // Audio-only again, in case the video view just handed playback back. `vo` stays `libmpv`
    // from the video path, but with `vid=no` there is no video for it to initialise.
    let _ = mpv.set_property("vid", "no");
    mpv.command("loadfile", &[url, "replace"])
        .context("mpv refused the stream URL")
}

/// Load a music video. Unlike [`load`], mpv fetches it itself through its yt-dlp hook — armed
/// once in [`Player::init`], since mpv ignores `ytdl` set any later. The app's resolver only
/// returns audio-only streams, so the video has to come from the hook.
pub fn load_video(mpv: &Mpv, watch_url: &str, volume: f64) -> Result<()> {
    // The software render context was created before this call, and `libmpv` is the vo that
    // hands frames to it. Without this the file plays into nothing and every frame is black.
    let _ = mpv.set_property("vo", "libmpv");
    let _ = mpv.set_property("vid", "auto");
    let _ = mpv.set_property("audio-display", "no");
    let _ = mpv.set_property("volume", volume.clamp(0.0, 130.0));
    let _ = mpv.set_property("pause", false);
    mpv.command("loadfile", &[watch_url, "replace"])
        .context("mpv refused the video URL")
}

/// What the player bar shows about the track currently loaded.
#[derive(Clone, Debug, Default)]
pub struct NowPlaying {
    pub track: Option<Track>,
    /// Set while we are resolving, so the UI can say so instead of looking frozen.
    pub resolving: bool,
    pub error: Option<String>,
    /// Kept in sync every frame from `PlayerState::paused` (see `App::ui`'s per-frame poll) —
    /// carried on `NowPlaying` rather than queried separately so every row/card that already
    /// takes `&NowPlaying` can tell "is my track the current one, and is it actually playing"
    /// without a second parameter threaded everywhere `now` already reaches.
    pub paused: bool,
    /// The playlist a card most recently started playing, and the video ids it fetched to do
    /// so. An album card can tell it's the one playing from `Track::album_id` alone — every
    /// track already carries it — but a playlist has no such back-reference on its tracks (the
    /// same song can sit in many playlists), so this is the only way a playlist card knows.
    /// Self-correcting the same way `App::queue_matches` already is: once `track` is no longer
    /// one of these ids, nothing checks this as a match, so it is never explicitly cleared —
    /// only ever replaced by the next card played this way. Carried forward (not reset) across
    /// every other `NowPlaying` rebuild, since it describes the queue's source, not this one
    /// track.
    pub playlist_playing: Option<(String, Vec<String>)>,
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
