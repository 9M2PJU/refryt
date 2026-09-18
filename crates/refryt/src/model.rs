//! Domain types.
//!
//! These are ours, deliberately. `ytmapi-rs` types stay behind `api.rs` so that swapping the
//! auth mode, or the API crate itself, touches one file rather than every page.

use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Track {
    pub video_id: String,
    pub title: String,
    /// Every artist on the track, each with its own browse id where the source linked it.
    ///
    /// Not one joined string: YouTube writes `"James Blake, Ludwig Göransson & Travis Scott"` as
    /// three separately-linked runs, and folding them together made only the first one clickable.
    pub artists: Vec<ArtistLink>,
    pub album: Option<String>,
    /// The album's browse id, when the source linked it. Only then is the album name clickable —
    /// a name without an id would be a dead link.
    pub album_id: Option<String>,
    /// Seconds. `None` when YouTube reported a duration we could not parse.
    pub duration: Option<u64>,
    /// Play count as YouTube shows it (`"12M plays"`). Not every source carries one.
    pub plays: Option<String>,
    pub explicit: bool,
    /// Your rating. Parsed where the row carries it (signed-in menus); `Indifferent` elsewhere,
    /// including every typed fallback — those responses never include it.
    pub like: LikeStatus,
    /// Whether the song is in your library, from the row's bookmark toggle. Only signed-in
    /// rows carry one.
    pub in_library: bool,
    /// The save/remove feedback tokens behind that toggle, as `Arc<str>` — cheap to clone in
    /// a long queue, like the thumbnail.
    pub library_add: Option<Arc<str>>,
    pub library_remove: Option<Arc<str>>,
    /// The playlist item id, from `playlistItemData.playlistSetVideoId`. Only playlist rows
    /// carry one, and only it makes "Remove from playlist" offerable.
    pub set_video_id: Option<Arc<str>>,
    /// Cover art URL, largest available.
    pub thumbnail: Option<Arc<str>>,
    /// A music video rather than an album-audio track: its still is 16:9, so cards draw it
    /// wide. Read off the response's own video kind — no `aspectRatio` field exists on any
    /// captured card, and the thumbnail URLs arrive square even for videos, so the kind is
    /// the only signal that survives.
    pub wide: bool,
    /// A chart rank (`"1"`), from the row's `customIndexColumn`. Only chart playlists carry
    /// one; everywhere else this is `None` and the row shows its position instead.
    pub rank: Option<String>,
    /// Which way the rank moved since the last chart. Unknown where the row named none.
    pub trend: ChartTrend,
    /// Set only for a track scanned off disk rather than fetched from YouTube — its playback
    /// source, and the thing that makes `video_id` a plain path string rather than a real
    /// YouTube id for this track. `None` for everything else in the app.
    pub local_path: Option<Arc<std::path::Path>>,
}

/// Your rating of a song, as the row's like button reports it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LikeStatus {
    #[default]
    Indifferent,
    Like,
    Dislike,
}

impl LikeStatus {
    /// The toggled state when its own thumb is pressed: pressing Like on a liked song
    /// un-likes it, as on the site.
    pub fn toggled(self, pressed: Self) -> Self {
        if self == pressed {
            Self::Indifferent
        } else {
            pressed
        }
    }
}

/// One artist credited on a track, and the page it leads to when the source linked it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtistLink {
    pub name: String,
    /// `None` where the source gave a name but no id — a dead link is worse than plain text.
    pub id: Option<String>,
}

impl ArtistLink {
    pub fn plain(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            id: None,
        }
    }
}

/// Something to play, as YouTube handed it over: a video to start on and the playlist it belongs
/// to. Neither id is derivable — an artist's shuffle and radio playlists are not built from the
/// channel id — so they are only ever read from a response, never constructed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchTarget {
    pub video_id: Option<String>,
    pub playlist_id: String,
    pub params: Option<String>,
}

/// Where a play reports itself, from the `player` response's `playbackTracking`.
///
/// Fetching bytes from a stream URL is invisible to YouTube's accounting — what counts a play
/// on your account is beaconing these URLs: the playback URL once when the track starts, the
/// watchtime URL every few seconds with the position. Both arrive with the session baked in,
/// so they are only ever read from a response, never constructed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaybackTracking {
    /// Fired once per play. A GET here is what writes the history item.
    pub playback_url: String,
    /// The base to append `rt`/`st`/`et`/`cmt`/`state` to while the track runs.
    pub watchtime_base: String,
}

