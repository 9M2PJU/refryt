//! Application state and the per-frame update.
//!
//! Rendering lives in `crate::ui`, keyed off a [`Route`] with a browser-style history stack
//! (Back / Forward in the titlebar). This file holds the state and the wiring: what the route
//! currently is, what has been fetched for it, and how playback advances.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::Color32;

use crate::backend::{Backend, Event};
use crate::chrome;
use crate::images::Images;
use crate::model::{Entry, LikeStatus, SearchResults, SearchScope, Track};
use crate::player::{NowPlaying, Player};
use crate::stats::Stats;
use crate::theme::{self, Palette};
use crate::ui;
use crate::widgets;

/// Autocomplete is cheap and wants to feel immediate; search itself is never triggered by typing,
/// only by an explicit submit — see `App::submit_search`.
const SUGGEST_DEBOUNCE: Duration = Duration::from_millis(120);

/// How long the crossfade ramps, at each end.
const CROSSFADE_SECS: f64 = 5.0;

/// The crossfade volume factor: 1.0 at full volume, dipping to 0 at the very end of a track and
/// ramping from 0 at the start of the next; the lower of the two wins. Pure, so the curve is
/// testable without a player.
fn crossfade_factor(
    active: bool,
    position: f64,
    duration: f64,
    fade_in_elapsed: Option<f64>,
) -> f64 {
    let out = if active && duration > 0.0 {
        let remaining = duration - position;
        if remaining < CROSSFADE_SECS {
            (remaining / CROSSFADE_SECS).clamp(0.0, 1.0)
        } else {
            1.0
        }
    } else {
        1.0
    };
    let inn = fade_in_elapsed
        .map(|e| (e / CROSSFADE_SECS).clamp(0.0, 1.0))
        .unwrap_or(1.0);
    out.min(inn)
}

/// While audio is playing the progress bar needs to move. Four times a second is enough to look
/// continuous and is two orders of magnitude cheaper than repainting every frame.
const PLAYING_REPAINT: Duration = Duration::from_millis(250);

/// How many "previous" steps the transport remembers within one queue. Enough to retrace a few
/// songs, not the whole session. The entries are `(index, queue length)` — cheap — because the
/// songs themselves are still in the queue; only the autoplay tail added since needs dropping.
const PLAY_HISTORY: usize = 12;
/// How many replaced queues are kept for `Previous` to bring back, so starting an album does not
/// erase the playlist you were in.
const QUEUE_HISTORY: usize = 5;

/// A step back within the current queue: where the transport was, and how long the queue was.
/// Restoring truncates any autoplay tail appended after that point.
struct PlayPoint {
    index: usize,
    queue_len: usize,
}

/// The queue in force before another replaced it, for `Previous` across a queue change.
struct QueuePoint {
    queue: Vec<Track>,
    index: usize,
    radio: Option<Radio>,
}

/// Where [`App::previous`] should rewind to.
enum Rewind {
    /// Within the current queue: drop anything past `queue_len`, then play `index`.
    WithinQueue { index: usize, queue_len: usize },
    /// A queue that was replaced: restore it wholesale.
    PreviousQueue {
        queue: Vec<Track>,
        index: usize,
        radio: Option<Radio>,
    },
}

/// What the transport does at the end of the queue.
/// The next index in a queue when not shuffling: the following track, wrapping to the start only
/// under Repeat All. `None` means the transport stops (or, for Repeat One, replays).
fn sequential_next(current: usize, len: usize, repeat_all: bool) -> Option<usize> {
    if current + 1 < len {
        Some(current + 1)
    } else if repeat_all && len > 0 {
        Some(0)
    } else {
        None
    }
}

/// A seed for the shuffle LCG: the clock mixed with the process id, so two starts differ.
fn rng_seed() -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9e3779b9);
    nanos ^ (std::process::id() as u64).wrapping_mul(0x2545f4914f6cdd1d)
}

/// Stable-sort playlists into the saved id order. Ids the saved list does not name keep their
/// relative order, after the named ones — which is where a newly created playlist belongs.
fn order_by_saved(items: &mut [crate::model::PlaylistRef], order: &[String]) {
    if order.is_empty() {
        return;
    }
    let rank: std::collections::HashMap<&str, usize> = order
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), i))
        .collect();
    items.sort_by_key(|p| rank.get(p.id.as_str()).copied().unwrap_or(usize::MAX));
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum RepeatMode {
    /// Play the queue once and stop.
    #[default]
    Off,
    /// Wrap to the start when the queue ends.
    All,
    /// Replay the current track when it ends.
    One,
}

/// What confirming the new-playlist modal actually creates.
#[derive(Default)]
pub(crate) enum NewPlaylistKind {
    #[default]
    YouTube,
    /// Seeded with a specific set of local files — the Library's Local tab's "Save as
    /// playlist," for whichever folder was open when it was pressed.
    Local {
        track_paths: Vec<std::path::PathBuf>,
    },
}

/// The playlist rename modal's live fields.
pub(crate) struct PlaylistEditor {
    pub id: String,
    pub title: String,
    pub description: String,
    /// The title before editing, so a refused save can be reverted.
    pub original_title: String,
}

/// The transport's hidden "previous" history: recent positions in the current queue, and whole
/// queues replaced since. Bounded on both ends so a long session cannot grow it without limit.
///
/// Split out from [`App`] so the rewind rules are unit-testable without an egui context.
#[derive(Default)]
struct PlayHistory {
    played: VecDeque<PlayPoint>,
    queues: VecDeque<QueuePoint>,
}

impl PlayHistory {
    /// Remember the track being left. Called for a forward move only — not for `Previous`, or
    /// the two would ping-pong.
    fn remember(&mut self, index: usize, queue_len: usize) {
        self.played.push_back(PlayPoint { index, queue_len });
        while self.played.len() > PLAY_HISTORY {
            self.played.pop_front();
        }
    }

    /// One queue is being replaced by another: keep the whole old queue, and clear the
    /// within-queue points, which index the queue now gone.
    fn leaving_queue(&mut self, queue: Vec<Track>, index: usize, radio: Option<Radio>) {
        self.queues.push_back(QueuePoint {
            queue,
            index,
            radio,
        });
        while self.queues.len() > QUEUE_HISTORY {
            self.queues.pop_front();
        }
        self.played.clear();
    }

    /// The next restore point, newest first: a position in this queue, else the queue before it.
    fn rewind(&mut self) -> Option<Rewind> {
        if let Some(point) = self.played.pop_back() {
            return Some(Rewind::WithinQueue {
                index: point.index,
                queue_len: point.queue_len,
            });
        }
        self.queues.pop_back().map(|point| Rewind::PreviousQueue {
            queue: point.queue,
            index: point.index,
            radio: point.radio,
        })
    }
}

/// Where the app is looking right now. `Search` carries no payload: the editable query lives in
/// `App::query`, so the box in the titlebar and the route never disagree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    Home,
    Search,
    /// What YouTube puts where Search used to be in the rail: new releases, moods, videos.
    Explore,
    /// Explore's first button: new albums & singles.
    NewReleases,
    /// Explore's second button: charts. The params are what the Explore feed sent with its
    /// Charts button, and a reply for params since changed is stale.
    Charts(Option<String>),
    /// Explore's third button: every mood category.
    Moods,
    /// What you played, in dated shelves. Signed in only.
    History,
    Library,
    Settings,
    Artist(String),
    Album(String),
    Playlist(String),
    /// A mood's playlists. The payload is the opaque params string from `MoodRef`.
    Mood(String),
}

impl Route {
    /// Parse a `--page` argument.
    ///
    /// The payload-free pages by name, and the rest as `kind:id` — `artist:UC…`, `album:MPRE…`,
    /// `playlist:VL…`. Those cannot be reached any other way from outside, and they are the pages
    /// whose headers are hardest to check without clicking.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "home" => Some(Route::Home),
            "search" => Some(Route::Search),
            "explore" => Some(Route::Explore),
            "releases" => Some(Route::NewReleases),
            "charts" => Some(Route::Charts(None)),
            "moods" => Some(Route::Moods),
            "history" => Some(Route::History),
            "library" => Some(Route::Library),
            "settings" => Some(Route::Settings),
            other => match other.split_once(':') {
                Some(("artist", id)) => Some(Route::Artist(id.to_owned())),
                Some(("album", id)) => Some(Route::Album(id.to_owned())),
                Some(("playlist", id)) => Some(Route::Playlist(id.to_owned())),
                _ => None,
            },
        }
    }
}

/// Which section the Library page shows.
///
/// All four come from `App::library`, which one fetch already fills, so switching costs no
/// request. It used to drive the sidebar rail, which is how the rail ended up duplicating the
/// page it points at — and saying "Library" twice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RailFilter {
    #[default]
    Playlists,
    Songs,
    Albums,
    Artists,
    /// Folders scanned from `settings.local_folders`, browsed as a tree rather than paged from
    /// the server like the other four.
    Local,
    /// Songs saved to `~/Music/refryt` — local, but still real YouTube tracks, so unlike `Local`
    /// they play through the resolver's download shortcut and scrobble.
    Downloads,
}

impl RailFilter {
    pub const ALL: [Self; 6] = [
        Self::Playlists,
        Self::Songs,
        Self::Albums,
        Self::Artists,
        Self::Local,
        Self::Downloads,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Playlists => "Playlists",
            Self::Songs => "Songs",
            Self::Albums => "Albums",
            Self::Artists => "Artists",
            Self::Local => "Local",
            Self::Downloads => "Downloads",
        }
    }
}

/// Song ordering for the library page — exactly the orders YouTube can perform, applied by the
/// server across the whole library.
///
/// Sorting by artist or album used to be offered and could only ever reorder the loaded page: 25
/// rows of a 689-row library, presented as a sort of the library. YouTube has no param for those,
/// so they are gone rather than wrong.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LibrarySort {
    /// Recently saved, which is also YouTube's default order.
    #[default]
    Latest,
    NameAsc,
    NameDesc,
}

impl LibrarySort {
    pub(crate) fn label(self) -> &'static str {
        match self {
            LibrarySort::Latest => "Latest",
            LibrarySort::NameAsc => "A–Z",
            LibrarySort::NameDesc => "Z–A",
        }
    }

    fn param(self) -> crate::api::LibrarySortParam {
        match self {
            LibrarySort::Latest => crate::api::LibrarySortParam::RecentlySaved,
            LibrarySort::NameAsc => crate::api::LibrarySortParam::NameAsc,
            LibrarySort::NameDesc => crate::api::LibrarySortParam::NameDesc,
        }
    }
}

/// Track ordering for a playlist page — unlike [`LibrarySort`], never a server param: YouTube's
/// playlist endpoints (`Api::playlist`/`playlist_more`) take no sort at all, so every one of
/// these is a client-side re-sort of the tracks already loaded. Title and duration only — an
/// "artist" order would need to pick one artist off a possibly-multi-artist credit, and "date
/// added" isn't offered because `Track` carries no such timestamp. `Reverse` is the honest
/// stand-in for that: YouTube serves an ordinary playlist oldest-added-first, so flipping the
/// served order is a free "newest first" for those — but it is labelled as what it actually
/// does, since an algorithmic playlist (a radio mix, say) has no "date added" for it to mean.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlaylistSort {
    /// The order YouTube served the playlist in.
    #[default]
    Default,
    /// The served order, flipped.
    Reverse,
    TitleAsc,
    TitleDesc,
    DurationAsc,
    DurationDesc,
}

impl PlaylistSort {
    pub(crate) fn label(self) -> &'static str {
        match self {
            PlaylistSort::Default => "Default",
            PlaylistSort::Reverse => "Reverse order",
            PlaylistSort::TitleAsc => "Title A–Z",
            PlaylistSort::TitleDesc => "Title Z–A",
            PlaylistSort::DurationAsc => "Shortest first",
            PlaylistSort::DurationDesc => "Longest first",
        }
    }

    /// Re-order `tracks` in place. Only ever called once every page is loaded — sorting a
    /// partial playlist would present a truncated list as the whole of it, exactly the mistake
    /// `LibrarySort`'s own doc comment describes YouTube's library sort avoiding by being a
    /// server param instead; a playlist has no such param to hand this to; see `sort_playlist`.
    ///
    /// `Default` and `Reverse` are never actually reached here — [`apply_playlist_sort`]
    /// intercepts both to restore the served order from its own snapshot instead, since this
    /// method only knows how to build a *new* order and has nowhere to restore an old one from.
    /// The arms stay as harmless no-ops rather than `unreachable!()`, so a future direct caller
    /// gets "did nothing" instead of a panic.
    fn apply(self, tracks: &mut [crate::model::Track]) {
        match self {
            PlaylistSort::Default | PlaylistSort::Reverse => {}
            PlaylistSort::TitleAsc => tracks.sort_by_key(|t| t.title.to_lowercase()),
            PlaylistSort::TitleDesc => {
                tracks.sort_by_key(|t| std::cmp::Reverse(t.title.to_lowercase()))
            }
            PlaylistSort::DurationAsc => tracks.sort_by_key(|t| t.duration.unwrap_or(0)),
            PlaylistSort::DurationDesc => {
                tracks.sort_by_key(|t| std::cmp::Reverse(t.duration.unwrap_or(0)))
            }
        }
    }
}

/// Browser-style navigation history.
///
/// Split out from `App` because it is the one piece of navigation that can be tested without an
/// egui context — and it is where the Back/Forward bug lived.
#[derive(Debug)]
pub(crate) struct History {
    entries: Vec<Route>,
    cursor: usize,
}

impl History {
    fn new(initial: Route) -> Self {
        Self {
            entries: vec![initial],
            cursor: 0,
        }
    }

    fn current(&self) -> &Route {
        &self.entries[self.cursor]
    }

    /// Push a route, dropping anything ahead of the cursor first. Returns false when the route
    /// is already on screen, so callers can skip the reload.
    fn push(&mut self, route: Route) -> bool {
        if *self.current() == route {
            return false;
        }
        self.entries.truncate(self.cursor + 1);
        self.entries.push(route);
        self.cursor = self.entries.len() - 1;
        true
    }

    fn can_go_back(&self) -> bool {
        self.cursor > 0
    }

    fn can_go_forward(&self) -> bool {
        self.cursor + 1 < self.entries.len()
    }

    fn back(&mut self) -> bool {
        if !self.can_go_back() {
            return false;
        }
        self.cursor -= 1;
        true
    }

    fn forward(&mut self) -> bool {
        if !self.can_go_forward() {
            return false;
        }
        self.cursor += 1;
        true
    }
}

/// What the command line asked for. Grouped so `App::new` takes one argument rather than eight.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Hand the window back to the OS instead of drawing our own chrome.
    pub decorated: bool,
    /// Print the time to first painted frame and exit.
    pub startup_timing: bool,
    /// Start on Search with this query.
    pub search: Option<String>,
    /// Play the first result once that search lands.
    pub autoplay: bool,
    /// Show the live counter overlay.
    pub stats: bool,
    /// Print one line of counters after this long, then exit.
    pub stats_after: Option<Duration>,
    /// Start on a named page, overriding the default of Home.
    pub page: Option<Route>,
    /// Scroll the current page steadily downward. A test aid: paging is driven by the viewport
    /// reaching the end of a list, and there is no way to reproduce that from the outside on
    /// Wayland.
    pub autoscroll: bool,
    /// Open the queue panel at startup. A test aid, like `autoscroll`: the panel is opened by a
    /// click in the player bar, and Wayland offers no way to synthesise one from outside.
    pub queue: bool,
    /// Open the lyrics panel at startup, the same kind of test aid as `queue`.
    pub lyrics: bool,
    /// Override the saved scheme — and optionally its flavour — for this run, without touching
    /// `settings.json`. A test aid, for screenshotting the whole matrix without clicking
    /// through Settings.
    pub scheme: Option<(crate::theme::Scheme, Option<String>)>,
    /// Override the saved opacity for this run, same rule as `scheme`.
    pub opacity: Option<crate::settings::Opacity>,
    /// Never bind the KDE blur protocol this run. Not a test aid but a safety hatch: a Wayland
    /// protocol error there is fatal to the whole connection, and `blur` persists — a session
    /// that dies on it needs a way back into Settings that does not depend on Settings.
    pub no_blur: bool,
    /// Turn Autoplay off for this run without touching `settings.json`. A test aid, like
    /// `scheme`: the switch lives in Settings, and a comparison run needs the off state without
    /// clicking through to it.
    pub no_autoplay: bool,
}

/// How far a river has flowed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum River {
    /// A hand-picked queue's latent river: no page yet, seeded from the queue's last track.
    Pending,
    /// Flowing: the `nextRadioContinuationData` token that extends it.
    Open(String),
    /// Ran dry or failed. Never fetched again: a dead continuation is not retried, and a page
    /// that adds nothing would leave the low-water mark tripped forever.
    Dead,
}

/// Autoplay river state. Every queue opens one: Start Mix and an artist's Shuffle/Mix open it
/// flowing, a hand-picked queue opens it pending (M21). The Autoplay setting decides whether it
/// is ever asked for a page; what it delivered stays either way.
#[derive(Clone, Debug)]
pub(crate) struct Radio {
    pub(crate) state: River,
    /// The video id a pending river seeds from — the queue's last track — and the reseed
    /// fallback once flowing.
    pub(crate) seed: String,
    /// The video ids the listener chose: the queue they started plus anything queued by hand.
    /// Tracks not in this list are the algorithm's and carry a "Radio" marker in Up Next.
    /// Ids, not indices — inserting into the queue shifts indices, and ids do not.
    pub(crate) chosen: Vec<String>,
}

impl Radio {
    /// The latent river for a hand-picked queue. `None` only for an empty queue.
    fn pending(tracks: &[Track]) -> Option<Self> {
        Some(Self {
            state: River::Pending,
            seed: nearest_song_id(tracks, tracks.len().checked_sub(1)?)?,
            chosen: tracks.iter().map(|t| t.video_id.clone()).collect(),
        })
    }

    /// Where the river's tail begins: the first queued track the listener did not choose.
    /// `None` while every track is theirs.
    fn tail_start(&self, queue: &[Track]) -> Option<usize> {
        queue
            .iter()
            .position(|t| !self.chosen.iter().any(|id| id == &t.video_id))
    }
}

/// The nearest song (not video) at or before `from`, falling back to whatever sits at `from`
/// if the scanned run is video all the way back — seeding or reseeding the radio from a video
/// makes YouTube's own algorithm answer mostly video, which then gets just as likely to be
/// shuffled into next: a loop with nothing to break it. Anchoring on the nearest song instead
/// keeps the algorithm answering songs, so a stray video pick doesn't compound.
///
/// A local track is excluded outright, from both the primary scan and the fallback — its
/// `video_id` is a filesystem path, not something YouTube's radio endpoint can seed from, so
/// unlike a video (a real, just suboptimal, seed) there is no "last resort" that makes sense.
/// `None` when every candidate in range is local, which is what leaves an all-local queue with
/// no radio at all rather than one seeded from nonsense.
fn nearest_song_id(tracks: &[Track], from: usize) -> Option<String> {
    let candidates: Vec<&Track> = tracks[..=from]
        .iter()
        .filter(|t| t.local_path.is_none())
        .collect();
    candidates
        .iter()
        .rev()
        .find(|t| !t.wide)
        .or_else(|| candidates.last())
        .map(|t| t.video_id.clone())
}

/// A local playlist's tracks, resolved against the current scan — a `track_paths` entry with
/// no matching current track (its file moved or was removed since) is simply dropped rather
/// than shown as a broken row. Free rather than an `App` method so the sidebar's rail-menu
/// preview (`ui::sidebar::rail_list`) can build one from its own partial destructure of `App`
/// without needing the whole thing.
pub(crate) fn build_local_playlist(
    lp: &crate::local_playlists::LocalPlaylist,
    local_tracks: &[Track],
) -> crate::model::Playlist {
    let tracks: Vec<Track> = lp
        .track_paths
        .iter()
        .filter_map(|path| {
            let mut t = local_tracks
                .iter()
                .find(|t| t.local_path.as_deref() == Some(path.as_path()))?
                .clone();
            // Not carried from the scan (a plain local track has no playlist to be a member
            // of) — set fresh per playlist, exactly as YouTube's own `set_video_id` is only
            // ever a playlist row's property, not a track's. This is what makes "Remove from
            // playlist" offerable for a local playlist's rows.
            t.set_video_id = Some(Arc::from(path.to_string_lossy().as_ref()));
            Some(t)
        })
        .collect();
    crate::model::Playlist {
        id: lp.id.clone(),
        title: lp.title.clone(),
        description: None,
        author: String::new(),
        author_id: None,
        year: None,
        duration_text: None,
        track_count: Some(format!("{} tracks", tracks.len())),
        thumbnail: None,
        editable: true,
        in_library: false,
        library_add: None,
        library_remove: None,
        saveable: false,
        tracks: crate::model::Paged::new(tracks, None),
        source: crate::model::PlaylistSource::Local,
    }
}

/// A local playlist as the `PlaylistRef` the sidebar and Library page already know how to
/// render — `editable: true` since it's always yours to add to, `track_count` computed rather
/// than fetched since there's no server round trip that would carry one.
fn local_playlist_ref(lp: &crate::local_playlists::LocalPlaylist) -> crate::model::PlaylistRef {
    crate::model::PlaylistRef {
        id: lp.id.clone(),
        title: lp.title.clone(),
        author: String::new(),
        author_id: None,
        track_count: Some(format!("{} tracks", lp.track_paths.len())),
        thumbnail: None,
        editable: true,
        source: crate::model::PlaylistSource::Local,
    }
}

/// What the river wants from the network.
enum RiverCall {
    /// The first page of a hand-picked queue's tail.
    Seed { seed: String },
    /// The next page of an open river.
    Follow { token: String, reseed: String },
}

/// Decide whether the river wants a page, borrowing its state rather than cloning it — the
/// chosen list is as long as the queue. A pending river seeds as soon as its queue starts
/// (YouTube Music shows the autoplay tail from the beginning), while an open river is followed
/// at the low-water mark.
fn river_call(
    radio: &Radio,
    current: Option<usize>,
    queue: &[Track],
    autoplay: bool,
) -> Option<RiverCall> {
    let current = current?;
    if !autoplay {
        return None;
    }
    match &radio.state {
        River::Pending => Some(RiverCall::Seed {
            seed: nearest_song_id(queue, queue.len().checked_sub(1)?)?,
        }),
        River::Open(token) if current + RADIO_LOW_WATER >= queue.len() => Some(RiverCall::Follow {
            token: token.clone(),
            reseed: nearest_song_id(queue, current)?,
        }),
        River::Open(_) | River::Dead => None,
    }
}

/// Where the river stands once a page lands: flowing while pages keep adding and handing back a
/// token, dead the moment either stops.
fn landed(added: usize, next: Option<String>) -> River {
    match next {
        Some(token) if added > 0 => River::Open(token),
        _ => River::Dead,
    }
}

/// How long a confirmation bubble stays up.
const TOAST_DURATION: Duration = Duration::from_secs(3);

/// A bubble above the player bar: "Song will play next", "Saved to library", or the reason
/// something did not happen. Fire-and-forget — it clears itself, and only a live one asks for
/// another frame.
struct Toast {
    text: String,
    until: Instant,
    /// Painted in the danger colour. A refusal that looks exactly like a confirmation is worse
    /// than no message at all, which is what every failure used to get.
    error: bool,
}

/// How close to the end of the queue autoplay starts fetching more.
const RADIO_LOW_WATER: usize = 10;

/// Append the tracks the queue does not already hold, by video id. A reseed returns the
/// seed itself as its first track, and continuations may overlap the tail — either way the
/// queue must not repeat itself. Returns how many were added.
fn append_new(queue: &mut Vec<Track>, fresh: &[Track]) -> usize {
    let mut added = 0;
    for t in fresh {
        if !queue.iter().any(|q| q.video_id == t.video_id) {
            queue.push(t.clone());
            added += 1;
        }
    }
    added
}

