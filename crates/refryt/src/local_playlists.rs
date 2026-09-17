//! Local playlists: client-only lists of local files, persisted to
//! `~/.config/refryt/local_playlists.json`.
//!
//! There is no server to round-trip through — a local playlist's contents are just paths on
//! this machine — so every mutation here is synchronous and immediate rather than going through
//! `Backend`/`Event` like a YouTube playlist edit does. `App` holds the loaded `Vec<LocalPlaylist>`
//! directly, mutates it in place, and calls [`save`] right after, mirroring how `settings::save`
//! is called from `App::update_settings`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LocalPlaylist {
    /// Always `"local:<token>"` — the same string a `PlaylistRef`/`Playlist` built from this
    /// carries as its `id`, and how `App::load_playlist` tells a local playlist apart from a
    /// YouTube one.
    pub id: String,
    pub title: String,
    /// In order. Reordering a local playlist is just reordering this `Vec` directly — there is
    /// no server-issued playlist-item id to track, unlike a YouTube playlist's `set_video_id`.
    pub track_paths: Vec<PathBuf>,
}

/// A process-local counter, folded into [`new_id`] alongside the clock so two playlists created
/// in the same nanosecond (plausible — creation is a plain synchronous call, not a network round
/// trip) still get distinct ids.
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh, unique-enough local playlist id. Not cryptographic — nothing here needs it to be,
/// only unique among playlists this app itself creates.
pub fn new_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("local:{nanos:x}-{seq:x}")
}

fn path() -> Result<std::path::PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "refryt")
        .context("could not find the config directory")?;
    Ok(dirs.config_dir().join("local_playlists.json"))
}

/// Load saved local playlists, falling back to an empty list for anything unreadable — a broken
/// file should not stop the app starting, same reasoning as `settings::load`.
pub fn load() -> Vec<LocalPlaylist> {
    let Ok(path) = path() else {
        return Vec::new();
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return Vec::new();
    };
    serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        tracing::warn!(error = %e, "local playlists file could not be parsed; starting empty");
        Vec::new()
    })
}

pub fn save(playlists: &[LocalPlaylist]) -> Result<()> {
    let path = path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("could not create {}", parent.display()))?;
    }
    let json = serde_json::to_vec_pretty(playlists).context("could not encode local playlists")?;
    crate::persist::write_json_atomic(&path, &json)
        .with_context(|| format!("could not write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_from_the_same_instant_are_still_distinct() {
        let a = new_id();
        let b = new_id();
        assert_ne!(a, b);
        assert!(a.starts_with("local:"));
    }
}