/// What an artist's own menu offers, where the response carried it.
///
/// Every field is optional because the same artist appears in several shapes: a library row
/// offers shuffle, mix and share; a home card adds subscribe and don't-recommend; a signed-out
/// page has no subscribe state at all.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ArtistActions {
    pub shuffle: Option<WatchTarget>,
    pub mix: Option<WatchTarget>,
    /// `None` when the source said nothing about it — which is not the same as "not subscribed".
    pub subscribed: Option<bool>,
    /// The `params` token the button carried for each direction. YouTube's subscribe and
    /// unsubscribe calls take the button's own token alongside the channel id, and the two
    /// directions carry different ones; missing is not fatal, it just falls back to the bare
    /// channel id the call used to send.
    pub subscribe_params: Option<Arc<str>>,
    pub unsubscribe_params: Option<Arc<str>>,
    /// The token that stops YouTube recommending this artist. One-way, so it is only offered
    /// where the response actually provided it.
    pub dont_recommend: Option<Arc<str>>,
}

/// A search hit that links to an artist page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtistRef {
    pub id: String,
    pub name: String,
    pub subscribers: Option<String>,
    pub thumbnail: Option<Arc<str>>,
    pub actions: ArtistActions,
}

/// A search hit that links to an album page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlbumRef {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub year: Option<String>,
    /// `"Album"`, `"Single"` or `"EP"` when the source labels it. Used to split a full
    /// discography; search hits leave it empty.
    pub kind: Option<String>,
    pub thumbnail: Option<Arc<str>>,
}

/// Where a playlist lives: a real YouTube playlist round-tripping through its API, or a
/// client-only one backed by `local_playlists.rs`. A track can only ever be offered playlists
/// of its own source as a save target — YouTube's API cannot accept a local file, and a local
/// playlist has nowhere to send a YouTube video id either.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlaylistSource {
    #[default]
    YouTube,
    Local,
}

/// A playlist reference: library entries and mood playlists. Links to a playlist page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaylistRef {
    pub id: String,
    pub title: String,
    pub author: String,
    /// The owner's channel id, when the card linked it — so the rail and the page header can
    /// open their profile rather than showing a dead name.
    pub author_id: Option<String>,
    pub track_count: Option<String>,
    pub thumbnail: Option<Arc<str>>,
    /// Whether this is a playlist you can add songs to.
    ///
    /// Read from the card's own menu — an editable playlist offers `EDIT`, an auto one (Liked
    /// Music) and the "New playlist" tile do not. The alternative was comparing the card's
    /// author against the account name, which does not survive the layout: a library card reads
    /// `"IV • 13 tracks"` while a home card reads `"Playlist • YouTube Music"`, so the author is
    /// in a different field in each. Icons do not move and do not localise.
    pub editable: bool,
    pub source: PlaylistSource,
}

impl PlaylistRef {
    /// The line under a playlist's name: how much is in it when it is yours — or Liked Music,
    /// which has no maker — and who made it, with its size, when it is someone else's. What a
    /// saved list needs from a rail row is its creator, not just another count.
    pub fn detail(&self) -> String {
        if self.editable || is_liked_auto(&self.id) {
            self.track_count.clone().unwrap_or_default()
        } else {
            match (self.author.trim(), self.track_count.as_deref()) {
                ("", Some(count)) => count.to_owned(),
                (author, Some(count)) => format!("{author} • {count}"),
                (author, None) => author.to_owned(),
            }
        }
    }
}

impl Track {
    /// The credit line as one string, for the places that still want one — a window title, a
    /// log, the resolver.
    pub fn artist_line(&self) -> String {
        let mut out = String::new();
        for (i, a) in self.artists.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            out.push_str(&a.name);
        }
        out
    }

    /// The first artist that carries an id — what "go to artist" means when there are several.
    pub fn primary_artist_id(&self) -> Option<&str> {
        self.artists.iter().find_map(|a| a.id.as_deref())
    }
}

/// One thing YouTube Music can show you, whatever kind it is.
///
/// Search results, an artist's shelves and the home feed are all lists of these, so one renderer
/// serves all three rather than three near-identical grid loops.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    Song(Track),
    /// A music video. Playable through the same resolver; its cover is a 16:9 still rather than
    /// square album art.
    Video(Track),
    Artist(ArtistRef),
    Album(AlbumRef),
    Playlist(PlaylistRef),
    /// A "Moods & genres" chip. Has no artwork — YouTube draws these as coloured buttons, and so
    /// do we.
    Mood(MoodRef),
}

impl Entry {
    /// The track to play, for the kinds that are playable.
    pub fn track(&self) -> Option<&Track> {
        match self {
            Entry::Song(t) | Entry::Video(t) => Some(t),
            _ => None,
        }
    }

