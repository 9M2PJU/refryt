//! Parsers we own, over the raw InnerTube JSON.
//!
//! `api.rs`'s rule is that `ytmapi-rs` types stop there. This is the same rule one layer down:
//! raw JSON stops here, and everything leaves as a `model` type.
//!
//! # Why this exists
//!
//! `ytmapi-rs` is still the right dependency — it owns authentication, request signing, the
//! consent cookie and query construction, none of which we want to reimplement. But two of its
//! *parsers* no longer match what YouTube sends, and both failures are visible in the app:
//!
//! * **Search.** An unfiltered search now returns a flat, relevance-ranked list of
//!   `musicResponsiveListItemRenderer`s inside `itemSectionRenderer`s, with a
//!   `musicCardShelfRenderer` on top. The `musicShelfRenderer` shelves titled "Songs" /
//!   "Artists" / "Albums" are gone. Every client in this ecosystem — including `ytmusicapi`,
//!   the Python reference the field ports from — looks for `musicShelfRenderer` and reads a bare
//!   `itemSectionRenderer` as *"no results"*. That is why the app used to fall back to three
//!   filtered queries and lose YouTube's cross-type ranking: searching `travis` gave you the
//!   Scottish band rather than Travis Scott.
//! * **Artist pages.** `ytmapi_rs::parse::ArtistSong` has no `thumbnails` field, so the per-song
//!   covers YouTube sends are discarded and every row fell back to the artist's photo.
//!
//! `YtMusic::json_query` hands back the raw response for the same request, so fixing both costs
//! no extra round trips and no new dependency.
//!
//! # Shape
//!
//! Everything here is total: a missing or renamed key yields `None` or is skipped, never a panic
//! and never an error that loses the whole response. A response we cannot read at all comes back
//! empty, and `api.rs` falls back to the typed query this replaced.

use std::sync::Arc;

use serde_json::Value;

use crate::model::{
    AlbumRef, Artist, ArtistActions, ArtistRef, Entry, HomeChip, LikeStatus, MoodPlaylists,
    MoodRef, PlaylistRef, SearchResults, Shelf, Track,
};

// ---------------------------------------------------------------------------------------------
// Small helpers over `serde_json`
// ---------------------------------------------------------------------------------------------