/// Apply a playlist sort in place, keeping "Default" (and "Reverse") honest.
///
/// [`PlaylistSort::apply`] on its own only knows how to build a *new* order — asking it for
/// `Default` did nothing, which left the tracks exactly as some earlier sort had left them
/// rather than back the way YouTube actually served them, until the playlist was closed and
/// reopened; `Reverse` has the same problem, needing that same served order as something to
/// flip. `original_order` is the fix: the video-id order captured the first time this playlist
/// is sorted to anything at all, consulted here for both instead of either being a no-op.
fn apply_playlist_sort(
    sort: PlaylistSort,
    tracks: &mut [Track],
    original_order: &mut Option<Vec<String>>,
) {
    // Captured on whichever sort is chosen first, `Default`/`Reverse` included: either one
    // needs the served order as a baseline just as much as Title/Duration need it as something
    // to get back to later.
    let order =
        original_order.get_or_insert_with(|| tracks.iter().map(|t| t.video_id.clone()).collect());
    match sort {
        PlaylistSort::Default | PlaylistSort::Reverse => {
            let position: std::collections::HashMap<&str, usize> = order
                .iter()
                .enumerate()
                .map(|(i, id)| (id.as_str(), i))
                .collect();
            let rank = |t: &Track| position.get(t.video_id.as_str()).copied().unwrap_or(0);
            if sort == PlaylistSort::Reverse {
                tracks.sort_by_key(|t| std::cmp::Reverse(rank(t)));
            } else {
                tracks.sort_by_key(rank);
            }
        }
        _ => sort.apply(tracks),
    }
}

/// The local half of a playlist drag-reorder: move `items[from]` to sit immediately before
/// whatever is at `to`. Returns the pair of playlist item ids (`setVideoId`s) the server call
/// needs — the moved track, then the one it now precedes — or `None` when there is nothing to
/// do: `from == to`, either index is out of range, or either track carries no item id (not a
/// real playlist row).
fn reorder_tracks(items: &mut Vec<Track>, from: usize, to: usize) -> Option<(Arc<str>, Arc<str>)> {
    if from == to || from >= items.len() || to >= items.len() {
        return None;
    }
    // The item that will still be at `to` once the drag has landed *before* it — read now,
    // since removing `from` first would shift what index `to` itself pointed at when `from < to`.
    let before_set_video_id = items[to].set_video_id.clone()?;
    let set_video_id = items[from].set_video_id.clone()?;
    let track = items.remove(from);
    let insert_at = if from < to { to - 1 } else { to };
    items.insert(insert_at, track);
    Some((set_video_id, before_set_video_id))
}

/// Whether this is a Wayland session — `$WAYLAND_DISPLAY`, the same environment variable every
/// cross-platform GUI toolkit checks for this, since there is no portable API for it.
pub(crate) fn is_wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
}

/// The command that actually hides or shows the window, on whichever platform this is.
///
/// `ViewportCommand::Visible` is a *documented no-op on Wayland* — winit's own Wayland backend
/// says so outright ("Not possible on Wayland") — which is why "Hide tray on close" looked
/// like a dead button there: the veto fired, `Visible(false)` was sent, and Wayland silently
/// ignored it, leaving the window exactly as it was. `Minimized` is the closest real substitute
/// Wayland's xdg-shell actually offers, but only in the hide direction — winit's own
/// `set_minimized(false)` is *also* a documented no-op there ("You can't unminimize the window
/// on Wayland"), and so is `ViewportCommand::Focus`'s `focus_window()`. So on Wayland, showing
/// the window again from the tray is a best-effort request only: the reliable way back is the
/// user's own taskbar/pager entry for the minimized window, the same as any other Wayland app
/// behind this exact protocol gap.
fn hide_command(hide: bool) -> egui::ViewportCommand {
    hide_command_for(is_wayland(), hide)
}

/// [`hide_command`], parameterised on the platform check instead of reading it — so the two
/// branches are each a plain unit test rather than something only checkable by hand on both
/// kinds of session.
fn hide_command_for(wayland: bool, hide: bool) -> egui::ViewportCommand {
    if wayland {
        egui::ViewportCommand::Minimized(hide)
    } else {
        egui::ViewportCommand::Visible(!hide)
    }
}

pub struct App {
    pub(crate) palette: Palette,
    history: History,
    pub(crate) decorated: bool,
    startup_timing: bool,
    /// The tray icon, while "Hide tray on close" is on — dropping it removes the icon. `None`
    /// the rest of the time, so nobody who never touches the setting gets a tray icon at all.
    tray: Option<crate::tray::Tray>,
    /// We hid the window ourselves (tray-icon close, or the tray menu's own Show/Hide) rather
    /// than the OS reporting it occluded or minimized — the two are different questions, and
    /// this is the one that decides whether the repaint heartbeat below needs to keep running.
    hidden: bool,
    /// The tray's own Quit was chosen: the next close request must actually go through rather
    /// than being caught by the hide-on-close veto, which would otherwise just hide it again.
    quitting: bool,

    pub(crate) backend: Backend,
    pub(crate) player: Option<Player>,
    /// Set when libmpv could not be initialised, so the UI can explain instead of silently
    /// doing nothing when you press play.
    pub(crate) player_error: Option<String>,

    pub(crate) query: String,
    /// When the query last changed, for debouncing.
    pub(crate) query_changed_at: Option<Instant>,
    /// Autocomplete for the search field: the items, the query they belong to (a staleness guard
    /// like `searched`), and where the keyboard cursor sits among them.
    pub(crate) suggestions: Vec<String>,
    pub(crate) suggestions_for: Option<String>,
    pub(crate) suggestion_cursor: usize,
    /// Set once the arrow keys have moved the suggestion highlight, so Enter accepts that
    /// suggestion; untouched, Enter searches exactly what was typed.
    pub(crate) suggestion_navigated: bool,
    /// A search waiting for `poll_search` to fire it. Search is submit-driven — Enter, a chosen
    /// suggestion, or the `--search` CLI — never a keystroke.
    pending_search: bool,
    pub(crate) searched: String,
    pub(crate) searching: bool,
    pub(crate) results: SearchResults,
    pub(crate) status: Option<String>,
    /// Which kind of result the search page is filtered to. `All` is a view over `results`
    /// (never a second request); every other scope drives a dedicated per-category fetch, since
    /// the mixed ranked response often surfaces far fewer of that kind than actually exist — see
    /// [`SearchScope`]'s own doc comment.
    pub(crate) search_scope: SearchScope,
    /// The dedicated per-category fetch's results, valid only while `scoped_for` names the
    /// `(query, scope)` pair currently on screen — mirrors the `artist_for` staleness-guard
    /// pattern used elsewhere in this file.
    pub(crate) scoped_results: Vec<Entry>,
    pub(crate) scoped_loading: bool,
    pub(crate) scoped_error: Option<String>,
    scoped_for: Option<(String, SearchScope)>,

    /// The artist page being shown, plus whether a fetch is outstanding / failed.
    ///
    /// `artist_for` is the id the other three fields describe. Without it "nothing loaded" and
    /// "a *different* artist is loaded" look identical, which is what let Back show the wrong
    /// page and let a stale reply latch `artist_loading` forever.
    pub(crate) artist: Option<crate::model::Artist>,
    pub(crate) artist_loading: bool,
    pub(crate) artist_error: Option<String>,
    artist_for: Option<String>,
    /// The album page being shown.
    pub(crate) album: Option<crate::model::Album>,
    pub(crate) album_loading: bool,
    pub(crate) album_error: Option<String>,
    album_for: Option<String>,

    /// The app's own icon, as a texture for the Settings page's branding. Loaded once, lazily
    /// (`ui::settings::page` fills this in on first use rather than at startup, since nothing
    /// else needs it), and kept for the rest of the session.
    pub(crate) logo: Option<egui::TextureHandle>,

    /// What the Settings page shows about authentication.
    pub(crate) auth: crate::auth::AuthState,
    /// Why the last sign-in attempt failed. Kept apart from `auth`, so a refused attempt does not
    /// make a working session look lost.
    pub(crate) sign_in_error: Option<String>,
    /// The signed-in user's library, in sections.
    pub(crate) library: crate::model::Library,
    pub(crate) library_loading: bool,
    pub(crate) library_error: Option<String>,
    pub(crate) library_loaded: bool,
    pub(crate) library_sort: LibrarySort,
    /// Every track found under `settings.local_folders` by the last full rescan — there is no
    /// incremental update, a changed folder list just rescans everything again.
    pub(crate) local_tracks: Vec<crate::model::Track>,
    pub(crate) local_scanning: bool,
    pub(crate) local_scan_error: Option<String>,
    /// Which folder the Library's Local tab is currently browsing, one path segment per level,
    /// relative to whichever configured root it sits under. Empty is the top level.
    pub(crate) local_browse_path: Vec<String>,
    /// Local playlists: loaded once at startup and mutated directly, since there is no server
    /// round trip to hide behind an `Event` — see `local_playlists.rs`'s own doc comment.
    /// Mirrored into `library.playlists.items` by `resync_local_playlists` so the sidebar and
    /// the Library page see one merged list without their own code knowing two sources exist.
    pub(crate) local_playlists: Vec<crate::local_playlists::LocalPlaylist>,
    /// The playlist page being shown.
    pub(crate) playlist: Option<crate::model::Playlist>,
    pub(crate) playlist_loading: bool,
    pub(crate) playlist_error: Option<String>,
    playlist_for: Option<String>,
    /// How the open playlist's track list is currently shown — client-side and session-only,
    /// reset whenever a different playlist opens (see [`PlaylistSort`]'s own doc comment for
    /// why this can't be a server order the way [`LibrarySort`] is).
    pub(crate) playlist_sort: PlaylistSort,
    /// A sort was chosen before the playlist had fully loaded: the remaining pages are being
    /// fetched, and the sort applies once `tracks.next` goes `None`.
    pub(crate) playlist_sort_pending: bool,
    /// The track order YouTube actually served, by video id — captured the first time this
    /// playlist is sorted to anything other than [`PlaylistSort::Default`], since choosing
    /// Default again has to put it back and sorting in place is otherwise destructive. `None`
    /// until then (no snapshot needed if nobody has left Default yet), and reset whenever a
    /// different playlist opens.
    playlist_original_order: Option<Vec<String>>,
    /// Full-list expansion of one artist shelf: which artist and which shelf, while it is in
    /// flight. Per shelf rather than global, because "Singles & EPs", "Videos" and "Playlists"
    /// each have their own expansion.
    pub(crate) artist_expand: Option<(String, String)>,
    pub(crate) artist_expand_loading: bool,
    pub(crate) artist_expand_error: Option<String>,
    /// The home feed, as YouTube serves it. Replaces the moods-and-history page we used to
    /// assemble ourselves, which used none of YouTube's ranking.
    ///
    /// Behind an `Arc` like every other feed here: the page renderers need `&mut App` while
    /// they draw, so they take a snapshot for the frame, and a refcount bump is what that
    /// costs instead of a deep clone of every shelf on every frame. A write during a frame
    /// goes through `Arc::make_mut`, which copies once and leaves the frame's snapshot alone.
    pub(crate) home: Option<Arc<Vec<crate::model::Shelf>>>,
    pub(crate) home_loading: bool,
    pub(crate) home_error: Option<String>,
    /// The mood bar above the feed (Workout, Relax, …). Parsed from the same browse.
    pub(crate) home_chips: Vec<crate::model::HomeChip>,
    /// The selected chip's params. `None` is the default feed.
    pub(crate) home_filter: Option<String>,
    pub(crate) mood_playlists: Option<crate::model::MoodPlaylists>,
    pub(crate) mood_playlists_loading: bool,
    pub(crate) mood_playlists_error: Option<String>,
    pub(crate) mood_playlists_params: Option<String>,

    pub(crate) now: NowPlaying,
    /// The playable list the transport operates on. Search results, an artist's top songs and
    /// an album's tracks each become the queue when one of them is started.
    pub(crate) queue: Vec<Track>,
    /// Index into `queue`.
    pub(crate) current: Option<usize>,
    /// Set once we have asked mpv to play, so we can tell "not started yet" from "track ended".
    pub(crate) playback_started: bool,
    /// Autoplay river state, opened by Start Mix. `None` for a hand-picked queue.
    pub(crate) radio: Option<Radio>,
    /// Hidden "previous" history for the transport. Bounded; see [`PlayHistory`].
    play_history: PlayHistory,
    /// What the transport does at the end of the queue.
    pub(crate) repeat: RepeatMode,
    /// Shuffle: next picks an unplayed track at random instead of the following one.
    pub(crate) shuffle: bool,
    /// Video ids played since shuffle began (or since the last wrap), so shuffle does not
    /// repeat before the queue is exhausted. Cleared when shuffle is toggled.
    shuffle_seen: std::collections::HashSet<String>,
    /// LCG state for the shuffle pick — no `rand` dependency for one choice.
    rng: u64,
    /// A river extension is in flight. Guards the low-water trigger, which runs every frame.
    radio_loading: bool,
    /// The Up Next panel is open.
    pub(crate) upnext_open: bool,
    /// The lyrics panel is open. Mutually exclusive with the queue: they share the right panel.
    pub(crate) lyrics_open: bool,
    /// The loaded lyrics for [`Self::lyrics_for`], if any.
    pub(crate) lyrics: Option<crate::model::Lyrics>,
    pub(crate) lyrics_loading: bool,
    pub(crate) lyrics_error: Option<String>,
    /// Which video the current lyrics (or in-flight request) belong to, so a reply for a track
    /// already left behind is dropped.
    lyrics_for: Option<String>,
    /// The playlist rename editor, when open.
    pub(crate) playlist_edit: Option<PlaylistEditor>,
    /// A playlist awaiting delete confirmation: its id and title.
    pub(crate) playlist_delete: Option<(String, String)>,
    /// A playlist whose editor should open as soon as its page loads (Edit from the rail).
    edit_on_load: Option<String>,
    /// The title a rename is reverting to if YouTube refuses it: (id, old title).
    edit_revert: Option<(String, String)>,
    /// Playlists fetched for the sidebar's context menu, so it can offer the page's track
    /// actions (shuffle, mix, queue, save) without opening the page. Bounded.
    pub(crate) rail_cache: std::collections::HashMap<String, crate::model::Playlist>,
    /// The same idea for albums: kept so an album card's context menu can seed its mix, queue
    /// and save items once the right-click prefetch lands. Bounded like the rail cache.
    pub(crate) album_cache: std::collections::HashMap<String, crate::model::Album>,
    /// Which library section the Library page shows.
    pub(crate) rail_filter: RailFilter,
    /// The name being typed into the rail's new-playlist field, while it is open.
    pub(crate) new_playlist: Option<String>,
    /// What that same field commits to once confirmed — a real YouTube playlist by default, or
    /// (when opened from the Library's Local tab's "Save as playlist") a local one seeded with
    /// a specific folder's tracks. One shared modal (`ui::sidebar::new_playlist_popup`) rather
    /// than a second one, since both are just "type a name, confirm."
    pub(crate) new_playlist_kind: NewPlaylistKind,
    pub(crate) explore: Option<Arc<Vec<crate::model::Shelf>>>,
    pub(crate) explore_loading: bool,
    pub(crate) explore_error: Option<String>,
    pub(crate) explore_links: Vec<crate::model::ExploreLink>,
    pub(crate) releases: Option<Arc<Vec<crate::model::Shelf>>>,
    pub(crate) releases_loading: bool,
    pub(crate) releases_error: Option<String>,
    pub(crate) charts: Option<Arc<crate::model::ChartsFeed>>,
    pub(crate) charts_loading: bool,
    pub(crate) charts_error: Option<String>,
    pub(crate) charts_params: Option<String>,
    pub(crate) moods: Option<Arc<Vec<crate::model::MoodSection>>>,
    pub(crate) moods_loading: bool,
    pub(crate) moods_error: Option<String>,
    pub(crate) history_shelves: Option<Arc<Vec<crate::model::Shelf>>>,
    pub(crate) history_loading: bool,
    pub(crate) history_error: Option<String>,
    /// The volume to restore on unmute. Updated whenever you set a nonzero volume.
    pub(crate) last_volume: f64,
    /// A restored position to seek to once the track loads: `(queue index, seconds)`. Consumed
    /// the first time that track is actually playing, and dropped if the queue moves elsewhere.
    pub(crate) resume: Option<(usize, f64)>,
    /// When the playback state was last written, so the periodic save is throttled.
    last_playback_save: Instant,
    /// Muted by the button versus turned down to zero. `last_volume` remembers what to come back
    /// to; this says whether the crossfade may write the volume at all.
    pub(crate) muted: bool,
    /// When the current track started, for the crossfade's ramp-in. `None` when not fading in.
    fade_in: Option<Instant>,
    /// Whether the crossfade is currently holding the volume down, so it can put it back once
    /// the setting is switched off mid-track.
    fade_active: bool,
    /// A confirmation bubble above the player bar, if one is live.
    toast: Option<Toast>,
    /// Play the first result as soon as a search lands (`--autoplay`).
    autoplay: bool,
    pub(crate) images: Images,
    /// User preferences. Everything in here changes something the app actually does.
    pub(crate) settings: crate::settings::Settings,
    /// The channels this account can browse as. Empty until Settings asks.
    pub(crate) accounts: Vec<crate::model::Account>,
    /// Live counters behind `--stats`; free when off.
    pub(crate) stats: Stats,
    /// Print one line of counters and exit, once this much time has passed. Used by
    /// `scripts/perf.sh` to measure a *loaded* app rather than an empty one.
    stats_after: Option<Duration>,

    /// Debug autoscroll offset, advanced each frame when `--autoscroll` is set.
    pub(crate) autoscroll: Option<f32>,
    /// Set for one frame after opening a page, so it starts at the top. Back and Forward do not
    /// set it: returning to a page should return you to where you were on it.
    pub(crate) scroll_to_top: bool,
    /// Set for one frame after jumping to a track from the Queue panel, so the panel scrolls
    /// back to its own top. Not set by every way `current` can change — skip and autoplay
    /// should not yank the list around under someone who is just watching it scroll by.
    pub(crate) queue_scroll_reset: bool,
    /// Set for one frame when the Lyrics panel opens or its song changes, so it starts at the
    /// top rather than wherever the previous song's lyrics happened to be left scrolled to.
    pub(crate) lyrics_scroll_reset: bool,
    /// The active lyric line last scrolled to, so the panel only scrolls again once that
    /// actually changes — not on every repaint a paused, manually-scrolled view would otherwise
    /// get undone by.
    pub(crate) lyrics_active: Option<usize>,
    /// A page's description, while its Info popup is open. Set by the header ••• menu's Info
    /// item; cleared by the popup's Close button.
    pub(crate) page_info: Option<String>,
    /// The keyboard-shortcut cheatsheet is open. Opened by `?`/F1 and the account menu, closed by
    /// the modal itself.
    pub(crate) help_open: bool,
    /// A crash the previous run left behind, offered once on this launch and then cleared.
    pub(crate) crash_report: Option<crate::crash::Crash>,
    /// What is downloaded to `~/Music/refryt`, indexed by video id. Pushed into the backend so a
    /// downloaded track plays from disk.
    pub(crate) downloads: crate::downloads::Index,
    /// In-flight download progress, by video id (0–100).
    pub(crate) download_progress: std::collections::HashMap<String, f32>,

    /// The content card's inner rect this frame: what the page is drawn into, before the
    /// card's own margin. Pages that paint to the card's edges read it rather than guessing
    /// from a clip rect. See where it is set in [`Self::ui`].
    pub(crate) content_rect: egui::Rect,
    /// The KDE blur-behind connection. Connected only on a KDE Wayland session that offers the
    /// protocol; `blur.available()` is false everywhere else — an X11 session, GNOME, a
    /// non-Linux build — and the Settings row for it disables itself off that rather than
    /// offering a switch that does nothing.
    pub(crate) blur: crate::blur::Blur,
    /// OS media controls and media keys, when the platform offers them. `None` on a session with
    /// no media service (e.g. no D-Bus session bus); the app runs the same without it.
    pub(crate) media: Option<crate::media::Media>,
    /// Kept for the video view, which creates its mpv render context lazily.
    ctx: egui::Context,
    /// Whether the in-app music video is showing in the content card.
    pub(crate) video_open: bool,
    /// The software render surface, created the first time a video is opened.
    pub(crate) video: Option<crate::video::Video>,
}

/// The window's native handle for the OS media service — SMTC attaches to a window on Windows.
/// `None` everywhere else, where `playwire` needs no handle.
#[cfg(target_os = "windows")]
fn window_hwnd(cc: &eframe::CreationContext<'_>) -> Option<u64> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match cc.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(w) => Some(w.hwnd.get()),
        _ => None,
    }
}