    /// The track to mutate, for the kinds that are playable. Ratings arrive optimistically and
    /// must reach every copy at once, which is what this is for.
    pub fn track_mut(&mut self) -> Option<&mut Track> {
        match self {
            Entry::Song(t) | Entry::Video(t) => Some(t),
            _ => None,
        }
    }

    /// The word shown in the row's type badge.
    pub fn kind(&self) -> &'static str {
        match self {
            Entry::Song(_) => "SONG",
            Entry::Video(_) => "VIDEO",
            Entry::Artist(_) => "ARTIST",
            Entry::Album(_) => "ALBUM",
            Entry::Playlist(_) => "PLAYLIST",
            Entry::Mood(_) => "MOOD",
        }
    }

    pub fn thumbnail(&self) -> Option<&Arc<str>> {
        match self {
            Entry::Song(t) | Entry::Video(t) => t.thumbnail.as_ref(),
            Entry::Artist(a) => a.thumbnail.as_ref(),
            Entry::Album(a) => a.thumbnail.as_ref(),
            Entry::Playlist(p) => p.thumbnail.as_ref(),
            Entry::Mood(_) => None,
        }
    }

    pub fn title(&self) -> &str {
        match self {
            Entry::Song(t) | Entry::Video(t) => &t.title,
            Entry::Artist(a) => &a.name,
            Entry::Album(a) => &a.title,
            Entry::Playlist(p) => &p.title,
            Entry::Mood(m) => &m.title,
        }
    }

    /// The line under the title, already assembled for display.
    pub fn subtitle(&self) -> String {
        match self {
            Entry::Song(t) | Entry::Video(t) => t.artist_line(),
            Entry::Artist(a) => a.subscribers.clone().unwrap_or_default(),
            Entry::Album(a) => match &a.year {
                Some(y) if !a.artist.is_empty() => format!("{} • {y}", a.artist),
                Some(y) => y.clone(),
                None => a.artist.clone(),
            },
            Entry::Playlist(p) => p.detail(),
            Entry::Mood(_) => String::new(),
        }
    }

    /// Artists are people and are drawn as circles; everything else is a square cover.
    pub fn is_artist(&self) -> bool {
        matches!(self, Entry::Artist(_))
    }
}

/// Which kind of result the search page is showing.
///
/// `All` is a client-side view over the one ranked, cross-type search response — `search.rs`'s
/// own doc comment explains why that response is never split up for its own sake
/// (`travis_means_travis_scott` is the regression test guarding it). Every other scope is the
/// opposite: a dedicated, category-specific request (`Api::search_scoped`), because the ranked
/// response was found to carry far too few of a given kind to be worth merely filtering —
/// searching an artist by name can carry a dozen real matches server-side while the ranked
/// response mentions one or two in passing. Living here rather than in `app.rs` (where
/// `LibrarySort`/`PlaylistSort`, its nearest siblings, do) is what lets `api.rs` depend on it
/// without depending on the app layer above it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SearchScope {
    #[default]
    All,
    Songs,
    Videos,
    Artists,
    Albums,
    Playlists,
}

impl SearchScope {
    pub const ALL: [SearchScope; 6] = [
        SearchScope::All,
        SearchScope::Songs,
        SearchScope::Videos,
        SearchScope::Artists,
        SearchScope::Albums,
        SearchScope::Playlists,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SearchScope::All => "All",
            SearchScope::Songs => "Songs",
            SearchScope::Videos => "Videos",
            SearchScope::Artists => "Artists",
            SearchScope::Albums => "Albums",
            SearchScope::Playlists => "Playlists",
        }
    }

    /// Whether `entry` belongs in this scope — only meaningful for `All`'s own client-side
    /// filter over the ranked response; the other scopes get their entries from a request that
    /// only ever returns their own kind in the first place.
    pub fn matches(self, entry: &Entry) -> bool {
        match self {
            SearchScope::All => true,
            SearchScope::Songs => matches!(entry, Entry::Song(_)),
            SearchScope::Videos => matches!(entry, Entry::Video(_)),
            SearchScope::Artists => matches!(entry, Entry::Artist(_)),
            SearchScope::Albums => matches!(entry, Entry::Album(_)),
            SearchScope::Playlists => matches!(entry, Entry::Playlist(_)),
        }
    }
}