/// `runs` joined into one string, or `simpleText`. Empty when neither is present.
fn text(v: &Value) -> String {
    if let Some(runs) = v.get("runs").and_then(Value::as_array) {
        return runs
            .iter()
            .filter_map(|r| r.get("text").and_then(Value::as_str))
            .collect();
    }
    v.get("simpleText")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn non_empty(s: String) -> Option<String> {
    (!s.is_empty()).then_some(s)
}

/// The largest thumbnail under `v`, wherever YouTube chose to nest it this week.
fn thumbnail(v: &Value) -> Option<Arc<str>> {
    for path in [
        // list rows and shelf items
        "/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails",
        // carousel cards and page headers
        "/thumbnailRenderer/musicThumbnailRenderer/thumbnail/thumbnails",
        "/thumbnail/croppedSquareThumbnailRenderer/thumbnail/thumbnails",
        "/thumbnail/thumbnails",
        // a user channel's visual header: the profile photo is the *foreground* thumbnail,
        // which is the one path that made a profile page open with no picture
        "/foregroundThumbnail/musicThumbnailRenderer/thumbnail/thumbnails",
        // account items and the account-menu header, which name the field after what it is
        // rather than calling it a thumbnail — the reason no profile picture ever drew
        "/accountPhoto/thumbnails",
    ] {
        if let Some(list) = v.pointer(path).and_then(Value::as_array)
            && let Some(url) = list
                .last()
                .and_then(|t| t.get("url"))
                .and_then(Value::as_str)
        {
            return Some(Arc::from(url));
        }
    }
    None
}

/// The flex columns of a list row, as plain strings.
fn flex_columns(row: &Value) -> Vec<String> {
    row.get("flexColumns")
        .and_then(Value::as_array)
        .map(|cols| {
            cols.iter()
                .map(|c| {
                    text(
                        c.pointer("/musicResponsiveListItemFlexColumnRenderer/text")
                            .unwrap_or(&Value::Null),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// What a row points at. Locale-independent — never parse the English word in the subtitle, which
/// changes with `hl`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    Browse { id: String, page: String },
    Watch { video_id: String, kind: String },
}

fn endpoint_target(endpoint: &Value) -> Option<Target> {
    if let Some(browse) = endpoint.get("browseEndpoint") {
        let id = browse.get("browseId").and_then(Value::as_str)?.to_owned();
        let page = browse
            .pointer(
                "/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType",
            )
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        return Some(Target::Browse { id, page });
    }
    if let Some(watch) = endpoint.get("watchEndpoint") {
        let video_id = watch.get("videoId").and_then(Value::as_str)?.to_owned();
        let kind = watch
            .pointer("/watchEndpointMusicSupportedConfigs/watchEndpointMusicConfig/musicVideoType")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        return Some(Target::Watch { video_id, kind });
    }
    None
}

/// A row's destination. YouTube puts it on the row, on the play overlay, or on a run inside the
/// first flex column depending on the item type, so all three are tried.
fn row_target(row: &Value) -> Option<Target> {
    if let Some(t) = row.get("navigationEndpoint").and_then(endpoint_target) {
        return Some(t);
    }
    if let Some(t) = row
        .pointer(
            "/overlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer\
             /playNavigationEndpoint",
        )
        .and_then(endpoint_target)
    {
        return Some(t);
    }
    row.pointer("/flexColumns/0/musicResponsiveListItemFlexColumnRenderer/text/runs")
        .and_then(Value::as_array)?
        .iter()
        .find_map(|run| run.get("navigationEndpoint").and_then(endpoint_target))
}

const PAGE_ARTIST: &str = "MUSIC_PAGE_TYPE_ARTIST";
/// A channel page behind a video's byline: browses like an artist page, so its id is kept
/// wherever an artist id would be.
const PAGE_CHANNEL: &str = "MUSIC_PAGE_TYPE_USER_CHANNEL";
const PAGE_ALBUM: &str = "MUSIC_PAGE_TYPE_ALBUM";
const PAGE_PLAYLIST: &str = "MUSIC_PAGE_TYPE_PLAYLIST";
/// The album-audio track: what YouTube Music calls a song.
const VIDEO_ATV: &str = "MUSIC_VIDEO_TYPE_ATV";
/// Music videos — playable, but the cover is a 16:9 still rather than square art.
const VIDEO_KINDS: [&str; 3] = [
    "MUSIC_VIDEO_TYPE_OMV",
    "MUSIC_VIDEO_TYPE_OFFICIAL_SOURCE_MUSIC",
    "MUSIC_VIDEO_TYPE_UGC",
];

/// One field of a row, with whatever it links to and which column it came from.
///
/// A field is bullet-separated *or* separately linked. YouTube writes several artists inside one
/// byline as plain `", "` and `" & "` runs between linked ones — `James Blake, Ludwig Göransson &
/// Travis Scott`, each name carrying its own browse id — and folding those into one field kept
/// only the first id. The rest were dropped here, at the parse, and never existed again, which is
/// why only the primary artist was ever clickable.
struct Segment {
    text: String,
    target: Option<Target>,
    column: usize,
    /// What separated this field from the one before it: `" • "` between fields, or YouTube's own
    /// `", "` / `" & "` between artists in the same byline. Empty for the first field.
    sep: String,
}

/// Split a flex column into its bullet-separated fields.
///
/// **Do not read these positionally.** YouTube sends at least two layouts for the same query
/// depending on the request context: with `hl`/`gl` set a song's subtitle is
/// `"Song • Nemzzz"`, and with the context `ytmapi-rs` actually sends it is
/// `"Nemzzz • GASS • 1:54"` — artist, album, duration, and no type word at all. Stripping "the
/// bit before the first bullet" therefore ate the artist and left the album and duration in its
/// place. What is stable is the *endpoints*: the artist and album fields link, the rest do not.
fn segments(row: &Value, column: usize) -> Vec<Segment> {
    let Some(runs) = row
        .pointer(&format!(
            "/flexColumns/{column}/musicResponsiveListItemFlexColumnRenderer/text/runs"
        ))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    segments_from_runs(runs, column)
}

/// Split one run list into its bullet-separated fields.
///
/// The inner half of [`segments`], factored out so rows that carry their runs outside flex
/// columns — the watch queue's `longBylineText` — parse through the same code, and
/// `linked_artists`, `linked_album`, `album_id_of` and `year_segment` all apply unchanged.
fn segments_from_runs(runs: &[Value], column: usize) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::new();
    let blank = |sep: String| Segment {
        text: String::new(),
        target: None,
        column,
        sep,
    };
    let mut current = blank(String::new());
    // The separator the *next* field will carry. A bullet ends a field outright; the text between
    // two linked runs is YouTube's own joiner and belongs to the field it introduces.
    let mut pending = String::new();

    let flush = |current: &mut Segment, out: &mut Vec<Segment>, next_sep: String| {
        if !current.text.trim().is_empty() || current.target.is_some() {
            current.text = current.text.trim().to_owned();
            out.push(std::mem::replace(
                current,
                Segment {
                    text: String::new(),
                    target: None,
                    column,
                    sep: next_sep,
                },
            ));
        } else {
            current.sep = next_sep;
        }
    };

    for run in runs {
        let text = run.get("text").and_then(Value::as_str).unwrap_or_default();
        let target = run.get("navigationEndpoint").and_then(endpoint_target);

        if text.trim() == "•" {
            flush(&mut current, &mut out, " • ".to_owned());
            pending.clear();
            continue;
        }
        // A linked run always begins its own field. That is the whole fix: three linked artist
        // runs become three segments with three ids instead of one segment with one.
        if target.is_some() {
            flush(&mut current, &mut out, std::mem::take(&mut pending));
            current.text.push_str(text);
            current.target = target;
            continue;
        }
        if current.target.is_some() {
            // Plain text after a link is the joiner before whatever comes next, not part of the
            // link's own name.
            pending.push_str(text);
            continue;
        }
        current.text.push_str(text);
    }
    flush(&mut current, &mut out, String::new());
    out
}

/// Every field of a row except its title, across all of its columns.
///
/// Rows do not agree on which column holds what. A search row puts the artist, album and duration
/// in column 1 and the play count in column 2; a library row puts the artist in column 1, the
/// album in column 2 and the duration in a `fixedColumn`; an artist page's top songs put plays in
/// column 2 and the album in column 3. Reading a fixed column index is how the library page ended
/// up with no album, no duration, and the album text rendered as a play count.
fn row_segments(row: &Value) -> Vec<Segment> {
    let columns = row
        .get("flexColumns")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    (1..columns).flat_map(|c| segments(row, c)).collect()
}

/// The duration, which some layouts put in a `fixedColumn` rather than a flex one.
fn fixed_duration(row: &Value) -> Option<u64> {
    let columns = row.get("fixedColumns").and_then(Value::as_array)?;
    columns.iter().find_map(|c| {
        let text = text(c.pointer("/musicResponsiveListItemFixedColumnRenderer/text")?);
        crate::model::parse_duration(&text)
    })
}

/// A play count: plain text in a column that links to nothing, and not a duration or a year.
///
/// Identified by shape rather than position, because the column it lives in moves between layouts
/// and its wording is localised.
fn plays_segment(segs: &[Segment]) -> Option<String> {
    let linked: std::collections::HashSet<usize> = segs
        .iter()
        .filter(|s| s.target.is_some())
        .map(|s| s.column)
        .collect();
    segs.iter()
        .find(|s| {
            s.target.is_none()
                && !linked.contains(&s.column)
                && !s.text.contains(':')
                && !(s.text.len() == 4 && s.text.chars().all(|c| c.is_ascii_digit()))
                && s.text.chars().any(|c| c.is_ascii_digit())
        })
        .map(|s| s.text.clone())
}

/// The credited artists as one display string, for the fields that are not links.
fn artist_line(segs: &[Segment]) -> Option<String> {
    let names: Vec<&str> = artist_link_segments(segs)
        .map(|s| s.text.as_str())
        .collect();
    (!names.is_empty()).then(|| names.join(", "))
}

/// The segments that link to an artist page — or to the channel page behind a video's
/// byline, which browses the same way.
fn artist_link_segments(segs: &[Segment]) -> impl Iterator<Item = &Segment> {
    segs.iter().filter(|s| {
        matches!(&s.target, Some(Target::Browse { page, .. }) if page == PAGE_ARTIST || page == PAGE_CHANNEL)
    })
}

/// Every artist credited in a row, each keeping its own browse id.
///
/// Falls back to one unlinked name when nothing links: a featured-artist byline like
/// `"Gorillaz & Del The Funky Homosapien"` sometimes arrives as plain text, and a name with no
/// page is still the right thing to show.
fn artist_links(segs: &[Segment]) -> Vec<crate::model::ArtistLink> {
    let linked: Vec<crate::model::ArtistLink> = artist_link_segments(segs)
        .map(|s| crate::model::ArtistLink {
            name: s.text.clone(),
            id: match &s.target {
                Some(Target::Browse { id, .. }) => Some(id.clone()),
                _ => None,
            },
        })
        .collect();
    if !linked.is_empty() {
        return linked;
    }
    segs.iter()
        .find(|s| s.target.is_none() && !s.text.trim().is_empty())
        .map(|s| vec![crate::model::ArtistLink::plain(s.text.clone())])
        .unwrap_or_default()
}

/// The browse id behind an album segment, so the album name in a row can be a link.
fn album_id_of(seg: &Segment) -> Option<String> {
    match &seg.target {
        Some(Target::Browse { id, .. }) => Some(id.clone()),
        _ => None,
    }
}

fn linked_album(segs: &[Segment]) -> Option<&Segment> {
    segs.iter()
        .find(|s| matches!(&s.target, Some(Target::Browse { page, .. }) if page == PAGE_ALBUM))
}

/// The album browse id out of an item's own action menu.
///
/// The byline is not the only place YouTube attaches the album. "Your daily discover" and the
/// radio feeds credit just the artist and a play count, yet still offer "Go to album" among the
/// card's menu actions — so the album is on the item the whole time, one level down, and without
/// this the song's title had nowhere to go. Matched on the locale-independent `pageType`, never
/// the English label, so it survives a non-English `hl`.
fn menu_album_id(item: &Value) -> Option<String> {
    item.pointer("/menu/menuRenderer/items")
        .and_then(Value::as_array)?
        .iter()
        .find_map(|entry| {
            let endpoint = entry
                .pointer("/menuNavigationItemRenderer/navigationEndpoint")
                .or_else(|| entry.pointer("/menuServiceItemRenderer/serviceEndpoint"))?;
            match endpoint_target(endpoint)? {
                Target::Browse { id, page } if page == PAGE_ALBUM => Some(id),
                _ => None,
            }
        })
}

/// `"4:16"` and friends, but never a year: `"1999"` has no colon.
fn duration_segment(segs: &[Segment]) -> Option<u64> {
    segs.iter()
        .filter(|s| s.target.is_none() && s.text.contains(':'))
        .find_map(|s| crate::model::parse_duration(&s.text))
}

fn year_segment(segs: &[Segment]) -> Option<String> {
    segs.iter()
        .find(|s| {
            s.target.is_none() && s.text.len() == 4 && s.text.chars().all(|c| c.is_ascii_digit())
        })
        .map(|s| s.text.clone())
}

/// The descriptive text of a row whose fields do not link — an artist's audience, a playlist's
/// author. Drops a leading type word when there is something after it.
fn plain_tail(segs: &[Segment]) -> String {
    let plain: Vec<&str> = segs
        .iter()
        .filter(|s| s.target.is_none())
        .map(|s| s.text.as_str())
        .collect();
    match plain.len() {
        0 => String::new(),
        1 => plain[0].to_owned(),
        _ => plain[1..].join(" • "),
    }
}

/// Whether a playlist card's own menu offers to edit it — which is what makes it a playlist
/// you can add songs to.
///
/// Measured against this account's library on 2026-09-08: 24 of 26 cards carry `EDIT`, and the
/// two that do not are "New playlist" (a create tile, not a playlist) and "Liked Music" (an auto
/// playlist). Read by icon, never by label: this account's UI is in Greek.
fn card_is_editable(item: &Value) -> bool {
    fn walk(v: &Value) -> bool {
        match v {
            Value::Object(map) => {
                if map
                    .get("icon")
                    .and_then(|i| i.get("iconType"))
                    .and_then(Value::as_str)
                    == Some("EDIT")
                {
                    return true;
                }
                map.values().any(walk)
            }
            Value::Array(items) => items.iter().any(walk),
            _ => false,
        }
    }
    item.get("menu").map(walk).unwrap_or(false)
}

/// Explicit is signalled by a badge on the row.
fn is_explicit(row: &Value) -> bool {
    row.get("badges")
        .and_then(Value::as_array)
        .map(|badges| {
            badges.iter().any(|b| {
                b.pointer("/musicInlineBadgeRenderer/icon/iconType")
                    .and_then(Value::as_str)
                    == Some("MUSIC_EXPLICIT_BADGE")
            })
        })
        .unwrap_or(false)
}

/// Every track row's rating and library toggle in a response, keyed by video id.
///
/// The typed parsers carry none of this — `ytmapi-rs` has no field for a `likeButtonRenderer` —
/// so an album's tracks arrived stamped `Indifferent` and a song you had liked did not look
/// liked once it was playing. The raw browse of the same page *does* carry it on every row, so
/// this reads it there and the caller merges it onto the typed tracks.
pub fn track_states(root: &Value) -> std::collections::HashMap<String, TrackState> {
    let mut out = std::collections::HashMap::new();
    fn walk(v: &Value, out: &mut std::collections::HashMap<String, TrackState>) {
        match v {
            Value::Object(map) => {
                if let Some(row) = map.get("musicResponsiveListItemRenderer") {
                    if let Some(id) = row
                        .pointer("/playlistItemData/videoId")
                        .or_else(|| find_key(row, "videoId"))
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                    {
                        let toggle = library_toggle(row);
                        let wide = matches!(
                            row_target(row),
                            Some(Target::Watch { kind, .. }) if kind != VIDEO_ATV
                        );
                        out.entry(id.to_owned()).or_insert(TrackState {
                            like: like_status(row),
                            in_library: toggle.as_ref().map(|t| t.in_library).unwrap_or(false),
                            library_add: toggle.as_ref().and_then(|t| t.add.clone()),
                            library_remove: toggle.as_ref().and_then(|t| t.remove.clone()),
                            set_video_id: row
                                .pointer("/playlistItemData/playlistSetVideoId")
                                .and_then(Value::as_str)
                                .filter(|s| !s.is_empty())
                                .map(Arc::from),
                            wide,
                        });
                    }
                    return;
                }
                map.values().for_each(|v| walk(v, out));
            }
            Value::Array(items) => items.iter().for_each(|v| walk(v, out)),
            _ => {}
        }
    }
    walk(root, &mut out);
    out
}

/// What a raw row knows about a track that a typed one does not.
#[derive(Clone, Debug, Default)]
pub struct TrackState {
    pub like: LikeStatus,
    pub in_library: bool,
    pub library_add: Option<Arc<str>>,
    pub library_remove: Option<Arc<str>>,
    pub set_video_id: Option<Arc<str>>,
    /// A music video rather than an album-audio track — see `Track::wide`'s own doc. The typed
    /// album parser has no field for this at all (it reads only the play button's video id), so
    /// this is the only place that ever learns it for an album/single track.
    pub wide: bool,
}

/// Your rating, from the row menu's like button.
///
/// The button lives at `menu/menuRenderer/topLevelButtons[0]/likeButtonRenderer`, but the menu
/// itself moves between row types — and is absent entirely on signed-out rows — so it is found
/// by key rather than by path. No button means no rating, never an error.
fn like_status(row: &Value) -> LikeStatus {
    match find_key(row, "likeButtonRenderer")
        .and_then(|b| b.get("likeStatus"))
        .and_then(Value::as_str)
    {
        Some("LIKE") => LikeStatus::Like,
        Some("DISLIKE") => LikeStatus::Dislike,
        _ => LikeStatus::Indifferent,
    }
}

/// The now-playing song's rating, from the player overlay.
///
/// Watch/`next` rows carry no like button — only `playerOverlays` does, at
/// `…/playerOverlayRenderer/actions/*/likeButtonRenderer/likeStatus` — so a mix's own seed
/// would otherwise look unrated in the player bar. Scoped to `playerOverlays` so a queue row's
/// menu button could never be mistaken for it. `None` when the overlay is absent, which is the
/// case on a continuation page.
pub fn playback_like(root: &Value) -> Option<LikeStatus> {
    let overlay = find_key(root, "playerOverlays")?;
    find_key(overlay, "likeButtonRenderer")?;
    Some(like_status(overlay))
}

/// The row's library toggle: whether the song is saved, and the tokens that flip it.
///
/// The item is identified by its `BOOKMARK_BORDER` icon, never by its label — labels localise
/// (this account's UI is in Greek) while the icon does not. `defaultText` describes the save
/// action and `toggledText` the remove one; `isToggled` is the current state.
///
/// Takes any subtree: rows pass the row, page headers pass the header — scoped, because a
/// document-wide scan would read a song's bookmark as the page's.
pub struct LibraryToggle {
    pub(crate) in_library: bool,
    pub(crate) add: Option<Arc<str>>,
    pub(crate) remove: Option<Arc<str>>,
}

pub(crate) fn library_toggle(row: &Value) -> Option<LibraryToggle> {
    fn walk(v: &Value) -> Option<&Value> {
        match v {
            Value::Object(map) => {
                if map
                    .get("defaultIcon")
                    .and_then(|i| i.get("iconType"))
                    .and_then(Value::as_str)
                    == Some("BOOKMARK_BORDER")
                {
                    return Some(v);
                }
                map.values().find_map(walk)
            }
            Value::Array(items) => items.iter().find_map(walk),
            _ => None,
        }
    }
    let item = walk(row)?;
    let token = |key: &str| {
        item.pointer(&format!("/{key}/feedbackEndpoint/feedbackToken"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(Arc::from)
    };
    Some(LibraryToggle {
        in_library: item
            .get("isToggled")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        add: token("defaultServiceEndpoint"),
        remove: token("toggledServiceEndpoint"),
    })
}

/// A page header's own bookmark toggle: the album's `musicDetailHeaderRenderer`, the
/// playlist's `musicResponsiveHeaderRenderer`. Scoped to the header subtree — see
/// [`PlaylistPage::library`].
pub(crate) fn find_header_toggle(root: &Value, header_key: &str) -> Option<LibraryToggle> {
    find_key(root, header_key).and_then(library_toggle)
}

/// What an artist's own menu offers, read by icon.
///
/// Icons, never labels: this account's UI is in Greek, and `MUSIC_SHUFFLE` / `MIX` / `SUBSCRIBE`
/// / `REMOVE_CIRCLE` say the same thing in every language. Neither radio playlist id can be
/// built from the channel id — I checked — so both are only ever read from here.
fn artist_actions(item: &Value) -> crate::model::ArtistActions {
    use crate::model::ArtistActions;
    let mut out = ArtistActions::default();
    let Some(items) = find_key(item, "menuRenderer")
        .and_then(|m| m.get("items"))
        .and_then(Value::as_array)
    else {
        return out;
    };
    for entry in items {
        for renderer in entry.as_object().into_iter().flatten().map(|(_, v)| v) {
            let icon = renderer
                .get("icon")
                .or_else(|| renderer.get("defaultIcon"))
                .and_then(|i| i.get("iconType"))
                .and_then(Value::as_str);
            match icon {
                Some("MUSIC_SHUFFLE") => out.shuffle = watch_target(renderer),
                Some("MIX") => out.mix = watch_target(renderer),
                Some("SUBSCRIBE") => {
                    // `defaultServiceEndpoint` is what the button does *now*: unsubscribing
                    // means you are subscribed.
                    out.subscribed = Some(
                        find_key(renderer, "unsubscribeEndpoint").is_some()
                            && renderer
                                .get("defaultServiceEndpoint")
                                .is_some_and(|d| d.get("unsubscribeEndpoint").is_some()),
                    );
                }
                Some("REMOVE_CIRCLE") => {
                    out.dont_recommend = find_key(renderer, "feedbackToken")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(Arc::from);
                }
                _ => {}
            }
        }
    }
    out
}

/// A `watchEndpoint` or `watchPlaylistEndpoint` as something we can play.
fn watch_target(renderer: &Value) -> Option<crate::model::WatchTarget> {
    let ep = find_key(renderer, "watchPlaylistEndpoint")
        .or_else(|| find_key(renderer, "watchEndpoint"))?;
    let playlist_id = ep
        .get("playlistId")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())?
        .to_owned();
    Some(crate::model::WatchTarget {
        video_id: ep
            .get("videoId")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
        playlist_id,
        params: ep
            .get("params")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
    })
}

// ---------------------------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------------------------

/// Turn a search response into a ranked list, whichever shape it arrived in.
///
/// Reads the flat `itemSectionRenderer` layout **and** the older `musicShelfRenderer` shelves in
/// one pass, so a flip back costs nothing. Order is YouTube's throughout — that ranking is the
/// whole point, and it is what a local heuristic could never reproduce.
///
/// # The top card
///
/// The response also carries a `musicCardShelfRenderer` — YouTube's own highlighted result.
/// Whether its own identity leads the list, trails it, or is dropped depends on what the ranked
/// rows themselves say, found by trying the two simpler rules first:
///
/// * **Always promoting it disagreed with YouTube's own ranking.** For `travis` the card is the
///   Scottish band while the *ranked list's own* top pick is Travis Scott — a real conflict
///   about who the one artist at the front should be, and the card is the loser of it.
/// * **Always dropping it (or always appending it) lost real results.** Searching an artist's
///   exact name (`the weeknd`, say) puts *only* that artist in the card — the rest of the ranked
///   rows are their songs, videos and unrelated collaborators, with no separate
///   "The Weeknd, Artist" row anywhere. There the card is not competing with anything; it is the
///   *only* source for that artist at all, so burying it at the end was just a milder version of
///   losing it outright — right kind of caution, wrong amount.
///
/// The rule that satisfies both: promote the card only when the ranked list's own leading entry
/// is not the same kind of thing. If the list's first row is already an artist, that *is*
/// YouTube's ranking casting a vote for who belongs at the front, and the card loses to it
/// (`travis`) — trailing, not dropped, so it is still findable rather than gone. If the list's
/// first row is a song, a video, anything that is not also naming a top artist, there is no vote
/// to lose to, and the card — which is, after all, YouTube's own highlighted pick — leads
/// (`the weeknd`). Either way, showing the same entity twice is avoided by skipping the card
/// outright when it already ranked into the list on its own. Only the artist case is extracted
/// — the one confirmed by actually hitting this — since every other card kind observed so far
/// *did* already have a matching row below it.
pub fn parse_search(root: &Value) -> SearchResults {
    let sections = root
        .pointer(
            "/contents/tabbedSearchResultsRenderer/tabs/0/tabRenderer/content\
             /sectionListRenderer/contents",
        )
        .and_then(Value::as_array);

    let mut items = Vec::new();
    let mut card = None;

    for section in sections.into_iter().flatten() {
        if let Some(shelf) = section.get("musicCardShelfRenderer") {
            card = parse_search_card(shelf);
            continue;
        }
        let rows = section
            .pointer("/itemSectionRenderer/contents")
            .or_else(|| section.pointer("/musicShelfRenderer/contents"))
            .and_then(Value::as_array);
        for row in rows.into_iter().flatten() {
            if let Some(row) = row.get("musicResponsiveListItemRenderer")
                && let Some(item) = parse_search_row(row)
            {
                items.push(item);
            }
        }
    }

    if let Some(card) = card {
        let already_ranked = items.iter().any(|item| entry_key(item) == entry_key(&card));
        // Only artist cards are ever parsed here (see `parse_search_card`), so an existing
        // artist in the lead position is the only conflict this needs to check for yet.
        let ranking_disagrees = matches!(items.first(), Some(Entry::Artist(_)));
        if already_ranked {
            // Nothing to do: it is already exactly where the ranking put it.
        } else if ranking_disagrees {
            items.push(card);
        } else {
            items.insert(0, card);
        }
    }

    SearchResults { items }
}

/// The identity search results are deduplicated by — video id for a song/video, browse id for
/// everything else with a page of its own. Two `Entry`s with the same key are the same real
/// thing, whichever renderer shape either one happened to be read from.
fn entry_key(entry: &Entry) -> &str {
    match entry {
        Entry::Song(t) | Entry::Video(t) => &t.video_id,
        Entry::Artist(a) => &a.id,
        Entry::Album(a) => &a.id,
        Entry::Playlist(p) => &p.id,
        Entry::Mood(m) => &m.params,
    }
}

/// The top card's own identity, when it is an artist — see [`parse_search`]'s own doc comment
/// for why only this one kind is worth the trouble. A song/album/playlist card's `title` has no
/// `navigationEndpoint` of the shape this reads (it opens the play overlay instead), so those
/// simply fail to parse here and `parse_search` falls back to its old behaviour for them: drop
/// the card, trust the row that (per every case observed) already exists for it below.
fn parse_search_card(card: &Value) -> Option<Entry> {
    let nav = card.pointer("/title/runs/0/navigationEndpoint")?;
    let Target::Browse { id, page } = endpoint_target(nav)? else {
        return None;
    };
    if page != PAGE_ARTIST {
        return None;
    }
    let name = non_empty(text(card.get("title").unwrap_or(&Value::Null)))?;
    // "Artist • 254M monthly audience" — everything after the bullet, when there is one.
    let subscribers = non_empty(text(card.get("subtitle").unwrap_or(&Value::Null)))
        .and_then(|s| s.split_once(" • ").map(|(_, tail)| tail.to_owned()));
    Some(Entry::Artist(ArtistRef {
        id,
        name,
        subscribers,
        thumbnail: thumbnail(card),
        // Not read from the card's own Shuffle/Mix buttons: nothing downstream currently reads
        // an artist's `actions` from a search row (`search.rs` shows only name and subscriber
        // count), so there is nothing yet worth the extra parsing to populate.
        actions: ArtistActions::default(),
    }))
}

/// A chart rank and its movement, from the row's `customIndexColumn` — or nothing, where the
/// row names none. Chart playlists carry the column; nothing else observed does, so absence
/// is the common case and degrades to an unranked row rather than an error.
fn rank_of(row: &Value) -> (Option<String>, crate::model::ChartTrend) {
    use crate::model::ChartTrend;
    let column = row.pointer("/customIndexColumn/musicCustomIndexColumnRenderer");
    let rank = column
        .and_then(|c| c.get("text"))
        .map(text)
        .and_then(non_empty);
    let trend = match column
        .and_then(|c| c.pointer("/icon/iconType"))
        .and_then(Value::as_str)
    {
        Some("ARROW_CHART_UP") => ChartTrend::Up,
        Some("ARROW_CHART_DOWN") => ChartTrend::Down,
        Some("ARROW_CHART_NEUTRAL") => ChartTrend::Steady,
        _ => ChartTrend::Unknown,
    };
    (rank, trend)
}

/// One ranked row. Returns `None` for the types the app has no page for — podcasts, episodes and
/// user profiles — rather than showing a row that goes nowhere or will not play.
fn parse_search_row(row: &Value) -> Option<Entry> {
    let cols = flex_columns(row);
    let title = non_empty(cols.first()?.clone())?;
    let segs = row_segments(row);
    let thumb = thumbnail(row);

    match row_target(row)? {
        Target::Browse { id, page } if page == PAGE_ARTIST => Some(Entry::Artist(ArtistRef {
            id,
            name: title,
            subscribers: non_empty(plain_tail(&segs)),
            thumbnail: thumb,
            actions: artist_actions(row),
        })),
        Target::Browse { id, page } if page == PAGE_ALBUM => Some(Entry::Album(AlbumRef {
            id,
            title,
            // The album's artist is a linked field; the year is the bare four digits.
            artist: artist_line(&segs).unwrap_or_else(|| plain_tail(&segs)),
            year: year_segment(&segs),
            kind: None,
            thumbnail: thumb,
        })),
        Target::Browse { id, page } if page == PAGE_PLAYLIST => {
            Some(Entry::Playlist(PlaylistRef {
                id,
                title,
                author: artist_line(&segs).unwrap_or_else(|| plain_tail(&segs)),
                author_id: artist_links(&segs).into_iter().find_map(|l| l.id),
                track_count: None,
                thumbnail: thumb,
                // A row in a result list carries no editability signal, and only the library
                // grid feeds the save picker, so nothing needs one here.
                editable: false,
                source: crate::model::PlaylistSource::YouTube,
            }))
        }
        Target::Browse { .. } => None,
        Target::Watch { video_id, kind } => {
            // Allow-list, not deny-list. A podcast episode's row has an *empty* top-level
            // `navigationEndpoint` and hides a watch endpoint on its play overlay, so "anything
            // that is not a song must be a music video" quietly let episodes through.
            let is_song = kind == VIDEO_ATV;
            if !is_song && !VIDEO_KINDS.contains(&kind.as_str()) {
                return None;
            }
            let toggle = library_toggle(row);
            // A chart rank, where the row names one — chart playlists do, and nothing else
            // observed does.
            let (rank, trend) = rank_of(row);
            let track = Track {
                video_id,
                title,
                artists: artist_links(&segs),
                album: linked_album(&segs).map(|s| s.text.clone()),
                album_id: linked_album(&segs)
                    .and_then(album_id_of)
                    .or_else(|| menu_album_id(row)),
                duration: fixed_duration(row).or_else(|| duration_segment(&segs)),
                plays: plays_segment(&segs),
                explicit: is_explicit(row),
                like: like_status(row),
                in_library: toggle.as_ref().map(|t| t.in_library).unwrap_or(false),
                library_add: toggle.as_ref().and_then(|t| t.add.clone()),
                library_remove: toggle.as_ref().and_then(|t| t.remove.clone()),
                // Only playlist rows carry one; only it makes "Remove from playlist"
                // offerable, so a wrong assumption about its location disables the button
                // rather than corrupting a playlist.
                set_video_id: row
                    .pointer("/playlistItemData/playlistSetVideoId")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(Arc::from),
                thumbnail: thumb,
                wide: !is_song,
                rank,
                trend,
                local_path: None,
            };
            Some(if is_song {
                Entry::Song(track)
            } else {
                Entry::Video(track)
            })
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Browse feeds (home, explore)
// ---------------------------------------------------------------------------------------------

/// Read any browse response's `sectionListRenderer` into shelves.
///
/// Serves `FEmusic_home` and `FEmusic_explore` alike: both are lists of titled carousels, and a
/// signed-in home adds `musicShelfRenderer` song lists ("Quick picks") which are handled too.
/// Anything unreadable is skipped rather than failing the page — a home feed with one odd shelf
/// should lose that shelf, not the feed.
pub fn parse_browse_shelves(root: &Value) -> Vec<Shelf> {
    let Some(sections) = find_section_list(root) else {
        return Vec::new();
    };
    sections
        .iter()
        .filter_map(|section| {
            if let Some(carousel) = section.get("musicCarouselShelfRenderer") {
                return parse_carousel(carousel);
            }
            // A song list, as "Quick picks" is when signed in.
            if let Some(shelf) = section.get("musicShelfRenderer") {
                let title = shelf.get("title").map(text).and_then(non_empty)?;
                let items: Vec<Entry> = shelf
                    .get("contents")
                    .and_then(Value::as_array)
                    .map(|rows| {
                        rows.iter()
                            .filter_map(|r| {
                                parse_search_row(r.get("musicResponsiveListItemRenderer")?)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                return (!items.is_empty()).then_some(Shelf {
                    title,
                    items,
                    more_params: None,
                    more_browse_id: None,
                    expanded: false,
                });
            }
            // `musicTastebuilderShelfRenderer` is the "tell us what you like" prompt shown to
            // signed-out and new accounts; it holds no content and has nowhere to go.
            None
        })
        .collect()
}

/// The buttons across the top of Explore, read off the feed itself: a `gridRenderer` of
/// `musicNavigationButtonRenderer` items, each carrying the browse id (and params, where the
/// feed sends them — Charts does) its page needs.
pub fn parse_explore_links(root: &Value) -> Vec<crate::model::ExploreLink> {
    let Some(sections) = find_section_list(root) else {
        return Vec::new();
    };
    sections
        .iter()
        .filter_map(|section| section.get("gridRenderer"))
        .flat_map(|grid| {
            grid.get("items")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .filter_map(|item| {
            let chip = item.get("musicNavigationButtonRenderer")?;
            let title = non_empty(text(chip.get("buttonText")?))?;
            let endpoint = chip.pointer("/clickCommand/browseEndpoint")?;
            let browse_id = endpoint.get("browseId").and_then(Value::as_str)?.to_owned();
            let params = endpoint
                .get("params")
                .and_then(Value::as_str)
                .map(str::to_owned);
            Some(crate::model::ExploreLink {
                title,
                browse_id,
                params,
            })
        })
        .collect()
}

/// The Charts page: ranked row shelves plus the card shelves that share it.
///
/// A ranked row carries its rank in `customIndexColumn` beside the flex columns, with the
/// movement in its icon. Both are optional — a row without them still parses, it just shows
/// no rank — because no captured response carries the column and its exact shape is read
/// off the live feed, not a fixture.
pub fn parse_charts(root: &Value) -> crate::model::ChartsFeed {
    use crate::model::{ChartRow, ChartSection, ChartsFeed};
    let Some(sections) = find_section_list(root) else {
        return ChartsFeed::default();
    };
    let mut ranked = Vec::new();
    let mut shelves = Vec::new();
    for section in sections {
        if let Some(carousel) = section.get("musicCarouselShelfRenderer")
            && let Some(shelf) = parse_carousel(carousel)
        {
            shelves.push(shelf);
            continue;
        }
        if let Some(shelf) = section.get("musicShelfRenderer") {
            let Some(title) = shelf.get("title").map(text).and_then(non_empty) else {
                continue;
            };
            let rows: Vec<ChartRow> = shelf
                .get("contents")
                .and_then(Value::as_array)
                .map(|rows| {
                    rows.iter()
                        .filter_map(|r| {
                            r.get("musicResponsiveListItemRenderer")
                                .map(parse_chart_row)
                        })
                        .flatten()
                        .collect()
                })
                .unwrap_or_default();
            if !rows.is_empty() {
                ranked.push(ChartSection { title, rows });
            }
        }
    }
    ChartsFeed { ranked, shelves }
}

/// One chart row: the entry as ever, plus the rank column beside it.
fn parse_chart_row(row: &Value) -> Option<crate::model::ChartRow> {
    let entry = parse_search_row(row)?;
    let (rank, trend) = rank_of(row);
    Some(crate::model::ChartRow {
        entry,
        rank: rank.unwrap_or_default(),
        trend,
    })
}

/// The home feed's mood bar: `sectionListRenderer/header/chipCloudRenderer/chips[]`.
///
/// Each chip is the same `FEmusic_home` feed browsed with different params; a chip with no
/// params cannot be opened and is skipped. Found by key: the header sits beside the section
/// list, not inside it, and continuation pages carry no header — the first page's chips stand
/// for the feed.
pub fn parse_home_chips(root: &Value) -> Vec<HomeChip> {
    find_key(root, "chipCloudRenderer")
        .and_then(|c| c.get("chips"))
        .and_then(Value::as_array)
        .map(|chips| chips.iter().filter_map(parse_home_chip).collect())
        .unwrap_or_default()
}

fn parse_home_chip(chip: &Value) -> Option<HomeChip> {
    let chip = chip.get("chipCloudChipRenderer")?;
    // Note: `text`, not `title` — the one field in this renderer not called what the shelf
    // header calls it, and exactly the kind of near-miss this parser exists to survive.
    let title = non_empty(text(chip.get("text").unwrap_or(&Value::Null)))?;
    let params = chip
        .pointer("/navigationEndpoint/browseEndpoint/params")
        .and_then(Value::as_str)
        .filter(|p| !p.is_empty())?
        .to_owned();
    Some(HomeChip {
        title,
        params,
        selected: chip
            .get("isSelected")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

/// The section list of a browse response, under any of the layouts YouTube uses.
///
/// A *continuation* nests its sections under `continuationContents` instead, and keeps a stub
/// `singleColumnBrowseResultsRenderer` at the top with nothing in it — so looking only at the
/// first-page paths finds an empty page and silently ends the walk. That is why following the home
/// feed's token added no shelves.
fn find_section_list(root: &Value) -> Option<&Vec<Value>> {
    for path in [
        "/continuationContents/sectionListContinuation/contents",
        "/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content\
         /sectionListRenderer/contents",
        "/contents/twoColumnBrowseResultsRenderer/tabs/0/tabRenderer/content\
         /sectionListRenderer/contents",
        "/contents/sectionListRenderer/contents",
    ] {
        if let Some(list) = root.pointer(path).and_then(Value::as_array) {
            return Some(list);
        }
    }
    None
}

/// One page of a feed: what it held, and how to ask for the next.
///
/// Every library and home response carries a continuation token. Ignoring it is what limited the
/// library to its first 25 of 689 songs, and made sorting a 25-row window look like a sort of the
/// whole library.
#[derive(Debug, Default)]
pub struct Page {
    pub items: Vec<Entry>,
    pub next: Option<String>,
}

/// The token for the next page, in either of the two shapes YouTube uses.
///
/// `nextContinuationData` is the older one and is what the library feeds still send;
/// `continuationCommand` is the newer. `None` means this is the last page.
pub fn continuation_token(root: &Value) -> Option<String> {
    fn walk(v: &Value) -> Option<String> {
        match v {
            Value::Object(map) => {
                if let Some(t) = map
                    .get("nextContinuationData")
                    .and_then(|c| c.get("continuation"))
                    .and_then(Value::as_str)
                    .filter(|t| !t.is_empty())
                {
                    return Some(t.to_owned());
                }
                if let Some(t) = map
                    .get("continuationCommand")
                    .and_then(|c| c.get("token"))
                    .and_then(Value::as_str)
                    .filter(|t| !t.is_empty())
                {
                    return Some(t.to_owned());
                }
                map.values().find_map(walk)
            }
            Value::Array(items) => items.iter().find_map(walk),
            _ => None,
        }
    }
    walk(root)
}

/// A page of a library section: its entries plus the token for the next page.
pub fn parse_library_page(root: &Value) -> Page {
    Page {
        items: parse_library_section(root),
        next: continuation_token(root),
    }
}

/// One library section, whichever layout it arrived in.
///
/// The four library browse ids do not agree: liked songs and followed artists come back as
/// `musicShelfRenderer` lists of `musicResponsiveListItemRenderer`, while liked albums and
/// playlists come back as `gridRenderer` of `musicTwoRowItemRenderer`. `ytmapi-rs` reads only the
/// first, which is why two of the four sections could never have loaded.
///
/// An empty result is a legitimate answer — an empty library, or a session YouTube is ignoring.
/// Use [`looks_signed_out`] to tell those apart.
pub fn parse_library_section(root: &Value) -> Vec<Entry> {
    // A continuation response has no section list — its items hang off the continuation node — so
    // fall back to scanning the whole document for the two item renderers.
    let Some(sections) = find_section_list(root) else {
        return continuation_items(root);
    };
    let mut out = Vec::new();
    for section in sections {
        // A grid of cards: albums, playlists.
        if let Some(items) = section
            .pointer("/gridRenderer/items")
            .and_then(Value::as_array)
        {
            out.extend(items.iter().filter_map(parse_card));
        }
        // A list of rows: songs, artists.
        if let Some(rows) = section
            .pointer("/musicShelfRenderer/contents")
            .and_then(Value::as_array)
        {
            out.extend(
                rows.iter()
                    .filter_map(|r| parse_search_row(r.get("musicResponsiveListItemRenderer")?)),
            );
        }
        // Some layouts wrap either of the above in an item section.
        if let Some(inner) = section
            .pointer("/itemSectionRenderer/contents")
            .and_then(Value::as_array)
        {
            for item in inner {
                if let Some(items) = item
                    .pointer("/gridRenderer/items")
                    .and_then(Value::as_array)
                {
                    out.extend(items.iter().filter_map(parse_card));
                }
                if let Some(rows) = item
                    .pointer("/musicShelfRenderer/contents")
                    .and_then(Value::as_array)
                {
                    out.extend(rows.iter().filter_map(|r| {
                        parse_search_row(r.get("musicResponsiveListItemRenderer")?)
                    }));
                }
            }
        }
    }
    out
}

/// One "Show all" artist expansion, whichever layout it arrived in.
///
/// `ytmapi-rs` walks a single hardcoded pointer for this (`…/contents/0/gridRenderer/items`)
/// and fails the whole request when the artist's response puts anything else in that slot —
/// which is exactly what one artist did, while the entries sat elsewhere in the same document.
/// Scan every section for every renderer the artist pages use instead, and skip what is not
/// recognised; an expansion that finds nothing is empty, not an error.
pub fn parse_artist_albums(root: &Value) -> Vec<Entry> {
    let Some(sections) = find_section_list(root) else {
        return continuation_items(root);
    };
    let mut out = Vec::new();
    for section in sections {
        collect_artist_items(section, &mut out);
    }
    out
}

/// The entry renderers an artist expansion can hold, anywhere under `node`.
///
/// `itemSectionRenderer` wraps others, so this recurses through it. Unknown renderers — a
/// title, a sort bar, a message — are skipped, never fatal, which is the point.
fn collect_artist_items(node: &Value, out: &mut Vec<Entry>) {
    if let Some(items) = node
        .pointer("/gridRenderer/items")
        .and_then(Value::as_array)
    {
        out.extend(items.iter().filter_map(parse_card));
    }
    if let Some(rows) = node
        .pointer("/musicShelfRenderer/contents")
        .and_then(Value::as_array)
    {
        out.extend(
            rows.iter()
                .filter_map(|r| parse_search_row(r.get("musicResponsiveListItemRenderer")?)),
        );
    }
    if let Some(rows) = node
        .pointer("/musicPlaylistShelfRenderer/contents")
        .and_then(Value::as_array)
    {
        out.extend(
            rows.iter()
                .filter_map(|r| parse_search_row(r.get("musicResponsiveListItemRenderer")?)),
        );
    }
    if let Some(carousel) = node.get("musicCarouselShelfRenderer") {
        // `parse_carousel` wants a titled header; an expansion carousel may have none, so fall
        // back to reading its contents directly.
        match parse_carousel(carousel) {
            Some(shelf) => out.extend(shelf.items),
            None => {
                if let Some(items) = carousel.get("contents").and_then(Value::as_array) {
                    out.extend(items.iter().filter_map(parse_card));
                }
            }
        }
    }
    if let Some(inner) = node
        .pointer("/itemSectionRenderer/contents")
        .and_then(Value::as_array)
    {
        for item in inner {
            collect_artist_items(item, out);
        }
    }
}

/// The items of a continuation response, which arrives without a section list.
fn continuation_items(root: &Value) -> Vec<Entry> {
    let mut out = Vec::new();
    fn walk(v: &Value, out: &mut Vec<Entry>) {
        match v {
            Value::Object(map) => {
                if let Some(row) = map.get("musicResponsiveListItemRenderer") {
                    if let Some(e) = parse_search_row(row) {
                        out.push(e);
                    }
                    return;
                }
                if map.contains_key("musicTwoRowItemRenderer") {
                    if let Some(e) = parse_card(v) {
                        out.push(e);
                    }
                    return;
                }
                for value in map.values() {
                    walk(value, out);
                }
            }
            Value::Array(items) => items.iter().for_each(|i| walk(i, out)),
            _ => {}
        }
    }
    walk(root, &mut out);
    out
}

/// A playlist browse response: its header and the first page of its tracks.
///
/// Parsed here because `ytmapi-rs` returns only a fraction of a long playlist — for a 5,042-track
/// "Liked music" it hands back 46 — and offers no way to ask for the rest. The raw response gives
/// 100 rows a page with a continuation token, like every other feed.
pub struct PlaylistPage {
    pub title: String,
    pub subtitle: String,
    pub second_subtitle: String,
    /// The owner's name from the header's facepile. A public playlist's subtitle names only
    /// the kind and the year; who made it lives here. `None` when the header has no facepile.
    pub author: Option<String>,
    /// The owner's channel id, from the facepile's tap target. `None` when it is not linked.
    pub author_id: Option<String>,
    pub description: Option<String>,
    pub thumbnail: Option<Arc<str>>,
    /// Whether this is your own playlist. Owned pages render
    /// `musicEditablePlaylistDetailHeaderRenderer` around the responsive header; its presence
    /// is the ownership signal — no id lists, no label matching.
    pub editable: bool,
    /// The header's own bookmark toggle, if it carried one. Scoped to the header subtree:
    /// the rows below carry their own bookmark menu items, and a document-wide scan would
    /// read a song's state as the page's.
    pub library: Option<LibraryToggle>,
    pub page: Page,
}

/// A playlist header's description. The text sits under `description/runs` on some pages and
/// under `description/musicDescriptionShelfRenderer/description/runs` on others — the second
/// shape is why a playlist with a description showed none.
fn playlist_description(header: &Value) -> Option<String> {
    let description = header.get("description")?;
    let runs = if description.get("runs").is_some() {
        description
    } else {
        description.pointer("/musicDescriptionShelfRenderer/description")?
    };
    non_empty(text(runs)).map(|d| d.trim().to_owned())
}

pub fn parse_playlist(root: &Value) -> Option<PlaylistPage> {
    let header = find_key(root, "musicResponsiveHeaderRenderer")?;
    let title = non_empty(text(header.get("title")?))?;
    let facepile = header.get("facepile");
    Some(PlaylistPage {
        title,
        subtitle: header.get("subtitle").map(text).unwrap_or_default(),
        second_subtitle: header.get("secondSubtitle").map(text).unwrap_or_default(),
        author: facepile
            .and_then(|f| f.pointer("/avatarStackViewModel/text/content"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .and_then(non_empty),
        // The facepile names the owner and, when linked, carries a channel browse endpoint on
        // its tap target — that is the profile the name should open.
        author_id: facepile
            .and_then(|f| find_key(f, "browseEndpoint"))
            .and_then(|b| b.get("browseId"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
        description: playlist_description(header),
        thumbnail: thumbnail(header),
        editable: find_key(root, "musicEditablePlaylistDetailHeaderRenderer").is_some(),
        library: find_header_toggle(root, "musicResponsiveHeaderRenderer"),
        page: parse_playlist_tracks(root),
    })
}

/// Whether a browse response is YouTube's "sign in" prompt rather than a page.
///
/// Signed out, a personal feed that needs the account (VLLM — "Liked Music") does not come back
/// as an empty shell: it answers with a `singleColumnBrowseResultsRenderer` holding a
/// `messageRenderer` — "Looking for what you've liked?" — and no header and no rows. That is why
/// the empty-shell test alone reads it as content and lets the typed fallback die on its
/// two-column key instead of saying the session is gone.
pub fn sign_in_prompt(root: &Value) -> bool {
    find_key(root, "messageRenderer").is_some()
        && find_key(root, "musicResponsiveHeaderRenderer").is_none()
        && find_key(root, "musicResponsiveListItemRenderer").is_none()
}

/// The tracks of a playlist response, first page or continuation.
pub fn parse_playlist_tracks(root: &Value) -> Page {
    let mut items = Vec::new();
    collect_rows(root, &mut items);
    Page {
        items,
        next: continuation_token(root),
    }
}

/// Every `musicResponsiveListItemRenderer` in a document, in order.
///
/// A playlist's rows live under `musicPlaylistShelfRenderer` on the first page and under a
/// continuation container after that, and the page uses the two-column layout rather than the
/// single-column one — so this scans rather than following a path.
fn collect_rows(v: &Value, out: &mut Vec<Entry>) {
    match v {
        Value::Object(map) => {
            if let Some(row) = map.get("musicResponsiveListItemRenderer") {
                if let Some(e) = parse_search_row(row) {
                    out.push(e);
                }
                return;
            }
            for value in map.values() {
                collect_rows(value, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|i| collect_rows(i, out)),
        _ => {}
    }
}

/// The first value under `key`, wherever it is nested.
fn find_key<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match v {
        Value::Object(map) => {
            if let Some(found) = map.get(key) {
                return Some(found);
            }
            map.values().find_map(|value| find_key(value, key))
        }
        Value::Array(items) => items.iter().find_map(|i| find_key(i, key)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// Watch queue
// ---------------------------------------------------------------------------------------------

/// A radio queue: its tracks, and how to ask for more.
///
/// A dedicated type rather than [`Page`]: a queue is tracks, not mixed entries, and its cursor
/// is a `nextRadioContinuationData` token rather than a browse continuation.
#[derive(Debug, Default)]
pub struct WatchQueue {
    pub tracks: Vec<Track>,
    /// Follow it with `next` for ~50 more tracks. `None` ends the river.
    pub next: Option<String>,
    /// The now-playing song's own rating, from the player overlay. Rows carry no like button,
    /// so this is the only way a mix knows whether its seed is liked. `None` on continuations.
    pub like: Option<LikeStatus>,
}

/// The `next` endpoint's watch queue, out of
/// `…/musicQueueRenderer/content/playlistPanelRenderer/contents[]/playlistPanelVideoRenderer`.
///
/// The queue is found by key rather than by tab index: the Up-next tab moves between layouts. A
/// follow-up page arrives under `continuationContents/playlistPanelContinuation` instead — same
/// panel, different envelope — so both are tried. The continuation is read explicitly from the
/// panel: radio's `nextRadioContinuationData` is a third token shape beside the two
/// [`continuation_token`] walks for, and conflating them would surprise the browse flows that
/// share that walker.
pub fn parse_watch_queue(root: &Value) -> WatchQueue {
    let Some(panel) = find_key(root, "musicQueueRenderer")
        .and_then(|q| q.get("content"))
        .and_then(|c| c.get("playlistPanelRenderer"))
        .or_else(|| root.pointer("/continuationContents/playlistPanelContinuation"))
    else {
        return WatchQueue::default();
    };
    let tracks = panel
        .get("contents")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(queue_row)
                .filter_map(parse_watch_row)
                .collect()
        })
        .unwrap_or_default();
    let next = panel
        .pointer("/continuations/0/nextRadioContinuationData/continuation")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .map(str::to_owned);
    WatchQueue {
        tracks,
        next,
        like: playback_like(root),
    }
}

/// One queue entry as a row: either a bare `playlistPanelVideoRenderer`, or one wrapped in a
/// `playlistPanelVideoWrapperRenderer` whose `primaryRenderer` holds the row.
///
/// The wrapper carries the same song twice — `primaryRenderer` plus `counterpart[]` alternate
/// versions — and only the primary is a queue entry. Signed-out responses seen so far are all
/// bare rows; signed-in ones wrap most of them. Reading only the bare shape is how a 50-track
/// river parsed to 11.
fn queue_row(entry: &Value) -> Option<&Value> {
    entry.get("playlistPanelVideoRenderer").or_else(|| {
        entry.pointer(
            "/playlistPanelVideoWrapperRenderer/primaryRenderer/playlistPanelVideoRenderer",
        )
    })
}

/// One `playlistPanelVideoRenderer`: title, `lengthText` duration, thumbnail, badges, and the
/// artist/album out of `longBylineText`'s runs.
///
/// No flex columns here, so the runs go straight to [`segments_from_runs`] — `linked_artists`,
/// `linked_album`, `album_id_of` and `year_segment` all apply unchanged. Radio rows carry
/// artist • album • year and no play count, so `plays` stays `None`.
fn parse_watch_row(row: &Value) -> Option<Track> {
    let target = row_target(row);
    if let Some(Target::Watch { kind, .. }) = &target
        && kind.as_str() != VIDEO_ATV
        && !VIDEO_KINDS.contains(&kind.as_str())
    {
        return None;
    }
    // A non-album-audio kind is a music video, and its still is 16:9.
    let wide = matches!(&target, Some(Target::Watch { kind, .. }) if kind.as_str() != VIDEO_ATV);
    let (rank, trend) = rank_of(row);
    let video_id = row
        .get("videoId")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())?
        .to_owned();
    let title = non_empty(text(row.get("title").unwrap_or(&Value::Null)))?;
    let segs = row
        .get("longBylineText")
        .and_then(|t| t.get("runs"))
        .and_then(Value::as_array)
        .map(|runs| segments_from_runs(runs, 1))
        .unwrap_or_default();
    let album = linked_album(&segs);
    let toggle = library_toggle(row);
    Some(Track {
        video_id,
        title,
        artists: artist_links(&segs),
        album: album.map(|s| s.text.clone()),
        album_id: album.and_then(album_id_of).or_else(|| menu_album_id(row)),
        duration: row
            .get("lengthText")
            .map(text)
            .and_then(|t| crate::model::parse_duration(&t)),
        plays: None,
        explicit: is_explicit(row),
        like: like_status(row),
        in_library: toggle.as_ref().map(|t| t.in_library).unwrap_or(false),
        library_add: toggle.as_ref().and_then(|t| t.add.clone()),
        library_remove: toggle.as_ref().and_then(|t| t.remove.clone()),
        set_video_id: None,
        thumbnail: thumbnail(row),
        // From the row's own watch kind, matched above.
        wide,
        rank,
        trend,
        local_path: None,
    })
}

/// Whether a browse response is YouTube's signed-out prompt rather than real content.
///
/// A `messageRenderer` where a shelf belongs is what an anonymous request gets — the app used to
/// surface the resulting missing-key error as "could not load your songs: Key /contents/…".
pub fn looks_signed_out(root: &Value) -> bool {
    find_section_list(root).is_some_and(|sections| {
        sections.iter().any(|s| {
            s.pointer("/itemSectionRenderer/contents/0/messageRenderer")
                .is_some()
        })
    })
}

/// The channels an account can browse as, from an `account/accounts_list` response.
///
/// The selected entry is the one in use; an entry without a page id is the personal channel.
pub fn parse_accounts(root: &Value) -> Vec<crate::model::Account> {
    let mut out = Vec::new();
    fn walk(v: &Value, out: &mut Vec<crate::model::Account>) {
        match v {
            Value::Object(map) => {
                if let Some(item) = map.get("accountItem") {
                    if let Some(a) = parse_account(item) {
                        out.push(a);
                    }
                    return;
                }
                for value in map.values() {
                    walk(value, out);
                }
            }
            Value::Array(items) => items.iter().for_each(|i| walk(i, out)),
            _ => {}
        }
    }
    walk(root, &mut out);
    out
}

fn parse_account(item: &Value) -> Option<crate::model::Account> {
    let name = non_empty(text(item.get("accountName")?))?;
    let page_id = find_key(item, "pageId")
        .and_then(Value::as_str)
        .map(str::to_owned);
    // The channel's `UC…` id. The account list carries it as an offline-cache key *without* the
    // `UC` prefix — the same id the library cards link with. A brand account's numeric page id
    // is not a channel id, so it is only used when it already looks like one.
    let channel_id = match find_key(item, "offlineCacheKeyToken")
        .and_then(|t| t.get("clientCacheKey"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        Some(key) => Some(format!("UC{key}")),
        None => page_id.clone().filter(|p| p.starts_with("UC")),
    };
    Some(crate::model::Account {
        name,
        detail: item.get("accountByline").map(text).unwrap_or_default(),
        page_id,
        channel_id,
        handle: non_empty(text(item.get("channelHandle").unwrap_or(&Value::Null))),
        selected: item
            .get("isSelected")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        thumbnail: thumbnail(item),
    })
}

/// Where a play reports itself, from a `player` response's `playbackTracking`.
///
/// Both URLs arrive with the session baked in. Either one missing means the response is not a
/// playable shape (an age gate, a rental, an error page wearing a 200) — there is nothing to
/// beacon to, so this is `None` rather than half a tracker.
pub fn parse_playback_tracking(root: &Value) -> Option<crate::model::PlaybackTracking> {
    let tracking = root.get("playbackTracking")?;
    let url = |key: &str| {
        tracking
            .pointer(&format!("/{key}/baseUrl"))
            .and_then(Value::as_str)
            .filter(|u| !u.is_empty())
            .map(str::to_owned)
    };
    Some(crate::model::PlaybackTracking {
        playback_url: url("videostatsPlaybackUrl")?,
        watchtime_base: url("videostatsWatchtimeUrl")?,
    })
}

/// The signed-in account's name from an `account/account_menu` response.
///
/// Absent for an anonymous request even when the cookie was accepted locally, which makes it the
/// discriminator sign-in needs.
pub fn account_name(root: &Value) -> Option<String> {
    fn walk(v: &Value) -> Option<String> {
        match v {
            Value::Object(map) => {
                if let Some(name) = map.get("accountName") {
                    let name = text(name);
                    if !name.trim().is_empty() {
                        return Some(name);
                    }
                }
                map.values().find_map(walk)
            }
            Value::Array(items) => items.iter().find_map(walk),
            _ => None,
        }
    }
    walk(root)
}

// ---------------------------------------------------------------------------------------------
// Artist page
// ---------------------------------------------------------------------------------------------

/// The playlists behind one mood/genre chip.
///
/// `ytmapi_rs::get_mood_playlists`'s typed parser only recognises exactly two shapes at
/// `sectionListRenderer/contents/N` — a bare `gridRenderer` or a `musicCarouselShelfRenderer`,
/// each of `musicTwoRowItemRenderer`s — and throws the moment a section is neither, which is
/// fatal to the whole page rather than costing just that one section. This walks every section
/// tolerantly instead: a titled carousel reuses [`parse_carousel`] (and so shares its card
/// parsing, its "more" handling, everything), a bare grid falls through to [`parse_card`]
/// directly, and anything else is simply skipped rather than failing the page. `Api::mood_playlists`
/// only falls back to the typed query when this finds nothing at all.
pub fn parse_mood_playlists(root: &Value) -> Option<MoodPlaylists> {
    let sections = root
        .pointer(
            "/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content\
             /sectionListRenderer/contents",
        )?
        .as_array()?;

    let mut title = None;
    let mut playlists = Vec::new();
    let mut as_playlist = |items: Vec<Entry>| {
        playlists.extend(items.into_iter().filter_map(|e| match e {
            Entry::Playlist(p) => Some(p),
            _ => None,
        }));
    };
    for section in sections {
        if let Some(carousel) = section.get("musicCarouselShelfRenderer") {
            if let Some(shelf) = parse_carousel(carousel) {
                title.get_or_insert_with(|| shelf.title.clone());
                as_playlist(shelf.items);
            }
            continue;
        }
        // The typed parser's other supported shape, kept here too so this stays at least as
        // tolerant: a plain grid, with no carousel/header/"more" button of its own.
        if let Some(items) = section
            .pointer("/gridRenderer/items")
            .and_then(Value::as_array)
        {
            as_playlist(items.iter().filter_map(parse_card).collect());
        }
    }

    (!playlists.is_empty()).then(|| MoodPlaylists {
        title: title.unwrap_or_else(|| "Playlists".to_owned()),
        playlists,
    })
}

/// Turn an artist browse response into an [`Artist`].
///
/// The reason this exists rather than `YtMusic::get_artist`: the "Top songs" shelf gives every row
/// its own square album cover, and `ytmapi_rs::parse::ArtistSong` has no field to put it in. The
/// app was showing the artist's photograph on every track as a result.
///
/// Returns `None` when the response has no name — `api.rs` then falls back to the typed query.
pub fn parse_artist(root: &Value) -> Option<Artist> {
    // A channel page browses like an artist page but renders a visual header instead of an
    // immersive one: a title, a thumbnail and carousel shelves, with no monthly audience and
    // no Shuffle/Mix buttons. The header shows what is there — every field below is optional
    // except the name, so a channel simply arrives with `None`s where an artist has values.
    let header = root
        .pointer("/header/musicImmersiveHeaderRenderer")
        .or_else(|| root.pointer("/header/musicVisualHeaderRenderer"))?;
    let name = non_empty(text(header.get("title")?))?;

    let subscribe = header.pointer("/subscriptionButton/subscribeButtonRenderer");
    let id = subscribe
        .and_then(|s| s.get("channelId"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    // `subscriberCountText` is the bare "20.8M"; `longSubscriberCountText` is "20.8M subscribers",
    // localised by YouTube. Both are kept: the short one is what fits on the Subscribe button,
    // and the long one is the sentence the header shows in place of a monthly audience when the
    // artist has none. Before, only the short one survived the fallback chain, and a small
    // artist — who has no monthly-listener line — showed a bare "965" under the name.
    let subscribers = subscribe
        .and_then(|s| s.get("subscriberCountText"))
        .map(text)
        .and_then(non_empty);
    let subscribers_long = subscribe
        .and_then(|s| s.get("longSubscriberCountText"))
        .map(text)
        .and_then(non_empty);
    // Already reads "213M monthly audience" — this is the line the page shows under the name.
    let monthly_listeners = header
        .get("monthlyListenerCount")
        .map(text)
        .and_then(non_empty);
    // Shuffle and Mix are buttons on the header rather than menu items, so they are read
    // directly; the subscribe state is the button's own boolean.
    let actions = crate::model::ArtistActions {
        shuffle: header.get("playButton").and_then(watch_target),
        mix: header.get("startRadioButton").and_then(watch_target),
        subscribed: subscribe
            .and_then(|s| s.get("subscribed"))
            .and_then(Value::as_bool),
        dont_recommend: None,
    };

    let sections = find_section_list(root);

    let mut top_songs = Vec::new();
    let mut top_songs_playlist = None;
    let mut shelves = Vec::new();
    let mut description = None;
    let mut views = None;

    for section in sections.into_iter().flatten() {
        if let Some(shelf) = section.get("musicShelfRenderer") {
            top_songs.extend(parse_artist_songs(shelf, &name, &id));
            // The shelf's own "Show all" target: YouTube embeds only 5 songs here, but this
            // browseId is the artist's full auto-generated top-songs playlist.
            top_songs_playlist = top_songs_playlist.or_else(|| {
                shelf
                    .pointer("/bottomEndpoint/browseEndpoint/browseId")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            });
        }
        if let Some(carousel) = section.get("musicCarouselShelfRenderer")
            && let Some(shelf) = parse_carousel(carousel)
        {
            shelves.push(shelf);
        }
        if let Some(about) = section.get("musicDescriptionShelfRenderer") {
            description = description.or_else(|| {
                about
                    .get("description")
                    .map(text)
                    .and_then(non_empty)
                    .map(|d| d.trim().to_owned())
            });
            // Already reads "12,419,399,960 views" — the word is part of the value, which is why
            // appending " views" in the UI printed it twice.
            views = views.or_else(|| about.get("subheader").map(text).and_then(non_empty));
        }
    }

    Some(Artist {
        id,
        name,
        subscribers,
        subscribers_long,
        monthly_listeners,
        views,
        actions,
        description,
        thumbnail: thumbnail(header),
        top_songs,
        top_songs_playlist,
        shelves,
    })
}

/// The "Top songs" shelf. Each row carries its own cover and, in the fourth column, its album.
fn parse_artist_songs(shelf: &Value, artist_name: &str, artist_id: &str) -> Vec<Track> {
    let Some(rows) = shelf.get("contents").and_then(Value::as_array) else {
        return Vec::new();
    };
    rows.iter()
        .filter_map(|row| {
            let row = row.get("musicResponsiveListItemRenderer")?;
            let cols = flex_columns(row);
            let title = non_empty(cols.first()?.clone())?;
            let Target::Watch { video_id, kind } = row_target(row)? else {
                return None;
            };
            let (rank, trend) = rank_of(row);
            let segs = row_segments(row);
            let toggle = library_toggle(row);
            Some(Track {
                video_id,
                title,
                // Falls back to the page's own artist: a top-songs row often credits only
                // the album, and the page it is on is the answer.
                artists: match artist_links(&segs) {
                    v if !v.is_empty() => v,
                    _ => vec![crate::model::ArtistLink {
                        name: artist_name.to_owned(),
                        id: (!artist_id.is_empty()).then(|| artist_id.to_owned()),
                    }],
                },
                plays: plays_segment(&segs),
                album: linked_album(&segs).map(|s| s.text.clone()),
                album_id: linked_album(&segs)
                    .and_then(album_id_of)
                    .or_else(|| menu_album_id(row)),
                duration: fixed_duration(row).or_else(|| duration_segment(&segs)),
                explicit: is_explicit(row),
                like: like_status(row),
                in_library: toggle.as_ref().map(|t| t.in_library).unwrap_or(false),
                library_add: toggle.as_ref().and_then(|t| t.add.clone()),
                library_remove: toggle.as_ref().and_then(|t| t.remove.clone()),
                set_video_id: None,
                // The whole reason this parser exists.
                thumbnail: thumbnail(row),
                wide: kind != VIDEO_ATV,
                rank,
                trend,
                local_path: None,
            })
        })
        .collect()
}

/// One titled carousel — "Albums", "Videos", "Fans might also like", "New releases" — as a
/// [`Shelf`].
///
/// Accepts cards of every target type rather than only albums. The old version kept albums and
/// threw the rest away, which is how the artist page lost its videos, playlists and related
/// artists, and how albums and singles ended up in one undifferentiated grid.
pub fn parse_carousel(carousel: &Value) -> Option<Shelf> {
    let header = carousel.pointer("/header/musicCarouselShelfBasicHeaderRenderer");
    let title = header
        .and_then(|h| h.get("title"))
        .map(text)
        .and_then(non_empty)?;
    let more_params = header
        .and_then(|h| {
            h.pointer("/moreContentButton/buttonRenderer/navigationEndpoint/browseEndpoint/params")
        })
        .and_then(Value::as_str)
        .map(str::to_owned);
    // The sibling of `params` above: a shelf's own "more" button routinely points somewhere
    // other than the artist's channel id (a discography page, even a playlist id for
    // "Videos"). Sending the artist's id with a shelf's `params` — the bug this fixes — asked
    // YouTube for an unpaired (browseId, params) combination, which fell back to a broader
    // page and returned everything, not just this shelf.
    let more_browse_id = header
        .and_then(|h| {
            h.pointer(
                "/moreContentButton/buttonRenderer/navigationEndpoint/browseEndpoint/browseId",
            )
        })
        .and_then(Value::as_str)
        .map(str::to_owned);

    let items: Vec<Entry> = carousel
        .get("contents")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(parse_card).collect())
        .unwrap_or_default();

    (!items.is_empty()).then_some(Shelf {
        title,
        items,
        more_params,
        more_browse_id,
        expanded: false,
    })
}

/// One carousel item: a card, or a "Moods & genres" chip.
/// The kinds a playlist card leads with when it names one; a library card does not — it leads
/// with its owner (`"IV • 13 tracks"`).
const PLAYLIST_KINDS: [&str; 2] = ["Playlist", "Auto playlist"];

/// Split a playlist card's subtitle into its owner and its count.
///
/// A browse card leads with the kind (`"Playlist • YouTube Music"`), a library card with its
/// owner (`"IV • 13 tracks"`) — and a card with no owner at all leads with the count
/// (`"Playlist • 50 tracks"`). English shapes, like the rest of this parser: a localised count
/// lands in the owner slot, which the rail then shows as a name only for someone else's list.
fn playlist_card_detail(fields: &[&str]) -> (String, Option<String>) {
    let rest = match fields.first() {
        Some(first) if PLAYLIST_KINDS.iter().any(|k| k.eq_ignore_ascii_case(first)) => &fields[1..],
        _ => fields,
    };
    let mut author = None;
    let mut count = None;
    for field in rest {
        let lower = field.to_lowercase();
        let is_count = (lower.contains("track") || lower.contains("song"))
            && field.chars().any(|c| c.is_ascii_digit());
        if is_count {
            count.get_or_insert_with(|| (*field).to_owned());
        } else {
            author.get_or_insert_with(|| (*field).to_owned());
        }
    }
    (author.unwrap_or_default(), count)
}

fn parse_card(item: &Value) -> Option<Entry> {
    // Mood chips are a different renderer entirely, and are the whole content of the
    // "Moods & genres" shelf — dropping them would silently empty it.
    if let Some(chip) = item.get("musicNavigationButtonRenderer") {
        let title = non_empty(text(chip.get("buttonText")?))?;
        let params = chip
            .pointer("/clickCommand/browseEndpoint/params")
            .and_then(Value::as_str)?
            .to_owned();
        // The chip's stripe: ARGB on the wire, RGB in the model.
        let colour = chip
            .pointer("/solid/leftStripeColor")
            .and_then(Value::as_u64)
            .map(|argb| {
                [
                    ((argb >> 16) & 0xFF) as u8,
                    ((argb >> 8) & 0xFF) as u8,
                    (argb & 0xFF) as u8,
                ]
            });
        return Some(Entry::Mood(MoodRef {
            title,
            params,
            colour,
        }));
    }

    let card = item.get("musicTwoRowItemRenderer")?;
    let title = non_empty(text(card.get("title")?))?;
    let subtitle = text(card.get("subtitle").unwrap_or(&Value::Null));
    let thumb = thumbnail(card);
    // The subtitle's *runs*, not its flattened text: a card credits its artists with real browse
    // endpoints, and `text()` throws them away before anything can see them. This is why a card's
    // artist could never be clicked.
    let segs = card
        .pointer("/subtitle/runs")
        .and_then(Value::as_array)
        .map(|runs| segments_from_runs(runs, 1))
        .unwrap_or_default();

    // Cards state their own type in the subtitle ("Single • 2025", "Playlist • YouTube Music"),
    // but the endpoint is what we trust — the subtitle is localised and sometimes absent.
    let fields: Vec<&str> = subtitle.split('•').map(str::trim).collect();
    let year = fields
        .iter()
        .find(|f| f.len() == 4 && f.chars().all(|c| c.is_ascii_digit()))
        .map(|f| (*f).to_owned());
    let (author, track_count) = playlist_card_detail(&fields);

    match card.get("navigationEndpoint").and_then(endpoint_target)? {
        Target::Browse { id, page } if page == PAGE_ALBUM => Some(Entry::Album(AlbumRef {
            id,
            title,
            artist: String::new(),
            year,
            // "Album", "Single" or "EP" as YouTube labels it, which is what lets a caller split
            // a full discography by type.
            kind: fields
                .first()
                .filter(|k| matches!(**k, "Album" | "Single" | "EP"))
                .map(|k| (*k).to_owned()),
            thumbnail: thumb,
        })),
        Target::Browse { id, page } if page == PAGE_ARTIST => Some(Entry::Artist(ArtistRef {
            id,
            name: title,
            subscribers: non_empty(subtitle),
            thumbnail: thumb,
            actions: artist_actions(card),
        })),
        Target::Browse { id, page } if page == PAGE_PLAYLIST => {
            Some(Entry::Playlist(PlaylistRef {
                id,
                title,
                author,
                author_id: artist_links(&segs).into_iter().find_map(|l| l.id),
                track_count,
                thumbnail: thumb,
                editable: card_is_editable(card),
                source: crate::model::PlaylistSource::YouTube,
            }))
        }
        Target::Browse { .. } => None,
        Target::Watch { video_id, kind } => {
            let is_song = kind == VIDEO_ATV;
            if !is_song && !VIDEO_KINDS.contains(&kind.as_str()) {
                return None;
            }
            let track = Track {
                video_id,
                title,
                artists: match artist_links(&segs) {
                    v if !v.is_empty() => v,
                    _ => fields
                        .first()
                        .map(|f| vec![crate::model::ArtistLink::plain(*f)])
                        .unwrap_or_default(),
                },
                album: linked_album(&segs).map(|s| s.text.clone()),
                album_id: linked_album(&segs)
                    .and_then(album_id_of)
                    .or_else(|| menu_album_id(card)),
                duration: None,
                plays: fields.get(1).map(|f| (*f).to_owned()),
                explicit: false,
                like: LikeStatus::Indifferent,
                // Cards carry no menu, so no rating and no toggle.
                in_library: false,
                library_add: None,
                library_remove: None,
                set_video_id: None,
                thumbnail: thumb,
                // A non-song kind is a music video, and its still is 16:9.
                wide: !is_song,
                // Cards carry no rank column.
                rank: None,
                trend: crate::model::ChartTrend::Unknown,
                local_path: None,
            };
            Some(if is_song {
                Entry::Song(track)
            } else {
                Entry::Video(track)
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(name: &str) -> Value {
        let raw = match name {
            // Captured with `hl`/`gl` set: song subtitles read "Song • Nemzzz".
            "flat" => include_str!("../fixtures/search-flat-travis.json"),
            // Captured with the context `ytmapi-rs` actually sends: song subtitles read
            // "Nemzzz • GASS • 1:54" — artist, album, duration, and no type word. Both are live
            // responses to the same query; the parser must not care which arrives.
            "flat-app" => include_str!("../fixtures/search-flat-travis-app.json"),
            "shelf" => include_str!("../fixtures/search-shelf-travis-songs.json"),
            "artist" => include_str!("../fixtures/artist-travis-scott.json"),
            "home" => include_str!("../fixtures/home-anonymous.json"),
            "explore" => include_str!("../fixtures/explore-anonymous.json"),
            // Anonymous radio for Radiohead's "Creep": 50 `playlistPanelVideoRenderer` rows, one
            // `nextRadioContinuationData` token, `isInfinite`. Tracking params stripped; row menus
            // kept, the song-menu milestone will read them.
            "watch" => include_str!("../fixtures/watch-radiohead-creep.json"),
            // The same river signed in: most rows arrive wrapped in
            // `playlistPanelVideoWrapperRenderer`, each carrying `counterpart[]` alternate
            // versions of the same song. Only the primary is a queue entry.
            "wrapped" => include_str!("../fixtures/watch-wrapped-radio.json"),
            // A follow-up page: `continuationContents/playlistPanelContinuation`, same panel
            // under a different envelope, with its own next token.
            "watch-more" => include_str!("../fixtures/watch-continuation.json"),
            // A `player` response, trimmed to the envelope that matters: `playabilityStatus`,
            // `videoDetails`, and the whole of `playbackTracking`. The beacon URLs arrive with
            // the session baked in, so the fixture keeps them verbatim.
            "player" => include_str!("../fixtures/player-creep.json"),
            // "Chill", from Explore's Moods & genres, anonymous — trimmed to its first two
            // `musicCarouselShelfRenderer` sections, three cards each. Confirms the current
            // shape (a titled carousel of `musicTwoRowItemRenderer`s) that
            // `parse_mood_playlists` walks instead of `ytmapi-rs`'s own typed parser, which
            // throws on some other mood/genre chip's section shape.
            "mood-playlists" => include_str!("../fixtures/mood-playlists-chill.json"),
            other => panic!("no fixture {other}"),
        };
        serde_json::from_str(raw).expect("fixture is valid json")
    }

    /// The two live layouts of the same query, so every search test runs against both.
    const FLAT: [&str; 2] = ["flat", "flat-app"];

    // -- search ------------------------------------------------------------------------------

    /// The bug this milestone exists for. Searching `travis` must lead with Travis Scott, because
    /// that is who YouTube ranks first — not the Scottish band, which is what an exact-name match
    /// picked.
    #[test]
    fn travis_means_travis_scott() {
        for fixture in FLAT {
            let r = parse_search(&json(fixture));
            let first = r.items.first().expect("at least one result");
            assert!(
                matches!(first, Entry::Artist(a) if a.name == "Travis Scott"),
                "{fixture}: expected Travis Scott first, got {:?}",
                first.title()
            );
        }
    }

    /// The top result must appear once. It used to be drawn as an oversized card *and* as the
    /// first row, because the card's contents repeat rows that follow it.
    #[test]
    fn no_result_is_listed_twice() {
        for fixture in FLAT {
            let items = parse_search(&json(fixture)).items;
            let mut seen = std::collections::HashSet::new();
            for item in &items {
                let key = match item {
                    Entry::Song(t) | Entry::Video(t) => t.video_id.clone(),
                    Entry::Artist(a) => a.id.clone(),
                    Entry::Album(a) => a.id.clone(),
                    Entry::Playlist(p) => p.id.clone(),
                    Entry::Mood(m) => m.params.clone(),
                };
                assert!(
                    seen.insert(key),
                    "{fixture}: {:?} appears twice",
                    item.title()
                );
            }
        }
    }

    /// A minimal card-plus-rows search response — real fixtures are the raw shape YouTube
    /// actually sent, but this one exists specifically to isolate `parse_search_card` itself.
    fn search_with_card(card_id: &str, row_id: &str) -> Value {
        serde_json::json!({"contents": {"tabbedSearchResultsRenderer": {"tabs": [{"tabRenderer": {"content": {"sectionListRenderer": {"contents": [
            {"musicCardShelfRenderer": {
                "title": {"runs": [{
                    "text": "Solo Artist",
                    "navigationEndpoint": {"browseEndpoint": {
                        "browseId": card_id,
                        "browseEndpointContextSupportedConfigs": {"browseEndpointContextMusicConfig": {"pageType": "MUSIC_PAGE_TYPE_ARTIST"}}
                    }}
                }]},
                "subtitle": {"runs": [{"text": "Artist"}, {"text": " • "}, {"text": "1M monthly audience"}]}
            }},
            {"itemSectionRenderer": {"contents": [
                {"musicResponsiveListItemRenderer": {
                    "flexColumns": [
                        {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [{
                            "text": "Other Artist",
                            "navigationEndpoint": {"browseEndpoint": {
                                "browseId": row_id,
                                "browseEndpointContextSupportedConfigs": {"browseEndpointContextMusicConfig": {"pageType": "MUSIC_PAGE_TYPE_ARTIST"}}
                            }}
                        }]}}},
                        {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [{"text": "2M monthly audience"}]}}}
                    ]
                }}
            ]}}
        ]}}}}]}}})
    }

    /// The original `travis` shape, isolated: the ranked list's own leading row is *also* an
    /// artist, a different one from the card. That is a real disagreement about who belongs at
    /// the front, so the ranking wins — the card trails instead of leading, but (unlike the
    /// version of this fix that shipped first) is still findable rather than dropped outright.
    #[test]
    fn a_card_artist_beaten_to_the_lead_by_a_different_artist_trails_instead() {
        let root = search_with_card("UCsoloartist", "UCotherartist");
        let results = parse_search(&root);
        let ids: Vec<&str> = results.items.iter().map(entry_key).collect();
        assert_eq!(
            ids,
            ["UCotherartist", "UCsoloartist"],
            "the card's own artist must trail the ranked row, never lead — leading it is \
             the `travis` bug this already fixed once"
        );
    }

    /// When the card's artist is *not* missing — it already ranked into the list under its own
    /// steam — appending it again would show it twice.
    #[test]
    fn a_card_artist_already_ranked_is_not_duplicated() {
        let root = search_with_card("UCsameartist", "UCsameartist");
        let results = parse_search(&root);
        let ids: Vec<&str> = results.items.iter().map(entry_key).collect();
        assert_eq!(ids, ["UCsameartist"]);
    }

    /// The bug report this fixes: searching an artist's exact name (`the weeknd`, live) buried
    /// them at the *end* of their own search results, because appending unconditionally was too
    /// timid — nothing here is actually competing for the lead position, so there is nothing for
    /// the card to lose to. Unlike the artist-vs-artist case above, the ranked list's own leading
    /// row is a song: no vote cast for who the top *artist* is, so the card — YouTube's own
    /// highlighted pick — leads.
    #[test]
    fn a_card_artist_with_nothing_competing_for_the_lead_is_promoted() {
        let root = serde_json::json!({"contents": {"tabbedSearchResultsRenderer": {"tabs": [{"tabRenderer": {"content": {"sectionListRenderer": {"contents": [
            {"musicCardShelfRenderer": {
                "title": {"runs": [{
                    "text": "Solo Artist",
                    "navigationEndpoint": {"browseEndpoint": {
                        "browseId": "UCsoloartist",
                        "browseEndpointContextSupportedConfigs": {"browseEndpointContextMusicConfig": {"pageType": "MUSIC_PAGE_TYPE_ARTIST"}}
                    }}
                }]},
                "subtitle": {"runs": [{"text": "Artist"}, {"text": " • "}, {"text": "1M monthly audience"}]}
            }},
            {"itemSectionRenderer": {"contents": [
                {"musicResponsiveListItemRenderer": {
                    "flexColumns": [
                        {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [{
                            "text": "Some Song",
                            "navigationEndpoint": {"watchEndpoint": {
                                "videoId": "vid123",
                                "watchEndpointMusicSupportedConfigs": {"watchEndpointMusicConfig": {"musicVideoType": "MUSIC_VIDEO_TYPE_ATV"}}
                            }}
                        }]}}}
                    ]
                }}
            ]}}
        ]}}}}]}}});

        let results = parse_search(&root);
        let ids: Vec<&str> = results.items.iter().map(entry_key).collect();
        assert_eq!(
            ids,
            ["UCsoloartist", "vid123"],
            "nothing in the ranked list disagrees about who the top artist is, so the card \
             should lead rather than trail"
        );
    }

    /// The regression that positional subtitle parsing caused: a song row showed
    /// "GASS • 1:54" — its album and duration — where its artist belongs.
    #[test]
    fn a_song_row_shows_its_artist_not_its_album() {
        for fixture in FLAT {
            let items = parse_search(&json(fixture)).items;
            let sicko = items
                .iter()
                .find_map(|i| match i {
                    Entry::Song(t) if t.title == "SICKO MODE" => Some(t),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{fixture}: no SICKO MODE"));
            assert_eq!(sicko.artist_line(), "Travis Scott", "{fixture}");
            assert_ne!(
                sicko.artist_line(),
                "ASTROWORLD",
                "{fixture}: that is the album"
            );
        }
    }

    /// The layout `ytmapi-rs` receives carries durations; the other does not. Taking them from the
    /// field that parses as `m:ss` rather than from a fixed position is what makes both work.
    #[test]
    fn durations_and_albums_are_read_where_the_layout_has_them() {
        let items = parse_search(&json("flat-app")).items;
        let sicko = items
            .iter()
            .find_map(|i| match i {
                Entry::Song(t) if t.title == "SICKO MODE" => Some(t),
                _ => None,
            })
            .expect("SICKO MODE");
        assert_eq!(sicko.duration, Some(313), "5:13");
        assert_eq!(sicko.album.as_deref(), Some("ASTROWORLD"));
    }

    /// The album name in a row is only clickable if we kept its browse id.
    #[test]
    fn a_song_row_carries_its_album_id() {
        let items = parse_search(&json("flat-app")).items;
        let sicko = items
            .iter()
            .find_map(|i| match i {
                Entry::Song(t) if t.title == "SICKO MODE" => Some(t),
                _ => None,
            })
            .expect("SICKO MODE");
        assert!(
            sicko.album_id.as_deref().is_some_and(|id| !id.is_empty()),
            "no album id, so the album name would be a dead link"
        );
        assert!(sicko.primary_artist_id().is_some(), "no artist id");
    }

    /// A four-digit year must never be mistaken for a duration, or an album row would claim to be
    /// 19 hours long.
    #[test]
    fn a_year_is_not_a_duration() {
        for fixture in FLAT {
            let items = parse_search(&json(fixture)).items;
            let album = items
                .iter()
                .find_map(|i| match i {
                    Entry::Album(a) if a.title == "The Man Who" => Some(a),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{fixture}: no The Man Who"));
            assert_eq!(album.year.as_deref(), Some("1999"), "{fixture}");
            assert_eq!(album.artist, "Travis", "{fixture}");
        }
    }

    /// The flat `itemSectionRenderer` layout that every client in this ecosystem reads as
    /// "no results".
    #[test]
    fn the_flat_layout_yields_the_whole_ranked_list() {
        for fixture in FLAT {
            let r = parse_search(&json(fixture));
            assert!(
                r.items.len() >= 18,
                "{fixture}: only {} items parsed — the flat layout regressed",
                r.items.len()
            );
            let kinds: Vec<&str> = r.items.iter().map(Entry::kind).collect();
            for want in ["ARTIST", "SONG", "ALBUM"] {
                assert!(kinds.contains(&want), "{fixture}: no {want} in {kinds:?}");
            }
        }
    }

    /// The old `musicShelfRenderer` shape still parses, so a flip back is not an outage.
    #[test]
    fn the_shelf_layout_still_parses() {
        let r = parse_search(&json("shelf"));
        assert!(!r.items.is_empty(), "the shelf fallback parsed nothing");
        assert!(
            r.items.iter().all(|i| i.track().is_some()),
            "a songs-filtered search should be all songs"
        );
    }

    /// Rows we cannot open must not be shown at all — the fixture contains podcasts, episodes and
    /// user profiles.
    #[test]
    fn unsupported_kinds_are_dropped() {
        for fixture in FLAT {
            let r = parse_search(&json(fixture));
            for item in &r.items {
                let title = item.title().to_lowercase();
                assert!(
                    !title.contains("kalanick") && !title.contains("makes money"),
                    "a podcast/episode leaked through: {title:?}"
                );
            }
            assert!(
                !r.items
                    .iter()
                    .any(|i| matches!(i, Entry::Artist(a) if a.id.is_empty())),
                "a profile was parsed as an artist"
            );
        }
    }

    /// Every row must be openable and must carry its own art.
    #[test]
    fn every_result_has_an_id_and_a_cover() {
        for fixture in FLAT {
            for item in parse_search(&json(fixture)).items {
                let label = item.title().to_owned();
                assert!(
                    item.thumbnail().is_some(),
                    "{label:?} has no thumbnail — rows would fall back to a placeholder"
                );
                match &item {
                    Entry::Song(t) | Entry::Video(t) => {
                        assert!(!t.video_id.is_empty(), "{label:?} has no video id")
                    }
                    Entry::Artist(a) => {
                        assert!(!a.id.is_empty(), "{label:?} has no browse id")
                    }
                    Entry::Album(a) => assert!(!a.id.is_empty(), "{label:?} has no browse id"),
                    Entry::Playlist(p) => {
                        assert!(!p.id.is_empty(), "{label:?} has no browse id")
                    }
                    Entry::Mood(_) => panic!("{label:?}: a mood chip in search results"),
                }
            }
        }
    }

    /// Nothing may depend on the English type word, which changes with `hl`.
    #[test]
    fn typing_does_not_depend_on_the_subtitle_language() {
        let mut doc = json("flat");
        // Replace every subtitle run text with a non-English stand-in.
        fn scrub(v: &mut Value) {
            match v {
                Value::Object(map) => {
                    for (k, val) in map.iter_mut() {
                        if k == "text"
                            && let Value::String(s) = val
                            && (s.starts_with("Song")
                                || s.starts_with("Album")
                                || s.starts_with("Artist")
                                || s.starts_with("Playlist"))
                        {
                            *s = s.replacen(
                                s.split(' ').next().unwrap_or_default(),
                                "Καλλιτέχνης",
                                1,
                            );
                        }
                        scrub(val);
                    }
                }
                Value::Array(a) => a.iter_mut().for_each(scrub),
                _ => {}
            }
        }
        scrub(&mut doc);
        let r = parse_search(&doc);
        assert!(
            matches!(r.items.first(), Some(Entry::Artist(a)) if a.name == "Travis Scott"),
            "typing broke when the locale changed"
        );
    }

    // -- artist ------------------------------------------------------------------------------

    /// A shelf's "Show all" carries its own browse id, distinct from the artist's channel id —
    /// sending the channel id instead (as `expand_artist_shelf` used to, unconditionally) asked
    /// YouTube for an unpaired (browseId, params) combination, which fell back to a broader
    /// page and mixed every shelf's content together.
    #[test]
    fn a_shelfs_more_browse_id_is_not_the_artists_channel_id() {
        let a = parse_artist(&json("artist")).expect("artist parsed");
        let with_more: Vec<_> = a
            .shelves
            .iter()
            .filter(|s| s.more_params.is_some())
            .collect();
        assert!(!with_more.is_empty(), "no shelf carried a \"Show all\"");
        assert!(
            with_more
                .iter()
                .any(|s| s.more_browse_id.as_deref() != Some(a.id.as_str())),
            "every shelf's more_browse_id fell back to the artist's own channel id: {:?}",
            with_more
                .iter()
                .map(|s| (&s.title, &s.more_browse_id))
                .collect::<Vec<_>>()
        );
    }

    /// The reason `parse_mood_playlists` exists: a real "Chill" response, and the typed
    /// `get_mood_playlists` throws on some other mood/genre chip's section shape.
    #[test]
    fn mood_playlists_walks_every_carousel_section() {
        let p = parse_mood_playlists(&json("mood-playlists")).expect("mood playlists parsed");
        assert_eq!(
            p.title, "Coffee shop blends",
            "title should be the first section's own"
        );
        let titles: Vec<&str> = p.playlists.iter().map(|pl| pl.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Coffee Shop Blend",
                "Cafecito & Chill",
                "Uncut Bollywood",
                "Pop Gold",
                "Soft Rock Ballads",
                "The Smoothest of Smooth Jazz",
            ],
            "should flatten every section's cards, not just the first"
        );
        assert!(
            p.playlists.iter().all(|pl| !pl.id.is_empty()),
            "every playlist needs an id to be playable"
        );
    }

    /// The reason `parse_artist` exists: `ytmapi-rs` drops these covers, so every top song showed
    /// the artist's photograph.
    #[test]
    fn top_songs_have_their_own_covers() {
        let a = parse_artist(&json("artist")).expect("artist parsed");
        assert!(!a.top_songs.is_empty(), "no top songs parsed");

        let covers: Vec<&str> = a
            .top_songs
            .iter()
            .map(|t| t.thumbnail.as_deref().expect("a song cover"))
            .collect();
        assert!(
            covers.iter().any(|c| Some(*c) != a.thumbnail.as_deref()),
            "every song reused the artist photo"
        );
        let unique: std::collections::HashSet<_> = covers.iter().collect();
        assert!(
            unique.len() > 1,
            "all {} songs share one cover",
            covers.len()
        );
    }

    /// The header strings arrive complete; the UI must not add words to them.
    #[test]
    fn the_header_counts_carry_their_own_units() {
        let a = parse_artist(&json("artist")).expect("artist parsed");
        // The subscriber count is deliberately the *bare* number now — it goes on the Subscribe
        // button, where "5.37M subscribers" would not fit and would repeat the word on it.
        let subs = a.subscribers.as_deref().expect("a subscriber count");
        assert!(
            !subs.contains("subscriber"),
            "the button wants the bare count: {subs:?}"
        );
        // The long form is the sentence the header falls back to when there is no monthly
        // audience; it must carry its own unit, or a small artist shows a bare number.
        let long = a
            .subscribers_long
            .as_deref()
            .expect("a long subscriber count");
        assert!(
            long.ends_with("subscribers"),
            "the sentence wants the long form: {long:?}"
        );
        // The view count still carries its own word, which is why appending one printed it twice.
        let views = a.views.as_deref().expect("a view count");
        assert!(views.ends_with("views"), "view count: {views:?}");
        assert_eq!(
            views.matches("views").count(),
            1,
            "the word is already there once: {views:?}"
        );
    }

    /// The header's own buttons: what the page needs for Shuffle, Mix and Subscribe. Neither
    /// radio id is derivable from the channel id, so a parser that misses them leaves the page
    /// with buttons it cannot wire.
    #[test]
    fn the_artist_header_carries_its_actions() {
        let a = parse_artist(&json("artist")).expect("artist parsed");
        // This fixture is an anonymous capture, which carries the subscribe state but no play
        // buttons; assert on what it does have, and that the rest degrades to `None` rather
        // than to something invented.
        assert_eq!(a.actions.subscribed, Some(false));
        assert!(a.actions.dont_recommend.is_none());
        for t in [&a.actions.shuffle, &a.actions.mix].into_iter().flatten() {
            assert!(!t.playlist_id.is_empty(), "a target with no playlist");
        }
    }

    #[test]
    fn the_artist_header_parses() {
        let a = parse_artist(&json("artist")).expect("artist parsed");
        assert_eq!(a.name, "Travis Scott");
        assert!(!a.id.is_empty(), "no channel id");
        assert!(a.thumbnail.is_some(), "no artist photo");
        assert!(
            a.description.as_deref().is_some_and(|d| d.len() > 20),
            "no description"
        );
    }

    /// A video card is 16:9 and a song card is square — the kind is the signal, because no
    /// captured card carries `aspectRatio` and the thumbnail URLs arrive square either way.
    #[test]
    fn a_video_card_parses_wide_and_a_song_does_not() {
        // The fixture's own video cards: no captured response carries `aspectRatio`, so this
        // is the proof the kind is enough.
        let a = parse_artist(&json("artist")).expect("artist parsed");
        let mut videos = 0;
        for shelf in &a.shelves {
            for item in &shelf.items {
                if let Entry::Video(t) = item {
                    videos += 1;
                    assert!(t.wide, "{:?} is a video but parses square", t.title);
                }
            }
        }
        assert!(
            videos > 0,
            "the fixture has no video cards to prove this with"
        );
        // No fixture holds an album-audio card, so the song side is built by hand.
        let card = serde_json::json!({"musicTwoRowItemRenderer": {
            "title": {"runs": [{"text": "Some Song"}]},
            "subtitle": {"runs": [{"text": "Song"}]},
            "navigationEndpoint": {"watchEndpoint": {
                "videoId": "vid2",
                "watchEndpointMusicSupportedConfigs": {"watchEndpointMusicConfig": {
                    "musicVideoType": "MUSIC_VIDEO_TYPE_ATV"
                }}
            }},
            "thumbnailRenderer": {"musicThumbnailRenderer": {"thumbnail": {"thumbnails": [
                {"url": "https://x/y=w60-h60"}
            ]}}}
        }});
        let entry = parse_card(&card).expect("a song card parsed");
        let Entry::Song(t) = entry else {
            panic!("an ATV card should be a song, got {entry:?}");
        };
        assert!(!t.wide, "an ATV card should parse square");
    }

    /// A card's subtitle can carry an album link the same way a search/artist-songs row's
    /// byline does — `parse_card` used to hardcode `album_id: None` unconditionally, so a
    /// track surfaced via a home/artist-page card shelf could never have a clickable title.
    #[test]
    fn a_card_with_an_album_link_keeps_its_album_id() {
        let card = serde_json::json!({"musicTwoRowItemRenderer": {
            "title": {"runs": [{"text": "Some Song"}]},
            "subtitle": {"runs": [
                {"text": "Some Artist"},
                {"text": " • "},
                {"text": "Some Album", "navigationEndpoint": {"browseEndpoint": {
                    "browseId": "MPREb_album123",
                    "browseEndpointContextSupportedConfigs": {"browseEndpointContextMusicConfig": {
                        "pageType": "MUSIC_PAGE_TYPE_ALBUM"
                    }}
                }}}
            ]},
            "navigationEndpoint": {"watchEndpoint": {
                "videoId": "vid3",
                "watchEndpointMusicSupportedConfigs": {"watchEndpointMusicConfig": {
                    "musicVideoType": "MUSIC_VIDEO_TYPE_ATV"
                }}
            }},
            "thumbnailRenderer": {"musicThumbnailRenderer": {"thumbnail": {"thumbnails": [
                {"url": "https://x/y=w60-h60"}
            ]}}}
        }});
        let entry = parse_card(&card).expect("a song card parsed");
        let Entry::Song(t) = entry else {
            panic!("an ATV card should be a song, got {entry:?}");
        };
        assert_eq!(t.album.as_deref(), Some("Some Album"));
        assert_eq!(t.album_id.as_deref(), Some("MPREb_album123"));
    }

    /// "Your daily discover" and the radio feeds never name the album in the byline — the
    /// subtitle is only "artist • plays" — yet the card still carries "Go to album" as a menu
    /// action. That menu is where the album id has to come from, or the title is a dead end.
    #[test]
    fn a_card_without_an_album_byline_reads_it_from_the_menu() {
        let card = serde_json::json!({"musicTwoRowItemRenderer": {
            "title": {"runs": [{"text": "Some Song"}]},
            "subtitle": {"runs": [
                {"text": "Some Artist", "navigationEndpoint": {"browseEndpoint": {
                    "browseId": "UCartist",
                    "browseEndpointContextSupportedConfigs": {"browseEndpointContextMusicConfig": {
                        "pageType": "MUSIC_PAGE_TYPE_ARTIST"
                    }}
                }}},
                {"text": " • "},
                {"text": "64M plays"}
            ]},
            "menu": {"menuRenderer": {"items": [
                {"menuNavigationItemRenderer": {
                    "icon": {"iconType": "ALBUM"},
                    "navigationEndpoint": {"browseEndpoint": {
                        "browseId": "MPREb_frommenu",
                        "params": "ggMEgAKcDQ%3D%3D",
                        "browseEndpointContextSupportedConfigs": {"browseEndpointContextMusicConfig": {
                            "pageType": "MUSIC_PAGE_TYPE_ALBUM"
                        }}
                    }},
                    "text": {"runs": [{"text": "Go to album"}]}
                }}
            ]}},
            "navigationEndpoint": {"watchEndpoint": {
                "videoId": "vid4",
                "watchEndpointMusicSupportedConfigs": {"watchEndpointMusicConfig": {
                    "musicVideoType": "MUSIC_VIDEO_TYPE_ATV"
                }}
            }},
            "thumbnailRenderer": {"musicThumbnailRenderer": {"thumbnail": {"thumbnails": [
                {"url": "https://x/y=w60-h60"}
            ]}}}
        }});
        let entry = parse_card(&card).expect("a song card parsed");
        let Entry::Song(t) = entry else {
            panic!("an ATV card should be a song, got {entry:?}");
        };
        assert_eq!(t.album_id.as_deref(), Some("MPREb_frommenu"));
    }

    /// A video's byline links its channel, not an artist page — and that id must survive the
    /// parse, or the channel is not clickable.
    #[test]
    fn a_user_channel_byline_keeps_its_id() {
        let card = serde_json::json!({"musicTwoRowItemRenderer": {
            "title": {"runs": [{"text": "Some Video"}]},
            "subtitle": {"runs": [
                {"text": "Some Channel", "navigationEndpoint": {"browseEndpoint": {
                    "browseId": "UC123",
                    "browseEndpointContextSupportedConfigs": {"browseEndpointContextMusicConfig": {
                        "pageType": "MUSIC_PAGE_TYPE_USER_CHANNEL"
                    }}
                }}}
            ]},
            "navigationEndpoint": {"watchEndpoint": {
                "videoId": "vid1",
                "watchEndpointMusicSupportedConfigs": {"watchEndpointMusicConfig": {
                    "musicVideoType": "MUSIC_VIDEO_TYPE_OMV"
                }}
            }},
            "thumbnailRenderer": {"musicThumbnailRenderer": {"thumbnail": {"thumbnails": [
                {"url": "https://x/y=w60-h60"}
            ]}}}
        }});
        let entry = parse_card(&card).expect("a video card parsed");
        let Entry::Video(t) = entry else {
            panic!("an OMV card should be a video, got {entry:?}");
        };
        assert!(t.wide, "an OMV card should parse wide");
        assert_eq!(
            t.artists
                .iter()
                .filter_map(|a| a.id.clone())
                .collect::<Vec<_>>(),
            ["UC123"],
            "the channel id did not survive: {:?}",
            t.artists
        );
    }

    /// A channel page renders a visual header — no monthly audience, no Shuffle/Mix — and
    /// parses as an artist with `None`s where an artist has values, not as nothing.
    #[test]
    fn a_channel_page_parses_as_an_artist() {
        let root = serde_json::json!({
            "header": {"musicVisualHeaderRenderer": {
                "title": {"runs": [{"text": "Some Channel"}]},
                // A channel's photo is the *foreground* thumbnail, not `thumbnail` — the path
                // that made a profile open with no picture.
                "foregroundThumbnail": {"musicThumbnailRenderer": {"thumbnail": {"thumbnails": [
                    {"url": "https://x/small", "width": 60, "height": 60},
                    {"url": "https://x/big", "width": 544, "height": 544}
                ]}}}
            }},
            "contents": {"singleColumnBrowseResultsRenderer": {"tabs": []}}
        });
        let a = parse_artist(&root).expect("a channel page parsed");
        assert_eq!(a.name, "Some Channel");
        assert_eq!(
            a.thumbnail.as_deref(),
            Some("https://x/big"),
            "the channel's profile picture must survive"
        );
        assert!(
            a.monthly_listeners.is_none(),
            "a channel has no monthly audience"
        );
        assert!(a.actions.shuffle.is_none() && a.actions.mix.is_none());
        assert!(a.top_songs.is_empty() && a.shelves.is_empty());
    }

    /// `parse_artist` used to hardcode a single-column section path, unlike every other feed
    /// parser (home/explore/history/library), which tries several shapes via
    /// `find_section_list`. A profile/channel response in the two-column shape used to parse
    /// with an empty `top_songs` even though its header (name/photo) still came through.
    #[test]
    fn a_two_column_profile_still_finds_its_top_songs() {
        let root = serde_json::json!({
            "header": {"musicVisualHeaderRenderer": {
                "title": {"runs": [{"text": "Some Channel"}]}
            }},
            "contents": {"twoColumnBrowseResultsRenderer": {"tabs": [{"tabRenderer": {"content": {
                "sectionListRenderer": {"contents": [{"musicShelfRenderer": {"contents": [
                    {"musicResponsiveListItemRenderer": {
                        "flexColumns": [
                            {"musicResponsiveListItemFlexColumnRenderer": {
                                "text": {"runs": [{"text": "Test Song"}]}
                            }}
                        ],
                        "navigationEndpoint": {"watchEndpoint": {
                            "videoId": "vid9",
                            "watchEndpointMusicSupportedConfigs": {"watchEndpointMusicConfig": {
                                "musicVideoType": "MUSIC_VIDEO_TYPE_ATV"
                            }}
                        }}
                    }}
                ]}}]}
            }}}]}}
        });
        let a = parse_artist(&root).expect("a two-column profile parsed");
        assert_eq!(a.name, "Some Channel");
        assert_eq!(
            a.top_songs.len(),
            1,
            "the two-column shape's shelf never got read: {:?}",
            a.top_songs
        );
        assert_eq!(a.top_songs[0].title, "Test Song");
    }

    /// Albums and singles are separate shelves, and the five that used to be discarded survive.
    /// Flattening the carousels into one list is the bug this guards.
    #[test]
    fn the_artist_keeps_youtubes_own_shelves() {
        let a = parse_artist(&json("artist")).expect("artist parsed");
        let titles: Vec<&str> = a.shelves.iter().map(|s| s.title.as_str()).collect();
        for want in [
            "Albums",
            "Singles & EPs",
            "Videos",
            "Featured on",
            "Fans might also like",
        ] {
            assert!(titles.contains(&want), "no {want:?} shelf in {titles:?}");
        }
        assert!(
            titles.iter().position(|t| *t == "Albums")
                < titles.iter().position(|t| *t == "Singles & EPs"),
            "shelves lost YouTube's order"
        );
    }

    /// Each shelf holds the kind of thing its title promises, with ids and covers.
    #[test]
    fn artist_shelves_hold_usable_entries() {
        let a = parse_artist(&json("artist")).expect("artist parsed");
        let shelf = |t: &str| {
            a.shelves
                .iter()
                .find(|s| s.title == t)
                .unwrap_or_else(|| panic!("no {t} shelf"))
        };
        assert!(
            shelf("Albums")
                .items
                .iter()
                .all(|i| matches!(i, Entry::Album(al) if !al.id.is_empty())),
            "the Albums shelf holds something other than albums"
        );
        assert!(
            shelf("Fans might also like")
                .items
                .iter()
                .all(|i| matches!(i, Entry::Artist(ar) if !ar.id.is_empty())),
            "related artists did not parse as artists"
        );
        assert!(
            shelf("Videos").items.iter().all(|i| i.track().is_some()),
            "videos are not playable"
        );
        for s in &a.shelves {
            assert!(
                s.items.iter().all(|i| i.thumbnail().is_some()),
                "{} has an item with no cover",
                s.title
            );
        }
        // "Singles & EPs" is the one YouTube offers a full list for.
        assert!(
            shelf("Singles & EPs").more_params.is_some(),
            "lost the params behind 'Show all'"
        );
        assert!(
            a.shelves.iter().any(|s| s.more_params.is_none()),
            "every shelf claims to be expandable"
        );
    }

    // -- library -----------------------------------------------------------------------------

    /// A library song row: title in column 0, artist in 1, album in 2, duration in a fixed column.
    /// Reading a fixed index — as the search layout does — gave library rows no album, no
    /// duration, and the album text rendered as a play count.
    #[test]
    fn a_library_row_finds_its_album_and_duration_in_other_columns() {
        let row = serde_json::json!({"musicResponsiveListItemRenderer": {
        "thumbnail": {"musicThumbnailRenderer": {"thumbnail":
            {"thumbnails": [{"url": "https://x/a=w60-h60"}]}}},
        "flexColumns": [
            {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [
                {"text": "Nude", "navigationEndpoint": {"watchEndpoint": {"videoId": "vvvvvvvvvvv",
                    "watchEndpointMusicSupportedConfigs": {"watchEndpointMusicConfig":
                        {"musicVideoType": "MUSIC_VIDEO_TYPE_ATV"}}}}}]}}},
            {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [
                {"text": "Radiohead", "navigationEndpoint": {"browseEndpoint": {"browseId": "UC1",
                    "browseEndpointContextSupportedConfigs": {"browseEndpointContextMusicConfig":
                        {"pageType": "MUSIC_PAGE_TYPE_ARTIST"}}}}}]}}},
            {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [
                {"text": "In Rainbows", "navigationEndpoint": {"browseEndpoint": {"browseId": "MPREb_1",
                    "browseEndpointContextSupportedConfigs": {"browseEndpointContextMusicConfig":
                        {"pageType": "MUSIC_PAGE_TYPE_ALBUM"}}}}}]}}}
        ],
        "fixedColumns": [
            {"musicResponsiveListItemFixedColumnRenderer": {"text": {"runs": [{"text": "4:16"}]}}}
        ]}});
        let entry = parse_search_row(row.get("musicResponsiveListItemRenderer").unwrap())
            .expect("the row should parse");
        let t = entry.track().expect("a playable track");
        assert_eq!(t.artist_line(), "Radiohead");
        assert_eq!(t.primary_artist_id(), Some("UC1"));
        assert_eq!(
            t.album.as_deref(),
            Some("In Rainbows"),
            "album lives in column 2 here"
        );
        assert_eq!(t.album_id.as_deref(), Some("MPREb_1"));
        assert_eq!(
            t.duration,
            Some(256),
            "duration lives in a fixed column here"
        );
        assert_eq!(t.plays, None, "the album is not a play count");
    }

    /// The two shapes the four library browse ids actually return. `ytmapi-rs` reads only the
    /// shelf, which is why liked albums and playlists could never have loaded.
    #[test]
    fn both_library_layouts_parse() {
        let grid = serde_json::json!({"contents": {"singleColumnBrowseResultsRenderer": {
        "tabs": [{"tabRenderer": {"content": {"sectionListRenderer": {"contents": [
            {"gridRenderer": {"items": [
                {"musicTwoRowItemRenderer": {
                    "title": {"runs": [{"text": "In Rainbows"}]},
                    "subtitle": {"runs": [{"text": "Album • 2007"}]},
                    "thumbnailRenderer": {"musicThumbnailRenderer": {"thumbnail":
                        {"thumbnails": [{"url": "https://x/y=w120-h120"}]}}},
                    "navigationEndpoint": {"browseEndpoint": {"browseId": "MPREb_1",
                        "browseEndpointContextSupportedConfigs":
                            {"browseEndpointContextMusicConfig":
                                {"pageType": "MUSIC_PAGE_TYPE_ALBUM"}}}}}}
            ]}}
        ]}}}}]}}});
        let items = parse_library_section(&grid);
        assert_eq!(items.len(), 1, "the grid layout did not parse");
        assert!(matches!(&items[0], Entry::Album(a) if a.id == "MPREb_1"));
        assert!(!looks_signed_out(&grid));

        let shelf = serde_json::json!({"contents": {"singleColumnBrowseResultsRenderer": {
        "tabs": [{"tabRenderer": {"content": {"sectionListRenderer": {"contents": [
            {"musicShelfRenderer": {"contents": [
                {"musicResponsiveListItemRenderer": {
                    "thumbnail": {"musicThumbnailRenderer": {"thumbnail":
                        {"thumbnails": [{"url": "https://x/z=w60-h60"}]}}},
                    "flexColumns": [
                        {"musicResponsiveListItemFlexColumnRenderer": {"text":
                            {"runs": [{"text": "Nude"}]}}},
                        {"musicResponsiveListItemFlexColumnRenderer": {"text":
                            {"runs": [{"text": "Radiohead"}]}}}
                    ],
                    "navigationEndpoint": {"watchEndpoint": {"videoId": "abcdefghijk",
                        "watchEndpointMusicSupportedConfigs": {"watchEndpointMusicConfig":
                            {"musicVideoType": "MUSIC_VIDEO_TYPE_ATV"}}}}}}
            ]}}
        ]}}}}]}}});
        let items = parse_library_section(&shelf);
        assert_eq!(items.len(), 1, "the shelf layout did not parse");
        assert_eq!(
            items[0].track().map(|t| t.video_id.as_str()),
            Some("abcdefghijk")
        );
        assert!(!looks_signed_out(&shelf));
    }

    /// "Show all" arrives in more shapes than one grid. The typed parser walks a single
    /// hardcoded pointer and fails when the first section is not the expected grid — while the
    /// entries sit in later sections. Every shape here must contribute, kinds preserved.
    #[test]
    fn artist_expansions_parse_every_layout() {
        let mixed = serde_json::json!({"contents": {"singleColumnBrowseResultsRenderer": {
        "tabs": [{"tabRenderer": {"content": {"sectionListRenderer": {"contents": [
            {"musicShelfRenderer": {"contents": [
                {"musicResponsiveListItemRenderer": {
                    "flexColumns": [
                        {"musicResponsiveListItemFlexColumnRenderer": {"text":
                            {"runs": [{"text": "Nude"}]}}},
                        {"musicResponsiveListItemFlexColumnRenderer": {"text":
                            {"runs": [{"text": "Radiohead"}]}}}
                    ],
                    "navigationEndpoint": {"watchEndpoint": {"videoId": "abcdefghijk",
                        "watchEndpointMusicSupportedConfigs": {"watchEndpointMusicConfig":
                            {"musicVideoType": "MUSIC_VIDEO_TYPE_OMV"}}}}}}
            ]}},
            {"gridRenderer": {"items": [
                {"musicTwoRowItemRenderer": {
                    "title": {"runs": [{"text": "In Rainbows"}]},
                    "subtitle": {"runs": [{"text": "Album • 2007"}]},
                    "navigationEndpoint": {"browseEndpoint": {"browseId": "MPREb_1",
                        "browseEndpointContextSupportedConfigs":
                            {"browseEndpointContextMusicConfig":
                                {"pageType": "MUSIC_PAGE_TYPE_ALBUM"}}}}}}
            ]}},
            {"itemSectionRenderer": {"contents": [
                {"musicCarouselShelfRenderer": {"contents": [
                    {"musicTwoRowItemRenderer": {
                        "title": {"runs": [{"text": "A Moon Shaped Pool"}]},
                        "subtitle": {"runs": [{"text": "Album • 2016"}]},
                        "navigationEndpoint": {"browseEndpoint": {"browseId": "MPREb_2",
                            "browseEndpointContextSupportedConfigs":
                                {"browseEndpointContextMusicConfig":
                                    {"pageType": "MUSIC_PAGE_TYPE_ALBUM"}}}}}}
                ]}}
            ]}}
        ]}}}}]}}});
        let items = parse_artist_albums(&mixed);
        assert_eq!(items.len(), 3, "every section shape must contribute");
        assert!(
            items
                .iter()
                .any(|e| matches!(e, Entry::Video(t) if t.video_id == "abcdefghijk")),
            "a video row must stay a video, not become an album"
        );
        assert!(
            items
                .iter()
                .any(|e| matches!(e, Entry::Album(a) if a.id == "MPREb_1"))
        );
        assert!(
            items
                .iter()
                .any(|e| matches!(e, Entry::Album(a) if a.id == "MPREb_2"))
        );

        // A continuation response has no section list; the walk still finds its items.
        let cont = serde_json::json!({"continuationContents": {"sectionListContinuation":
        {"contents": [{"gridRenderer": {"items": [
            {"musicTwoRowItemRenderer": {
                "title": {"runs": [{"text": "Kid A"}]},
                "subtitle": {"runs": [{"text": "Album • 2000"}]},
                "navigationEndpoint": {"browseEndpoint": {"browseId": "MPREb_3",
                    "browseEndpointContextSupportedConfigs":
                        {"browseEndpointContextMusicConfig":
                            {"pageType": "MUSIC_PAGE_TYPE_ALBUM"}}}}}}
        ]}}]}}});
        assert_eq!(parse_artist_albums(&cont).len(), 1);

        // Unreadable input is empty, not a panic.
        assert!(parse_artist_albums(&Value::Null).is_empty());
    }

    /// The response that produced "could not load your songs: Key /contents/… not found". It must
    /// read as "nothing here", and be distinguishable from a genuinely empty library.
    #[test]
    fn the_signed_out_empty_state_is_recognised_not_an_error() {
        let anonymous = serde_json::json!({"contents": {"singleColumnBrowseResultsRenderer": {
        "tabs": [{"tabRenderer": {"content": {"sectionListRenderer": {"contents": [
            {"itemSectionRenderer": {"contents": [
                {"messageRenderer": {"text": {"runs": [{"text": "Explore your favorites"}]}}}
            ]}}
        ]}}}}]}}});
        assert!(parse_library_section(&anonymous).is_empty());
        assert!(
            looks_signed_out(&anonymous),
            "the signed-out prompt must be recognised, or the session cannot be refreshed"
        );
    }

    #[test]
    fn a_library_response_we_cannot_read_is_empty_not_a_panic() {
        assert!(parse_library_section(&Value::Null).is_empty());
        assert!(!looks_signed_out(&Value::Null));
    }

    // -- paging ------------------------------------------------------------------------------

    /// Both token shapes YouTube uses. Missing either is how a 689-song library looked like 25.
    #[test]
    fn a_continuation_token_is_found_in_either_shape() {
        let old = serde_json::json!({"contents": {"a": [
            {"nextContinuationData": {"continuation": "TOKEN-OLD", "clickTrackingParams": "x"}}]}});
        assert_eq!(continuation_token(&old).as_deref(), Some("TOKEN-OLD"));

        let new = serde_json::json!({"onResponseReceivedActions": [{"appendContinuationItemsAction":
            {"continuationItems": [{"continuationItemRenderer": {"continuationEndpoint":
                {"continuationCommand": {"token": "TOKEN-NEW"}}}}]}}]});
        assert_eq!(continuation_token(&new).as_deref(), Some("TOKEN-NEW"));
    }

    /// A final page has no token, which is what ends the walk.
    #[test]
    fn a_last_page_has_no_token() {
        assert_eq!(
            continuation_token(&serde_json::json!({"contents": []})),
            None
        );
        assert_eq!(continuation_token(&Value::Null), None);
        // An empty token is not a token; following it would loop.
        let blank = serde_json::json!({"nextContinuationData": {"continuation": ""}});
        assert_eq!(continuation_token(&blank), None);
    }

    /// A continuation response carries no section list — its rows hang off the continuation node.
    /// Reading only the section list is why a second page would have parsed as empty.
    #[test]
    fn a_continuation_page_parses_its_items() {
        let page = serde_json::json!({"continuationContents": {"musicShelfContinuation": {
            "contents": [
                {"musicResponsiveListItemRenderer": {
                    "thumbnail": {"musicThumbnailRenderer": {"thumbnail":
                        {"thumbnails": [{"url": "https://x/a=w60-h60"}]}}},
                    "flexColumns": [
                        {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [
                            {"text": "Second page song", "navigationEndpoint": {"watchEndpoint":
                                {"videoId": "pageTwoAbc",
                                 "watchEndpointMusicSupportedConfigs": {"watchEndpointMusicConfig":
                                    {"musicVideoType": "MUSIC_VIDEO_TYPE_ATV"}}}}}]}}}
                    ]}}
            ],
            "continuations": [{"nextContinuationData": {"continuation": "TOKEN-3"}}]}}});
        let parsed = parse_library_page(&page);
        assert_eq!(parsed.items.len(), 1, "a continuation page parsed as empty");
        assert_eq!(
            parsed.items[0].track().map(|t| t.video_id.as_str()),
            Some("pageTwoAbc")
        );
        assert_eq!(
            parsed.next.as_deref(),
            Some("TOKEN-3"),
            "the walk must continue"
        );
    }

    // -- account -----------------------------------------------------------------------------

    /// The discriminator sign-in depends on: present for a real session, absent for a cookie
    /// YouTube is ignoring. Without it, "signed in with an empty library" and "not signed in"
    /// look identical.
    #[test]
    fn the_account_name_is_found_wherever_it_is_nested() {
        let menu = serde_json::json!({"actions": [{"openPopupAction": {"popup": {"multiPageMenuRenderer":
            {"header": {"activeAccountHeaderRenderer": {
                "accountName": {"runs": [{"text": "Someone"}]}}}}}}}]});
        assert_eq!(account_name(&menu).as_deref(), Some("Someone"));
    }

    /// The photo the account chip draws. It hangs off `accountPhoto`, not `thumbnail`, which is
    /// why every account parsed without one and the chip only ever showed an initial.
    #[test]
    fn an_account_carries_its_profile_picture() {
        let list = serde_json::json!({"contents": [{"accountItem": {
            "accountName": {"runs": [{"text": "IV"}]},
            "accountByline": {"runs": [{"text": "Brand account"}]},
            "isSelected": true,
            "accountPhoto": {"thumbnails": [
                {"url": "https://yt3.ggpht.com/small=s48", "width": 48, "height": 48},
                {"url": "https://yt3.ggpht.com/big=s108", "width": 108, "height": 108}
            ]},
            "serviceEndpoint": {"selectActiveIdentityEndpoint": {"supportedTokens": [
                {"pageIdToken": {"pageId": "UC123"}},
                {"offlineCacheKeyToken": {"clientCacheKey": "abcDEF123"}}
            ]}},
            "channelHandle": {"runs": [{"text": "@iv"}]}
        }}]});
        let accounts = parse_accounts(&list);
        assert_eq!(accounts.len(), 1);
        assert_eq!(
            accounts[0].thumbnail.as_deref(),
            Some("https://yt3.ggpht.com/big=s108"),
            "the largest photo, as everywhere else"
        );
        assert_eq!(accounts[0].page_id.as_deref(), Some("UC123"));
        assert_eq!(
            accounts[0].channel_id.as_deref(),
            Some("UCabcDEF123"),
            "the offline cache key is the channel id without its UC prefix"
        );
        assert_eq!(accounts[0].handle.as_deref(), Some("@iv"));
    }

    #[test]
    fn an_anonymous_account_menu_has_no_name() {
        assert_eq!(account_name(&serde_json::json!({"actions": []})), None);
        assert_eq!(
            account_name(&serde_json::json!({"accountName": {"runs": [{"text": "  "}]}})),
            None,
            "a blank name is not a session"
        );
    }

    // -- home feed ---------------------------------------------------------------------------

    /// The anonymous feed, verbatim.
    #[test]
    fn the_home_feed_parses_youtubes_shelves() {
        let shelves = parse_browse_shelves(&json("home"));
        let titles: Vec<&str> = shelves.iter().map(|s| s.title.as_str()).collect();
        assert!(titles.contains(&"New releases"), "{titles:?}");
        assert!(
            shelves.iter().all(|s| !s.items.is_empty()),
            "an empty shelf was kept: {titles:?}"
        );
        // The "tell us what you like" prompt has no content and nowhere to go.
        assert!(
            !titles.iter().any(|t| t.contains("Tastebuilder")),
            "{titles:?}"
        );
    }

    /// Explore is what fills the page while the anonymous home is two shelves.
    #[test]
    fn the_explore_feed_supplies_the_filler_shelves() {
        let shelves = parse_browse_shelves(&json("explore"));
        let titles: Vec<&str> = shelves.iter().map(|s| s.title.as_str()).collect();
        for want in ["New albums & singles", "Moods & genres", "New music videos"] {
            assert!(titles.contains(&want), "no {want:?} in {titles:?}");
        }
        // The mood chips are a different renderer from the cards around them and would otherwise
        // be dropped, leaving the shelf empty.
        let moods = shelves
            .iter()
            .find(|s| s.title == "Moods & genres")
            .expect("moods shelf");
        assert!(moods.is_mood_list(), "mood chips did not parse as moods");
        assert!(
            moods
                .items
                .iter()
                .all(|i| matches!(i, Entry::Mood(m) if !m.params.is_empty())),
            "a mood chip has no params, so it could not open"
        );
        // Item counts are capped in the fixtures; the live check is `just api`.
        assert!(shelves.iter().all(|s| !s.items.is_empty()));
    }

    /// The three buttons above Explore's shelves, with the destinations the feed sent — Charts
    /// carries params, the other two do not.
    #[test]
    fn the_explore_buttons_come_from_the_feed() {
        let links = parse_explore_links(&json("explore"));
        let ids: Vec<&str> = links.iter().map(|l| l.browse_id.as_str()).collect();
        for want in [
            "FEmusic_new_releases",
            "FEmusic_charts",
            "FEmusic_moods_and_genres",
        ] {
            assert!(ids.contains(&want), "no {want:?} in {ids:?}");
        }
        let charts = links
            .iter()
            .find(|l| l.browse_id == "FEmusic_charts")
            .expect("charts button");
        assert!(
            charts.params.as_deref().is_some_and(|p| !p.is_empty()),
            "the charts button lost its params"
        );
        assert!(
            links
                .iter()
                .all(|l| !l.title.is_empty() && !l.browse_id.is_empty()),
            "a button has no title or destination"
        );
    }

    /// The mood chips' stripes, from the same fixture: ARGB on the wire, RGB in the model.
    #[test]
    fn mood_chips_carry_their_stripe_colour() {
        let shelves = parse_browse_shelves(&json("explore"));
        let moods = shelves
            .iter()
            .find(|s| s.title == "Moods & genres")
            .expect("moods shelf");
        let chill = moods
            .items
            .iter()
            .find_map(|i| match i {
                Entry::Mood(m) if m.title == "Chill" => Some(m),
                _ => None,
            })
            .expect("Chill chip");
        assert_eq!(chill.colour, Some([164, 197, 255]));
        assert!(
            moods
                .items
                .iter()
                .all(|i| matches!(i, Entry::Mood(m) if m.colour.is_some())),
            "a mood chip lost its stripe"
        );
    }

    /// A chart row is the entry as ever, plus the rank column beside it — and a row without
    /// the column still parses, because no captured response carries one.
    #[test]
    fn a_chart_row_parses_rank_and_trend() {
        let row = |rank: &str, icon: Option<&str>| {
            let mut col = serde_json::json!({});
            if !rank.is_empty() {
                col["text"] = serde_json::json!({"runs": [{"text": rank}]});
            }
            if let Some(icon) = icon {
                col["icon"] = serde_json::json!({"iconType": icon});
            }
            serde_json::json!({"musicResponsiveListItemRenderer": {
                "flexColumns": [
                    {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [
                        {"text": "Some Song", "navigationEndpoint": {"watchEndpoint": {
                            "videoId": "vid9",
                            "watchEndpointMusicSupportedConfigs": {"watchEndpointMusicConfig": {
                                "musicVideoType": "MUSIC_VIDEO_TYPE_ATV"
                            }}
                        }}}
                    ]}}},
                    {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [
                        {"text": "Someone"}
                    ]}}}
                ],
                "navigationEndpoint": {"watchEndpoint": {
                    "videoId": "vid9",
                    "watchEndpointMusicSupportedConfigs": {"watchEndpointMusicConfig": {
                        "musicVideoType": "MUSIC_VIDEO_TYPE_ATV"
                    }}
                }},
                "customIndexColumn": {"musicCustomIndexColumnRenderer": col}
            }})
        };
        let up = parse_chart_row(
            row("1", Some("ARROW_CHART_UP"))
                .get("musicResponsiveListItemRenderer")
                .unwrap(),
        )
        .expect("a ranked row parsed");
        assert_eq!(up.rank, "1");
        assert_eq!(up.trend, crate::model::ChartTrend::Up);
        assert!(matches!(up.entry, Entry::Song(_)));

        let down = parse_chart_row(
            row("12", Some("ARROW_CHART_DOWN"))
                .get("musicResponsiveListItemRenderer")
                .unwrap(),
        )
        .expect("a ranked row parsed");
        assert_eq!(down.trend, crate::model::ChartTrend::Down);

        let steady = parse_chart_row(
            row("3", Some("ARROW_CHART_NEUTRAL"))
                .get("musicResponsiveListItemRenderer")
                .unwrap(),
        )
        .expect("a ranked row parsed");
        assert_eq!(steady.trend, crate::model::ChartTrend::Steady);

        // No column at all: the entry survives, the rank is empty, the trend unknown.
        let bare = serde_json::json!({"musicResponsiveListItemRenderer": {
            "flexColumns": [
                {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [
                    {"text": "Some Song", "navigationEndpoint": {"watchEndpoint": {
                        "videoId": "vid9",
                        "watchEndpointMusicSupportedConfigs": {"watchEndpointMusicConfig": {
                            "musicVideoType": "MUSIC_VIDEO_TYPE_ATV"
                        }}
                    }}}
                ]}}}
            ],
            "navigationEndpoint": {"watchEndpoint": {
                "videoId": "vid9",
                "watchEndpointMusicSupportedConfigs": {"watchEndpointMusicConfig": {
                    "musicVideoType": "MUSIC_VIDEO_TYPE_ATV"
                }}
            }}
        }});
        let plain = parse_chart_row(bare.get("musicResponsiveListItemRenderer").unwrap())
            .expect("a rankless row parsed");
        assert!(plain.rank.is_empty());
        assert_eq!(plain.trend, crate::model::ChartTrend::Unknown);
    }

    #[test]
    fn every_home_card_can_be_opened() {
        for fixture in ["home", "explore"] {
            for s in parse_browse_shelves(&json(fixture)) {
                for item in &s.items {
                    let ok = match item {
                        Entry::Album(a) => !a.id.is_empty(),
                        Entry::Artist(a) => !a.id.is_empty(),
                        Entry::Playlist(p) => !p.id.is_empty(),
                        Entry::Song(t) | Entry::Video(t) => !t.video_id.is_empty(),
                        Entry::Mood(m) => !m.params.is_empty(),
                    };
                    assert!(ok, "{fixture}/{}: {:?} has no id", s.title, item.title());
                }
            }
        }
    }

    #[test]
    fn an_unreadable_feed_yields_no_shelves_rather_than_panicking() {
        assert!(parse_browse_shelves(&Value::Null).is_empty());
        assert!(parse_browse_shelves(&serde_json::json!({"contents": []})).is_empty());
    }

    /// The mood bar rides the home header: 11 chips, one feed with different params each.
    #[test]
    fn the_home_feed_carries_its_mood_bar() {
        let chips = parse_home_chips(&json("home"));
        assert_eq!(chips.len(), 11, "the chip bar came back short");
        assert!(
            chips
                .iter()
                .all(|c| !c.title.is_empty() && !c.params.is_empty()),
            "a chip cannot be shown or opened"
        );
        for want in ["Workout", "Energize", "Feel good", "Relax"] {
            assert!(
                chips.iter().any(|c| c.title == want),
                "no {want:?} in {:?}",
                chips.iter().map(|c| &c.title).collect::<Vec<_>>()
            );
        }
        // Same feed, different params — that is what makes a chip a filter.
        assert!(
            chips.iter().all(|c| c.params.starts_with("gg")),
            "a chip stopped carrying browse params"
        );
        assert!(
            chips.iter().all(|c| !c.selected),
            "the default feed arrived with a chip selected"
        );
    }

    #[test]
    fn chips_without_params_cannot_open_and_are_skipped() {
        let root = serde_json::json!({
            "header": {"chipCloudRenderer": {"chips": [
                {"chipCloudChipRenderer": {
                    "text": {"runs": [{"text": "Relax"}]},
                    "navigationEndpoint": {"browseEndpoint": {
                        "browseId": "FEmusic_home", "params": "ggX"
                    }},
                    "isSelected": true
                }},
                {"chipCloudChipRenderer": {
                    "text": {"runs": [{"text": "Broken"}]},
                    "navigationEndpoint": {"browseEndpoint": {"browseId": "FEmusic_home"}}
                }},
                {"chipCloudChipRenderer": {"text": {"simpleText": "Also broken"}}}
            ]}}
        });
        let chips = parse_home_chips(&root);
        assert_eq!(chips.len(), 1);
        assert_eq!(chips[0].title, "Relax");
        assert_eq!(chips[0].params, "ggX");
        assert!(chips[0].selected);
        assert!(parse_home_chips(&Value::Null).is_empty());
    }

    /// A response we cannot read must degrade to the typed fallback, not panic.
    #[test]
    fn nonsense_input_is_survivable() {
        assert!(parse_artist(&Value::Null).is_none());
        assert!(parse_artist(&serde_json::json!({"header": {}})).is_none());
        assert!(parse_search(&Value::Null).is_empty());
        assert!(parse_search(&serde_json::json!({"contents": 3})).is_empty());
        let empty = parse_watch_queue(&Value::Null);
        assert!(empty.tracks.is_empty() && empty.next.is_none());
    }

    // -- watch queue -------------------------------------------------------------------------

    /// Radio for "Creep": 50 tracks with distinct video ids, artists and durations — the whole
    /// point of the `next` endpoint is a queue long enough to play, not a page to look at.
    #[test]
    fn the_watch_queue_yields_fifty_tracks() {
        let queue = parse_watch_queue(&json("watch"));
        assert_eq!(queue.tracks.len(), 50, "radio came back short");
        let mut ids: Vec<&str> = queue.tracks.iter().map(|t| t.video_id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 50, "the river repeats itself on page one");
        assert!(
            queue.tracks.iter().all(|t| !t.artists.is_empty()),
            "a track lost its artist"
        );
        assert!(
            queue.tracks.iter().all(|t| t.duration.is_some()),
            "a track lost its duration"
        );
        assert!(
            queue.tracks.iter().all(|t| t.thumbnail.is_some()),
            "a track lost its cover"
        );
    }

    /// The player overlay is the only like state a watch queue carries. A full `next` page has
    /// it (this is how a mix knows its seed is liked); a continuation does not.
    #[test]
    fn the_watch_queue_carries_the_overlay_rating() {
        let queue = parse_watch_queue(&json("watch"));
        assert_eq!(queue.like, Some(LikeStatus::Indifferent));
        let more = parse_watch_queue(&json("watch-more"));
        assert_eq!(more.like, None, "a continuation has no player overlay");
    }

    /// The seed row carries everything a row further down the river does, plus a badge.
    #[test]
    fn the_watch_queue_seed_row_parses_fully() {
        let queue = parse_watch_queue(&json("watch"));
        let seed = queue.tracks.first().expect("seed row");
        assert_eq!(seed.title, "Creep");
        assert_eq!(seed.video_id, "9RfVp-GhKfs");
        assert_eq!(seed.artist_line(), "Radiohead");
        assert_eq!(
            seed.primary_artist_id(),
            Some("UCr_iyUANcn9OX_yy9piYoLw"),
            "the artist name must stay a link"
        );
        assert_eq!(seed.album.as_deref(), Some("Creep"));
        assert!(
            seed.album_id
                .as_deref()
                .is_some_and(|id| id.starts_with("MPREb_")),
            "the album name must stay a link"
        );
        // The year rides along in the byline but must not be mistaken for the duration.
        assert_eq!(seed.duration, Some(239), "3:59 from lengthText, not 1992");
        assert!(seed.explicit, "the badge is on the row");
    }

    /// Featured-artist bylines do not link: "Clint Eastwood" reads
    /// "Gorillaz & Del The Funky Homosapien", all plain text, with the album linked after it.
    /// The artist falls back to the first plain field — not `plain_tail`, which would return
    /// the year.
    #[test]
    fn the_watch_queue_keeps_unlinked_artists() {
        let queue = parse_watch_queue(&json("watch"));
        let track = queue
            .tracks
            .iter()
            .find(|t| t.video_id == "I7yqFVEvdY0")
            .expect("Clint Eastwood");
        assert_eq!(track.artist_line(), "Gorillaz & Del The Funky Homosapien");
        assert_eq!(track.album.as_deref(), Some("Gorillaz"));
        assert!(track.album_id.is_some());
    }

    /// The river continues: one radio token, non-empty, and the playlist id it extends.
    #[test]
    fn the_watch_queue_carries_its_continuation() {
        let queue = parse_watch_queue(&json("watch"));
        assert!(
            queue.next.as_deref().is_some_and(|t| !t.is_empty()),
            "no token, so autoplay would end at 50"
        );
    }

    /// Signed-in responses wrap most rows in `playlistPanelVideoWrapperRenderer`, each with
    /// `counterpart[]` alternate versions. The queue is still 50 tracks — reading only the bare
    /// shape parsed this river to 11, and reading the counterparts would double-count songs.
    #[test]
    fn the_wrapped_watch_queue_yields_fifty_tracks() {
        let queue = parse_watch_queue(&json("wrapped"));
        assert_eq!(queue.tracks.len(), 50, "radio came back short");
        let mut ids: Vec<&str> = queue.tracks.iter().map(|t| t.video_id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 50, "counterparts leaked into the queue");
        assert!(
            queue.next.as_deref().is_some_and(|t| !t.is_empty()),
            "no token, so autoplay would end at 50"
        );
    }

    /// A follow-up page arrives bare-rowed under `continuationContents` — 49 more tracks and a
    /// fresh token, which is what keeps autoplay past the first page.
    #[test]
    fn the_watch_continuation_extends_the_river() {
        let queue = parse_watch_queue(&json("watch-more"));
        assert_eq!(queue.tracks.len(), 49, "the continuation came back short");
        assert!(
            queue.tracks.iter().all(|t| !t.video_id.is_empty()),
            "a continuation track has no video id"
        );
        assert!(
            queue.next.as_deref().is_some_and(|t| !t.is_empty()),
            "no token, so autoplay would end at 99"
        );
    }

    // -- scrobble ----------------------------------------------------------------------------

    /// The `player` response carries both beacon URLs under `playbackTracking`. The playback
    /// URL fires once per play, the watchtime base every few seconds with the position — and
    /// the session is baked into both, so they are only useful read from a live response.
    #[test]
    fn playback_tracking_reads_both_beacon_urls() {
        let tracking = parse_playback_tracking(&json("player")).expect("both urls");
        assert!(
            tracking.playback_url.contains("/api/stats/playback"),
            "unexpected playback url: {}",
            tracking.playback_url
        );
        assert!(
            tracking.watchtime_base.contains("/api/stats/watchtime"),
            "unexpected watchtime url: {}",
            tracking.watchtime_base
        );
        assert!(
            tracking.watchtime_base.contains("docid=XFkzRNyygfk"),
            "the beacon must name its video: {}",
            tracking.watchtime_base
        );
    }

    /// Either URL missing is an unplayable shape (an age gate, a rental, an error page wearing
    /// a 200) — half a tracker would beacon nowhere, so the answer is `None`, not half a struct.
    #[test]
    fn playback_tracking_missing_either_url_is_none() {
        assert!(parse_playback_tracking(&serde_json::json!({})).is_none());
        let playback_only = serde_json::json!({"playbackTracking": {
            "videostatsPlaybackUrl": {"baseUrl": "https://example.com/playback"},
        }});
        assert!(parse_playback_tracking(&playback_only).is_none());
        let watchtime_only = serde_json::json!({"playbackTracking": {
            "videostatsWatchtimeUrl": {"baseUrl": "https://example.com/watchtime"},
        }});
        assert!(parse_playback_tracking(&watchtime_only).is_none());
        let empty = serde_json::json!({"playbackTracking": {
            "videostatsPlaybackUrl": {"baseUrl": ""},
            "videostatsWatchtimeUrl": {"baseUrl": "https://example.com/watchtime"},
        }});
        assert!(parse_playback_tracking(&empty).is_none());
    }

    // -- ratings ---------------------------------------------------------------------------

    /// The menu's like button, in all three states. Found by key: the menu moves between row
    /// types, and only the button's own subtree matters.
    #[test]
    fn like_status_reads_all_three_states() {
        for (status, want) in [
            ("LIKE", LikeStatus::Like),
            ("DISLIKE", LikeStatus::Dislike),
            ("INDIFFERENT", LikeStatus::Indifferent),
        ] {
            let row = serde_json::json!({"menu": {"menuRenderer": {"topLevelButtons": [
                {"likeButtonRenderer": {"likeStatus": status}}
            ]}}});
            assert_eq!(like_status(&row), want);
        }
    }

    /// No button means no rating, never an error. Signed-out rows carry minimal menus without
    /// one — every track in every fixture parses Indifferent today; signed-in rows will carry
    /// the button and parse above.
    #[test]
    fn rows_without_a_like_button_are_unrated() {
        assert_eq!(like_status(&Value::Null), LikeStatus::Indifferent);
        assert_eq!(
            like_status(&serde_json::json!({"menu": {}})),
            LikeStatus::Indifferent
        );
        for fixture in ["watch", "wrapped", "watch-more"] {
            assert!(
                parse_watch_queue(&json(fixture))
                    .tracks
                    .iter()
                    .all(|t| t.like == LikeStatus::Indifferent),
                "{fixture}: a track arrived rated without a button"
            );
        }
    }

    // -- library and playlist writes -------------------------------------------------------

    /// The bookmark toggle is identified by its `BOOKMARK_BORDER` icon, never by its label —
    /// labels localise (this account's UI is in Greek) while the icon does not.
    #[test]
    fn the_library_toggle_is_found_by_icon_not_label() {
        let item = |toggled: bool| {
            serde_json::json!({"menu": {"menuRenderer": {"items": [
                {"toggleMenuServiceItemRenderer": {
                    "defaultIcon": {"iconType": "BOOKMARK_BORDER"},
                    "defaultText": {"runs": [{"text": "Αποθήκευση στη βιβλιοθήκη"}]},
                    "defaultServiceEndpoint": {"feedbackEndpoint": {
                        "feedbackToken": "SAVE-TOKEN"}},
                    "toggledText": {"runs": [{"text": "Κατάργηση"}]},
                    "toggledServiceEndpoint": {"feedbackEndpoint": {
                        "feedbackToken": "REMOVE-TOKEN"}},
                    "isToggled": toggled
                }},
                {"toggleMenuServiceItemRenderer": {
                    "defaultIcon": {"iconType": "KEEP"},
                    "defaultServiceEndpoint": {"feedbackEndpoint": {
                        "feedbackToken": "PIN-TOKEN"}}
                }}
            ]}}})
        };
        let saved = library_toggle(&item(true)).expect("the bookmark toggle");
        assert!(saved.in_library);
        assert_eq!(saved.add.as_deref(), Some("SAVE-TOKEN"));
        assert_eq!(saved.remove.as_deref(), Some("REMOVE-TOKEN"));
        let unsaved = library_toggle(&item(false)).expect("the bookmark toggle");
        assert!(!unsaved.in_library);
        // The pin toggle must not be mistaken for the bookmark one.
        let pinned = serde_json::json!({"menu": {"menuRenderer": {"items": [{
            "toggleMenuServiceItemRenderer": {
                "defaultIcon": {"iconType": "KEEP"},
                "defaultServiceEndpoint": {"feedbackEndpoint": {"feedbackToken": "PIN-TOKEN"}}
            }
        }]}}});
        assert!(library_toggle(&pinned).is_none());
        assert!(library_toggle(&Value::Null).is_none());
    }

    /// Page headers carry their own bookmark toggle — the album's `musicDetailHeaderRenderer`,
    /// the playlist's `musicResponsiveHeaderRenderer` — and the scan stays inside the header:
    /// rows below carry their own bookmark menu items, and a document-wide scan would read a
    /// song's state as the page's.
    #[test]
    fn page_headers_read_their_own_bookmark_toggle() {
        use serde_json::{Map, Value, json};
        fn bookmark(token: &str, toggled: bool) -> Value {
            json!({"toggleMenuServiceItemRenderer": {
                "defaultIcon": {"iconType": "BOOKMARK_BORDER"},
                "defaultServiceEndpoint": {"feedbackEndpoint": {
                    "feedbackToken": token}},
                "toggledServiceEndpoint": {"feedbackEndpoint": {
                    "feedbackToken": "ROW-REMOVE"}},
                "isToggled": toggled
            }})
        }
        fn header(key: &str, buttons: Value) -> Value {
            let mut head = Map::new();
            head.insert("title".to_owned(), json!({"runs": [{"text": "x"}]}));
            head.insert("buttons".to_owned(), buttons);
            let mut row_menu = Map::new();
            row_menu.insert(
                "items".to_owned(),
                Value::Array(vec![bookmark("ROW-TOKEN", true)]),
            );
            let mut row = Map::new();
            row.insert("menuRenderer".to_owned(), Value::Object(row_menu));
            let mut item = Map::new();
            item.insert("menu".to_owned(), Value::Object(row));
            let mut renderer = Map::new();
            renderer.insert(
                "musicResponsiveListItemRenderer".to_owned(),
                Value::Object(item),
            );
            let mut shelf = Map::new();
            shelf.insert(
                "contents".to_owned(),
                Value::Array(vec![Value::Object(renderer)]),
            );
            let mut contents = Map::new();
            contents.insert(key.to_owned(), Value::Object(head));
            contents.insert("musicShelfRenderer".to_owned(), Value::Object(shelf));
            json!({"contents": [Value::Object(contents)]})
        }
        let buttons = json!([{"toggleButtonRenderer": {
            "defaultIcon": {"iconType": "BOOKMARK_BORDER"},
            "defaultServiceEndpoint": {"feedbackEndpoint": {"feedbackToken": "PAGE-SAVE"}},
            "toggledServiceEndpoint": {"feedbackEndpoint": {"feedbackToken": "PAGE-REMOVE"}},
            "isToggled": false
        }}]);
        let toggle = find_header_toggle(
            &header("musicDetailHeaderRenderer", buttons),
            "musicDetailHeaderRenderer",
        )
        .expect("the page toggle");
        assert!(!toggle.in_library);
        assert_eq!(toggle.add.as_deref(), Some("PAGE-SAVE"));
        assert_eq!(toggle.remove.as_deref(), Some("PAGE-REMOVE"));
        // A header without a bookmark button is no toggle, not an error — the rows'
        // bookmarks must not leak in.
        let bare = header("musicResponsiveHeaderRenderer", json!([]));
        assert!(find_header_toggle(&bare, "musicResponsiveHeaderRenderer").is_none());
        assert!(find_header_toggle(&bare, "musicDetailHeaderRenderer").is_none());
    }

    /// A playlist you can add songs to says so in its own menu. Liked Music — an auto
    /// playlist — and the "New playlist" tile do not, and both would otherwise have been
    /// offered in the save picker and refused by YouTube.
    #[test]
    fn only_editable_playlists_are_marked_editable() {
        let card = |menu: serde_json::Value| {
            serde_json::json!({"musicTwoRowItemRenderer": {
                "title": {"runs": [{"text": "anime bangers"}]},
                "subtitle": {"runs": [{"text": "IV • 29 tracks"}]},
                "menu": menu,
                "navigationEndpoint": {"browseEndpoint": {
                    "browseId": "VLPLabc",
                    "browseEndpointContextSupportedConfigs": {
                        "browseEndpointContextMusicConfig": {
                            "pageType": "MUSIC_PAGE_TYPE_PLAYLIST"}}}}}})
        };
        let editable = |c: &serde_json::Value| match parse_card(c) {
            Some(Entry::Playlist(p)) => p.editable,
            other => panic!("expected a playlist, got {other:?}"),
        };
        let owned = card(serde_json::json!({"menuRenderer": {"items": [
            {"menuNavigationItemRenderer": {"text": {"runs": [{"text": "Edit playlist"}]},
             "icon": {"iconType": "EDIT"}}},
            {"menuNavigationItemRenderer": {"text": {"runs": [{"text": "Share"}]},
             "icon": {"iconType": "SHARE"}}}]}}));
        assert!(editable(&owned));

        let auto = card(serde_json::json!({"menuRenderer": {"items": [
            {"menuNavigationItemRenderer": {"text": {"runs": [{"text": "Start mix"}]},
             "icon": {"iconType": "MIX"}}}]}}));
        assert!(!editable(&auto));

        assert!(!editable(&card(serde_json::json!({}))));
    }

    /// A card's subtitle is shaped by what the card is: a browse card leads with its kind
    /// ("Playlist • YouTube Music"), a library card with its owner ("IV • 29 tracks"). The old
    /// parse kept everything after the first field in `author`, which is the count on a library
    /// card — the rail's "creator" line was really the track count.
    #[test]
    fn a_playlist_card_splits_owner_from_count() {
        let card = |subtitle: &str| {
            serde_json::json!({"musicTwoRowItemRenderer": {
                "title": {"runs": [{"text": "anime bangers"}]},
                "subtitle": {"runs": [{"text": subtitle}]},
                "menu": {"menuRenderer": {"items": []}},
                "navigationEndpoint": {"browseEndpoint": {
                    "browseId": "VLPLabc",
                    "browseEndpointContextSupportedConfigs": {
                        "browseEndpointContextMusicConfig": {
                            "pageType": "MUSIC_PAGE_TYPE_PLAYLIST"}}}}}})
        };
        let detail = |subtitle: &str| match parse_card(&card(subtitle)) {
            Some(Entry::Playlist(p)) => (p.author, p.track_count),
            other => panic!("expected a playlist, got {other:?}"),
        };
        assert_eq!(
            detail("IV • 29 tracks"),
            ("IV".to_owned(), Some("29 tracks".to_owned()))
        );
        assert_eq!(
            detail("Playlist • YouTube Music"),
            ("YouTube Music".to_owned(), None)
        );
        assert_eq!(
            detail("Playlist • YouTube Music • 1,000 songs"),
            ("YouTube Music".to_owned(), Some("1,000 songs".to_owned()))
        );
        assert_eq!(
            detail("Playlist • 50 tracks"),
            (String::new(), Some("50 tracks".to_owned())),
            "a card with no owner still has a count"
        );
        assert_eq!(detail("Auto playlist"), (String::new(), None));
    }

    /// YouTube writes several artists inside one byline as separately-linked runs joined by
    /// plain `", "` and `" & "` text. Folding those into one field kept only the first browse id,
    /// which is why only the primary artist was ever clickable — anywhere.
    #[test]
    fn every_credited_artist_keeps_its_own_link() {
        let runs = serde_json::json!([
            {"text": "James Blake", "navigationEndpoint": {"browseEndpoint": {
                "browseId": "UC-A",
                "browseEndpointContextSupportedConfigs": {
                    "browseEndpointContextMusicConfig": {"pageType": "MUSIC_PAGE_TYPE_ARTIST"}}}}},
            {"text": ", "},
            {"text": "Ludwig Göransson", "navigationEndpoint": {"browseEndpoint": {
                "browseId": "UC-B",
                "browseEndpointContextSupportedConfigs": {
                    "browseEndpointContextMusicConfig": {"pageType": "MUSIC_PAGE_TYPE_ARTIST"}}}}},
            {"text": " & "},
            {"text": "Travis Scott", "navigationEndpoint": {"browseEndpoint": {
                "browseId": "UC-C",
                "browseEndpointContextSupportedConfigs": {
                    "browseEndpointContextMusicConfig": {"pageType": "MUSIC_PAGE_TYPE_ARTIST"}}}}},
            {"text": " • "},
            {"text": "12M views"},
        ]);
        let segs = segments_from_runs(runs.as_array().unwrap(), 1);
        let links = artist_links(&segs);
        assert_eq!(
            links
                .iter()
                .map(|a| (a.name.as_str(), a.id.as_deref()))
                .collect::<Vec<_>>(),
            vec![
                ("James Blake", Some("UC-A")),
                ("Ludwig Göransson", Some("UC-B")),
                ("Travis Scott", Some("UC-C")),
            ]
        );
        // The separators YouTube used are kept, so the line still reads the way it is written.
        assert_eq!(segs[1].sep, ", ");
        assert_eq!(segs[2].sep, " & ");
        // And the trailing view count is still its own field rather than being swallowed into
        // the last artist's name. (`plays_segment` will not claim it here: it ignores any column
        // that contains a link, which is what keeps an artist name from being read as a count.)
        assert_eq!(segs.len(), 4);
        assert_eq!(segs[3].text, "12M views");
        assert!(segs[3].target.is_none());
    }

    /// The same byline in a real response, so the synthetic test above cannot drift from what
    /// YouTube actually sends.
    #[test]
    fn a_real_three_artist_byline_yields_three_links() {
        let json: serde_json::Value =
            serde_json::from_str(include_str!("../fixtures/artist-travis-scott.json"))
                .expect("fixture parses");
        let card = find_card_with_runs(&json, "James Blake").expect("the three-artist card");
        let segs = segments_from_runs(&card, 1);
        let links = artist_links(&segs);
        assert_eq!(links.len(), 3, "{links:?}");
        assert!(links.iter().all(|a| a.id.is_some()));
    }

    /// Find a subtitle run list containing `needle`, wherever it sits in a response.
    fn find_card_with_runs(v: &serde_json::Value, needle: &str) -> Option<Vec<serde_json::Value>> {
        match v {
            serde_json::Value::Object(map) => {
                if let Some(runs) = map.get("runs").and_then(|r| r.as_array())
                    && runs
                        .iter()
                        .any(|r| r.get("text").and_then(|t| t.as_str()) == Some(needle))
                {
                    return Some(runs.clone());
                }
                map.values().find_map(|v| find_card_with_runs(v, needle))
            }
            serde_json::Value::Array(items) => {
                items.iter().find_map(|v| find_card_with_runs(v, needle))
            }
            _ => None,
        }
    }

    /// A playlist row carries its item id; anything else does not. Only the id makes "Remove
    /// from playlist" offerable.
    #[test]
    fn a_playlist_row_carries_its_item_id() {
        let row = serde_json::json!({"musicResponsiveListItemRenderer": {
        "flexColumns": [
            {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [
                {"text": "Nude", "navigationEndpoint": {"watchEndpoint": {"videoId": "vvvvvvvvvvv",
                    "watchEndpointMusicSupportedConfigs": {"watchEndpointMusicConfig":
                        {"musicVideoType": "MUSIC_VIDEO_TYPE_ATV"}}}}}]}}},
            {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [
                {"text": "Radiohead"}]}}}
        ],
        "playlistItemData": {"playlistSetVideoId": "ITEM-1", "videoId": "vvvvvvvvvvv"}}});
        let t = parse_search_row(row.get("musicResponsiveListItemRenderer").unwrap())
            .expect("the row should parse")
            .track()
            .expect("a playable track")
            .clone();
        assert_eq!(t.set_video_id.as_deref(), Some("ITEM-1"));
        let bare = serde_json::json!({"musicResponsiveListItemRenderer": {
        "flexColumns": [
            {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [
                {"text": "Nude", "navigationEndpoint": {"watchEndpoint": {"videoId": "vvvvvvvvvvv",
                    "watchEndpointMusicSupportedConfigs": {"watchEndpointMusicConfig":
                        {"musicVideoType": "MUSIC_VIDEO_TYPE_ATV"}}}}}]}}},
            {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [
                {"text": "Radiohead"}]}}}
        ]}});
        let t = parse_search_row(bare.get("musicResponsiveListItemRenderer").unwrap())
            .expect("the row should parse")
            .track()
            .expect("a playable track")
            .clone();
        assert_eq!(t.set_video_id, None);
    }

    /// Signed out, VLLM does not come back as an empty shell: it is a
    /// `singleColumnBrowseResultsRenderer` holding a `messageRenderer` and nothing else. Without
    /// recognising that shape the page looks like content the typed fallback could read, and it
    /// dies on its two-column key instead of reporting the session.
    #[test]
    fn the_sign_in_prompt_is_told_from_a_page() {
        let prompt = serde_json::json!({"contents": {"singleColumnBrowseResultsRenderer": {
        "tabs": [{"tabRenderer": {"content": {"sectionListRenderer": {"contents": [
            {"messageRenderer": {"text": {"runs": [{"text": "Looking for what you’ve liked?"}]}}}
        ]}}}}]}}});
        assert!(sign_in_prompt(&prompt), "the sign-in prompt went unseen");
        assert!(parse_playlist(&prompt).is_none());

        // A real page is not a prompt, even with an unrelated `messageRenderer` somewhere.
        let page = serde_json::json!({
            "contents": [{"musicResponsiveHeaderRenderer": {"title": {"runs": [{"text": "Liked Music"}]}}}],
            "messageRenderer": {"text": {"simpleText": "note"}}
        });
        assert!(!sign_in_prompt(&page));

        // Nor is a row-only response with no header at all.
        let rows = serde_json::json!({"contents": {"singleColumnBrowseResultsRenderer": {
        "tabs": [{"tabRenderer": {"content": {"sectionListRenderer": {"contents": [
            {"musicPlaylistShelfRenderer": {"contents": [
                {"musicResponsiveListItemRenderer": {"flexColumns": []}}
            ]}}
        ]}}}}]}}});
        assert!(!sign_in_prompt(&rows), "a row-only page is not a prompt");
    }

    /// Owned pages wrap their header in `musicEditablePlaylistDetailHeaderRenderer`; anything
    /// else — LM, subscribed lists — does not, and "Remove from playlist" stays hidden.
    #[test]
    fn the_editable_header_marks_owned_playlists() {
        let owned = serde_json::json!({"contents": [
            {"musicEditablePlaylistDetailHeaderRenderer": {"header": {
                "musicResponsiveHeaderRenderer": {
                    "title": {"runs": [{"text": "Mine"}]}}}}}]
        });
        let page = parse_playlist(&owned).expect("the owned page should parse");
        assert_eq!(page.title, "Mine");
        assert!(page.editable, "the editable header went unseen");
        let others = serde_json::json!({"contents": [
            {"musicResponsiveHeaderRenderer": {
                "title": {"runs": [{"text": "Theirs"}]}}}]
        });
        let page = parse_playlist(&others).expect("the page should parse");
        assert!(!page.editable);
    }

    /// A public playlist's subtitle names only its kind and year; the owner lives in the
    /// header's facepile, and that is the name the page header links to their profile.
    #[test]
    fn the_facepile_names_the_owner() {
        let page = serde_json::json!({"contents": [
        {"musicResponsiveHeaderRenderer": {
            "title": {"runs": [{"text": "chill lofi"}]},
            "facepile": {"avatarStackViewModel": {"avatars": [{}],
                "rendererContext": {"commandContext": {"onTap": {"innertubeCommand":
                    {"browseEndpoint": {
                        "browseEndpointContextSupportedConfigs":
                            {"browseEndpointContextMusicConfig":
                                {"pageType": "MUSIC_PAGE_TYPE_USER_CHANNEL"}},
                        "browseId": "UCawzzfqDLFQLrXKvrEnrWKw"}}}}},
                "text": {"content": "Manop Silaket"}}}}}]});
        let parsed = parse_playlist(&page).expect("the page should parse");
        assert_eq!(parsed.author.as_deref(), Some("Manop Silaket"));
        assert_eq!(
            parsed.author_id.as_deref(),
            Some("UCawzzfqDLFQLrXKvrEnrWKw"),
            "the facepile's tap target is the profile"
        );

        let anonymous = serde_json::json!({"contents": [
        {"musicResponsiveHeaderRenderer": {
            "title": {"runs": [{"text": "chill lofi"}]}}}]});
        let parsed = parse_playlist(&anonymous).expect("the page should parse");
        assert_eq!(parsed.author, None, "no facepile, no author");
        assert_eq!(parsed.author_id, None, "no facepile, no link");
    }

    /// A playlist's description arrives either as plain runs or wrapped in a description shelf;
    /// the wrapper is the shape that made a described playlist show nothing.
    #[test]
    fn a_playlists_description_survives_both_shapes() {
        let nested = serde_json::json!({"contents": [
        {"musicResponsiveHeaderRenderer": {
            "title": {"runs": [{"text": "x"}]},
            "description": {"musicDescriptionShelfRenderer": {"description":
                {"runs": [{"text": "calm your anxiety, relax music"}]}}}}}]});
        assert_eq!(
            parse_playlist(&nested).unwrap().description.as_deref(),
            Some("calm your anxiety, relax music")
        );
        let plain = serde_json::json!({"contents": [
        {"musicResponsiveHeaderRenderer": {
            "title": {"runs": [{"text": "x"}]},
            "description": {"runs": [{"text": "plain text"}]}}}]});
        assert_eq!(
            parse_playlist(&plain).unwrap().description.as_deref(),
            Some("plain text")
        );
    }

    /// An empty shell has no header and no rows: it parses to nothing, which is what sends
    /// the playlist down the empty-shell error instead of the typed queries.
    #[test]
    fn an_empty_shell_parses_to_nothing() {
        let empty = serde_json::json!({"contents": {}});
        assert!(parse_playlist(&empty).is_none());
    }
}