#[cfg(not(target_os = "windows"))]
fn window_hwnd(_: &eframe::CreationContext<'_>) -> Option<u64> {
    None
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, options: Options) -> anyhow::Result<Self> {
        let Options {
            decorated,
            startup_timing,
            search,
            autoplay,
            stats,
            stats_after,
            page,
            autoscroll,
            queue,
            lyrics,
            scheme,
            opacity,
            no_blur,
            no_autoplay,
        } = options;
        theme::install(&cc.egui_ctx);
        let mut settings = crate::settings::load();
        // Session-only, and applied before the first palette build so the very first frame
        // already shows what was asked for. Never saved: `update_settings` only ever writes
        // `self.settings`, and this happens before that struct is stored on `App`, so a later
        // settings edit persists the override right back — which is fine for a scripted run
        // and would be wrong for anything else, which is why these stay CLI-only.
        if let Some((scheme, flavour)) = scheme {
            settings.scheme = scheme;
            settings.scheme_variant = flavour;
        }
        if let Some(opacity) = opacity {
            settings.opacity = opacity;
        }
        if no_blur {
            settings.blur = false;
        }
        if no_autoplay {
            settings.autoplay = false;
        }
        let palette = settings.palette(&cc.egui_ctx);
        theme::apply(&cc.egui_ctx, &palette);
        let blur = crate::blur::Blur::new(cc);

        let backend = Backend::new(cc.egui_ctx.clone())?;
        backend.set_audio_quality(settings.quality.format());
        // One handle per process; a failure just means running without OS media controls.
        let media = crate::media::Media::new(cc.egui_ctx.clone(), window_hwnd(cc));
        // libmpv is initialised on the first play, not here — see `Player`.
        let player = Some(Player::new());
        // A stored session is only a claim until YouTube confirms it — cookies expire between
        // runs, and asserting otherwise is what showed an empty library under a "Signed in" label.
        let auth = if backend.is_signed_in_sync() {
            backend.verify_session();
            crate::auth::AuthState::Checking
        } else {
            crate::auth::AuthState::SignedOut
        };

        // The rail must not wait on the session round trip: with a stored cookie the sidebar
        // fetch fires immediately, and a dead cookie fails it into the empty rail — the state
        // that would have shown anyway.
        let mut app = Self {
            palette,
            // Open on Home, as YouTube Music does — it is a real feed now rather than a page we
            // assembled. `--search` starts on Search, and `--page` overrides both.
            history: History::new(match (page, search.is_some()) {
                (Some(route), _) => route,
                (None, true) => Route::Search,
                (None, false) => Route::Home,
            }),
            decorated,
            startup_timing,
            // A tray icon from a previous run's setting shows up again this run without
            // needing the toggle re-flipped.
            tray: settings.hide_tray_on_close.then(crate::tray::spawn),
            hidden: false,
            quitting: false,
            backend,
            player,
            player_error: None,
            query: search.clone().unwrap_or_default(),
            query_changed_at: None,
            suggestions: Vec::new(),
            suggestions_for: None,
            suggestion_cursor: 0,
            suggestion_navigated: false,
            pending_search: search.is_some(),
            searched: String::new(),
            searching: false,
            results: SearchResults::default(),
            status: None,
            search_scope: SearchScope::default(),
            scoped_results: Vec::new(),
            scoped_loading: false,
            scoped_error: None,
            scoped_for: None,
            artist: None,
            artist_loading: false,
            artist_error: None,
            artist_for: None,
            album: None,
            album_loading: false,
            album_error: None,
            album_for: None,
            logo: None,
            auth,
            sign_in_error: None,
            library: crate::model::Library::default(),
            library_loading: false,
            library_error: None,
            library_loaded: false,
            library_sort: LibrarySort::Latest,
            playlist: None,
            playlist_loading: false,
            playlist_error: None,
            playlist_for: None,
            playlist_sort: PlaylistSort::default(),
            playlist_sort_pending: false,
            playlist_original_order: None,
            artist_expand: None,
            artist_expand_loading: false,
            artist_expand_error: None,
            home: None,
            home_loading: false,
            home_error: None,
            home_chips: Vec::new(),
            home_filter: None,
            mood_playlists: None,
            mood_playlists_loading: false,
            mood_playlists_error: None,
            mood_playlists_params: None,
            now: NowPlaying::default(),
            queue: Vec::new(),
            current: None,
            playback_started: false,
            radio: None,
            play_history: PlayHistory::default(),
            repeat: RepeatMode::Off,
            shuffle: false,
            shuffle_seen: std::collections::HashSet::new(),
            rng: rng_seed(),
            radio_loading: false,
            upnext_open: queue,
            lyrics_open: lyrics,
            lyrics: None,
            lyrics_loading: false,
            lyrics_error: None,
            lyrics_for: None,
            playlist_edit: None,
            playlist_delete: None,
            edit_on_load: None,
            edit_revert: None,
            rail_cache: std::collections::HashMap::new(),
            album_cache: std::collections::HashMap::new(),
            rail_filter: RailFilter::default(),
            new_playlist: None,
            new_playlist_kind: NewPlaylistKind::default(),
            explore: None,
            explore_loading: false,
            explore_error: None,
            explore_links: Vec::new(),
            releases: None,
            releases_loading: false,
            releases_error: None,
            charts: None,
            charts_loading: false,
            charts_error: None,
            charts_params: None,
            moods: None,
            moods_loading: false,
            moods_error: None,
            history_shelves: None,
            history_loading: false,
            history_error: None,
            last_volume: 70.0,
            resume: None,
            last_playback_save: Instant::now(),
            muted: false,
            fade_in: None,
            fade_active: false,
            toast: None,
            autoplay,
            images: Images::new(),
            settings,
            accounts: Vec::new(),
            autoscroll: autoscroll.then_some(0.0),
            scroll_to_top: true,
            queue_scroll_reset: false,
            lyrics_scroll_reset: false,
            lyrics_active: None,
            page_info: None,
            help_open: false,
            crash_report: crate::crash::last(),
            downloads: crate::downloads::Index::default(),
            download_progress: std::collections::HashMap::new(),
            stats: Stats::new(stats),
            stats_after,
            content_rect: egui::Rect::NOTHING,
            blur,
            media,
            ctx: cc.egui_ctx.clone(),
            video_open: false,
            video: None,
            local_tracks: Vec::new(),
            local_scanning: false,
            local_scan_error: None,
            local_browse_path: Vec::new(),
            local_playlists: crate::local_playlists::load(),
        };
        app.resync_local_playlists();
        if !app.settings.local_folders.is_empty() {
            app.local_scanning = true;
            app.backend
                .scan_local_folders(app.settings.local_folders.clone());
        }
        app.load_sidebar_playlists();
        app.downloads = crate::downloads::load();
        app.push_downloads_to_backend();
        app.restore_playback();
        Ok(app)
    }

    /// Keyboard shortcuts. Space is play/pause, unless a text field has focus — otherwise it
    /// would be impossible to type a space into the search box.
    /// Spotify's own desktop defaults, kept identical rather than invented, so the muscle
    /// memory carries over. `command` is egui's own per-platform stand-in for the modifier each
    /// OS actually uses (Ctrl on Windows/Linux, ⌘ on macOS) — the same reasoning Spotify's own
    /// shortcuts follow across platforms.
    fn handle_keys(&mut self, ctx: &egui::Context) {
        let typing = ctx.memory(|m| m.focused().is_some());
        if typing {
            return;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Space)) {
            self.toggle_playback();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
            self.skip(-1);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
            self.skip(1);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::M)) {
            self.toggle_mute();
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::S)) {
            self.toggle_shuffle();
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::R)) {
            self.cycle_repeat();
        }
        // Spotify binds both; neither ever hurts to also have here.
        if ctx.input(|i| {
            i.modifiers.command && (i.key_pressed(egui::Key::K) || i.key_pressed(egui::Key::L))
        }) {
            ctx.memory_mut(|m| m.request_focus(crate::ui::topbar::search_id()));
        }
        // The cheatsheet. `?` arrives as `Questionmark` on layouts that produce it and as
        // Shift+/ elsewhere, so both are accepted; F1 for keyboards without a `?`.
        if ctx.input(|i| {
            i.key_pressed(egui::Key::F1)
                || i.key_pressed(egui::Key::Questionmark)
                || (i.modifiers.shift && i.key_pressed(egui::Key::Slash))
        }) {
            self.help_open = true;
        }
    }

    /// Drop everything that belonged to the previous account.
    ///
    /// A brand channel has its own library, playlists and home feed; keeping the old ones on
    /// screen after a switch would be showing one account's data under another's name.
    pub(crate) fn forget_account_data(&mut self) {
        self.library = crate::model::Library::default();
        self.library_loaded = false;
        self.library_error = None;
        self.home = None;
        self.home_error = None;
        self.history_shelves = None;
        self.history_error = None;
        self.playlist = None;
        self.playlist_for = None;
        self.accounts.clear();
        // Local playlists are not tied to any YouTube account — they should survive an account
        // switch exactly like they survive sign-out.
        self.resync_local_playlists();
    }

    /// Apply and persist a settings change.
    ///
    /// Applied immediately rather than on restart: the palette is rebuilt, the resolver retuned,
    /// and only then is the file written — a failed write should not leave the UI disagreeing
    /// with what the app is doing.
    pub(crate) fn update_settings(
        &mut self,
        ctx: &egui::Context,
        change: impl FnOnce(&mut crate::settings::Settings),
    ) {
        let before = self.settings.clone();
        change(&mut self.settings);
        if self.settings == before {
            return;
        }
        // Any of the four that make a palette: the family, the flavour chosen within it, the
        // polarity, or how much of the desktop shows through it.
        if self.settings.theme != before.theme
            || self.settings.scheme != before.scheme
            || self.settings.scheme_variant != before.scheme_variant
            || self.settings.opacity != before.opacity
        {
            self.palette = self.settings.palette(ctx);
            theme::apply(ctx, &self.palette);
        }
        if self.settings.quality != before.quality {
            self.backend
                .set_audio_quality(self.settings.quality.format());
        }
        if self.settings.account_page_id != before.account_page_id {
            self.backend
                .switch_account(self.settings.account_page_id.clone());
        }
        if self.settings.hide_tray_on_close != before.hide_tray_on_close {
            // Built once, ever, the first time this turns on — not rebuilt on every toggle,
            // which used to leak a thread and a duplicate tray icon per toggle (see `tray`'s
            // own module doc comment). Every toggle after the first is just `set_visible`.
            match &self.tray {
                Some(tray) => tray.set_visible(self.settings.hide_tray_on_close),
                None if self.settings.hide_tray_on_close => {
                    self.tray = Some(crate::tray::spawn());
                }
                None => {}
            }
        }
        #[cfg(target_os = "linux")]
        if self.settings.start_on_boot != before.start_on_boot {
            let result = if self.settings.start_on_boot {
                crate::autostart::enable()
            } else {
                crate::autostart::disable()
            };
            if let Err(e) = result {
                tracing::warn!(error = %format!("{e:#}"), "could not update the autostart entry");
            }
        }
        if self.settings.local_folders != before.local_folders {
            self.local_scanning = true;
            self.local_scan_error = None;
            self.backend
                .scan_local_folders(self.settings.local_folders.clone());
        }
        if let Err(e) = crate::settings::save(&self.settings) {
            tracing::warn!(error = %format!("{e:#}"), "could not save settings");
        }
    }

    /// The titlebar's own close button was clicked: hide to the tray if that's configured,
    /// otherwise actually close.
    ///
    /// Handled here rather than inline in `chrome::window_buttons` because a self-issued
    /// `ViewportCommand::Close` on the root viewport does not surface as a cancelable
    /// `close_requested()` the way an OS-originated close does (Alt+F4, or the OS's own window
    /// controls in `--decorated` mode) — egui's own doc for `ViewportCommand::Close` says as
    /// much: for the root viewport it "usually results in the application shutting down"
    /// outright. That OS-originated path is instead caught centrally in `poll_tray`, every
    /// frame, since it can arrive with no button click at all.
    pub(crate) fn request_close(&mut self, ctx: &egui::Context) {
        if self.settings.hide_tray_on_close {
            self.hidden = true;
            ctx.send_viewport_cmd(hide_command(true));
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// Drain tray-icon and tray-menu events, and veto a genuine OS close request into a hide
    /// when "Hide tray on close" is on. Called once a frame regardless of whether a tray icon
    /// currently exists — `tray::poll` is a cheap non-blocking read of a channel that is simply
    /// empty when there is nothing to show a tray icon at all.
    fn poll_tray(&mut self, ctx: &egui::Context) {
        while let Some(action) = crate::tray::poll() {
            match action {
                crate::tray::TrayAction::ToggleVisible => {
                    self.hidden = !self.hidden;
                    ctx.send_viewport_cmd(hide_command(self.hidden));
                    if !self.hidden {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    }
                }
                crate::tray::TrayAction::Quit => {
                    // Bypasses the veto below on the same or a following frame — Quit means
                    // quit, not "hide again".
                    self.quitting = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
        if !self.quitting
            && self.settings.hide_tray_on_close
            && ctx.input(|i| i.viewport().close_requested())
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(hide_command(true));
            self.hidden = true;
        }
        if self.hidden {
            // Nothing else wakes a hidden window — no cursor motion reaches it, nothing is
            // animating — so without this, a tray click asking to show it again would sit
            // unnoticed until some unrelated event happened to schedule a repaint.
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    /// Drain and act on whatever the OS media service sent — a media key, the shell widget, or
    /// another MPRIS client. Events arrive on a backend thread and are queued by `media.rs`.
    fn poll_media(&mut self, ctx: &egui::Context) {
        let events = match self.media.as_ref() {
            Some(media) => media.drain(),
            None => return,
        };
        for event in events {
            match event {
                playwire::Event::Play => {
                    if !self.playback_started || self.now.paused {
                        self.toggle_playback();
                    }
                }
                playwire::Event::Pause => {
                    if self.playback_started && !self.now.paused {
                        self.toggle_playback();
                    }
                }
                playwire::Event::PlayPause => self.toggle_playback(),
                playwire::Event::Stop => {
                    if self.playback_started && !self.now.paused {
                        self.toggle_playback();
                    }
                    if let Some(player) = self.player.as_ref() {
                        player.seek(0.0);
                    }
                }
                playwire::Event::Next => self.skip(1),
                playwire::Event::Previous => self.skip(-1),
                playwire::Event::SeekTo(position) => {
                    if let Some(player) = self.player.as_ref() {
                        player.seek(position.as_secs_f64());
                    }
                }
                playwire::Event::SeekBy(offset) => {
                    if let Some(player) = self.player.as_ref() {
                        let to = (player.state().position + offset).max(0.0);
                        player.seek(to);
                    }
                }
                playwire::Event::SetVolume(v) => {
                    let volume = (v * 100.0).clamp(0.0, 100.0);
                    self.last_volume = volume;
                    self.muted = volume <= 0.0;
                    if let Some(player) = self.player.as_mut() {
                        player.set_volume(volume);
                    }
                }
                playwire::Event::SetShuffle(on) => self.shuffle = on,
                playwire::Event::SetRepeat(repeat) => {
                    self.repeat = match repeat {
                        playwire::Repeat::Off => RepeatMode::Off,
                        playwire::Repeat::One => RepeatMode::One,
                        playwire::Repeat::All => RepeatMode::All,
                    };
                }
                playwire::Event::Raise => {
                    self.hidden = false;
                    ctx.send_viewport_cmd(hide_command(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                playwire::Event::Quit => {
                    self.quitting = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                // `OpenUri`, and anything a future backend adds: nothing to do.
                _ => {}
            }
        }
    }

    /// Publish the transport to the OS widget. Once a frame is fine — `playwire` diffs, so an
    /// unchanged snapshot costs almost nothing.
    fn publish_media(&mut self) {
        let state = self.player.as_ref().map(|p| p.state()).unwrap_or_default();
        // A restored track is not loaded yet, so mpv's position is 0; show the saved one so the
        // OS scrubber matches the bar.
        let position = if self.playback_started {
            state.position
        } else {
            self.resume
                .filter(|(i, _)| Some(*i) == self.current)
                .map(|(_, p)| p)
                .unwrap_or(state.position)
        };
        let snapshot = crate::media::playback_state(
            self.now.track.as_ref(),
            self.playback_started && !state.paused,
            position,
            state.duration,
            if self.muted {
                0.0
            } else {
                (self.last_volume / 100.0).clamp(0.0, 1.0)
            },
            self.repeat,
            self.shuffle,
            self.current.is_some_and(|c| c > 0),
            !self.queue.is_empty(),
        );
        if let Some(media) = self.media.as_mut() {
            media.set_state(&snapshot);
        }
    }

    /// Bring back the last queue, track and transport from `playback.json`. The track is shown
    /// paused; pressing play loads it and seeks to `resume`. Nothing is fetched or scrobbled
    /// here — a restore is not a play.
    fn restore_playback(&mut self) {
        let Some(store) = crate::playback::load() else {
            return;
        };
        if store.queue.is_empty() {
            return;
        }
        self.queue = store.queue.into_iter().map(|t| t.into_track()).collect();
        self.current = store.current.filter(|i| *i < self.queue.len());
        self.now.track = self.current.and_then(|i| self.queue.get(i).cloned());
        self.repeat = store.repeat.map(Into::into).unwrap_or(RepeatMode::Off);
        self.shuffle = store.shuffle;
        if store.volume > 0.5 {
            self.last_volume = store.volume;
        }
        // Anything past a couple of seconds is worth resuming; the very start is not.
        self.resume = (store.position > 1.0)
            .then(|| self.current.map(|i| (i, store.position)))
            .flatten();
    }

    /// Write the queue and transport. Throttled by the caller; best-effort (a failure is only a
    /// debug line — losing the queue is not worth a toast).
    fn save_playback(&self) {
        if self.queue.is_empty() {
            return;
        }
        let position = if self.playback_started {
            self.player
                .as_ref()
                .map(|p| p.state().position)
                .unwrap_or(0.0)
        } else {
            self.resume.map(|(_, p)| p).unwrap_or(0.0)
        };
        let store = crate::playback::Store {
            version: 1,
            queue: self
                .queue
                .iter()
                .map(crate::playback::PersistedTrack::from_track)
                .collect(),
            current: self.current,
            position,
            repeat: Some(self.repeat.into()),
            shuffle: self.shuffle,
            volume: self.last_volume,
        };
        if let Err(e) = crate::playback::save(&store) {
            tracing::debug!(error = %format!("{e:#}"), "could not save the playback state");
        }
    }

    /// Act on a restored position once its track is actually playing, and save the transport
    /// every few seconds so a crash or a kill loses little.
    fn poll_playback(&mut self, state: &crate::player::PlayerState) {
        if let Some((index, position)) = self.resume {
            if self.current != Some(index) {
                // The queue moved on; the saved position belongs to a track no longer loaded.
                self.resume = None;
            } else if self.playback_started && state.duration > 0.0 {
                if let Some(player) = self.player.as_ref() {
                    player.seek(position);
                }
                self.resume = None;
            }
        }
        if self.last_playback_save.elapsed() >= Duration::from_secs(5) {
            self.last_playback_save = Instant::now();
            self.save_playback();
        }
    }

    /// The single-instance crossfade: dip to silence over the last few seconds and back up as the
    /// next track begins. mpv has no crossfade of its own and nothing is appended to its
    /// playlist, so a real overlap would need two players; this is the cheap approximation the
    /// setting offers. Only touches the volume while a ramp is actually in progress.
    fn apply_crossfade(&mut self, state: &crate::player::PlayerState) {
        let base = if self.muted { 0.0 } else { self.last_volume };
        if !self.settings.crossfade {
            // Switched off mid-fade: put the true volume back once.
            if self.fade_active {
                if let Some(player) = self.player.as_mut() {
                    player.set_volume(base);
                }
                self.fade_active = false;
            }
            return;
        }
        let active = self.playback_started && !state.paused && state.duration > 0.0;
        let elapsed = self.fade_in.map(|started| started.elapsed().as_secs_f64());
        if elapsed.is_some_and(|e| e >= CROSSFADE_SECS) {
            self.fade_in = None;
        }
        let factor = crossfade_factor(active, state.position, state.duration, elapsed);
        if factor < 1.0 {
            self.fade_active = true;
            if let Some(player) = self.player.as_mut() {
                player.set_volume(base * factor);
            }
        } else if self.fade_active {
            self.fade_active = false;
            if let Some(player) = self.player.as_mut() {
                player.set_volume(base);
            }
        }
    }

    /// Tell the backend which video ids are on disk, so `play` uses the file.
    fn push_downloads_to_backend(&self) {
        let map = self
            .downloads
            .entries
            .iter()
            .map(|(id, r)| (id.clone(), std::path::PathBuf::from(&r.path)))
            .collect();
        self.backend.set_downloads(map);
    }

    fn start_download(&mut self, track: Track) {
        if self.downloads.entries.contains_key(&track.video_id) {
            self.toast("Already downloaded");
            return;
        }
        self.download_progress.insert(track.video_id.clone(), 0.0);
        self.backend
            .download_track(track, self.settings.quality.format());
        self.toast("Downloading…");
    }

    fn start_download_many(&mut self, tracks: Vec<Track>) {
        let pending: Vec<Track> = tracks
            .into_iter()
            .filter(|t| !self.downloads.entries.contains_key(&t.video_id))
            .collect();
        if pending.is_empty() {
            self.toast("Already downloaded");
            return;
        }
        for track in &pending {
            self.download_progress.insert(track.video_id.clone(), 0.0);
        }
        let count = pending.len();
        self.backend
            .download_many(pending, self.settings.quality.format());
        self.toast(&format!("Downloading {count} tracks…"));
    }

    fn remove_download(&mut self, video_id: &str) {
        if let Some(record) = self.downloads.entries.remove(video_id) {
            let _ = std::fs::remove_file(&record.path);
            if let Err(e) = crate::downloads::save(&self.downloads) {
                tracing::warn!(error = %format!("{e:#}"), "could not save the download index");
            }
            self.push_downloads_to_backend();
            self.toast("Removed download");
        }
    }

    /// Show or hide the in-app music video. Opening creates the software render surface (once)
    /// and reloads the current track as video through mpv's yt-dlp hook; closing hands playback
    /// back to the audio resolver — both at the current position.
    pub(crate) fn toggle_video(&mut self) {
        if self.video_open {
            self.video_open = false;
            self.reload_current();
            return;
        }
        if !self.now.track.as_ref().is_some_and(|t| t.wide) {
            return;
        }
        if self.video.is_none()
            && let Some(player) = self.player.as_mut()
            && let Some(handle) = player.handle()
        {
            self.video = crate::video::Video::new(&handle, self.ctx.clone());
        }
        self.video_open = self.video.is_some();
        if self.video_open {
            self.reload_current();
        }
    }

    /// Reload the current track in the mode the video toggle is now in, keeping the position.
    fn reload_current(&mut self) {
        let Some(index) = self.current else { return };
        let Some(track) = self.queue.get(index).cloned() else {
            return;
        };
        let position = self
            .player
            .as_ref()
            .map(|p| p.state().position)
            .unwrap_or(0.0);
        let volume = if self.muted { 0.0 } else { self.last_volume };
        let Some(player) = self.player.as_mut() else {
            return;
        };
        let Some(handle) = player.handle() else {
            return;
        };
        // The old file is still loaded for an instant; without clearing this, `poll_playback`
        // would seek the *outgoing* file to the saved position and consume `resume` before the
        // new one starts — which is why handing playback back restarted from zero. Same guard
        // `play_index_restore` uses: wait for the new file to actually report playing.
        self.playback_started = false;
        if self.video_open {
            self.backend.play_video(track, handle, volume);
        } else {
            self.backend
                .play(track, handle, volume, self.settings.normalise_volume);
        }
        if position > 1.0 {
            self.resume = Some((index, position));
        }
    }

    /// Pause or resume, or start the queue if nothing has played yet.
    pub(crate) fn toggle_playback(&mut self) {
        match (&self.player, self.playback_started) {
            (Some(pl), true) => pl.toggle_pause(),
            (Some(_), false) if !self.queue.is_empty() => {
                self.play_index(self.current.unwrap_or(0))
            }
            _ => {}
        }
    }

    fn is_maximized(ctx: &egui::Context) -> bool {
        ctx.input(|i| i.viewport().maximized.unwrap_or(false))
    }

    // ------------------------------------------------------------------------------------------
    // Navigation
    // ------------------------------------------------------------------------------------------

    /// The page currently on screen.
    pub(crate) fn current_route(&self) -> Route {
        self.history.current().clone()
    }

    pub(crate) fn navigate(&mut self, route: Route) {
        if self.history.push(route) {
            self.scroll_to_top = true;
            self.ensure_route_loaded();
        }
    }

    pub(crate) fn can_go_back(&self) -> bool {
        self.history.can_go_back()
    }

    pub(crate) fn can_go_forward(&self) -> bool {
        self.history.can_go_forward()
    }

    /// Back and Forward must reload, not just move the cursor. Going back from artist B to
    /// artist A used to leave B's content on screen because these only touched the cursor.
    /// Both clear a pending scroll-to-top: it belonged to the page being left, and Back must
    /// return to where you were, not to the top.
    pub(crate) fn back(&mut self) {
        if self.history.back() {
            self.scroll_to_top = false;
            self.ensure_route_loaded();
        }
    }

    pub(crate) fn forward(&mut self) {
        if self.history.forward() {
            self.scroll_to_top = false;
            self.ensure_route_loaded();
        }
    }

    /// Make the loaded data match the route on screen.
    ///
    /// Every `load_*` below is idempotent — it compares the wanted id against the `_for` field
    /// that says what the current state describes, and returns when they already agree. That is
    /// what makes this safe to call from `navigate`, from Back and Forward, *and* once per frame
    /// as a safety net for a route whose fetch was dropped while we were elsewhere.
    pub(crate) fn ensure_route_loaded(&mut self) {
        match self.current_route() {
            Route::Artist(id) => self.load_artist(id),
            Route::Album(id) => self.load_album(id),
            Route::Playlist(id) => self.load_playlist(id),
            Route::Mood(params) => self.load_mood_playlists(params),
            Route::Library => self.load_library(),
            Route::Home => self.load_home(),
            Route::Explore => self.load_explore(),
            Route::NewReleases => self.load_new_releases(),
            Route::Charts(params) => self.load_charts(params),
            Route::Moods => self.load_moods(),
            Route::History => self.load_history(),
            Route::Search | Route::Settings => {}
        }
    }

    /// Fetch an artist page unless the current state already describes this one — loaded,
    /// in flight, or failed. A latched error is cleared by [`Self::retry_route`].
    fn load_artist(&mut self, id: String) {
        if self.artist_for.as_deref() == Some(id.as_str()) {
            return;
        }
        self.artist = None;
        self.artist_error = None;
        self.artist_loading = true;
        self.artist_for = Some(id.clone());
        self.artist_expand = None;
        self.artist_expand_loading = false;
        self.artist_expand_error = None;
        self.backend.fetch_artist(id);
    }

    fn load_album(&mut self, id: String) {
        if self.album_for.as_deref() == Some(id.as_str()) {
            return;
        }
        self.album = None;
        self.album_error = None;
        self.album_loading = true;
        self.album_for = Some(id.clone());
        self.backend.fetch_album(id);
    }

    /// An album card's own play button: start playing it without navigating to its page —
    /// see `Event::CardTracksReady`'s own doc comment for why this is a separate fetch from
    /// `load_album`'s.
    pub(crate) fn play_album(&mut self, id: String) {
        self.backend.play_album(id);
    }

    /// [`Self::play_album`], for a playlist card.
    pub(crate) fn play_playlist(&mut self, id: String) {
        self.backend.play_playlist(id);
    }

    fn load_playlist(&mut self, id: String) {
        if self.playlist_for.as_deref() == Some(id.as_str()) {
            return;
        }
        self.playlist_for = Some(id.clone());
        // A new playlist starts back at its own order, not whatever the last one was sorted
        // to, and cancels any load-the-rest-then-sort chain that was still in flight.
        self.playlist_sort = PlaylistSort::default();
        self.playlist_sort_pending = false;
        self.playlist_original_order = None;
        // No fetch, no loading state: a local playlist is already fully in memory, built fresh
        // each time so a rescan's updated tags show up without needing its own cache-busting.
        if let Some(lp) = self.local_playlists.iter().find(|p| p.id == id).cloned() {
            self.playlist = Some(self.local_playlist_as_playlist(&lp));
            self.playlist_loading = false;
            self.playlist_error = None;
            // Never fires `Event::PlaylistLoaded` — the event that would otherwise consume
            // this — so `begin_edit_playlist`'s "arrive, then open the editor" has to happen
            // right here instead.
            if self.edit_on_load.as_deref() == Some(id.as_str()) {
                self.edit_on_load = None;
                self.open_playlist_editor();
            }
            return;
        }
        self.playlist = None;
        self.playlist_error = None;
        self.playlist_loading = true;
        self.backend.fetch_playlist(id);
    }

    /// A local playlist's tracks, resolved against the last scan — a `track_paths` entry with
    /// no matching current track (its file moved or was removed since) is simply dropped
    /// rather than shown as a broken row.
    fn local_playlist_as_playlist(
        &self,
        lp: &crate::local_playlists::LocalPlaylist,
    ) -> crate::model::Playlist {
        build_local_playlist(lp, &self.local_tracks)
    }

    fn load_mood_playlists(&mut self, params: String) {
        if self.mood_playlists_params.as_deref() == Some(params.as_str()) {
            return;
        }
        self.mood_playlists = None;
        self.mood_playlists_error = None;
        self.mood_playlists_loading = true;
        self.mood_playlists_params = Some(params.clone());
        self.backend.fetch_mood_playlists(params);
    }

    /// The signed-in library, once per session. Signed out, the page shows a prompt instead.
    fn load_library(&mut self) {
        if !matches!(self.auth, crate::auth::AuthState::SignedIn { .. })
            || self.library_loaded
            || self.library_loading
            || self.library_error.is_some()
        {
            return;
        }
        self.library_loading = true;
        self.backend.fetch_library(self.library_sort.param());
    }

    /// The home feed loads once per filter and stays cached while it is selected.
    ///
    /// Waits out `Checking` rather than firing on whatever cookie is stored before it is
    /// actually confirmed: a stored cookie `verify_session` still needs to refresh (Google
    /// rotates it under us) makes this exact fetch come back in the thin, signed-out shape —
    /// and unlike `home()`'s own signed-out padding, which only ever covers a *genuinely*
    /// signed-out response, there is nothing here to retry it once the real session settles.
    /// `ensure_route_loaded` runs every frame, so this simply tries again once `Checking`
    /// resolves either way.
    fn load_home(&mut self) {
        if matches!(self.auth, crate::auth::AuthState::Checking) {
            return;
        }
        if self.home.is_none() && self.home_error.is_none() && !self.home_loading {
            self.home_loading = true;
            self.backend.fetch_home(self.home_filter.clone());
        }
    }

    /// Filter the home feed through a mood-bar chip. `None` clears back to the default feed,
    /// as the site does when the selected chip is clicked again. The scroll reset is sticky:
    /// the filtered feed arrives frames later, and it must open at the top, not at the old
    /// feed's offset.
    pub(crate) fn set_home_filter(&mut self, params: Option<String>) {
        if self.home_filter == params {
            return;
        }
        self.home_filter = params;
        self.home = None;
        self.home_error = None;
        self.home_loading = true;
        self.scroll_to_top = true;
        self.backend.fetch_home(self.home_filter.clone());
    }

    fn artist_is_for(&self, id: &str) -> bool {
        self.artist_for.as_deref() == Some(id)
    }

    fn album_is_for(&self, id: &str) -> bool {
        self.album_for.as_deref() == Some(id)
    }

    fn playlist_is_for(&self, id: &str) -> bool {
        self.playlist_for.as_deref() == Some(id)
    }

    /// Clear whatever latched the current route's failure and fetch it again. This is what the
    /// "Try again" buttons call: navigating to the route you are already on is a no-op, so a
    /// retry has to go through the state rather than through the history. A retry refetches
    /// fresh content, so it resets the scroll like a fresh open — otherwise the reloaded page
    /// lands at the failed attempt's offset.
    pub(crate) fn retry_route(&mut self) {
        match self.current_route() {
            Route::Artist(_) => {
                self.artist_for = None;
                self.artist_error = None;
            }
            Route::Album(_) => {
                self.album_for = None;
                self.album_error = None;
            }
            Route::Playlist(_) => {
                self.playlist_for = None;
                self.playlist_error = None;
            }
            Route::Mood(_) => {
                self.mood_playlists_params = None;
                self.mood_playlists_error = None;
            }
            Route::Library => {
                self.library_error = None;
                self.library_loaded = false;
                self.library = crate::model::Library::default();
                self.resync_local_playlists();
            }
            Route::Home => {
                self.home_error = None;
                self.home = None;
            }
            Route::Explore => {
                self.explore_error = None;
                self.explore = None;
                self.explore_links.clear();
            }
            Route::NewReleases => {
                self.releases_error = None;
                self.releases = None;
            }
            Route::Charts(_) => {
                self.charts_error = None;
                self.charts = None;
            }
            Route::Moods => {
                self.moods_error = None;
                self.moods = None;
            }
            Route::History => {
                self.history_error = None;
                self.history_shelves = None;
            }
            Route::Search | Route::Settings => return,
        }
        self.scroll_to_top = true;
        self.ensure_route_loaded();
    }

    /// Fetch the full list behind one artist shelf's "Show all".
    pub(crate) fn expand_artist_shelf(&mut self, title: &str) {
        if self.artist_expand_loading {
            return;
        }
        let Some(artist) = &self.artist else { return };
        let Some(shelf) = artist.shelves.iter().find(|s| s.title == title) else {
            return;
        };
        let Some(params) = shelf.more_params.clone() else {
            return;
        };
        // The shelf's own browse id, when we have one — a "more" button routinely points
        // somewhere other than the artist's own channel id (see `Shelf::more_browse_id`).
        // Falling back to the artist's id here only matters for the typed-API fallback shelves,
        // which never carry one; every raw-parsed shelf has its real id.
        let browse_id = shelf
            .more_browse_id
            .clone()
            .unwrap_or_else(|| artist.id.clone());
        self.artist_expand = Some((browse_id.clone(), title.to_owned()));
        self.artist_expand_loading = true;
        self.artist_expand_error = None;
        self.backend.fetch_artist_albums(browse_id, params);
    }

    /// Change the library's order.
    ///
    /// Refetches from page one, because the order is the server's and applies to the whole
    /// library — not to whatever happens to be loaded.
    pub(crate) fn sort_library(&mut self, sort: LibrarySort) {
        if self.library_sort == sort {
            return;
        }
        self.library_sort = sort;
        self.library = crate::model::Library::default();
        self.library_loaded = false;
        self.library_error = None;
        self.resync_local_playlists();
        if matches!(self.auth, crate::auth::AuthState::SignedIn { .. }) {
            self.library_loading = true;
            self.backend.fetch_library(sort.param());
        }
    }

    /// Change how the open playlist's track list is shown.
    ///
    /// The playlist endpoints take no sort of their own — unlike [`Self::sort_library`], there
    /// is no server order to hand this to, so this always sorts the tracks already loaded. If
    /// the playlist has not fully loaded yet (`tracks.next.is_some()`), sorting only what's
    /// loaded would present a truncated list as the whole playlist — the mistake `LibrarySort`
    /// was already changed to avoid — so the remaining pages load first, with the ordinary
    /// loading-row affordance shown meanwhile, and the sort applies once the last page arrives
    /// (see the `Event::PlaylistMore` handler).
    pub(crate) fn sort_playlist(&mut self, sort: PlaylistSort) {
        self.playlist_sort = sort;
        let Some(playlist) = self.playlist.as_mut() else {
            self.playlist_sort_pending = false;
            return;
        };
        if playlist.tracks.next.is_none() {
            self.playlist_sort_pending = false;
            apply_playlist_sort(
                sort,
                &mut playlist.tracks.items,
                &mut self.playlist_original_order,
            );
            return;
        }
        self.playlist_sort_pending = true;
        self.load_more_playlist();
    }

    /// Ask for the next page of the open playlist's tracks.
    pub(crate) fn load_more_playlist(&mut self) {
        let Some(playlist) = self.playlist.as_mut() else {
            return;
        };
        if !playlist.tracks.can_load_more() {
            return;
        }
        let Some(token) = playlist.tracks.next.clone() else {
            return;
        };
        playlist.tracks.loading = true;
        let id = playlist.id.clone();
        self.backend.fetch_playlist_more(id, token);
    }

    /// Ask for the next page of one library section, if there is one and none is in flight.
    pub(crate) fn load_more_library(&mut self, section: crate::api::LibrarySection) {
        let (can, token) = match section {
            crate::api::LibrarySection::Songs => (
                self.library.songs.can_load_more(),
                self.library.songs.next.clone(),
            ),
            crate::api::LibrarySection::Artists => (
                self.library.artists.can_load_more(),
                self.library.artists.next.clone(),
            ),
            crate::api::LibrarySection::Albums => (
                self.library.albums.can_load_more(),
                self.library.albums.next.clone(),
            ),
            crate::api::LibrarySection::Playlists => (
                self.library.playlists.can_load_more(),
                self.library.playlists.next.clone(),
            ),
        };
        let (true, Some(token)) = (can, token) else {
            return;
        };
        match section {
            crate::api::LibrarySection::Songs => self.library.songs.loading = true,
            crate::api::LibrarySection::Artists => self.library.artists.loading = true,
            crate::api::LibrarySection::Albums => self.library.albums.loading = true,
            crate::api::LibrarySection::Playlists => self.library.playlists.loading = true,
        }
        self.backend.fetch_library_more(section, token);
    }

    /// Anything with a request in flight — the progress bar's condition and the repaint
    /// schedule's, which must agree or the sweep stalls mid-travel.
    fn anything_loading(&self) -> bool {
        self.searching
            || self.now.resolving
            || self.artist_loading
            || self.album_loading
            || self.playlist_loading
            || self.artist_expand_loading
            || self.home_loading
            || self.mood_playlists_loading
            || self.explore_loading
            || self.history_loading
            || self.releases_loading
            || self.charts_loading
            || self.moods_loading
            || self.library_loading
            || self.radio_loading
            || self.library.playlists.loading
    }

    /// Fill the sidebar's playlist list as soon as there is a session.
    ///
    /// They come from the library, and the library only loaded when the Library page was opened
    /// — so the rail stayed empty until you clicked Library once, every run. Only the playlists
    /// section is fetched: it is the only one the rail shows, and pulling all four at startup
    /// cost 20 MB of the idle memory budget for three lists nothing was displaying.
    ///
    /// A stored cookie counts: the fetch fires while the session is still `Checking`, and a
    /// refusal simply leaves the rail empty — the state that would have shown anyway.
    fn load_sidebar_playlists(&mut self) {
        if !matches!(
            self.auth,
            crate::auth::AuthState::SignedIn { .. } | crate::auth::AuthState::Checking
        ) || self.library_loaded
            || self
                .library
                .playlists
                .items
                .iter()
                .any(|p| p.source == crate::model::PlaylistSource::YouTube)
            || self.library.playlists.loading
        {
            return;
        }
        self.library.playlists.loading = true;
        self.backend.fetch_library_playlists();
    }

    /// Mirrors `local_playlists` into `library.playlists.items` as `PlaylistRef`s, so the
    /// sidebar and the Library page render one merged list without knowing two sources exist.
    /// Called after anything that fully replaces `library.playlists` (a real fetch landing, a
    /// sign-out clearing it) and after any local-playlist mutation — the YouTube-sourced entries
    /// those places just set are left alone; only the local ones are dropped and rebuilt.
    pub(crate) fn resync_local_playlists(&mut self) {
        self.library
            .playlists
            .items
            .retain(|p| p.source != crate::model::PlaylistSource::Local);
        self.library
            .playlists
            .items
            .extend(self.local_playlists.iter().map(local_playlist_ref));
    }

    // ------------------------------------------------------------------------------------------
    // Search
    // ------------------------------------------------------------------------------------------

    /// Run the typed query. Search is submit-driven, not live: typing only refreshes the
    /// autocomplete list, so the results change on Enter, on a chosen suggestion, or from the
    /// `--search` CLI — never mid-keystroke. This is what keeps a network round trip from firing
    /// for every character and what lets the listener finish typing before the page moves.
    pub(crate) fn submit_search(&mut self) {
        if !matches!(self.current_route(), Route::Search) {
            self.navigate(Route::Search);
        }
        // Mark autocomplete as already fetched for this exact text so the field does not
        // immediately re-open its suggestion list for the query that was just submitted.
        self.suggestions_for = Some(self.query.trim().to_owned());
        self.pending_search = true;
    }

    /// Drop the autocomplete list and its cursor state.
    pub(crate) fn clear_suggestions(&mut self) {
        self.suggestions.clear();
        self.suggestions_for = None;
        self.suggestion_cursor = 0;
        self.suggestion_navigated = false;
    }

    /// Fire a submitted search. Returns how long until the next check is due, so the caller can
    /// schedule exactly one repaint rather than spinning.
    fn poll_search(&mut self) {
        if !std::mem::take(&mut self.pending_search) {
            return;
        }
        let query = self.query.trim().to_owned();
        if query.is_empty() || query == self.searched {
            return;
        }

        self.searched = query.clone();
        self.searching = true;
        self.status = None;
        // Drop the previous query's rows so the page shows its skeleton instead of results that
        // no longer answer what was asked.
        self.results = SearchResults::default();
        self.backend.search(query);
        if self.search_scope != SearchScope::All {
            self.fetch_scoped_search(self.search_scope);
        }
    }

    /// Fetch autocomplete once per settled query, and only while the field has focus — a hidden
    /// field needs none. The guard is `suggestions_for`, so re-typing the same text does not
    /// refetch. Returns a repaint delay while the debounce is still running. This is the *only*
    /// thing typing triggers; the search itself waits for a submit.
    fn poll_suggestions(&mut self, ctx: &egui::Context) -> Option<Duration> {
        if self.query.trim().is_empty() {
            self.suggestions.clear();
            self.suggestions_for = None;
            self.suggestion_cursor = 0;
            return None;
        }
        if !ctx.memory(|m| m.has_focus(crate::ui::topbar::search_id())) {
            return None;
        }
        if self.suggestions_for.as_deref() == Some(self.query.as_str()) {
            return None;
        }
        let waited = self.query_changed_at?.elapsed();
        if waited < SUGGEST_DEBOUNCE {
            return Some(SUGGEST_DEBOUNCE - waited);
        }
        self.suggestions_for = Some(self.query.clone());
        self.backend.fetch_suggestions(self.query.clone());
        None
    }

    /// Kicks off the dedicated per-category fetch for `scope` against the current `searched`
    /// query, unless a fetch for that exact `(query, scope)` pair is already outstanding or
    /// done. Called when a scope chip is picked, and again from `poll_search` whenever a fresh
    /// search lands while a non-`All` scope is already selected.
    pub(crate) fn fetch_scoped_search(&mut self, scope: SearchScope) {
        if scope == SearchScope::All || self.searched.is_empty() {
            return;
        }
        if self
            .scoped_for
            .as_ref()
            .is_some_and(|(q, s)| q == &self.searched && *s == scope)
        {
            return;
        }
        self.scoped_for = Some((self.searched.clone(), scope));
        self.scoped_loading = true;
        self.scoped_error = None;
        self.scoped_results.clear();
        self.backend.search_scoped(self.searched.clone(), scope);
    }

    fn handle_events(&mut self, ctx: &egui::Context) {
        // Mouse side buttons walk the history, like a browser — on press, not on click.
        // A click needs a clean press/release cycle and egui tracks one shared press
        // origin, so any concurrent drag or pointer movement voids the release and the
        // button "does nothing". `back`/`forward` are no-ops with nowhere to go.
        if ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Extra1)) {
            self.back();
        }
        if ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Extra2)) {
            self.forward();
        }
        for event in self.backend.drain() {
            match event {
                // A slow search that finished after the user typed something else must not
                // replace fresher results.
                Event::SearchDone { query, results } if query == self.searched => {
                    self.searching = false;
                    self.status = results.is_empty().then(|| "No results".to_owned());
                    self.results = results;
                    let playable = self.results.playable();
                    if self.autoplay && !playable.is_empty() {
                        self.autoplay = false;
                        self.start_from(playable, 0);
                    }
                }
                Event::SearchFailed { query, error } if query == self.searched => {
                    self.searching = false;
                    self.status = Some(error);
                }
                Event::SearchDone { .. } | Event::SearchFailed { .. } => {}

                // Same staleness guard as above, but against `scoped_for` rather than
                // `searched` — a scope switch (or a fresh search) can outrun an earlier
                // scoped fetch, and that reply must not overwrite a newer one.
                Event::ScopedSearchDone {
                    query,
                    scope,
                    results,
                } if self
                    .scoped_for
                    .as_ref()
                    .is_some_and(|(q, s)| q == &query && *s == scope) =>
                {
                    self.scoped_loading = false;
                    self.scoped_results = results;
                }
                Event::ScopedSearchFailed {
                    query,
                    scope,
                    error,
                } if self
                    .scoped_for
                    .as_ref()
                    .is_some_and(|(q, s)| q == &query && *s == scope) =>
                {
                    self.scoped_loading = false;
                    self.scoped_error = Some(error);
                }
                Event::ScopedSearchDone { .. } | Event::ScopedSearchFailed { .. } => {}

                // A reply for a query no longer in the box is dropped, the same staleness guard
                // `scoped_for` uses: typing on must not flash the previous suggestions back up.
                Event::SuggestionsDone { query, items } => {
                    if self.query == query {
                        self.suggestions = items;
                        self.suggestions_for = Some(query);
                        self.suggestion_cursor = 0;
                    }
                }
                Event::SuggestionsFailed { query, error } => {
                    tracing::debug!(%query, %error, "suggestions failed");
                }

                Event::DownloadProgress { video_id, percent } => {
                    self.download_progress.insert(video_id, percent);
                }
                Event::DownloadDone {
                    video_id,
                    path,
                    track,
                } => {
                    self.download_progress.remove(&video_id);
                    self.downloads.entries.insert(
                        video_id,
                        crate::downloads::Record {
                            path: path.to_string_lossy().into_owned(),
                            track: crate::playback::PersistedTrack::from_track(&track),
                        },
                    );
                    if let Err(e) = crate::downloads::save(&self.downloads) {
                        tracing::warn!(error = %format!("{e:#}"), "could not save the download index");
                    }
                    self.push_downloads_to_backend();
                    self.toast(&format!("Downloaded “{}”", track.title));
                }
                Event::DownloadFailed { video_id, error } => {
                    self.download_progress.remove(&video_id);
                    tracing::warn!(%video_id, %error, "download failed");
                    self.toast_error("Could not download that track");
                }

                Event::LocalScanDone { tracks } => {
                    self.local_scanning = false;
                    self.local_scan_error = None;
                    self.local_tracks = tracks;
                }
                Event::LocalScanFailed { error } => {
                    self.local_scanning = false;
                    self.local_scan_error = Some(error);
                }

                // A reply is kept only when it answers the fetch still outstanding. Matching on
                // `*_for` rather than on the visible route matters: a reply that arrives while
                // you are on another page is still the answer to the fetch we are waiting for,
                // and dropping it silently is what used to leave the page loading forever.
                Event::ArtistLoaded { id, artist } if self.artist_is_for(&id) => {
                    self.artist_loading = false;
                    // The page itself carries only the first 5 top songs; the rest (up to the
                    // 15 Top Songs shows) live in the shelf's own auto-generated playlist,
                    // fetched here so the page opens already showing as many as it can.
                    let more = artist.top_songs_playlist.clone();
                    self.artist = Some(artist);
                    if let Some(playlist_id) = more {
                        self.backend.fetch_playlist(playlist_id);
                    }
                }
                Event::ArtistFailed { id, error } if self.artist_is_for(&id) => {
                    self.artist_loading = false;
                    self.artist_error = Some(error);
                }
                Event::ArtistLoaded { .. } | Event::ArtistFailed { .. } => {}

                Event::AlbumLoaded { id, album } if self.album_is_for(&id) => {
                    self.album_loading = false;
                    self.cache_album(&id, &album);
                    self.album = Some(album);
                }
                Event::AlbumFailed { id, error } if self.album_is_for(&id) => {
                    self.album_loading = false;
                    self.album_error = Some(error);
                }
                // A release card's right-click prefetch, not a page view.
                Event::AlbumLoaded { id, album } => self.cache_album(&id, &album),
                Event::AlbumFailed { .. } => {}

                // An album/playlist card's play button, not a page view — see
                // `Event::CardTracksReady`'s own doc comment for why this isn't folded into
                // `AlbumLoaded`/`PlaylistLoaded`. Same "already loaded → pause/resume" rule
                // the album/playlist header's own Play button follows.
                Event::CardTracksReady { id, tracks } if !tracks.is_empty() => {
                    // See `NowPlaying::playlist_playing`'s own doc comment: a playlist card has
                    // no per-track field naming it the way an album card reads `album_id`, so
                    // this is what lets its own play button show Pause too.
                    self.now.playlist_playing =
                        Some((id, tracks.iter().map(|t| t.video_id.clone()).collect()));
                    if self.queue_matches(&tracks) {
                        self.toggle_playback();
                    } else {
                        self.start_from(tracks, 0);
                    }
                }
                Event::CardTracksReady { .. } => {}

                // The expansion replaces the shelf it came from, in place, and drops its
                // "Show all" button along with it. `expanded` is what `artist.rs::shelves()`
                // reads to switch that shelf from its carousel preview to the full boxed grid.
                Event::ArtistAlbumsLoaded { artist_id, items } => {
                    self.artist_expand_loading = false;
                    if let Some((id, title)) = self.artist_expand.clone()
                        && id == artist_id
                        && let Some(artist) = self.artist.as_mut()
                        && let Some(shelf) = artist.shelves.iter_mut().find(|s| s.title == title)
                    {
                        shelf.items = items;
                        shelf.more_params = None;
                        shelf.expanded = true;
                    }
                }
                Event::ArtistAlbumsFailed { artist_id, error } => {
                    self.artist_expand_loading = false;
                    if self
                        .artist_expand
                        .as_ref()
                        .is_some_and(|(id, _)| *id == artist_id)
                    {
                        self.artist_expand_error = Some(error);
                    }
                }

                Event::PlaylistLoaded { id, playlist } if self.playlist_is_for(&id) => {
                    self.playlist_loading = false;
                    // Keep it for the rail's context menu too.
                    self.cache_rail_playlist(&id, &playlist);
                    self.playlist = Some(playlist);
                    // Edit from the rail: the page has arrived, so open its editor now.
                    if self.edit_on_load.as_deref() == Some(id.as_str()) {
                        self.edit_on_load = None;
                        self.open_playlist_editor();
                    }
                }
                // A further page of the open playlist, appended in order.
                Event::PlaylistMore { id, page } if self.playlist_is_for(&id) => {
                    if let Some(playlist) = self.playlist.as_mut() {
                        let next = page.next;
                        playlist.tracks.extend(
                            page.items
                                .into_iter()
                                .filter_map(|e| e.track().cloned())
                                .collect(),
                            next,
                        );
                    }
                    // A sort is waiting on the rest of the playlist: either it just arrived, or
                    // there is still more to fetch before it can apply.
                    if self.playlist_sort_pending {
                        let fully_loaded = self
                            .playlist
                            .as_ref()
                            .is_some_and(|p| p.tracks.next.is_none());
                        if fully_loaded {
                            self.playlist_sort_pending = false;
                            let sort = self.playlist_sort;
                            if let Some(playlist) = self.playlist.as_mut() {
                                apply_playlist_sort(
                                    sort,
                                    &mut playlist.tracks.items,
                                    &mut self.playlist_original_order,
                                );
                            }
                        } else {
                            self.load_more_playlist();
                        }
                    }
                }
                Event::PlaylistMoreFailed { id, error } if self.playlist_is_for(&id) => {
                    tracing::warn!(%id, %error, "could not load another playlist page");
                    if let Some(playlist) = self.playlist.as_mut() {
                        playlist.tracks.loading = false;
                    }
                    // The chain that was fetching the rest of the playlist to sort it broke —
                    // leave what's loaded in its current order rather than retrying forever.
                    self.playlist_sort_pending = false;
                }
                Event::PlaylistMore { .. } | Event::PlaylistMoreFailed { .. } => {}

                Event::PlaylistFailed { id, error } if self.playlist_is_for(&id) => {
                    self.playlist_loading = false;
                    self.playlist_error = Some(error);
                }
                // The open artist's own top-songs prefetch, not the playlist page: replace the
                // page's 5 embedded songs with up to 15 of the real list, so Top Songs can page
                // through them (see `track_shelf_pages`) instead of sitting at just the 5.
                Event::PlaylistLoaded { id, playlist }
                    if self
                        .artist
                        .as_ref()
                        .is_some_and(|a| a.top_songs_playlist.as_deref() == Some(id.as_str())) =>
                {
                    if let Some(artist) = self.artist.as_mut() {
                        artist.top_songs = playlist.tracks.items.into_iter().take(15).collect();
                    }
                }
                // A prefetch for the rail's context menu: keep it, do not touch the page.
                Event::PlaylistLoaded { id, playlist } => self.cache_rail_playlist(&id, &playlist),
                Event::PlaylistFailed { .. } => {}

                Event::LibraryLoaded { library } => {
                    self.library_loading = false;
                    self.library = library;
                    self.library_loaded = true;
                    self.resync_local_playlists();
                    self.apply_playlist_order();
                }
                // A further page of one section, appended in order.
                Event::LibraryMore { section, page } => {
                    let next = page.next;
                    match section {
                        crate::api::LibrarySection::Songs => self.library.songs.extend(
                            page.items
                                .into_iter()
                                .filter_map(|e| e.track().cloned())
                                .collect(),
                            next,
                        ),
                        crate::api::LibrarySection::Artists => self.library.artists.extend(
                            page.items
                                .into_iter()
                                .filter_map(|e| match e {
                                    crate::model::Entry::Artist(a) => Some(a),
                                    _ => None,
                                })
                                .collect(),
                            next,
                        ),
                        crate::api::LibrarySection::Albums => self.library.albums.extend(
                            page.items
                                .into_iter()
                                .filter_map(|e| match e {
                                    crate::model::Entry::Album(a) => Some(a),
                                    _ => None,
                                })
                                .collect(),
                            next,
                        ),
                        crate::api::LibrarySection::Playlists => {
                            self.library.playlists.extend(
                                page.items
                                    .into_iter()
                                    .filter_map(|e| match e {
                                        crate::model::Entry::Playlist(p) => Some(p),
                                        _ => None,
                                    })
                                    .collect(),
                                next,
                            );
                            self.apply_playlist_order();
                        }
                    }
                }
                // The sidebar's own prefetch. It *replaces* rather than appends, and yields
                // entirely to a real library load: the two race, and appending after one had
                // already landed listed every playlist twice.
                Event::SidebarPlaylists { page } => {
                    if self.library_loaded {
                        self.library.playlists.loading = false;
                    } else {
                        self.library.playlists = crate::model::Paged::new(
                            page.items
                                .into_iter()
                                .filter_map(|e| match e {
                                    crate::model::Entry::Playlist(p) => Some(p),
                                    _ => None,
                                })
                                .collect(),
                            page.next,
                        );
                        self.resync_local_playlists();
                        self.apply_playlist_order();
                    }
                }
                Event::SidebarPlaylistsFailed { error } => {
                    tracing::warn!(%error, "could not load the sidebar's playlists");
                    self.library.playlists.loading = false;
                }
                Event::LibraryMoreFailed { section, error } => {
                    tracing::warn!(?section, %error, "could not load another library page");
                    match section {
                        crate::api::LibrarySection::Songs => self.library.songs.loading = false,
                        crate::api::LibrarySection::Artists => self.library.artists.loading = false,
                        crate::api::LibrarySection::Albums => self.library.albums.loading = false,
                        crate::api::LibrarySection::Playlists => {
                            self.library.playlists.loading = false
                        }
                    }
                }
                Event::LibraryFailed { error } => {
                    self.library_loading = false;
                    self.library_error = Some(error);
                }

                Event::SubscriptionChanged {
                    channel_id,
                    subscribed,
                } => {
                    tracing::debug!(%channel_id, subscribed, "subscription updated");
                    self.toast(if subscribed {
                        "Subscribed"
                    } else {
                        "Unsubscribed"
                    });
                }
                Event::SubscriptionFailed {
                    channel_id,
                    previous,
                    error,
                } => {
                    tracing::warn!(%channel_id, %error, "subscription refused; reverting");
                    self.set_subscribed_state(&channel_id, previous);
                    self.toast_error("Could not update that subscription");
                }
                Event::ExploreLoaded { shelves, links } => {
                    self.explore_loading = false;
                    self.explore = Some(Arc::new(shelves));
                    self.explore_links = links;
                }
                Event::ExploreFailed { error } => {
                    self.explore_loading = false;
                    self.explore_error = Some(error);
                }
                Event::ReleasesLoaded { shelves } => {
                    self.releases_loading = false;
                    self.releases = Some(Arc::new(shelves));
                }
                Event::ReleasesFailed { error } => {
                    self.releases_loading = false;
                    self.releases_error = Some(error);
                }
                Event::ChartsLoaded { params, feed } => {
                    if self.charts_params != params {
                        return;
                    }
                    self.charts_loading = false;
                    self.charts = Some(Arc::new(feed));
                }
                Event::ChartsFailed { params, error } => {
                    if self.charts_params != params {
                        return;
                    }
                    self.charts_loading = false;
                    self.charts_error = Some(error);
                }
                Event::MoodsLoaded { sections } => {
                    self.moods_loading = false;
                    self.moods = Some(Arc::new(sections));
                }
                Event::MoodsFailed { error } => {
                    self.moods_loading = false;
                    self.moods_error = Some(error);
                }
                Event::HistoryLoaded { shelves } => {
                    self.history_loading = false;
                    self.history_shelves = Some(Arc::new(shelves));
                }
                Event::HistoryFailed { error } => {
                    self.history_loading = false;
                    self.history_error = Some(error);
                }
                Event::PlaylistCreated { title } => {
                    self.toast(&format!("Created “{title}”"));
                    // The rail lists what the library holds, so it has to be asked again before
                    // the new playlist can appear in it.
                    self.library.playlists = crate::model::Paged::default();
                    self.library_loaded = false;
                    self.resync_local_playlists();
                    self.load_sidebar_playlists();
                }
                Event::PlaylistCreateFailed { error } => {
                    tracing::warn!(%error, "could not create the playlist");
                    self.toast_error("Could not create that playlist");
                }
                Event::HomeLoaded {
                    filter,
                    shelves,
                    chips,
                } => {
                    // A reply for a filter since changed is stale — drop it rather than
                    // showing one filter's shelves under another's chips.
                    if self.home_filter == filter {
                        self.home_loading = false;
                        self.home = Some(Arc::new(shelves));
                        self.home_chips = chips;
                    }
                }
                Event::HomeFailed { filter, error } => {
                    if self.home_filter == filter {
                        self.home_loading = false;
                        self.home_error = Some(error);
                    }
                }

                Event::MoodPlaylistsLoaded { params, mood }
                    if self.mood_playlists_params.as_deref() == Some(params.as_str()) =>
                {
                    self.mood_playlists_loading = false;
                    self.mood_playlists = Some(mood);
                }
                Event::MoodPlaylistsFailed { params, error }
                    if self.mood_playlists_params.as_deref() == Some(params.as_str()) =>
                {
                    self.mood_playlists_loading = false;
                    self.mood_playlists_error = Some(error);
                }
                Event::MoodPlaylistsLoaded { .. } | Event::MoodPlaylistsFailed { .. } => {
                    self.mood_playlists_loading = false;
                }

                Event::AccountsLoaded { accounts } => self.accounts = accounts,

                Event::SignInFailed { error } => {
                    self.sign_in_error = Some(error);
                }

                Event::AuthChanged(state) => {
                    match &state {
                        // A fresh session invalidates any previously fetched library.
                        crate::auth::AuthState::SignedIn { .. } => {
                            self.library_loaded = false;
                            self.sign_in_error = None;
                            // `Checking` → `SignedIn` is the ordinary *startup* confirmation of
                            // a stored session — and `Api::client_ref` already uses that stored
                            // cookie optimistically for every request from the very first
                            // frame, before this event ever fires (see its own doc comment), so
                            // a home feed fetched while still `Checking` already came back
                            // personalised. Wiping and refetching it here regardless used to
                            // show a pointless second load on every single launch: the feed
                            // flashing back to its skeleton and reappearing with, as far as the
                            // user can see, the same content. Only a *real* transition into
                            // signed-in — from `SignedOut`/`Failed`, finishing sign-in from
                            // Settings mid-session — actually invalidates what's already
                            // loaded, since that feed genuinely was the anonymous one.
                            if !matches!(self.auth, crate::auth::AuthState::Checking) {
                                self.home = None;
                                self.home_error = None;
                                self.history_shelves = None;
                                self.history_error = None;
                            }
                            // The topbar's photo chip reads these; asking once here beats
                            // every-frame refires from the pages that display them.
                            self.backend.fetch_accounts();
                            // Liked Songs is also how a Radio/Mix track's like status gets
                            // backfilled (`reconcile_state`) — waiting for the Library page to
                            // be visited left it stale for most of a session that never opened
                            // one.
                            self.load_library();
                        }
                        crate::auth::AuthState::SignedOut => {
                            self.library = crate::model::Library::default();
                            self.library_loaded = false;
                            // Local playlists have nothing to do with a YouTube session — they
                            // stay visible signed out too.
                            self.resync_local_playlists();
                            // The feed is personalised; a session change invalidates it.
                            self.home = None;
                            self.home_error = None;
                            self.history_shelves = None;
                            self.history_error = None;
                        }
                        _ => {}
                    }
                    self.auth = state;
                    self.load_sidebar_playlists();
                }

                Event::Resolving(mut track) => {
                    self.reconcile_state(&mut track);
                    self.now = NowPlaying {
                        track: Some(track),
                        resolving: true,
                        error: None,
                        paused: false,
                        playlist_playing: self.now.playlist_playing.clone(),
                    };
                }
                Event::Playing(mut track) => {
                    self.reconcile_state(&mut track);
                    let video_id = track.video_id.clone();
                    let wide = track.wide;
                    self.now = NowPlaying {
                        track: Some(track),
                        resolving: false,
                        error: None,
                        paused: false,
                        playlist_playing: self.now.playlist_playing.clone(),
                    };
                    self.playback_started = true;
                    // A track with no video cannot keep the video view up.
                    if self.video_open && !wide {
                        self.video_open = false;
                    }
                    // A fresh track begins the crossfade's ramp-in (a no-op when it is off).
                    self.fade_in = Some(Instant::now());
                    self.prefetch_next();
                    // Lyrics follow the track while the panel is open; a song change starts a
                    // fresh request, and the guard drops it if the track changes again.
                    if self.lyrics_open
                        && self.lyrics_for.as_deref() != Some(video_id.as_str())
                        && let Some(track) = self.now.track.clone()
                    {
                        self.fetch_lyrics(track);
                    }
                }
                Event::MixLoaded { seed, mut queue } => {
                    // The player overlay is the now-playing song's own rating and the only like
                    // state a mix carries; write it onto the first row so the bar's thumbs show
                    // it. (Rows have no button of their own.)
                    if let Some(like) = queue.like
                        && let Some(t) = queue.tracks.first_mut()
                    {
                        t.like = like;
                    }
                    self.start_from(queue.tracks, 0);
                    // Set after `start_from`: a hand-picked start opens a latent river, a mix
                    // opens a flowing one. No token means the mix itself was one page long.
                    self.radio = Some(Radio {
                        state: queue.next.map_or(River::Dead, River::Open),
                        chosen: vec![seed.clone()],
                        seed,
                    });
                }
                Event::MixFailed { seed, error } => {
                    tracing::warn!(%seed, %error, "start mix failed");
                    self.toast_error("Could not start a mix");
                }
                Event::MixSeeded { seed, queue } => {
                    self.radio_loading = false;
                    // Only the pending river this page was fetched for may take it: a queue
                    // started while the request was in flight is pending with its own seed, and
                    // an open river (Start Mix) is not pending at all. Switching Autoplay off
                    // mid-flight drops the page the same way.
                    let fresh = self.settings.autoplay
                        && self
                            .radio
                            .as_ref()
                            .is_some_and(|r| r.state == River::Pending && r.seed == seed);
                    if fresh {
                        let added = append_new(&mut self.queue, &queue.tracks);
                        tracing::debug!(added, "seeded the autoplay tail");
                        if let Some(radio) = &mut self.radio {
                            radio.state = landed(added, queue.next);
                        }
                    }
                }
                Event::MixSeedFailed { seed, error } => {
                    self.radio_loading = false;
                    // Same rule as a dead continuation: never retried, so a request storm is
                    // impossible. The queue simply ends where the listener's tracks do.
                    if self
                        .radio
                        .as_ref()
                        .is_some_and(|r| r.state == River::Pending && r.seed == seed)
                    {
                        tracing::warn!(%error, "autoplay seed failed");
                        if let Some(radio) = &mut self.radio {
                            radio.state = River::Dead;
                        }
                    }
                }
                Event::LyricsLoaded { video_id, lyrics }
                    if self.lyrics_for.as_deref() == Some(video_id.as_str()) =>
                {
                    self.lyrics_loading = false;
                    self.lyrics_error = None;
                    self.lyrics = Some(lyrics);
                }
                Event::LyricsFailed { video_id, error }
                    if self.lyrics_for.as_deref() == Some(video_id.as_str()) =>
                {
                    tracing::debug!(%video_id, %error, "no lyrics");
                    self.lyrics_loading = false;
                    self.lyrics = None;
                    self.lyrics_error = Some(error);
                }
                Event::LyricsLoaded { .. } | Event::LyricsFailed { .. } => {}
                Event::MixMore { token, queue } => {
                    self.radio_loading = false;
                    // A newer river may have opened while this was in flight — appending
                    // another river's tracks would corrupt the queue, so stale replies drop.
                    let fresh = self.settings.autoplay
                        && self
                            .radio
                            .as_ref()
                            .is_some_and(|r| matches!(&r.state, River::Open(t) if t == &token));
                    if fresh {
                        let added = append_new(&mut self.queue, &queue.tracks);
                        tracing::debug!(added, "extended the radio river");
                        if let Some(radio) = &mut self.radio {
                            // A page that adds nothing ends the river. Keeping its token would
                            // leave the low-water mark still tripped, so the next frame asks
                            // again — and a river that keeps handing back tracks we already
                            // hold would be requested forever, one round trip at a time.
                            radio.state = landed(added, queue.next);
                        }
                    }
                }
                Event::MixMoreFailed { token, error } => {
                    self.radio_loading = false;
                    // Both the follow and the reseed failed — the network or the session is
                    // at fault, and retrying every frame would be a request storm. End the
                    // river quietly; what it delivered keeps playing and stays marked.
                    if self
                        .radio
                        .as_ref()
                        .is_some_and(|r| matches!(&r.state, River::Open(t) if t == &token))
                    {
                        tracing::warn!(%error, "radio river ended");
                        if let Some(radio) = &mut self.radio {
                            radio.state = River::Dead;
                        }
                    }
                }
                Event::RatingFailed {
                    video_id,
                    attempted,
                    previous,
                    error,
                } => {
                    // Revert only when the failed attempt is still what is on screen — a newer
                    // rating has its own refusal coming if it needs one.
                    if self.like_of(&video_id) == attempted {
                        tracing::warn!(%video_id, %error, "rating refused; reverting");
                        self.set_like(&video_id, previous);
                        self.toast_error("YouTube refused that rating");
                    }
                }
                Event::LibraryToggled {
                    video_id,
                    in_library,
                } => {
                    // Confirmation only: the state was painted optimistically when asked for,
                    // and per-video toggles resolve in the order they were tapped.
                    tracing::debug!(%video_id, in_library, "library toggle confirmed");
                    self.toast(if in_library {
                        "Saved to library"
                    } else {
                        "Removed from library"
                    });
                }
                Event::LibraryToggleFailed {
                    video_id,
                    previous,
                    error,
                } => {
                    tracing::warn!(%video_id, %error, "library toggle refused; reverting");
                    self.set_library_state(&video_id, previous);
                    self.toast_error("Could not update your library");
                }
                Event::PageLibraryToggled { in_library, .. } => {
                    tracing::debug!(in_library, "page library toggle confirmed");
                    // Already painted optimistically; this is the confirmation. The rail was
                    // spliced by the page that asked (see `splice_rail_playlist`); refetching
                    // here would race server propagation and can put a stale list back over
                    // the splice, so the next library load stays authoritative instead.
                    self.toast(if in_library {
                        "Saved to library"
                    } else {
                        "Removed from library"
                    });
                }
                Event::PlaylistEdited { id } => {
                    tracing::debug!(%id, "playlist updated");
                    self.edit_revert = None;
                    self.toast("Playlist updated");
                }
                Event::PlaylistEditFailed { id, error } => {
                    tracing::warn!(%id, %error, "playlist update refused");
                    if let Some((revert_id, old)) = self.edit_revert.take()
                        && revert_id == id
                    {
                        self.set_playlist_title(&id, old);
                    }
                    self.toast_error("Could not update the playlist");
                }
                Event::PlaylistDeleted { id } => {
                    tracing::debug!(%id, "playlist deleted");
                    self.drop_playlist(&id);
                    let was_open = self.current_route() == Route::Playlist(id.clone());
                    if was_open {
                        self.navigate(Route::Library);
                    }
                    self.toast("Playlist deleted");
                }
                Event::PlaylistDeleteFailed { id, error } => {
                    tracing::warn!(%id, %error, "playlist delete refused");
                    self.toast_error("Could not delete the playlist");
                }
                Event::PlaylistLibraryChanged { id, saved } => {
                    tracing::debug!(%id, saved, "playlist library membership changed");
                    if !saved {
                        self.drop_playlist(&id);
                        self.toast("Removed from library");
                    }
                }
                Event::PlaylistLibraryFailed { id, error } => {
                    tracing::warn!(%id, %error, "playlist library write refused");
                    self.toast_error("Could not update your library");
                }
                Event::PageLibraryFailed {
                    album,
                    previous,
                    error,
                } => {
                    tracing::warn!(album, %error, "page library toggle refused; reverting");
                    self.set_page_library(album, previous);
                    self.toast_error("Could not update your library");
                }
                Event::PlaylistBulkSaved {
                    playlist_id,
                    added,
                    total,
                } => {
                    tracing::debug!(%playlist_id, added, total, "bulk save landed");
                    if added == 0 {
                        self.toast_error("Could not save to that playlist");
                    } else if added < total {
                        self.toast(&format!("{added} of {total} songs saved to playlist"));
                    } else if added == 1 {
                        self.toast("Saved to playlist");
                    } else {
                        self.toast(&format!("{added} songs saved to playlist"));
                    }
                }
                Event::PlaylistSaved {
                    playlist_id,
                    video_id,
                } => {
                    tracing::debug!(%playlist_id, %video_id, "saved to playlist");
                    self.toast("Saved to playlist");
                }
                Event::PlaylistSaveFailed {
                    playlist_id,
                    video_id,
                    error,
                } => {
                    tracing::warn!(%playlist_id, %video_id, %error, "save to playlist refused");
                    self.toast_error("Could not save to that playlist");
                }
                Event::PlaylistRemoved {
                    playlist_id,
                    video_id,
                } => {
                    tracing::debug!(%playlist_id, %video_id, "removed from playlist");
                    self.toast("Removed from playlist");
                }
                Event::PlaylistRemoveFailed {
                    playlist_id,
                    video_id,
                    error,
                } => {
                    // The row is already gone and cannot be reconstructed — reload the page.
                    // `playlist_for` is cleared first: `load_playlist` treats a matching id
                    // as already loading and would no-op.
                    tracing::warn!(%playlist_id, %video_id, %error, "remove refused; reloading");
                    self.toast_error("Could not remove that song");
                    if self.playlist.as_ref().is_some_and(|p| p.id == playlist_id) {
                        self.playlist = None;
                        self.playlist_for = None;
                        self.load_playlist(playlist_id);
                    }
                }
                Event::PlaylistMoved { playlist_id } => {
                    tracing::debug!(%playlist_id, "playlist reorder confirmed");
                }
                Event::PlaylistMoveFailed { playlist_id, error } => {
                    // The row already moved locally when the drag was released, and cannot be
                    // put back precisely — reload, the same recovery `PlaylistRemoveFailed`
                    // uses. `playlist_for` is cleared first: `load_playlist` treats a matching
                    // id as already loading and would no-op.
                    tracing::warn!(%playlist_id, %error, "reorder refused; reloading");
                    self.toast_error("Could not reorder that playlist");
                    if self.playlist.as_ref().is_some_and(|p| p.id == playlist_id) {
                        self.playlist = None;
                        self.playlist_for = None;
                        self.load_playlist(playlist_id);
                    }
                }
                Event::ImageReady(loaded) => self.images.insert(ctx, *loaded),
                Event::ImageFailed(url, px, wide) => self.images.mark_failed(url, px, wide),

                Event::PlaybackFailed { track, error } => {
                    self.now = NowPlaying {
                        track: Some(track),
                        resolving: false,
                        error: Some(error),
                        paused: false,
                        playlist_playing: self.now.playlist_playing.clone(),
                    };
                    self.playback_started = false;
                }
            }
        }
    }

    // ------------------------------------------------------------------------------------------
    // Playback
    // ------------------------------------------------------------------------------------------

    /// Make `tracks` the queue and start playing `index`.
    pub(crate) fn start_from(&mut self, tracks: Vec<Track>, index: usize) {
        if index >= tracks.len() {
            return;
        }
        // The clicked row is already the one loaded: pause/resume it, the same convention an
        // album/playlist header's own Play button already follows (`queue_matches`/
        // `toggle_playback`), rather than restarting it from the top.
        if self.queue_matches(&tracks[index..=index]) {
            self.toggle_playback();
            return;
        }
        // Leaving one queue for another: keep the whole old queue so `Previous` can bring it
        // back. The within-queue history indexes the old queue, so it is cleared here.
        if let Some(current) = self.current {
            let old = std::mem::take(&mut self.queue);
            self.play_history
                .leaving_queue(old, current, self.radio.take());
        }
        // Every hand-picked queue carries a latent river; `Event::MixLoaded` overwrites it
        // with a flowing one. The Autoplay setting decides whether either is asked for a
        // page, so switching it on mid-queue works.
        self.radio = Radio::pending(&tracks);
        self.queue = tracks;
        // A new queue has different tracks; the shuffle seen-set does not carry over.
        self.shuffle_seen.clear();
        self.play_index_restore(index);
        // A brand new queue, so the panel should show its start — matching what the queue
        // panel's own row click already does for the in-queue case. Set here rather than in
        // `play_index_restore` itself, which is also what natural end-of-track auto-advance and
        // Previous/Next share: neither of those should jerk the scroll position around on every
        // track change.
        self.queue_scroll_reset = true;
    }

    /// Start the queue item at `index`, remembering the track we are leaving for `Previous`.
    pub(crate) fn play_index(&mut self, index: usize) {
        // Already the loaded track: pause/resume it rather than restarting from the top — the
        // same convention `start_from` follows for the same reason. `playback_started` matters:
        // a restored queue arrives with `current` set but nothing loaded, and that case must
        // fall through and actually load the track rather than toggle (which would recurse).
        if self.current == Some(index) && self.playback_started {
            self.toggle_playback();
            return;
        }
        self.record_play();
        self.play_index_restore(index);
    }

    /// `play_index` without touching the history — for `Previous` itself and a fresh queue.
    fn play_index_restore(&mut self, index: usize) {
        let Some(track) = self.queue.get(index).cloned() else {
            return;
        };
        let Some(player) = self.player.as_mut() else {
            return;
        };
        let handle = player.handle();
        // The user's volume, not `player.volume()`: a crossfade may have faded the live value
        // down, and the next track must not inherit that.
        let volume = if self.muted { 0.0 } else { self.last_volume };
        let init_error = player.init_error().map(str::to_owned);
        let Some(handle) = handle else {
            // libmpv could not be brought up — surface it now, the same message an eager
            // failure used to show at startup.
            self.player_error = init_error;
            return;
        };
        self.current = Some(index);
        self.playback_started = false;
        self.backend
            .play(track, handle, volume, self.settings.normalise_volume);
    }

    /// Remember the track the transport is leaving, so `Previous` can rewind to it.
    fn record_play(&mut self) {
        let Some(current) = self.current else { return };
        self.play_history.remember(current, self.queue.len());
    }

    fn prefetch_next(&self) {
        // Under shuffle the next track is chosen at advance time; there is nothing stable to
        // prefetch, and guessing would fetch art and audio that never plays.
        if self.shuffle {
            return;
        }
        if let Some(next) = self.current.and_then(|i| self.queue.get(i + 1)) {
            self.backend.prefetch(next.clone());
        }
    }

    pub(crate) fn skip(&mut self, delta: isize) {
        if delta < 0 {
            self.previous();
            return;
        }
        match self.next_index() {
            Some(next) => self.play_index(next),
            // At the end with Repeat One, "next" replays the current track.
            None if self.repeat == RepeatMode::One => {
                if let Some(current) = self.current {
                    self.play_index(current);
                }
            }
            None => {}
        }
    }

    /// Advance one repeat mode: Off → All → One → Off.
    pub(crate) fn cycle_repeat(&mut self) {
        self.repeat = match self.repeat {
            RepeatMode::Off => RepeatMode::All,
            RepeatMode::All => RepeatMode::One,
            RepeatMode::One => RepeatMode::Off,
        };
    }

    /// Mute at the current volume, or restore whatever it was — the same "remember, then put
    /// back" the volume slider itself uses (`player_bar.rs` only overwrites `last_volume` when
    /// the new value is above silent, so muting from here never clobbers it with 0).
    pub(crate) fn toggle_mute(&mut self) {
        self.muted = !self.muted;
        let volume = if self.muted { 0.0 } else { self.last_volume };
        if let Some(player) = self.player.as_mut() {
            player.set_volume(volume);
        }
    }

    /// Turn shuffle on or off. On, the current track is marked played so it is not picked again
    /// before the rest of the queue; off, the seen set is dropped.
    pub(crate) fn toggle_shuffle(&mut self) {
        self.shuffle = !self.shuffle;
        self.shuffle_seen.clear();
        if self.shuffle
            && let Some(t) = self.current.and_then(|i| self.queue.get(i))
        {
            self.shuffle_seen.insert(t.video_id.clone());
        }
    }

    /// The next queue index: the following track, or a random unplayed one under shuffle.
    /// Wraps only under Repeat All; `None` means the transport should stop.
    fn next_index(&mut self) -> Option<usize> {
        let current = self.current?;
        if self.shuffle && self.queue.len() > 1 {
            if let Some(t) = self.queue.get(current) {
                self.shuffle_seen.insert(t.video_id.clone());
            }
            let mut cand: Vec<usize> = (0..self.queue.len())
                .filter(|&i| i != current && !self.shuffle_seen.contains(&self.queue[i].video_id))
                .collect();
            // Exhaust the tracks the listener actually chose before spilling into whatever
            // autoplay appended — otherwise shuffle drifts into radio content the moment the
            // river tops the queue up, long before the playlist's own tracks are done.
            if let Some(radio) = &self.radio {
                let chosen: Vec<usize> = cand
                    .iter()
                    .copied()
                    .filter(|&i| radio.chosen.iter().any(|id| id == &self.queue[i].video_id))
                    .collect();
                if !chosen.is_empty() {
                    cand = chosen;
                }
            }
            if cand.is_empty() {
                if self.repeat != RepeatMode::All {
                    return None;
                }
                // Every track has played: start the shuffle over, keeping the current one out
                // of the first pick so it does not immediately repeat.
                self.shuffle_seen.clear();
                if let Some(t) = self.queue.get(current) {
                    self.shuffle_seen.insert(t.video_id.clone());
                }
                let all: Vec<usize> = (0..self.queue.len()).filter(|&i| i != current).collect();
                if all.is_empty() {
                    return Some(current);
                }
                let r = self.next_rng();
                return Some(all[(r >> 33) as usize % all.len()]);
            }
            let r = self.next_rng();
            return Some(cand[(r >> 33) as usize % cand.len()]);
        }
        sequential_next(current, self.queue.len(), self.repeat == RepeatMode::All)
    }

    /// One LCG step, seeded from the clock at startup — enough for a shuffle pick.
    fn next_rng(&mut self) -> u64 {
        self.rng = self
            .rng
            .wrapping_mul(0x5851f42d4c957f2d)
            .wrapping_add(0x14057b7ef767814f);
        self.rng
    }

    /// Go back: first within this queue, then to the queue that preceded it. Going back rewinds
    /// the queue state itself — an autoplay tail added since is dropped, and a replaced queue is
    /// restored — rather than only moving the cursor.
    pub(crate) fn previous(&mut self) {
        match self.play_history.rewind() {
            Some(Rewind::WithinQueue { index, queue_len }) => {
                self.queue.truncate(queue_len);
                self.play_index_restore(index);
            }
            Some(Rewind::PreviousQueue {
                queue,
                index,
                radio,
            }) => {
                self.queue = queue;
                self.radio = radio;
                self.shuffle_seen.clear();
                self.play_index_restore(index);
            }
            // Nothing remembered: a plain positional step, like any transport — except under
            // shuffle, where array order carries no relationship to play order, so there is
            // nothing honest to step back to. Say nothing rather than fabricate a random-
            // looking jump.
            None => {
                if !self.shuffle
                    && let Some(current) = self.current
                    && current > 0
                {
                    self.play_index_restore(current - 1);
                }
            }
        }
    }

    /// Open or close the lyrics panel. Independent of the queue, which shares the same panel.
    pub(crate) fn toggle_lyrics(&mut self) {
        self.lyrics_open = !self.lyrics_open;
        if self.lyrics_open
            && let Some(track) = self.now.track.clone()
        {
            self.fetch_lyrics(track);
        }
    }

    /// Start a lyrics request for `track`, replacing whatever is shown. The `lyrics_for` guard
    /// drops the reply if the track changes before it lands.
    fn fetch_lyrics(&mut self, track: Track) {
        self.lyrics_for = Some(track.video_id.clone());
        self.lyrics = None;
        self.lyrics_error = None;
        self.lyrics_loading = true;
        self.lyrics_scroll_reset = true;
        self.backend.fetch_lyrics(track);
    }

    /// Queue `track` to play next: inserted after the current one, without disturbing what is
    /// playing. The already-resolved next track is now the wrong one, so it is prefetched
    /// again — same rule as jumping through the Up Next panel.
    pub(crate) fn play_next(&mut self, track: Track) {
        match self.current {
            Some(i) => {
                self.queue.insert(i + 1, track.clone());
                if let Some(radio) = &mut self.radio {
                    radio.chosen.push(track.video_id);
                }
                self.prefetch_next();
            }
            // Nothing playing yet: next is meaningless, so this becomes a plain append.
            None => self.add_to_queue(track),
        }
    }

    /// Append `track` to the end of the queue, without disturbing what is playing. With a
    /// river's tail already queued, "the end" is before it: the listener's additions come after
    /// the songs they chose and before the algorithm's.
    pub(crate) fn add_to_queue(&mut self, track: Track) {
        if let Some(radio) = &mut self.radio {
            radio.chosen.push(track.video_id.clone());
        }
        match self.radio.as_ref().and_then(|r| r.tail_start(&self.queue)) {
            Some(i) => self.queue.insert(i, track),
            None => self.queue.push(track),
        }
    }

    /// Whether there is a confirmed session. Every write — rating, the library, playlists —
    /// needs one, and the controls for them are hidden without it.
    pub(crate) fn signed_in(&self) -> bool {
        matches!(self.auth, crate::auth::AuthState::SignedIn { .. })
    }

    /// Whether the track currently loaded — playing, paused, or stopped — is one of `tracks`.
    /// An album/playlist header's own Play button reads this to decide what clicking it means:
    /// resume or pause what is already loaded (see [`Self::is_playing_any`] and
    /// [`Self::toggle_playback`]), rather than always restarting the whole list from the top.
    ///
    /// Membership in the list, not "did the queue start from this exact page": tracking the
    /// latter needs a separate flag that autoplay's own radio continuation, a manual reorder or
    /// "play next" would all have to remember to keep honest, and any place that missed one
    /// would show a stale Play/Pause. Whether the current track is still one of this page's own
    /// is self-correcting every frame instead, at the cost of not noticing once playback has
    /// moved on to radio tracks past the end of a shorter list.
    pub(crate) fn queue_matches(&self, tracks: &[Track]) -> bool {
        self.now
            .track
            .as_ref()
            .is_some_and(|t| tracks.iter().any(|x| x.video_id == t.video_id))
    }

    /// [`Self::queue_matches`], and actually playing right now rather than paused or stopped —
    /// what an album/playlist header's own Play button shows Pause for.
    pub(crate) fn is_playing_any(&self, tracks: &[Track]) -> bool {
        self.playback_started
            && self.player.as_ref().is_some_and(|p| !p.state().paused)
            && self.queue_matches(tracks)
    }

    /// One handler for every song menu in the app, so the pages forward the choice rather
    /// than copying the logic.
    pub(crate) fn row_action(&mut self, action: crate::ui::RowAction) {
        use crate::ui::RowAction;
        match action {
            RowAction::PlayTarget { seed, target } => self.play_target(seed, target),
            RowAction::Subscribe {
                channel_id,
                subscribe,
            } => self.toggle_subscribed(&channel_id, subscribe),
            RowAction::DontRecommend { token, name } => {
                if !self.signed_in() {
                    self.toast_error("Sign in to change your recommendations");
                    return;
                }
                self.backend.send_feedback(token);
                self.toast(&format!("Won't recommend {name}"));
            }
            RowAction::StartMix(id) => {
                // Show what it did: a mix replaces the queue with fifty tracks you have not
                // seen, and leaving the panel shut makes that invisible.
                self.upnext_open = true;
                self.backend.start_mix(id);
            }
            RowAction::StartMixPlaylist(id) => {
                self.upnext_open = true;
                self.backend.start_playlist_mix(id);
            }
            RowAction::Download(track) => self.start_download(track),
            RowAction::DownloadMany(tracks) => self.start_download_many(tracks),
            RowAction::RemoveDownload(video_id) => self.remove_download(&video_id),
            RowAction::PlayNext(track) => {
                self.play_next(track);
                self.toast("Song will play next");
            }
            RowAction::AddToQueue(track) => {
                self.add_to_queue(track);
                self.toast("Song added to queue");
            }
            RowAction::GoToAlbum(id) => self.navigate(Route::Album(id)),
            RowAction::GoToArtist(id) => self.navigate(Route::Artist(id)),
            RowAction::Share(id) => {
                self.backend.copy_text(crate::model::track_url(&id));
                self.toast("Link copied");
            }
            RowAction::Rate { video_id, status } => {
                if !self.signed_in() {
                    // No optimistic update and no request: without a session the write is
                    // impossible, and painting it first would flicker.
                    self.toast_error("Sign in to like songs");
                    return;
                }
                // Optimistic: a thumb that waits for a round trip feels broken. The refusal
                // carries the previous state back.
                let previous = self.like_of(&video_id);
                self.set_like(&video_id, status);
                self.backend.rate(video_id, status, previous);
            }
            RowAction::ToggleLibrary(track) => {
                if !self.signed_in() {
                    self.toast_error("Sign in to save songs");
                    return;
                }
                let saving = !track.in_library;
                let token = if saving {
                    track.library_add.as_deref()
                } else {
                    track.library_remove.as_deref()
                };
                let Some(token) = token else {
                    tracing::warn!(video = %track.video_id, "no bookmark token; not saved");
                    self.toast_error("This song cannot be saved");
                    return;
                };
                self.set_library_state(&track.video_id, saving);
                self.backend
                    .toggle_library(token.to_owned(), track.video_id, saving);
            }
            RowAction::SaveToPlaylist {
                playlist_id,
                video_id,
            } => {
                // A local playlist's own target list (`widgets::row_menu`) only ever offers
                // other local playlists, so this id names one — `video_id` is that track's
                // canonical path, per `Track.local_path`'s own doc comment.
                if self.is_local_playlist(&playlist_id) {
                    self.add_to_local_playlist(&playlist_id, std::path::PathBuf::from(video_id));
                    return;
                }
                if !self.signed_in() {
                    self.toast_error("Sign in to edit playlists");
                    return;
                }
                self.backend.add_to_playlist(playlist_id, video_id);
            }
            RowAction::RemoveFromPlaylist {
                playlist_id,
                video_id,
                set_video_id,
            } => {
                if self.is_local_playlist(&playlist_id) {
                    // `video_id` is the track's own canonical path, per `Track.local_path`'s
                    // own doc comment — the same string `local_playlist_as_playlist` gave this
                    // row as its `set_video_id` so this action could even be offered.
                    let path = std::path::PathBuf::from(&video_id);
                    if let Some(lp) = self
                        .local_playlists
                        .iter_mut()
                        .find(|p| p.id == playlist_id)
                    {
                        lp.track_paths.retain(|p| p != &path);
                    }
                    if let Err(e) = crate::local_playlists::save(&self.local_playlists) {
                        tracing::warn!(error = %format!("{e:#}"), "could not save local playlists");
                    }
                    self.remove_playlist_row(&playlist_id, &video_id);
                    self.resync_local_playlists();
                    return;
                }
                if !self.signed_in() {
                    self.toast_error("Sign in to edit playlists");
                    return;
                }
                // Optimistic: the row leaves at once; a refusal cannot put it back, so that
                // case refetches the page instead of guessing.
                self.remove_playlist_row(&playlist_id, &video_id);
                self.backend
                    .remove_from_playlist(playlist_id, video_id, set_video_id);
            }
            RowAction::PageShuffle(tracks) => {
                self.shuffle_play(tracks);
            }
            RowAction::PagePlayNext(tracks) => {
                let n = tracks.len();
                match self.current {
                    Some(mut i) => {
                        for t in tracks {
                            i += 1;
                            if let Some(radio) = &mut self.radio {
                                radio.chosen.push(t.video_id.clone());
                            }
                            self.queue.insert(i, t);
                        }
                        self.prefetch_next();
                    }
                    // Nothing playing yet: next is meaningless, so this becomes an append.
                    None => self.queue_many(tracks),
                }
                let text = Self::songs_will(n, "will play next");
                self.toast(&text);
            }
            RowAction::PageQueue(tracks) => {
                let n = tracks.len();
                self.queue_many(tracks);
                let text = Self::songs_will(n, "added to queue");
                self.toast(&text);
            }
            RowAction::PageSaveToPlaylist {
                playlist_id,
                title,
                video_ids,
            } => {
                if self.is_local_playlist(&playlist_id) {
                    let n = video_ids.len();
                    for video_id in video_ids {
                        self.add_to_local_playlist(
                            &playlist_id,
                            std::path::PathBuf::from(video_id),
                        );
                    }
                    self.toast(&format!("Saved {n} songs to {title}"));
                    return;
                }
                if !self.signed_in() {
                    self.toast_error("Sign in to edit playlists");
                    return;
                }
                let n = video_ids.len();
                self.backend.add_many_to_playlist(playlist_id, video_ids);
                // Says the work started; the bulk event's verdict overwrites it.
                self.toast(&format!("Saving {n} songs to {title}…"));
            }
            RowAction::PageSaveAsNewPlaylist { title, video_ids } => {
                // Same two-line handoff `save_as_playlist_row`'s button already does — this
                // just reaches it from a folder card's right-click menu instead.
                self.new_playlist = Some(title);
                self.new_playlist_kind = NewPlaylistKind::Local {
                    track_paths: video_ids
                        .into_iter()
                        .map(std::path::PathBuf::from)
                        .collect(),
                };
            }
            RowAction::PageLibrary {
                album,
                write,
                in_library,
            } => {
                if !self.signed_in() {
                    self.toast_error("Sign in to save to your library");
                    return;
                }
                self.set_page_library(album, in_library);
                self.backend.toggle_page_library(album, write, in_library);
            }
            RowAction::PageShare(url) => {
                self.backend.copy_text(url);
                self.toast("Link copied");
            }
            RowAction::PageInfo => {
                // The text lives on the page; the page sets `page_info` when forwarding.
            }
            RowAction::EditPlaylist(id) => self.begin_edit_playlist(id),
            RowAction::DeletePlaylist(id) => self.request_delete_playlist(id),
            RowAction::UnsavePlaylist(id) => self.backend.unsave_playlist(id),
            RowAction::RemoveFromQueue(i) => {
                // The "Now playing" row never carries this menu, but a defensive guard here
                // costs nothing and keeps the transport from ever removing what it is on.
                if self.current != Some(i) && i < self.queue.len() {
                    self.queue.remove(i);
                    if let Some(current) = self.current
                        && i < current
                    {
                        self.current = Some(current - 1);
                    }
                }
            }
        }
    }

    /// Play `tracks` in random order. Fisher–Yates over a time-seeded LCG — no rand dependency
    /// for one shuffle. A hand-picked queue closes the river, same as `start_from`.
    pub(crate) fn shuffle_play(&mut self, mut tracks: Vec<Track>) {
        let mut x = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9e3779b9)
            .wrapping_add(std::process::id() as u64);
        for i in (1..tracks.len()).rev() {
            x = x
                .wrapping_mul(0x5851f42d4c957f2d)
                .wrapping_add(0x14057b7ef767814f);
            tracks.swap(i, (x >> 33) as usize % (i + 1));
        }
        self.start_from(tracks, 0);
    }

    /// Append `tracks` to the queue in order, each landing before the river's tail.
    fn queue_many(&mut self, tracks: Vec<Track>) {
        for t in tracks {
            self.add_to_queue(t);
        }
    }

    /// Paint a page header's library state at once; a refusal carries the previous state back.
    fn set_page_library(&mut self, album: bool, in_library: bool) {
        if album {
            if let Some(a) = self.album.as_mut() {
                a.in_library = in_library;
            }
        } else if let Some(p) = self.playlist.as_mut() {
            p.in_library = in_library;
        }
    }

    /// Show a playlist save or removal in the rail at once. A saved list is "recently saved",
    /// so it lands after Liked Music — the top of the user's own lists, where YouTube's order
    /// would put it — and a removed one leaves. The next library load re-sorts authoritatively.
    pub(crate) fn splice_rail_playlist(
        &mut self,
        playlist: crate::model::PlaylistRef,
        saved: bool,
    ) {
        self.library.playlists.items.retain(|p| p.id != playlist.id);
        if saved {
            let at = self
                .library
                .playlists
                .items
                .iter()
                .position(|p| crate::model::is_liked_auto(&p.id))
                .map_or(0, |i| i + 1);
            self.library.playlists.items.insert(at, playlist);
        }
        self.save_playlist_order();
    }

    /// Apply the saved rail order: named playlists keep the order the user dragged them into;
    /// anything new stays after them in YouTube's own order (a stable sort by rank). Ids the
    /// saved list does not know about rank last, so a new playlist appears at the end.
    fn apply_playlist_order(&mut self) {
        order_by_saved(
            &mut self.library.playlists.items,
            &self.settings.playlist_order,
        );
    }

    /// Remember the rail's current order, by id, so it survives a library refresh and a restart.
    fn save_playlist_order(&mut self) {
        self.settings.playlist_order = self
            .library
            .playlists
            .items
            .iter()
            .map(|p| p.id.clone())
            .collect();
        if let Err(e) = crate::settings::save(&self.settings) {
            tracing::warn!(error = %format!("{e:#}"), "could not save the playlist order");
        }
    }

    /// Move a rail playlist from one index to another (drag and drop), and remember it.
    pub(crate) fn move_playlist(&mut self, from: usize, to: usize) {
        let items = &mut self.library.playlists.items;
        if from >= items.len() || to >= items.len() || from == to {
            return;
        }
        let p = items.remove(from);
        items.insert(to, p);
        self.save_playlist_order();
    }

    /// Follow or unfollow, painted at once and reverted if YouTube refuses.
    pub(crate) fn toggle_subscribed(&mut self, channel_id: &str, subscribe: bool) {
        if !self.signed_in() {
            self.toast_error("Sign in to subscribe");
            return;
        }
        self.set_subscribed_state(channel_id, subscribe);
        self.backend
            .set_subscribed(channel_id.to_owned(), subscribe);
    }

    /// Start whatever YouTube handed over as something to play — an artist's shuffle or mix.
    pub(crate) fn play_target(&mut self, seed: String, target: crate::model::WatchTarget) {
        self.upnext_open = true;
        self.backend.play_target(seed, target);
    }

    /// Paint a subscription state wherever this artist appears, so the button answers at once.
    fn set_subscribed_state(&mut self, channel_id: &str, subscribed: bool) {
        if let Some(a) = self.artist.as_mut()
            && a.id == channel_id
        {
            a.actions.subscribed = Some(subscribed);
        }
        let mark = |items: &mut Vec<crate::model::Entry>| {
            for e in items {
                if let crate::model::Entry::Artist(a) = e
                    && a.id == channel_id
                {
                    a.actions.subscribed = Some(subscribed);
                }
            }
        };
        mark(&mut self.results.items);
        self.for_each_feed_shelf_mut(|shelf| mark(&mut shelf.items));
    }

    /// Fetch the Explore feed once. Works signed out, so no auth gate.
    fn load_explore(&mut self) {
        if self.explore.is_some() || self.explore_loading || self.explore_error.is_some() {
            return;
        }
        self.explore_loading = true;
        self.backend.fetch_explore();
    }

    /// Fetch New releases once. Works signed out, so no auth gate.
    fn load_new_releases(&mut self) {
        if self.releases.is_some() || self.releases_loading || self.releases_error.is_some() {
            return;
        }
        self.releases_loading = true;
        self.backend.fetch_new_releases();
    }

    /// Fetch Charts once per params. Works signed out, so no auth gate.
    fn load_charts(&mut self, params: Option<String>) {
        if self.charts_params == params
            && (self.charts.is_some() || self.charts_loading || self.charts_error.is_some())
        {
            return;
        }
        self.charts = None;
        self.charts_error = None;
        self.charts_loading = true;
        self.charts_params = params.clone();
        self.backend.fetch_charts(params);
    }

    /// Fetch every mood category once. Works signed out, so no auth gate.
    fn load_moods(&mut self) {
        if self.moods.is_some() || self.moods_loading || self.moods_error.is_some() {
            return;
        }
        self.moods_loading = true;
        self.backend.fetch_moods();
    }

    /// Fetch what you played, once per visit. Signed out the page shows a prompt instead —
    /// YouTube answers history with "sign in" and parsing that would only produce an error.
    fn load_history(&mut self) {
        if !self.signed_in()
            || self.history_shelves.is_some()
            || self.history_loading
            || self.history_error.is_some()
        {
            return;
        }
        self.history_loading = true;
        self.backend.fetch_history();
    }

    /// Make a new playlist on your account and show it in the rail.
    pub(crate) fn create_playlist(&mut self, title: String) {
        if !self.signed_in() {
            self.toast_error("Sign in to make playlists");
            return;
        }
        self.backend.create_playlist(title);
    }

    /// A local playlist commits immediately — there is no server to wait on, so this doesn't
    /// need sign-in and doesn't go through `Backend`/`Event` the way `create_playlist` does.
    pub(crate) fn create_local_playlist(
        &mut self,
        title: String,
        track_paths: Vec<std::path::PathBuf>,
    ) {
        self.local_playlists
            .push(crate::local_playlists::LocalPlaylist {
                id: crate::local_playlists::new_id(),
                title: title.clone(),
                track_paths,
            });
        if let Err(e) = crate::local_playlists::save(&self.local_playlists) {
            tracing::warn!(error = %format!("{e:#}"), "could not save local playlists");
        }
        self.resync_local_playlists();
        self.toast(&format!("Created “{title}”"));
    }

    pub(crate) fn is_local_playlist(&self, id: &str) -> bool {
        self.local_playlists.iter().any(|p| p.id == id)
    }

    /// Appends one file to a local playlist and saves immediately — the same "no server, so no
    /// need for an `Event` round trip" reasoning as `create_local_playlist`. Silently does
    /// nothing if `id` doesn't name a local playlist (defensive; `row_menu`'s own filter should
    /// already prevent this from ever being called that way).
    fn add_to_local_playlist(&mut self, id: &str, path: std::path::PathBuf) {
        let Some(playlist) = self.local_playlists.iter_mut().find(|p| p.id == id) else {
            return;
        };
        if playlist.track_paths.contains(&path) {
            self.toast_error("Already in that playlist");
            return;
        }
        playlist.track_paths.push(path);
        if let Err(e) = crate::local_playlists::save(&self.local_playlists) {
            tracing::warn!(error = %format!("{e:#}"), "could not save local playlists");
        }
        self.resync_local_playlists();
        self.toast("Saved to playlist");
    }

    /// Open the rename editor for the currently loaded playlist.
    pub(crate) fn open_playlist_editor(&mut self) {
        let Some(pl) = self.playlist.as_ref() else {
            return;
        };
        self.playlist_edit = Some(PlaylistEditor {
            id: pl.id.clone(),
            title: pl.title.clone(),
            description: pl.description.clone().unwrap_or_default(),
            original_title: pl.title.clone(),
        });
    }

    /// Edit a playlist from the rail: edit it here if it is the open page, else navigate and let
    /// the page open the editor once it arrives.
    pub(crate) fn begin_edit_playlist(&mut self, id: String) {
        if self.playlist.as_ref().is_some_and(|pl| pl.id == id) {
            self.open_playlist_editor();
        } else {
            self.edit_on_load = Some(id.clone());
            self.navigate(Route::Playlist(id));
        }
    }

    /// Save the editor's title and description. Optimistic; a refusal reverts the title.
    pub(crate) fn save_playlist_editor(&mut self) {
        let Some(editor) = self.playlist_edit.take() else {
            return;
        };
        let title = editor.title.trim().to_owned();
        if title.is_empty() {
            self.toast_error("A playlist needs a name");
            self.playlist_edit = Some(editor);
            return;
        }
        self.set_playlist_title(&editor.id, title.clone());
        if let Some(pl) = self.playlist.as_mut()
            && pl.id == editor.id
        {
            pl.description = Some(editor.description.clone());
        }
        if self.is_local_playlist(&editor.id) {
            if let Some(lp) = self.local_playlists.iter_mut().find(|p| p.id == editor.id) {
                lp.title = title;
            }
            if let Err(e) = crate::local_playlists::save(&self.local_playlists) {
                tracing::warn!(error = %format!("{e:#}"), "could not save local playlists");
            }
            self.resync_local_playlists();
            self.toast("Playlist updated");
            return;
        }
        self.edit_revert = Some((editor.id.clone(), editor.original_title));
        self.backend
            .update_playlist(editor.id, title, editor.description);
    }

    /// Ask to delete a playlist: raises the confirm modal.
    pub(crate) fn request_delete_playlist(&mut self, id: String) {
        let title = self.playlist_title(&id);
        self.playlist_delete = Some((id, title));
    }

    /// Delete the playlist the modal is confirming.
    pub(crate) fn confirm_delete_playlist(&mut self) {
        let Some((id, _)) = self.playlist_delete.take() else {
            return;
        };
        if self.is_local_playlist(&id) {
            self.local_playlists.retain(|p| p.id != id);
            if let Err(e) = crate::local_playlists::save(&self.local_playlists) {
                tracing::warn!(error = %format!("{e:#}"), "could not save local playlists");
            }
            self.drop_playlist(&id);
            self.resync_local_playlists();
            if self.current_route() == Route::Playlist(id.clone()) {
                self.navigate(Route::Library);
            }
            self.toast("Playlist deleted");
            return;
        }
        self.backend.delete_playlist(id);
    }

    /// A playlist's title from whichever list holds it, for the delete prompt.
    fn playlist_title(&self, id: &str) -> String {
        if let Some(pl) = &self.playlist
            && pl.id == id
        {
            return pl.title.clone();
        }
        self.library
            .playlists
            .items
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.title.clone())
            .unwrap_or_else(|| "this playlist".to_owned())
    }

    /// Paint a playlist's new title on the open page and in the rail.
    fn set_playlist_title(&mut self, id: &str, title: String) {
        if let Some(pl) = self.playlist.as_mut()
            && pl.id == id
        {
            pl.title = title.clone();
        }
        if let Some(p) = self.library.playlists.items.iter_mut().find(|p| p.id == id) {
            p.title = title;
        }
    }

    /// Drop a playlist from the rail and the saved order (deleted or unsaved).
    fn drop_playlist(&mut self, id: &str) {
        self.library.playlists.items.retain(|p| p.id != id);
        self.settings.playlist_order.retain(|pid| pid != id);
        if let Err(e) = crate::settings::save(&self.settings) {
            tracing::warn!(error = %format!("{e:#}"), "could not save the playlist order");
        }
    }

    /// Keep a fetched playlist for the rail's context menu, bounded so it cannot grow forever.
    fn cache_rail_playlist(&mut self, id: &str, playlist: &crate::model::Playlist) {
        const RAIL_CACHE: usize = 8;
        if self.rail_cache.len() >= RAIL_CACHE && !self.rail_cache.contains_key(id) {
            // Drop an arbitrary other entry; the cache is a convenience, not a source of truth.
            if let Some(key) = self.rail_cache.keys().next().cloned() {
                self.rail_cache.remove(&key);
            }
        }
        self.rail_cache.insert(id.to_owned(), playlist.clone());
    }

    /// Keep a fetched album for a release card's context menu, bounded like the rail cache.
    fn cache_album(&mut self, id: &str, album: &crate::model::Album) {
        const ALBUM_CACHE: usize = 8;
        if self.album_cache.len() >= ALBUM_CACHE && !self.album_cache.contains_key(id) {
            // Drop an arbitrary other entry; the cache is a convenience, not a source of truth.
            if let Some(key) = self.album_cache.keys().next().cloned() {
                self.album_cache.remove(&key);
            }
        }
        self.album_cache.insert(id.to_owned(), album.clone());
    }

    /// Show a confirmation bubble above the player bar for a few seconds.
    fn toast(&mut self, text: &str) {
        self.toast = Some(Toast {
            text: text.to_owned(),
            until: Instant::now() + TOAST_DURATION,
            error: false,
        });
    }

    /// Say that something did not happen, and why.
    ///
    /// Every write can be refused — a stale token, a playlist that is not yours, no session —
    /// and each of those used to be a log line the user never saw, while every success spoke up.
    fn toast_error(&mut self, text: &str) {
        self.toast = Some(Toast {
            text: text.to_owned(),
            until: Instant::now() + TOAST_DURATION,
            error: true,
        });
    }

    /// A bulk queue toast with a singular that matches the single-song wording: "Song will
    /// play next", "2 songs will play next".
    fn songs_will(n: usize, what: &str) -> String {
        if n == 1 {
            format!("Song {what}")
        } else {
            format!("{n} songs {what}")
        }
    }

    /// Every shelf the app is holding, across each feed that is made of them.
    ///
    /// Home, Explore, New releases, Charts and History are all lists of the same shelves of the
    /// same songs, so the fan-outs below have to reach all of them — a page whose copy is
    /// skipped shows a rating that snaps back the moment you look at it. One place to add the
    /// next feed to, rather than five.
    fn feed_shelves(&self) -> impl Iterator<Item = &crate::model::Shelf> {
        self.home
            .iter()
            .flat_map(|v| v.iter())
            .chain(self.explore.iter().flat_map(|v| v.iter()))
            .chain(self.releases.iter().flat_map(|v| v.iter()))
            .chain(self.history_shelves.iter().flat_map(|v| v.iter()))
            .chain(self.charts.iter().flat_map(|c| c.shelves.iter()))
            .chain(self.artist.iter().flat_map(|a| a.shelves.iter()))
    }

    /// The mutable twin of [`Self::feed_shelves`], and it must list the same feeds.
    ///
    /// `Arc::make_mut` is what makes a write safe while a page is drawing from a snapshot it
    /// took this frame: the copy happens here, the frame keeps painting what it had, and the
    /// next frame reads the updated one.
    fn for_each_feed_shelf_mut(&mut self, mut f: impl FnMut(&mut crate::model::Shelf)) {
        for feed in [
            &mut self.home,
            &mut self.explore,
            &mut self.releases,
            &mut self.history_shelves,
        ] {
            if let Some(shelves) = feed.as_mut() {
                for shelf in Arc::make_mut(shelves) {
                    f(shelf);
                }
            }
        }
        if let Some(charts) = self.charts.as_mut() {
            for shelf in &mut Arc::make_mut(charts).shelves {
                f(shelf);
            }
        }
        if let Some(artist) = self.artist.as_mut() {
            for shelf in &mut artist.shelves {
                f(shelf);
            }
        }
    }

    /// Visit every `Track` the app holds — what is playing, the queue, and each loaded page.
    /// The read-only twin of the fan-out in [`Self::set_like`], and it must stay in step with
    /// it: a list one of them knows about and the other does not is a copy that goes stale.
    fn for_each_track(&self, mut f: impl FnMut(&Track)) {
        if let Some(t) = self.now.track.as_ref() {
            f(t);
        }
        for t in &self.queue {
            f(t);
        }
        for t in self
            .results
            .items
            .iter()
            .filter_map(crate::model::Entry::track)
        {
            f(t);
        }
        if let Some(artist) = self.artist.as_ref() {
            for t in &artist.top_songs {
                f(t);
            }
        }
        if let Some(album) = self.album.as_ref() {
            for t in &album.tracks {
                f(t);
            }
        }
        if let Some(playlist) = self.playlist.as_ref() {
            for t in &playlist.tracks.items {
                f(t);
            }
        }
        for t in &self.library.songs.items {
            f(t);
        }
        for t in self
            .feed_shelves()
            .flat_map(|s| s.items.iter())
            .filter_map(crate::model::Entry::track)
        {
            f(t);
        }
        // The chart cells are tracks too, and rating one from there is an ordinary rating.
        for t in self
            .charts
            .iter()
            .flat_map(|c| c.ranked.iter())
            .flat_map(|s| s.rows.iter())
            .filter_map(|r| r.entry.track())
        {
            f(t);
        }
    }

    /// Fill a track's rating and library state in from any copy that knows better.
    ///
    /// A card carries no menu, so a song started from a shelf arrives unrated and with no
    /// bookmark tokens — and the player echoes that very copy back as [`Event::Playing`],
    /// which is what the bar reads. Any list that parsed the same song from a *row* does know,
    /// so ask them before believing an empty state.
    fn reconcile_state(&self, track: &mut Track) {
        let mut like = None;
        let mut library = None;
        self.for_each_track(|t| {
            if t.video_id != track.video_id {
                return;
            }
            if like.is_none() && t.like != LikeStatus::Indifferent {
                like = Some(t.like);
            }
            if library.is_none() && (t.library_add.is_some() || t.library_remove.is_some()) {
                library = Some((
                    t.in_library,
                    t.library_add.clone(),
                    t.library_remove.clone(),
                ));
            }
        });
        if track.like == LikeStatus::Indifferent
            && let Some(like) = like
        {
            track.like = like;
        }
        if track.library_add.is_none()
            && track.library_remove.is_none()
            && let Some((in_library, add, remove)) = library
        {
            track.in_library = in_library;
            track.library_add = add;
            track.library_remove = remove;
        }
    }

    /// A track's rating, from the first copy that holds it. Everything is kept in sync by
    /// [`Self::set_like`], so any copy answers for all of them.
    fn like_of(&self, video_id: &str) -> LikeStatus {
        self.queue
            .iter()
            .find(|t| t.video_id == video_id)
            .map(|t| t.like)
            .or_else(|| {
                self.now
                    .track
                    .as_ref()
                    .filter(|t| t.video_id == video_id)
                    .map(|t| t.like)
            })
            .unwrap_or(LikeStatus::Indifferent)
    }

    /// Set a track's library membership everywhere it appears. Same lists as
    /// [`Self::set_like`]: the toggle arrives optimistically and reverts on failure.
    fn set_library_state(&mut self, video_id: &str, in_library: bool) {
        let set = |t: &mut Track| {
            if t.video_id == video_id {
                t.in_library = in_library;
            }
        };
        if let Some(t) = self.now.track.as_mut() {
            set(t);
        }
        for t in &mut self.queue {
            set(t);
        }
        for t in self
            .results
            .items
            .iter_mut()
            .filter_map(crate::model::Entry::track_mut)
        {
            set(t);
        }
        if let Some(artist) = self.artist.as_mut() {
            for t in &mut artist.top_songs {
                set(t);
            }
        }
        if let Some(album) = self.album.as_mut() {
            for t in &mut album.tracks {
                set(t);
            }
        }
        if let Some(playlist) = self.playlist.as_mut() {
            for t in &mut playlist.tracks.items {
                set(t);
            }
        }
        for t in &mut self.library.songs.items {
            set(t);
        }
        self.for_each_feed_shelf_mut(|shelf| {
            for t in shelf
                .items
                .iter_mut()
                .filter_map(crate::model::Entry::track_mut)
            {
                set(t);
            }
        });
        if let Some(charts) = self.charts.as_mut() {
            for row in Arc::make_mut(charts)
                .ranked
                .iter_mut()
                .flat_map(|s| s.rows.iter_mut())
            {
                if let Some(t) = row.entry.track_mut() {
                    set(t);
                }
            }
        }
    }

    /// Drop a track from the open playlist page, when it is the playlist it was removed from.
    /// A refused removal cannot reconstruct the row — position, neighbours, continuation —
    /// so that case reloads the page instead of guessing.
    fn remove_playlist_row(&mut self, playlist_id: &str, video_id: &str) {
        if let Some(playlist) = self.playlist.as_mut()
            && playlist.id == playlist_id
        {
            playlist.tracks.items.retain(|t| t.video_id != video_id);
        }
    }

    /// A row was dragged onto another one in the open playlist's own track list: move it to sit
    /// immediately before whatever was at `to`, both locally and on the server.
    ///
    /// Optimistic, like every other playlist edit here: the row moves at once, rather than
    /// waiting on the round trip, and a refusal reloads the whole page instead of trying to
    /// compute its way back (`Event::PlaylistMoveFailed`). `from`/`to` index the list as it was
    /// the moment the drag was released — this only ever runs the same frame that produced
    /// them, so nothing else has had a chance to change it first.
    pub(crate) fn reorder_playlist_track(&mut self, from: usize, to: usize) {
        let Some(playlist) = self.playlist.as_mut() else {
            return;
        };
        let is_local = playlist.source == crate::model::PlaylistSource::Local;
        let Some((set_video_id, before_set_video_id)) =
            reorder_tracks(&mut playlist.tracks.items, from, to)
        else {
            return;
        };
        if is_local {
            // No `set_video_id`/`before_set_video_id` to send anywhere — a local playlist's
            // order is just this `Vec`'s order, so the reorder `reorder_tracks` already did to
            // `playlist.tracks.items` is rebuilt straight back into `track_paths`.
            let playlist_id = playlist.id.clone();
            let track_paths: Vec<std::path::PathBuf> = playlist
                .tracks
                .items
                .iter()
                .filter_map(|t| t.local_path.as_deref().map(std::path::Path::to_path_buf))
                .collect();
            if let Some(lp) = self
                .local_playlists
                .iter_mut()
                .find(|p| p.id == playlist_id)
            {
                lp.track_paths = track_paths;
            }
            if let Err(e) = crate::local_playlists::save(&self.local_playlists) {
                tracing::warn!(error = %format!("{e:#}"), "could not save local playlists");
            }
            self.resync_local_playlists();
            return;
        }
        self.backend.move_playlist_item(
            playlist.id.clone(),
            set_video_id.to_string(),
            before_set_video_id.to_string(),
        );
    }

    /// Set a track's rating everywhere it appears: the queue, the now-playing copy, and every
    /// loaded list. Ratings arrive optimistically and revert on failure, so every copy must
    /// move together or the row and the player bar disagree.
    fn set_like(&mut self, video_id: &str, like: LikeStatus) {
        let set = |t: &mut Track| {
            if t.video_id == video_id {
                t.like = like;
            }
        };
        if let Some(t) = self.now.track.as_mut() {
            set(t);
        }
        for t in &mut self.queue {
            set(t);
        }
        for t in self
            .results
            .items
            .iter_mut()
            .filter_map(crate::model::Entry::track_mut)
        {
            set(t);
        }
        if let Some(artist) = self.artist.as_mut() {
            for t in &mut artist.top_songs {
                set(t);
            }
        }
        if let Some(album) = self.album.as_mut() {
            for t in &mut album.tracks {
                set(t);
            }
        }
        if let Some(playlist) = self.playlist.as_mut() {
            for t in &mut playlist.tracks.items {
                set(t);
            }
        }
        for t in &mut self.library.songs.items {
            set(t);
        }
        self.for_each_feed_shelf_mut(|shelf| {
            for t in shelf
                .items
                .iter_mut()
                .filter_map(crate::model::Entry::track_mut)
            {
                set(t);
            }
        });
        if let Some(charts) = self.charts.as_mut() {
            for row in Arc::make_mut(charts)
                .ranked
                .iter_mut()
                .flat_map(|s| s.rows.iter_mut())
            {
                if let Some(t) = row.entry.track_mut() {
                    set(t);
                }
            }
        }
    }
}

impl eframe::App for App {
    /// eframe calls this periodically and on exit, so the queue survives a normal close as well
    /// as the five-second periodic save.
    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        self.save_playback();
    }

    /// Transparent so the rounded corners show the desktop through them.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        Color32::TRANSPARENT.to_normalized_gamma_f32()
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.stats.begin_frame();
        self.handle_keys(&ctx);
        if ctx.input(|i| i.key_pressed(egui::Key::F2)) {
            self.stats.enabled = !self.stats.enabled;
        }
        self.handle_events(&ctx);
        self.poll_tray(&ctx);
        self.poll_media(&ctx);
        // Publish the playback click model for the row and card widgets this frame, so no page
        // has to thread the setting through.
        crate::widgets::set_double_click_play(&ctx, self.settings.double_click_to_play);
        crate::widgets::set_downloaded(&ctx, self.downloads.entries.keys().cloned().collect());

        let maximized = Self::is_maximized(&ctx);
        let rounded = !self.decorated && !maximized;
        let p = self.palette;

        chrome::paint_background(ui, &p, rounded);

        // Ask KWin to blur what the transparency shows through. A no-op call every frame once
        // the size and state settle — `Blur::set` compares against what it last pushed and only
        // touches the compositor when something actually changed. `content_rect` — the same
        // rect `chrome::paint_background` just filled — is already in the window's own
        // logical-pixel space, which is what a Wayland surface's local coordinates are.
        let size = ctx.content_rect().size();
        self.blur.set(
            self.settings.blur && p.translucent(),
            (
                size.x.round().max(0.0) as u32,
                size.y.round().max(0.0) as u32,
            ),
            if rounded {
                theme::WINDOW_RADIUS as u32
            } else {
                0
            },
        );

        // Safety net: a route whose fetch was dropped while we were elsewhere must load rather
        // than render an empty page. Idempotent, so calling it every frame costs a comparison.
        self.ensure_route_loaded();
        let route = self.current_route();

        // Claim order is the shell. The player bar goes first so it spans the whole window;
        // the sidebar second so it runs from the top edge down to the player; the top bar
        // third so it covers everything to the right of the rail and its window buttons stay
        // in the window's corner; the queue after that; the content takes what is left.
        let card = |fill: Color32, margin: egui::Margin| {
            egui::Frame::NONE
                .fill(fill)
                .corner_radius(egui::CornerRadius::same(theme::PANEL_RADIUS))
                .inner_margin(margin)
        };
        let g = theme::GUTTER as i8;

        egui::Panel::bottom("player")
            .exact_size(theme::PLAYER_BAR_HEIGHT)
            .resizable(false)
            // No separator line: the gap is the edge, and egui draws one by default.
            .show_separator_line(false)
            // No card: the player bar sits directly on the gutter, as the reference does —
            // and transparent rather than window-coloured, because the gutter is already
            // painted underneath. Filling it again would stack two alphas and leave the bar
            // more opaque than the gutter it is supposed to sit on.
            .frame(
                egui::Frame::NONE
                    .fill(Color32::TRANSPARENT)
                    .inner_margin(egui::Margin::symmetric(g + 6, 0)),
            )
            .show(ui, |ui| ui::player_bar::player_bar(self, ui));

        egui::Panel::left("sidebar")
            .exact_size(theme::SIDEBAR_WIDTH)
            .resizable(false)
            .show_separator_line(false)
            .frame(
                card(p.card_fill(), egui::Margin::symmetric(8, 10)).outer_margin(egui::Margin {
                    left: g,
                    right: 0,
                    top: g,
                    bottom: 0,
                }),
            )
            .show(ui, |ui| ui::sidebar::sidebar(self, ui));

        egui::Panel::top("topbar")
            .exact_size(theme::TITLEBAR_HEIGHT)
            .resizable(false)
            .show_separator_line(false)
            .frame(
                egui::Frame::NONE
                    .fill(Color32::TRANSPARENT)
                    .inner_margin(egui::Margin::symmetric(g + 6, 0)),
            )
            .show(ui, |ui| ui::topbar::top_bar(self, ui));

        if self.upnext_open || self.lyrics_open {
            // The queue is auxiliary, so it yields before the page does: a flat 300 is 40%
            // of a 750pt window, and the card behind it fell to scraps — every name in the
            // artist and playlist headers ellipsising while a third of the window sat in an
            // empty queue. Down to a floor it keeps enough rail to be a queue.
            let queue_w = theme::UPNEXT_WIDTH
                .min(
                    ui.ctx()
                        .input(|i| i.raw.screen_rect.map_or(0.0, |r| r.width()))
                        * 0.30,
                )
                .max(200.0);
            let show_queue = self.upnext_open;
            let show_lyrics = self.lyrics_open;
            // The card insets: the top gap keeps the first section off the rounded corner, the
            // sides hold the content off the edge.
            let margins = egui::Margin {
                left: 8,
                right: 8,
                top: 12,
                bottom: 10,
            };
            if show_queue && show_lyrics {
                // Two *cards* sharing the column, not one card split by a hairline: the right
                // panel itself is transparent, and each half is rounded with the shell's gutter
                // between them, exactly like the sidebar and the page card.
                egui::Panel::right("right-panel")
                    .exact_size(queue_w)
                    .resizable(false)
                    .show_separator_line(false)
                    .frame(egui::Frame::NONE.outer_margin(egui::Margin {
                        left: 0,
                        right: g,
                        top: 0,
                        bottom: 0,
                    }))
                    .show(ui, |ui| {
                        let half = ((ui.available_height() - f32::from(g)) / 2.0).max(80.0);
                        egui::Panel::top("queue-card")
                            .exact_size(half + f32::from(g))
                            .resizable(false)
                            .show_separator_line(false)
                            .frame(card(p.card_fill(), margins).outer_margin(egui::Margin {
                                left: 0,
                                right: 0,
                                top: 0,
                                bottom: g,
                            }))
                            .show(ui, |ui| ui::upnext::panel(self, ui));
                        egui::Panel::top("lyrics-card")
                            .exact_size(half)
                            .resizable(false)
                            .show_separator_line(false)
                            .frame(card(p.card_fill(), margins))
                            .show(ui, |ui| ui::lyrics::panel(self, ui));
                    });
            } else if show_queue || show_lyrics {
                egui::Panel::right("right-panel")
                    .exact_size(queue_w)
                    .resizable(false)
                    .show_separator_line(false)
                    .frame(card(p.card_fill(), margins).outer_margin(egui::Margin {
                        left: 0,
                        right: g,
                        top: 0,
                        bottom: 0,
                    }))
                    .show(ui, |ui| {
                        if show_queue {
                            ui::upnext::panel(self, ui);
                        } else {
                            ui::lyrics::panel(self, ui);
                        }
                    });
            }
        }

        egui::CentralPanel::default()
            .frame(
                card(p.card_fill(), egui::Margin::symmetric(16, 10)).outer_margin(egui::Margin {
                    left: g,
                    right: g,
                    top: 0,
                    bottom: 0,
                }),
            )
            .show(ui, |ui| {
                // Captured first: `available_rect_before_wrap` only means "the top" before
                // the page allocates. It is the card's *content* rect — the panel less its
                // inner margin — which is the only honest source for where the card's own
                // edges are. A scroll area's clip rect is not it: egui builds that from the
                // viewport and the parent's clip, and it lands a few points out on every side,
                // which is how the header wash ended up short at the top and over the edge on
                // the right.
                let top = ui.available_rect_before_wrap();
                self.content_rect = top;
                // The video view takes the content card; the page it was opened from is still
                // behind it, one toggle away.
                if self.video_open {
                    ui::video::panel(self, ui);
                    return;
                }
                match route {
                    Route::Search => ui::search::page(self, ui),
                    Route::Explore => ui::explore::page(self, ui),
                    Route::NewReleases => ui::releases::page(self, ui),
                    Route::Charts(_) => ui::charts::page(self, ui),
                    Route::Moods => ui::moods::page(self, ui),
                    Route::History => ui::history::page(self, ui),
                    Route::Home => ui::home::page(self, ui),
                    Route::Library => ui::library::page(self, ui),
                    Route::Settings => ui::settings::page(self, ui),
                    Route::Artist(_) => ui::artist::page(self, ui),
                    Route::Album(_) => ui::album::page(self, ui),
                    Route::Playlist(_) => ui::playlist::page(self, ui),
                    Route::Mood(_) => ui::mood::page(self, ui),
                }
            });
        // On the foreground layer, so draw order relative to the panels above doesn't matter —
        // see `widgets::progress_bar`'s own doc comment for why it moved here from the content
        // card's own top edge.
        if self.anything_loading() {
            widgets::progress_bar(ui, &p, if rounded { theme::WINDOW_RADIUS } else { 0 });
        }

        // The playlist management modals, over everything and on every route.
        ui::playlist::modals(self, ui);
        ui::help::modal(self, ui);
        ui::crash::modal(self, ui);

        if !self.decorated && !maximized {
            chrome::resize_handles(ui);
        }

        // Advance the queue when mpv runs out of file.
        let state = self
            .player
            .as_ref()
            .map(|pl| pl.state())
            .unwrap_or_default();
        // Kept in sync here rather than at every `toggle_pause`/`set_paused` call site — mpv is
        // the single source of truth for this, and every row/card's play-vs-pause glyph reads
        // it off `now` rather than reaching for the player directly.
        self.now.paused = state.paused;
        // Restore the saved position once its track is loaded, and keep `playback.json` fresh.
        self.poll_playback(&state);
        self.apply_crossfade(&state);
        if self.playback_started && state.idle && !self.now.resolving {
            self.playback_started = false;
            // Repeat One replays the track that just ended; anything else advances normally.
            if self.repeat == RepeatMode::One {
                if let Some(current) = self.current {
                    self.play_index(current);
                }
            } else {
                self.skip(1);
            }
        }

        // Autoplay: one `next` call returns ~50 tracks. A hand-picked queue seeds its tail as
        // soon as it starts, so the tail is there from the beginning; an open river is
        // extended at the low-water mark. Nothing is fetched while the setting is off.
        if !self.radio_loading {
            let call = self.radio.as_ref().and_then(|radio| {
                river_call(radio, self.current, &self.queue, self.settings.autoplay)
            });
            if let Some(call) = call {
                self.radio_loading = true;
                match call {
                    RiverCall::Seed { seed } => {
                        // The guard compares the pending river's seed: record what was asked
                        // for, which is the queue's tail as it stands now, not when it started.
                        if let Some(radio) = &mut self.radio {
                            radio.seed = seed.clone();
                        }
                        self.backend.seed_mix(seed);
                    }
                    RiverCall::Follow { token, reseed } => {
                        self.backend.extend_mix(token, reseed);
                    }
                }
            }
        }

        // Hand the OS widget the current transport. After the advance/autoplay above, so a track
        // that just changed is what the OS shows.
        self.publish_media();

        // Repaint discipline. Exactly one of these, and only when something is actually moving:
        // a pending autocomplete debounce, or a progress bar that needs to advance. Search itself
        // is submit-driven and fires here once, on demand.
        let suggest_due = self.poll_suggestions(&ctx);
        self.poll_search();
        if let Some(due) = suggest_due {
            ctx.request_repaint_after(due);
        } else {
            // The sweep moves visibly, so a load asks for frames faster than playback
            // progress needs — but only while something is actually in flight.
            let busy = self.anything_loading();
            let advancing = self.playback_started && !state.paused;
            if busy {
                ctx.request_repaint_after(Duration::from_millis(16));
            } else if advancing {
                ctx.request_repaint_after(PLAYING_REPAINT);
            }
        }

        // `scroll_to_top` is NOT cleared here: it is consumed by `page_scroll` itself, so a
        // reset requested while the page still shows its loader survives until the content
        // draws. Clearing it here reopened every fresh fetch at the previous offset.

        // Toasts, centred above the player bar. Only a live one asks for another frame, so an
        // idle app with no toast paints nothing extra.
        let remaining = self
            .toast
            .as_ref()
            .map(|t| t.until.saturating_duration_since(Instant::now()));
        match remaining {
            Some(left) if !left.is_zero() => {
                let (text, error) = self
                    .toast
                    .as_ref()
                    .map(|t| (t.text.clone(), t.error))
                    .unwrap_or_default();
                egui::Area::new(egui::Id::new("toast"))
                    .anchor(
                        egui::Align2::CENTER_BOTTOM,
                        egui::vec2(0.0, -(theme::PLAYER_BAR_HEIGHT + 16.0)),
                    )
                    .order(egui::Order::Foreground)
                    .show(&ctx, |ui| {
                        egui::Frame::NONE
                            .fill(self.palette.surface)
                            .stroke(egui::Stroke::new(1.0, self.palette.outline))
                            .corner_radius(egui::CornerRadius::same(8))
                            .inner_margin(egui::Margin::symmetric(14, 8))
                            .show(ui, |ui| {
                                // One line, always: a toast is a short confirmation, and the
                                // anchored area's width is not something to wrap a two-word
                                // phrase against. `Extend` sizes the bubble to the text.
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(text).font(theme::medium(13.0)).color(
                                            if error {
                                                self.palette.danger
                                            } else {
                                                self.palette.text
                                            },
                                        ),
                                    )
                                    .wrap_mode(egui::TextWrapMode::Extend),
                                );
                            });
                    });
                ctx.request_repaint_after(left);
            }
            _ => {
                self.toast = None;
            }
        }

        // Take only the link commands, leaving clipboard ones for eframe to handle.
        let mut urls = Vec::new();
        ctx.output_mut(|o| {
            o.commands.retain(|c| match c {
                egui::OutputCommand::OpenUrl(open) => {
                    urls.push(open.url.clone());
                    false
                }
                _ => true,
            });
        });
        for url in urls {
            crate::open_url(&url);
        }

        // Advance the debug scroll, and keep painting so it can actually move.
        if let Some(offset) = &mut self.autoscroll {
            *offset += 40.0;
            ctx.request_repaint();
        }

        self.stats.end_frame();
        self.stats.overlay(ui, &p, &self.images);

        // A loaded-state measurement: give the app time to fetch and draw real content, then
        // report the counters and quit. This is what `scripts/perf.sh` reads.
        if let Some(after) = self.stats_after
            && crate::elapsed_since_start() >= after
        {
            self.stats_after = None;
            self.stats.snapshot(&self.images).print();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if self.stats_after.is_some() {
            // Keep painting until the deadline, or the app would idle and never get there.
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        if self.startup_timing {
            // egui runs one more pass after the close command, so latch it.
            self.startup_timing = false;
            println!(
                "first_frame_ms={:.1}",
                crate::elapsed_since_start().as_secs_f64() * 1000.0
            );
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artist(id: &str) -> Route {
        Route::Artist(id.to_owned())
    }

    /// The fade curve: nothing mid-track, a ramp over the last `CROSSFADE_SECS`, and the same at
    /// the start; a paused player never fades out.
    #[test]
    fn the_crossfade_curve_dips_and_ramps() {
        let d = 240.0;
        assert_eq!(crossfade_factor(true, 60.0, d, None), 1.0);
        assert_eq!(crossfade_factor(true, d - CROSSFADE_SECS, d, None), 1.0);
        assert!((crossfade_factor(true, d - CROSSFADE_SECS / 2.0, d, None) - 0.5).abs() < 1e-9);
        assert_eq!(crossfade_factor(true, d, d, None), 0.0);
        assert_eq!(crossfade_factor(false, d - 1.0, d, None), 1.0);
        assert_eq!(crossfade_factor(true, 1.0, d, Some(0.0)), 0.0);
        assert!((crossfade_factor(true, 1.0, d, Some(CROSSFADE_SECS / 2.0)) - 0.5).abs() < 1e-9);
        assert_eq!(crossfade_factor(true, 1.0, d, Some(CROSSFADE_SECS)), 1.0);
        // Both ramps at once: the lower one wins (a very short track).
        assert_eq!(crossfade_factor(true, 0.0, 2.0, Some(0.5)), 0.1);
    }

    #[test]
    fn back_and_forward_walk_the_pages_in_order() {
        let mut h = History::new(Route::Search);
        assert!(h.push(artist("a")));
        assert!(h.push(Route::Album("x".into())));

        assert!(h.back());
        assert_eq!(*h.current(), artist("a"));
        assert!(h.back());
        assert_eq!(*h.current(), Route::Search);
        assert!(!h.can_go_back());
        assert!(!h.back());

        assert!(h.forward());
        assert_eq!(*h.current(), artist("a"));
    }

    /// The shape of the Back bug: two pages of the same kind must stay distinguishable, so the
    /// caller is told to reload rather than keep whatever is on screen.
    #[test]
    fn back_between_two_artists_lands_on_the_first_one() {
        let mut h = History::new(artist("a"));
        h.push(artist("b"));
        assert!(h.back());
        assert_eq!(*h.current(), artist("a"));
    }

    #[test]
    fn navigating_drops_the_forward_history() {
        let mut h = History::new(Route::Search);
        h.push(artist("a"));
        h.push(artist("b"));
        h.back();
        assert!(h.can_go_forward());

        h.push(Route::Settings);
        assert!(!h.can_go_forward(), "artist b should have been dropped");
        assert!(h.back());
        assert_eq!(*h.current(), artist("a"));
    }

    #[test]
    fn re_entering_the_same_page_is_not_a_new_entry() {
        let mut h = History::new(Route::Search);
        assert!(h.push(artist("a")));
        assert!(!h.push(artist("a")), "same route should be refused");
        assert!(h.back());
        assert_eq!(*h.current(), Route::Search);
    }

    fn track(id: &str) -> Track {
        Track {
            video_id: id.to_owned(),
            title: id.to_owned(),
            artists: vec![crate::model::ArtistLink::plain("Someone")],
            album: None,
            album_id: None,
            duration: None,
            plays: None,
            explicit: false,
            like: crate::model::LikeStatus::Indifferent,
            in_library: false,
            library_add: None,
            library_remove: None,
            set_video_id: None,
            thumbnail: None,
            rank: None,
            trend: crate::model::ChartTrend::Unknown,
            wide: false,
            local_path: None,
        }
    }

    /// A reseed returns the seed as its first track and continuations may overlap the tail —
    /// either way the queue must not repeat itself.
    #[test]
    fn extending_the_river_never_repeats_a_track() {
        let mut queue = vec![track("a"), track("b")];
        let added = append_new(&mut queue, &[track("b"), track("c"), track("c")]);
        assert_eq!(added, 1);
        assert_eq!(
            queue
                .iter()
                .map(|t| t.video_id.as_str())
                .collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
    }

    fn queue_ids(ids: &[&str]) -> Vec<Track> {
        ids.iter().map(|s| track(s)).collect()
    }

    /// The bug this fixes: sorting and then choosing Default again left the tracks exactly as
    /// the earlier sort had ordered them, since `PlaylistSort::apply`'s own `Default => {}` arm
    /// did nothing — only closing and reopening the playlist (which re-fetches YouTube's own
    /// order from scratch) actually put it back.
    #[test]
    fn choosing_default_again_restores_the_order_youtube_served() {
        let mut tracks = queue_ids(&["a", "b", "c"]);
        let mut original = None;
        apply_playlist_sort(PlaylistSort::TitleDesc, &mut tracks, &mut original);
        assert_eq!(
            tracks
                .iter()
                .map(|t| t.video_id.as_str())
                .collect::<Vec<_>>(),
            ["c", "b", "a"]
        );
        assert!(
            original.is_some(),
            "the first non-default sort must capture what order to restore"
        );

        apply_playlist_sort(PlaylistSort::Default, &mut tracks, &mut original);
        assert_eq!(
            tracks
                .iter()
                .map(|t| t.video_id.as_str())
                .collect::<Vec<_>>(),
            ["a", "b", "c"],
            "Default must restore the served order, not leave the tracks as the last sort left them"
        );
    }

    /// Choosing Default when nothing has ever been sorted away from it is a no-op, not a crash
    /// on a snapshot that was never taken.
    #[test]
    fn default_before_any_sort_leaves_the_order_alone() {
        let mut tracks = queue_ids(&["a", "b", "c"]);
        let mut original = None;
        apply_playlist_sort(PlaylistSort::Default, &mut tracks, &mut original);
        assert_eq!(
            tracks
                .iter()
                .map(|t| t.video_id.as_str())
                .collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
    }

    /// `Reverse` flips the order YouTube actually served, not whatever the tracks currently
    /// happen to be in. Served order here is `b, a, c` (deliberately not alphabetical) so
    /// reversing *that* (`c, a, b`) reads differently from reversing the Title-A–Z view
    /// (`c, b, a`) it's applied on top of — a bug that reversed the current order instead would
    /// produce the latter.
    #[test]
    fn reverse_flips_the_served_order_not_the_current_one() {
        let mut tracks = queue_ids(&["b", "a", "c"]);
        let mut original = None;
        apply_playlist_sort(PlaylistSort::TitleAsc, &mut tracks, &mut original);
        assert_eq!(
            tracks
                .iter()
                .map(|t| t.video_id.as_str())
                .collect::<Vec<_>>(),
            ["a", "b", "c"]
        );

        apply_playlist_sort(PlaylistSort::Reverse, &mut tracks, &mut original);
        assert_eq!(
            tracks
                .iter()
                .map(|t| t.video_id.as_str())
                .collect::<Vec<_>>(),
            ["c", "a", "b"]
        );
    }

    /// `All` matches everything search can return; every other scope matches only its own
    /// `Entry` variant. Nothing here ever changes ranking or order — see `SearchScope`'s own
    /// doc comment for why a scope is purely a filter over the one ranked response.
    #[test]
    fn search_scope_matches_only_its_own_entry_kind() {
        use crate::model::{AlbumRef, ArtistRef, PlaylistRef};

        let song = Entry::Song(track("a"));
        let video = Entry::Video(track("a"));
        let artist = Entry::Artist(ArtistRef {
            id: "a".to_owned(),
            name: "A".to_owned(),
            subscribers: None,
            thumbnail: None,
            actions: crate::model::ArtistActions::default(),
        });
        let album = Entry::Album(AlbumRef {
            id: "a".to_owned(),
            title: "A".to_owned(),
            artist: "A".to_owned(),
            year: None,
            kind: None,
            thumbnail: None,
        });
        let playlist = Entry::Playlist(PlaylistRef {
            id: "a".to_owned(),
            title: "A".to_owned(),
            author: String::new(),
            author_id: None,
            track_count: None,
            thumbnail: None,
            editable: false,
            source: crate::model::PlaylistSource::YouTube,
        });
        let all = [&song, &video, &artist, &album, &playlist];

        assert!(all.iter().all(|e| SearchScope::All.matches(e)));
        assert_eq!(
            all.iter().filter(|e| SearchScope::Songs.matches(e)).count(),
            1
        );
        assert!(SearchScope::Songs.matches(&song));
        assert!(SearchScope::Videos.matches(&video));
        assert!(SearchScope::Artists.matches(&artist));
        assert!(SearchScope::Albums.matches(&album));
        assert!(SearchScope::Playlists.matches(&playlist));
        assert!(!SearchScope::Songs.matches(&video));
        assert!(!SearchScope::Playlists.matches(&album));
    }

    /// A track with a playlist item id, unlike plain `track()` — what `reorder_tracks` needs to
    /// have anything to send the server.
    fn playlist_track(id: &str) -> Track {
        Track {
            set_video_id: Some(id.into()),
            ..track(id)
        }
    }

    #[test]
    fn reorder_moves_the_dragged_track_before_the_target() {
        let mut items = vec![
            playlist_track("a"),
            playlist_track("b"),
            playlist_track("c"),
            playlist_track("d"),
        ];
        // Drag "a" to land before "c".
        let moved = reorder_tracks(&mut items, 0, 2);
        assert_eq!(
            items
                .iter()
                .map(|t| t.video_id.as_str())
                .collect::<Vec<_>>(),
            ["b", "a", "c", "d"]
        );
        let (set_video_id, before_set_video_id) = moved.expect("a real move");
        assert_eq!(&*set_video_id, "a");
        assert_eq!(&*before_set_video_id, "c");
    }

    #[test]
    fn reorder_upward_also_lands_before_the_target() {
        let mut items = vec![
            playlist_track("a"),
            playlist_track("b"),
            playlist_track("c"),
            playlist_track("d"),
        ];
        // Drag "d" to land before "b".
        let moved = reorder_tracks(&mut items, 3, 1);
        assert_eq!(
            items
                .iter()
                .map(|t| t.video_id.as_str())
                .collect::<Vec<_>>(),
            ["a", "d", "b", "c"]
        );
        let (set_video_id, before_set_video_id) = moved.expect("a real move");
        assert_eq!(&*set_video_id, "d");
        assert_eq!(&*before_set_video_id, "b");
    }

    #[test]
    fn reorder_onto_itself_or_out_of_range_does_nothing() {
        let mut items = vec![playlist_track("a"), playlist_track("b")];
        assert!(reorder_tracks(&mut items, 0, 0).is_none());
        assert!(reorder_tracks(&mut items, 0, 5).is_none());
        assert!(reorder_tracks(&mut items, 5, 0).is_none());
        assert_eq!(
            items
                .iter()
                .map(|t| t.video_id.as_str())
                .collect::<Vec<_>>(),
            ["a", "b"],
            "a no-op drag must not touch the list"
        );
    }

    #[test]
    fn reorder_refuses_a_track_with_no_playlist_item_id() {
        // `track()` alone (not `playlist_track()`) leaves `set_video_id: None` — a row that
        // is not really a playlist item, which the server call has nothing to identify it by.
        let mut items = vec![track("a"), playlist_track("b")];
        assert!(reorder_tracks(&mut items, 0, 1).is_none());
    }

    /// The bug this fixes: `ViewportCommand::Visible` is a documented no-op on Wayland (winit's
    /// own Wayland backend says so — "Not possible on Wayland"), so "Hide tray on close" looked
    /// like a dead button there — the veto fired, `Visible(false)` went out, and nothing
    /// happened. `Minimized` is the real substitute Wayland's xdg-shell offers, but only in the
    /// hide direction (`set_minimized(false)` is *also* a documented no-op there).
    #[test]
    fn hiding_on_wayland_minimizes_instead_of_the_broken_visible_command() {
        assert_eq!(
            hide_command_for(true, true),
            egui::ViewportCommand::Minimized(true)
        );
        assert_eq!(
            hide_command_for(true, false),
            egui::ViewportCommand::Minimized(false),
            "still the honest command to ask for — that winit ignores it on Wayland is a \
             platform fact this function isn't responsible for hiding"
        );
        assert_eq!(
            hide_command_for(false, true),
            egui::ViewportCommand::Visible(false)
        );
        assert_eq!(
            hide_command_for(false, false),
            egui::ViewportCommand::Visible(true)
        );
    }

    fn pref(id: &str) -> crate::model::PlaylistRef {
        crate::model::PlaylistRef {
            id: id.to_owned(),
            title: id.to_owned(),
            author: String::new(),
            author_id: None,
            track_count: None,
            thumbnail: None,
            editable: false,
            source: crate::model::PlaylistSource::YouTube,
        }
    }

    /// The saved rail order sorts known ids and leaves unknown ones in place at the end; an
    /// empty order is a no-op, which is what a fresh install has.
    #[test]
    fn saved_order_sorts_known_ids_and_appends_new_ones() {
        let mut items = vec![pref("a"), pref("b"), pref("c")];
        order_by_saved(&mut items, &["c".into(), "a".into()]);
        assert_eq!(
            items.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            ["c", "a", "b"]
        );
        order_by_saved(&mut items, &[]);
        assert_eq!(items[0].id, "c", "an empty order changes nothing");
    }

    /// The plain transport: step forward, wrap only under Repeat All, and stop otherwise.
    #[test]
    fn sequential_next_wraps_only_for_repeat_all() {
        assert_eq!(sequential_next(0, 3, false), Some(1));
        assert_eq!(
            sequential_next(2, 3, false),
            None,
            "end without repeat stops"
        );
        assert_eq!(sequential_next(2, 3, true), Some(0), "Repeat All wraps");
        assert_eq!(sequential_next(0, 1, false), None);
        assert_eq!(
            sequential_next(0, 1, true),
            Some(0),
            "a single track repeats"
        );
        assert_eq!(
            sequential_next(0, 0, true),
            None,
            "an empty queue never advances"
        );
    }

    /// A forward move is remembered and `Previous` steps back to it, newest first; when nothing
    /// is left the history reports nothing so the caller can fall back to a positional step.
    #[test]
    fn history_pops_the_newest_position_first() {
        let mut h = PlayHistory::default();
        h.remember(0, 5);
        h.remember(1, 5);
        assert!(matches!(
            h.rewind(),
            Some(Rewind::WithinQueue {
                index: 1,
                queue_len: 5
            })
        ));
        assert!(matches!(
            h.rewind(),
            Some(Rewind::WithinQueue { index: 0, .. })
        ));
        assert!(h.rewind().is_none(), "emptied history must report nothing");
    }

    /// A replaced queue is only reached after the points in the current one, and the points
    /// that indexed the queue now gone were cleared when it was replaced.
    #[test]
    fn a_replaced_queue_comes_back_after_the_current_ones() {
        let mut h = PlayHistory::default();
        h.remember(3, 9);
        h.leaving_queue(queue_ids(&["a", "b"]), 1, None);
        h.remember(2, 5);
        assert!(matches!(
            h.rewind(),
            Some(Rewind::WithinQueue { index: 2, .. })
        ));
        match h.rewind() {
            Some(Rewind::PreviousQueue { queue, index, .. }) => {
                assert_eq!(index, 1);
                assert_eq!(
                    queue
                        .iter()
                        .map(|t| t.video_id.as_str())
                        .collect::<Vec<_>>(),
                    ["a", "b"]
                );
            }
            _ => panic!("expected the replaced queue"),
        }
        assert!(h.rewind().is_none());
    }

    /// Both stacks are bounded: old entries fall off the front rather than growing forever.
    #[test]
    fn history_is_bounded() {
        let mut h = PlayHistory::default();
        for i in 0..(PLAY_HISTORY + 5) {
            h.remember(i, 100);
        }
        let mut seen = Vec::new();
        while let Some(Rewind::WithinQueue { index, .. }) = h.rewind() {
            seen.push(index);
        }
        assert_eq!(seen.len(), PLAY_HISTORY);
        assert_eq!(seen.first(), Some(&16), "the newest point is first");
        assert_eq!(seen.last(), Some(&5), "the oldest points fell off");

        for i in 0..(QUEUE_HISTORY + 3) {
            h.leaving_queue(queue_ids(&[&i.to_string()]), 0, None);
        }
        let mut queues = 0;
        while let Some(Rewind::PreviousQueue { .. }) = h.rewind() {
            queues += 1;
        }
        assert_eq!(queues, QUEUE_HISTORY);
    }

    fn radio(state: River, seed: &str, chosen: &[&str]) -> Radio {
        Radio {
            state,
            seed: seed.to_owned(),
            chosen: chosen.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    /// A hand-picked queue seeds its tail as soon as playback starts, from its last track —
    /// position in the queue is irrelevant; YouTube Music shows the tail from the beginning.
    #[test]
    fn a_pending_river_seeds_from_the_queue_tail_immediately() {
        let queue = vec![track("a"), track("b"), track("c")];
        let r = radio(River::Pending, "c", &["a", "b", "c"]);
        for current in [0, 1, 2] {
            match river_call(&r, Some(current), &queue, true) {
                Some(RiverCall::Seed { seed }) => assert_eq!(seed, "c"),
                _ => panic!("pending river should seed at position {current}"),
            }
        }
        assert!(river_call(&r, None, &queue, true).is_none());
    }

    /// An open river follows its token only once the queue is within the low-water mark; a dead
    /// river and a queue with nothing playing never ask for anything.
    #[test]
    fn an_open_river_follows_only_at_the_low_water_mark() {
        let queue: Vec<Track> = (0..20).map(|i| track(&i.to_string())).collect();
        let open = radio(River::Open("tok".into()), "v", &[]);
        assert!(river_call(&open, Some(5), &queue, true).is_none());
        match river_call(&open, Some(10), &queue, true) {
            Some(RiverCall::Follow { token, reseed }) => {
                assert_eq!(token, "tok");
                assert_eq!(reseed, "10");
            }
            _ => panic!("expected a follow at the low-water mark"),
        }
        let dead = radio(River::Dead, "v", &[]);
        assert!(river_call(&dead, Some(19), &queue, true).is_none());
    }

    /// One gate over every river: the setting stops a pending seed and an open follow alike.
    #[test]
    fn the_autoplay_setting_gates_every_river() {
        let queue = vec![track("a")];
        let pending = radio(River::Pending, "a", &["a"]);
        assert!(river_call(&pending, Some(0), &queue, false).is_none());
        let open = radio(River::Open("tok".into()), "a", &[]);
        assert!(river_call(&open, Some(0), &queue, false).is_none());
    }

    /// A page only keeps the river flowing while it adds something and hands back a token.
    #[test]
    fn a_river_dies_when_a_page_adds_nothing_or_has_no_token() {
        assert_eq!(landed(2, Some("t".into())), River::Open("t".into()));
        assert_eq!(landed(0, Some("t".into())), River::Dead);
        assert_eq!(landed(2, None), River::Dead);
    }

    /// The eager tail changes where "add to queue" lands: after the songs the listener chose,
    /// before the algorithm's.
    #[test]
    fn the_tail_starts_at_the_first_track_the_listener_did_not_choose() {
        let queue = vec![track("a"), track("b"), track("t1"), track("t2")];
        let mut r = radio(River::Open("tok".into()), "b", &["a", "b"]);
        assert_eq!(r.tail_start(&queue), Some(2));
        r.chosen.push("t1".into());
        r.chosen.push("t2".into());
        assert_eq!(r.tail_start(&queue), None);

        // And the pending constructor records the queue as chosen, so nothing marks it Radio.
        let pending = Radio::pending(&queue).expect("non-empty queue");
        assert_eq!(pending.state, River::Pending);
        assert_eq!(pending.seed, "t2");
        assert_eq!(pending.tail_start(&queue), None);
    }
}