/// A titled row of entries: an artist page's "Singles & EPs", a home feed's "New releases".
///
/// Keeping YouTube's own shelf titles is what makes an artist page look the way it should. The
/// parser used to append every carousel into one flat list, which is why albums and singles arrived
/// indistinguishable and five other shelves were lost entirely.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shelf {
    pub title: String,
    pub items: Vec<Entry>,
    /// Opaque params for fetching the full list, when YouTube offers a "more" button.
    pub more_params: Option<String>,
    /// The browse id that `more_params` was actually paired with — a shelf's own "more" button
    /// frequently points somewhere other than the artist's own channel id (a discography page,
    /// or even a playlist id for "Videos"). `None` for shelves the typed API fallback built,
    /// which exposes no such id; the caller falls back to the artist's id there, as before.
    pub more_browse_id: Option<String>,
    /// Whether "Show all" has already replaced `items` with the full list. Persists per shelf,
    /// independent of which shelf was expanded most recently.
    pub expanded: bool,
}

impl Shelf {
    /// Whether this shelf is a list of playable songs rather than a grid of cards.
    ///
    /// "Top songs" and "Quick picks" are lists; "Albums", "New releases" and "Videos" are
    /// grids — a video is a `Track` too (so it can play), but its 16:9 art wants the same
    /// thumbnail-forward card layout as everything else with a cover, not a text row.
    pub fn is_track_list(&self) -> bool {
        !self.items.is_empty() && self.items.iter().all(|i| matches!(i, Entry::Song(_)))
    }

    /// Whether this shelf is "Moods & genres", which is chips rather than cards.
    pub fn is_mood_list(&self) -> bool {
        !self.items.is_empty() && self.items.iter().all(|i| matches!(i, Entry::Mood(_)))
    }

    /// The playable tracks in this shelf, for starting a queue from one of them.
    pub fn tracks(&self) -> Vec<Track> {
        self.items
            .iter()
            .filter_map(|i| i.track().cloned())
            .collect()
    }

    /// Where `items[index]` sits among [`Self::tracks`].
    pub fn track_index(&self, index: usize) -> Option<usize> {
        self.items.get(index)?.track()?;
        Some(
            self.items[..index]
                .iter()
                .filter(|i| i.track().is_some())
                .count(),
        )
    }
}

/// Keep an expanded shelf to the kinds its own preview already showed.
///
/// A shelf's "Show all" can answer with a whole artist page — the captured "From your library"
/// expansion did — and flattening every renderer then mixed albums and playlists into a shelf
/// that was songs. The preview names the kinds this shelf is actually made of; everything else
/// belongs to another shelf. Never empties a shelf: when nothing in the expansion matches, the
/// unfiltered list stands rather than the shelf silently vanishing.
pub fn shelf_expansion(items: Vec<Entry>, preview: &[Entry]) -> Vec<Entry> {
    let wanted: std::collections::HashSet<&str> = preview.iter().map(Entry::kind).collect();
    if wanted.is_empty() || !items.iter().any(|e| wanted.contains(e.kind())) {
        return items;
    }
    items
        .into_iter()
        .filter(|e| wanted.contains(e.kind()))
        .collect()
}

/// A search, in YouTube's own relevance order.
///
/// One ranked list rather than three parallel ones: the unfiltered response ranks every type
/// against every other, and splitting it into Songs / Artists / Albums shelves throws that away —
/// a highly relevant artist would sit below every song.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchResults {
    pub items: Vec<Entry>,
}

impl SearchResults {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The playable entries, in order, paired with their index in `items`.
    ///
    /// Starting a song from search queues the other playable results and nothing else, so pressing
    /// next does not stop at the first artist row.
    pub fn playable(&self) -> Vec<Track> {
        self.items
            .iter()
            .filter_map(|i| i.track().cloned())
            .collect()
    }

    /// Where `items[index]` sits among [`Self::playable`].
    pub fn playable_index(&self, index: usize) -> Option<usize> {
        self.items.get(index)?.track()?;
        Some(
            self.items[..index]
                .iter()
                .filter(|i| i.track().is_some())
                .count(),
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Artist {
    pub id: String,
    pub name: String,
    /// The short count from `subscriberCountText` ("20.8M") — what fits on the Subscribe
    /// button. It carries no unit on purpose; the long form is [`Self::subscribers_long`].
    pub subscribers: Option<String>,
    /// `"20.8M subscribers"`, complete and localised by YouTube. The header shows this under
    /// the name when the artist has no monthly-audience line (a small artist), so the page
    /// never prints the unitless button count as a sentence.
    pub subscribers_long: Option<String>,
    /// `"213M monthly audience"`, complete from YouTube. This is what the page shows under the
    /// name; the subscriber count belongs on the Subscribe button and the view total on the
    /// About section, which is where they were double-billed before.
    pub monthly_listeners: Option<String>,
    pub views: Option<String>,
    pub actions: ArtistActions,
    pub description: Option<String>,
    pub thumbnail: Option<Arc<str>>,
    /// The wide channel-art banner (`musicImmersiveHeaderRenderer`'s own thumbnail list, ~2.4:1)
    /// that the artist page's full-bleed hero draws. `None` on a page that only carries a square
    /// portrait — a channel's visual header — and on the typed fallback, where the hero slot
    /// falls back to the circle-art header this page has always had.
    pub hero: Option<Arc<str>>,
    pub top_songs: Vec<Track>,
    /// YouTube's own title for that shelf. "Top songs" on every capture so far, but the artist
    /// page draws whatever the response called it rather than inventing a heading of its own.
    /// `None` on the typed fallback, which has no shelf to read one from.
    pub top_songs_title: Option<String>,
    /// The id of the "Top songs" shelf's own auto-generated playlist — its "Show all" target.
    /// YouTube embeds only 5 songs directly in the artist page; the rest live here, one playlist
    /// fetch away, the same way any other playlist opens.
    pub top_songs_playlist: Option<String>,
    /// The page's shelves, with YouTube's own titles and order: Albums, Singles & EPs, Videos,
    /// Live performances, Featured on, Playlists, Fans might also like. Flattening these into one
    /// list is what made albums and singles indistinguishable and lost the rest.
    ///
    /// `Arc`-wrapped like every sibling feed (`home`/`explore`/`releases`/`moods`/
    /// `history_shelves`) so a per-frame snapshot is a refcount bump, not a deep clone.
    pub shelves: Arc<Vec<Shelf>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Album {
    pub id: String,
    pub title: String,
    /// Every credited artist joined for display. Kept beside [`Self::artists`] because most
    /// callers want the string; the header wants the individual links.
    pub artist: String,
    pub artist_id: Option<String>,
    /// The credits with their own ids, so a multi-artist album header links each of them.
    pub artists: Vec<ArtistLink>,
    pub year: Option<String>,
    /// `"Single"`, `"Album"` or `"EP"`, as YouTube labels it.
    pub kind: String,
    pub description: Option<String>,
    pub thumbnail: Option<Arc<str>>,
    /// Total duration verbatim (`"32 min"`). Stored raw: only `m:ss` strings parse, and a
    /// missing duration must not hide the rest of the header.
    pub duration_text: Option<String>,
    /// The header toggle's state, read off the page. The flip rides the like endpoint, not
    /// a feedback token — album headers carry none — so this travels with the like-playlist
    /// id below rather than add/remove tokens.
    pub in_library: bool,
    /// The play button's playlist id, which is also the like target that saves the album.
    /// Absent where the page gave no play button; the header save control hides itself then.
    pub like_id: Option<String>,
    pub tracks: Vec<Track>,
}

/// A full playlist: header plus playable tracks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Playlist {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub author: String,
    /// The owner's channel id, from the header facepile's tap target. `None` when the page
    /// gave no link (the typed fallback never does).
    pub author_id: Option<String>,
    pub year: Option<String>,
    pub duration_text: Option<String>,
    pub track_count: Option<String>,
    pub thumbnail: Option<Arc<str>>,
    /// Whether this is your own playlist. Read from the page header — owned playlists render
    /// `musicEditablePlaylistDetailHeaderRenderer` — and the only thing that enables
    /// "Remove from playlist". Conservative by design: the typed fallback cannot tell, so it
    /// says false and the button stays hidden rather than risking a write.
    pub editable: bool,
    /// The header's own library toggle, like [`Album::in_library`]. Playlist headers rarely
    /// carry a bookmark button (neither Liked Music nor an owned list did when checked), so
    /// this is usually `None` and the header save control stays hidden.
    pub in_library: bool,
    pub library_add: Option<Arc<str>>,
    pub library_remove: Option<Arc<str>>,
    /// Whether the page actually offered a save control — the header's bookmark toggle.
    ///
    /// Set by the raw parser when it found one; a public playlist has it (saving rides the like
    /// endpoint, not a feedback token) while your own header does not. The typed fallback cannot
    /// tell, so it says false and the control stays hidden rather than writing blind.
    pub saveable: bool,
    /// Paged: a long playlist arrives 100 tracks at a time. "Liked music" is 5,042.
    pub tracks: Paged<Track>,
    pub source: PlaylistSource,
}

impl Playlist {
    /// YouTube's own auto-generated mixes/radios ("My Mix", "Supermix", and the rest) open
    /// through the same `Route::Playlist`/`Api::playlist` path as a real playlist, but their id
    /// wraps a radio id — `VLRDCLAK5uy_…` rather than a real `VLPL…`/`VLOLAK…` playlist id — and
    /// unlike a real playlist they are an unbounded algorithmic river with no fixed track list to
    /// sort.
    pub fn is_expanding(&self) -> bool {
        is_expanding_playlist_id(&self.id)
    }
}

/// `Playlist::is_expanding`'s own check, free of the struct so it's testable without building one.
/// `bare_playlist_id` already strips the browse-only `VL` prefix this needs.
fn is_expanding_playlist_id(id: &str) -> bool {
    bare_playlist_id(id).starts_with("RD")
}

/// Lyrics for a track: either time-synced or plain text.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Lyrics {
    pub lines: Vec<LyricLine>,
    /// Whether `at_ms` is meaningful. Plain lyrics carry every line at zero.
    pub synced: bool,
    /// Where they came from, shown above the text.
    pub source: String,
}

/// One lyric line and, when `synced`, the millisecond it starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LyricLine {
    pub at_ms: u64,
    pub text: String,
}

/// A playlist id without its browse `VL` prefix. Browses want `VL{id}`; the edit endpoint
/// wants the bare id, and the library hands out the prefixed form — so writes strip it.
pub fn bare_playlist_id(id: &str) -> &str {
    id.strip_prefix("VL").unwrap_or(id)
}

/// The public YouTube Music URL for a track. One source of truth for the Share menu, the
/// player bar and the page menus.
pub fn track_url(video_id: &str) -> String {
    format!("https://music.youtube.com/watch?v={video_id}")
}

/// The public YouTube Music URL for an album.
pub fn album_url(album_id: &str) -> String {
    format!("https://music.youtube.com/browse/{album_id}")
}

/// The public YouTube Music URL for a playlist. Accepts the prefixed (`VL…`) or bare id.
pub fn playlist_url(playlist_id: &str) -> String {
    format!(
        "https://music.youtube.com/playlist?list={}",
        bare_playlist_id(playlist_id)
    )
}

/// The public YouTube Music URL for an artist's channel.
pub fn artist_url(artist_id: &str) -> String {
    format!("https://music.youtube.com/channel/{artist_id}")
}

/// Whether an id is the Liked-Music auto playlist, which no edit endpoint accepts.
pub fn is_liked_auto(id: &str) -> bool {
    bare_playlist_id(id) == "LM"
}

/// Whether an id is the "Episodes for later" auto playlist — the personal `SE` list, which is
/// podcasts wearing a playlist's id. The app has no episode page and no podcast playback, so it
/// is dropped wherever playlists are parsed rather than listed unopenable, the same way Liked
/// Music is special-cased where an edit endpoint would refuse it.
pub fn is_episodes_for_later(id: &str) -> bool {
    bare_playlist_id(id) == "SE"
}

/// A list that arrives a page at a time.
///
/// The cursor lives with the items so every place that shows a list has to acknowledge that there
/// may be more — which is the thing the library page silently got wrong, presenting the first 25
/// of 689 songs as the whole of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paged<T> {
    pub items: Vec<T>,
    /// Token for the next page; `None` once the list is complete.
    pub next: Option<String>,
    /// A page is in flight, so the view must not ask for another.
    pub loading: bool,
}

impl<T> Default for Paged<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            next: None,
            loading: false,
        }
    }
}

/// Read like the `Vec` it wraps, so a paged list is a slice everywhere it is only being shown.
impl<T> std::ops::Deref for Paged<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        &self.items
    }
}

impl<T> Paged<T> {
    /// A first page.
    pub fn new(items: Vec<T>, next: Option<String>) -> Self {
        Self {
            items,
            next,
            loading: false,
        }
    }

    /// Whether asking for more would do anything.
    pub fn can_load_more(&self) -> bool {
        self.next.is_some() && !self.loading
    }

    pub fn extend(&mut self, items: Vec<T>, next: Option<String>) {
        self.items.extend(items);
        self.next = next;
        self.loading = false;
    }
}

/// The signed-in user's library, in four sections, each paged independently.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Library {
    pub songs: Paged<Track>,
    pub artists: Paged<ArtistRef>,
    pub albums: Paged<AlbumRef>,
    pub playlists: Paged<PlaylistRef>,
}

/// A channel the signed-in Google account can browse as.
///
/// The personal channel has no `page_id`; brand accounts do, and that id is what selects one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub name: String,
    pub detail: String,
    pub page_id: Option<String>,
    /// The channel's `UC…` browse id, for opening its profile. Derived from the account list's
    /// `offlineCacheKeyToken` (the id without its `UC` prefix) or a `UC…` page id. `None` when
    /// the Google account has no channel.
    pub channel_id: Option<String>,
    /// The `@handle`, when YouTube sent one.
    pub handle: Option<String>,
    pub selected: bool,
    pub thumbnail: Option<Arc<str>>,
}

/// The buttons across the top of Explore — New releases, Charts, Moods & genres — read off
/// the feed itself, so their browse ids and params never go stale in code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExploreLink {
    pub title: String,
    pub browse_id: String,
    pub params: Option<String>,
}

/// The Explore landing: shelves plus the buttons that lead off it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExploreFeed {
    pub shelves: Vec<Shelf>,
    pub links: Vec<ExploreLink>,
}

/// One ranked row on a chart: the entry plus its rank and movement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChartRow {
    pub entry: Entry,
    /// The rank as YouTube writes it (`"1"`). Empty where the row named none.
    pub rank: String,
    pub trend: ChartTrend,
}

/// Which way a chart row moved since the last chart.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ChartTrend {
    Up,
    Down,
    Steady,
    #[default]
    Unknown,
}

/// One ranked chart — Top songs, Trending — plus the card shelves that share its page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChartSection {
    pub title: String,
    pub rows: Vec<ChartRow>,
}

/// The Charts page: ranked sections first, then ordinary card shelves.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChartsFeed {
    pub ranked: Vec<ChartSection>,
    pub shelves: Vec<Shelf>,
}
/// One "Moods & genres" chip. `params` is opaque; it goes back into `get_mood_playlists`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MoodRef {
    pub title: String,
    pub params: String,
    /// The chip's left stripe, from the card's `solid.leftStripeColor` (ARGB on the wire, RGB
    /// here). `None` where the source carries none — the typed mood categories do not.
    pub colour: Option<[u8; 3]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MoodSection {
    pub name: String,
    pub moods: Vec<MoodRef>,
}

/// The playlists behind one mood chip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MoodPlaylists {
    pub title: String,
    pub playlists: Vec<PlaylistRef>,
}

/// One chip of the home feed's mood bar (Workout, Relax, …): the same feed browsed with
/// different params. Not an [`Entry::Mood`] — those open a genre page, this filters the feed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HomeChip {
    pub title: String,
    /// Opaque browse params. `None` selected means the default feed.
    pub params: String,
    /// Whether the feed arrived with this filter applied.
    pub selected: bool,
}

/// The home feed: its shelves and its mood-bar chips, from one browse.
#[derive(Clone, Debug, Default)]
pub struct HomeFeed {
    pub shelves: Vec<Shelf>,
    pub chips: Vec<HomeChip>,
}

/// Parse YouTube Music's `"m:ss"` / `"h:mm:ss"` duration strings.
pub fn parse_duration(s: &str) -> Option<u64> {
    let mut total = 0u64;
    let mut any = false;
    for part in s.trim().split(':') {
        let n: u64 = part.trim().parse().ok()?;
        total = total.checked_mul(60)?.checked_add(n)?;
        any = true;
    }
    any.then_some(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(id: &str) -> Entry {
        Entry::Song(Track {
            video_id: id.to_owned(),
            title: id.to_owned(),
            artists: vec![ArtistLink::plain("Someone")],
            album: None,
            album_id: None,
            duration: None,
            plays: None,
            explicit: false,
            like: LikeStatus::Indifferent,
            in_library: false,
            library_add: None,
            library_remove: None,
            set_video_id: None,
            thumbnail: None,
            rank: None,
            trend: crate::model::ChartTrend::Unknown,
            wide: false,
            local_path: None,
        })
    }

    fn artist(id: &str) -> Entry {
        Entry::Artist(ArtistRef {
            id: id.to_owned(),
            name: id.to_owned(),
            subscribers: None,
            thumbnail: None,
            actions: ArtistActions::default(),
        })
    }

    fn album(id: &str) -> Entry {
        Entry::Album(AlbumRef {
            id: id.to_owned(),
            title: id.to_owned(),
            artist: String::new(),
            year: None,
            kind: None,
            thumbnail: None,
        })
    }

    /// An expansion that came back as a whole artist page must not mix other shelves' content
    /// into the one that was expanded — and must never empty the shelf either.
    #[test]
    fn an_expansion_keeps_only_the_kinds_its_preview_showed() {
        let mixed = vec![song("s1"), album("a1"), artist("x")];
        let kept = shelf_expansion(mixed.clone(), &[song("s1")]);
        assert_eq!(kept, vec![song("s1")]);

        let kept = shelf_expansion(mixed.clone(), &[artist("x")]);
        assert_eq!(kept, vec![artist("x")]);

        // Nothing of the preview's kind in the expansion: the list stands rather than the
        // shelf emptying itself.
        let kept = shelf_expansion(
            mixed.clone(),
            &[Entry::Mood(MoodRef {
                title: "Chill".to_owned(),
                params: "p".to_owned(),
                colour: None,
            })],
        );
        assert_eq!(kept, mixed);

        // No preview at all: nothing to narrow to.
        let kept = shelf_expansion(mixed.clone(), &[]);
        assert_eq!(kept, mixed);
    }

    /// Playing the third row of a mixed list must start the third *playable* item, not the third
    /// row — otherwise clicking a song after two artists plays the wrong track.
    #[test]
    fn playing_a_row_maps_past_the_unplayable_ones() {
        let r = SearchResults {
            items: vec![artist("a"), song("s1"), artist("b"), song("s2")],
        };
        assert_eq!(r.playable().len(), 2);
        assert_eq!(r.playable_index(1), Some(0));
        assert_eq!(r.playable_index(3), Some(1));
        assert_eq!(r.playable_index(0), None, "an artist row is not playable");
        assert_eq!(r.playable_index(9), None, "out of range");
    }

    #[test]
    fn parses_the_duration_formats_youtube_returns() {
        assert_eq!(parse_duration("3:33"), Some(213));
        assert_eq!(parse_duration("0:09"), Some(9));
        assert_eq!(parse_duration("1:02:05"), Some(3725));
        assert_eq!(parse_duration(" 4:20 "), Some(260));
    }

    #[test]
    fn rejects_things_that_are_not_durations() {
        assert_eq!(parse_duration(""), None);
        assert_eq!(parse_duration("live"), None);
        assert_eq!(parse_duration("3:xx"), None);
    }

    /// Pressing a thumb rates; pressing it again un-rates, as on the site.
    #[test]
    fn pressing_a_thumb_toggles() {
        use LikeStatus::{Dislike, Indifferent, Like};
        assert_eq!(Indifferent.toggled(Like), Like);
        assert_eq!(Like.toggled(Like), Indifferent);
        assert_eq!(Like.toggled(Dislike), Dislike);
        assert_eq!(Dislike.toggled(Dislike), Indifferent);
    }

    /// Writes want the bare id; browses and the library hand out the `VL` form.
    #[test]
    fn playlist_ids_shed_one_browse_prefix() {
        assert_eq!(bare_playlist_id("VLLM"), "LM");
        assert_eq!(bare_playlist_id("VLPLabc"), "PLabc");
        assert_eq!(bare_playlist_id("LM"), "LM");
        assert_eq!(bare_playlist_id("PLabc"), "PLabc");
        assert!(is_liked_auto("VLLM"));
        assert!(is_liked_auto("LM"));
        assert!(!is_liked_auto("VLPLabc"));
        assert!(is_episodes_for_later("VLSE"));
        assert!(is_episodes_for_later("SE"));
        assert!(!is_episodes_for_later("VLPLabc"));
        assert!(is_expanding_playlist_id("VLRDCLAK5uy_khy"));
        assert!(is_expanding_playlist_id("RDCLAK5uy_khy"));
        assert!(!is_expanding_playlist_id("VLPLabc"));
        assert!(!is_expanding_playlist_id("VLLM"));
    }

    /// The rail's second line: your own lists show how much is in them — as does Liked Music,
    /// which has no maker — while someone else's list leads with who made it, count and all.
    #[test]
    fn a_playlist_detail_leads_with_its_maker_when_it_is_someone_elses() {
        let mut p = PlaylistRef {
            id: "VLPLabc".to_owned(),
            title: "anime bangers".to_owned(),
            author: "IV".to_owned(),
            author_id: None,
            track_count: Some("29 tracks".to_owned()),
            thumbnail: None,
            editable: true,
            source: PlaylistSource::YouTube,
        };
        assert_eq!(p.detail(), "29 tracks");
        p.editable = false;
        p.author = "Someone Else".to_owned();
        assert_eq!(p.detail(), "Someone Else • 29 tracks");
        p.id = "VLLM".to_owned();
        assert_eq!(p.detail(), "29 tracks", "Liked Music keeps its count");
        p.id = "VLPLabc".to_owned();
        p.track_count = None;
        assert_eq!(p.detail(), "Someone Else");
        p.author = String::new();
        p.track_count = Some("3 songs".to_owned());
        assert_eq!(p.detail(), "3 songs", "no name, just the count");
        p.track_count = None;
        assert_eq!(p.detail(), "", "nothing to say is said as nothing");
    }
}
