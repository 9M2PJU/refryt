//! Fallback resolver that shells out to `yt-dlp`.
//!
//! Slower (~1-2s, dominated by Python startup) but maintained by people who fight YouTube's
//! arms race full time, and it carries support for PO token providers and the SABR
//! downloader. This is what keeps the application alive on the day the native path breaks,
//! without us having to ship a release.

use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use url::Url;

use crate::{
    AudioStream, Codec, StreamResolver, VideoId, VideoResolver, VideoStream, parse_expiry,
};

/// Prefer Opus, falling back to whatever the best audio-only stream is.
const DEFAULT_FORMAT: &str = "bestaudio[acodec=opus]/bestaudio";

/// The in-app video view's selector: 720p at most, H.264 before anything else.
///
/// The view renders in software, so this is the ceiling that keeps decoding and colour
/// conversion affordable — H.264 is far cheaper to software-decode than VP9 or AV1, and 720p
/// is what the 1280px-wide frame buffer can actually show. The later branches only matter when
/// a video has no AVC rendition at all.
pub const VIDEO_FORMAT: &str = "bestvideo[height<=720][vcodec^=avc1]+bestaudio[acodec^=opus]\
/bestvideo[height<=720]+bestaudio/best[height<=720]";

/// `CREATE_NO_WINDOW`: without it, every spawn of a console-subsystem binary (yt-dlp is one)
/// from this GUI-subsystem app makes Windows allocate it a brand new console, which flashes on
/// screen for the length of the call — once per track resolved. Linux/macOS have no such
/// concept, so this is a no-op there.
#[cfg(windows)]
fn no_console(cmd: &mut tokio::process::Command) -> &mut tokio::process::Command {
    cmd.creation_flags(0x0800_0000)
}
#[cfg(not(windows))]
fn no_console(cmd: &mut tokio::process::Command) -> &mut tokio::process::Command {
    cmd
}

pub struct YtDlpResolver {
    program: String,
    /// Extra arguments, e.g. a PO token provider or a cookies file.
    extra_args: Vec<String>,
    /// The `-f` selector. Changeable at runtime so an audio-quality preference is a real change
    /// to what gets fetched rather than a label; read per resolve, so no rebuild is needed.
    format: std::sync::Mutex<String>,
    timeout: Duration,
}

impl YtDlpResolver {
    pub fn new() -> Self {
        Self {
            program: "yt-dlp".to_owned(),
            extra_args: Vec::new(),
            format: std::sync::Mutex::new(DEFAULT_FORMAT.to_owned()),
            timeout: Duration::from_secs(30),
        }
    }

    pub fn with_program(mut self, program: impl Into<String>) -> Self {
        self.program = program.into();
        self
    }

    /// Start out asking for something other than the default audio-only selector — the video
    /// view's own resolver is built with [`VIDEO_FORMAT`].
    pub fn with_format(self, format: &str) -> Self {
        self.set_format(format);
        self
    }

    /// Replace the format selector. Takes `&self`: the resolver lives behind an `Arc` in the
    /// chain, and a preference change must not require rebuilding it.
    pub fn set_format(&self, format: &str) {
        if let Ok(mut f) = self.format.lock() {
            format.clone_into(&mut f);
        }
    }

    pub fn with_extra_args(mut self, args: Vec<String>) -> Self {
        self.extra_args = args;
        self
    }

    /// Whether the configured yt-dlp is actually present, so the UI can warn early rather
    /// than at the moment playback is wanted.
    pub async fn is_available(&self) -> bool {
        let mut cmd = tokio::process::Command::new(&self.program);
        cmd.arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        no_console(&mut cmd)
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

impl Default for YtDlpResolver {
    fn default() -> Self {
        Self::new()
    }
}

/// The subset of yt-dlp's `-j` output we care about.
#[derive(Debug, Deserialize)]
struct YtDlpFormat {
    /// Absent entirely when the selection was `bestvideo+bestaudio`: yt-dlp then prints the
    /// merged selection with only `requested_formats` carrying URLs. Audio-only payloads always
    /// have it, and `resolve` still rejects an empty one.
    #[serde(default)]
    url: String,
    #[serde(default)]
    acodec: Option<String>,
    #[serde(default)]
    vcodec: Option<String>,
    #[serde(default)]
    abr: Option<f64>,
    #[serde(default)]
    tbr: Option<f64>,
    #[serde(default)]
    format_id: Option<String>,
    #[serde(default)]
    filesize: Option<u64>,
    #[serde(default)]
    filesize_approx: Option<u64>,
    #[serde(default)]
    duration: Option<f64>,
    /// With a `bestvideo+bestaudio` selector, `-j` prints the merged selection's top-level
    /// fields and lists the streams it picked here. The top-level `url` is empty in that case,
    /// so this is the only place the playable URLs exist.
    #[serde(default)]
    requested_formats: Option<Vec<YtDlpFormat>>,
}

impl YtDlpResolver {
    /// Ask yt-dlp for one format and parse its `-j` output.
    async fn run_json(&self, id: &VideoId, format: &str) -> Result<YtDlpFormat> {
        let mut cmd = tokio::process::Command::new(&self.program);
        cmd.args(["-j", "--no-warnings", "--no-playlist", "-f", format])
            .args(&self.extra_args)
            .arg(id.watch_url())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        no_console(&mut cmd);

        let output = tokio::time::timeout(self.timeout, cmd.output())
            .await
            .with_context(|| format!("yt-dlp timed out resolving {id}"))?
            .with_context(|| format!("failed to run {}", self.program))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("yt-dlp failed for {id}: {}", stderr.trim());
        }

        serde_json::from_slice(&output.stdout).context("could not parse yt-dlp JSON output")
    }
}

#[async_trait::async_trait]
impl StreamResolver for YtDlpResolver {
    fn name(&self) -> &'static str {
        "yt-dlp"
    }

    async fn resolve(&self, id: &VideoId) -> Result<AudioStream> {
        let format = self
            .format
            .lock()
            .map(|f| f.clone())
            .unwrap_or_else(|_| DEFAULT_FORMAT.to_owned());
        let parsed = self.run_json(id, &format).await?;

        if parsed.url.is_empty() {
            bail!("yt-dlp returned no URL for {id}");
        }

        let codec = parsed
            .acodec
            .as_deref()
            .map(Codec::from_codec_str)
            .unwrap_or(Codec::Other);

        let url = Url::parse(&parsed.url)
            .map_err(|e| anyhow!("yt-dlp returned an unparseable URL for {id}: {e}"))?;

        Ok(AudioStream {
            itag: parsed
                .format_id
                .as_deref()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
            codec,
            bitrate_kbps: parsed.abr.or(parsed.tbr).unwrap_or(0.0).round() as u32,
            content_length: parsed.filesize.or(parsed.filesize_approx),
            duration: parsed.duration.map(Duration::from_secs_f64),
            expires_at: parse_expiry(&url),
            // yt-dlp does not surface YouTube's loudness measurement in this shape; mpv
            // falls back to its own ReplayGain handling when this is absent.
            loudness_db: None,
            url,
            resolved_by: "yt-dlp",
        })
    }
}

/// Turn one parsed `-j` payload into a [`VideoStream`].
///
/// Split out from the command so the parsing — which is the fragile half — is testable without
/// a network call. `requested_formats` is the normal shape for a `+` selector: the video and
/// audio URLs live there, each marked by which codec it has. A single entry (a progressive
/// format, or a video with no separate audio) falls back to the top-level `url`.
fn video_stream_from(parsed: &YtDlpFormat, id: &VideoId) -> Result<VideoStream> {
    let has_video = |f: &YtDlpFormat| f.vcodec.as_deref().is_some_and(|c| c != "none");
    let has_audio = |f: &YtDlpFormat| f.acodec.as_deref().is_some_and(|c| c != "none");

    let mut video = None;
    let mut audio = None;
    if let Some(formats) = &parsed.requested_formats {
        for f in formats {
            let url = Url::parse(&f.url)
                .map_err(|e| anyhow!("yt-dlp returned an unparseable video URL for {id}: {e}"))?;
            if has_video(f) {
                video = Some(url);
            } else if has_audio(f) {
                audio = Some(url);
            }
        }
    }
    let video = match video {
        Some(url) => url,
        None => {
            if parsed.url.is_empty() || !has_video(parsed) {
                bail!("yt-dlp returned no video stream for {id}");
            }
            Url::parse(&parsed.url)
                .map_err(|e| anyhow!("yt-dlp returned an unparseable video URL for {id}: {e}"))?
        }
    };
    Ok(VideoStream {
        expires_at: parse_expiry(&video),
        video,
        audio,
        duration: parsed.duration.map(Duration::from_secs_f64),
        resolved_by: "yt-dlp",
    })
}

#[async_trait::async_trait]
impl VideoResolver for YtDlpResolver {
    fn name(&self) -> &'static str {
        "yt-dlp"
    }

    async fn resolve_video(&self, id: &VideoId) -> Result<VideoStream> {
        let parsed = self.run_json(id, VIDEO_FORMAT).await?;
        video_stream_from(&parsed, id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_representative_yt_dlp_payload() {
        let json = r#"{
            "url": "https://r5.googlevideo.com/videoplayback?expire=1757200000&itag=251",
            "acodec": "opus",
            "abr": 159.99,
            "format_id": "251",
            "filesize": 3456789,
            "duration": 213.5
        }"#;
        let f: YtDlpFormat = serde_json::from_str(json).unwrap();
        assert_eq!(f.format_id.as_deref(), Some("251"));
        assert_eq!(
            Codec::from_codec_str(f.acodec.as_deref().unwrap()),
            Codec::Opus
        );
        assert_eq!(f.abr.unwrap().round() as u32, 160);
    }

    #[test]
    fn tolerates_a_payload_missing_optional_fields() {
        let json = r#"{"url": "https://example.com/x"}"#;
        let f: YtDlpFormat = serde_json::from_str(json).unwrap();
        assert!(f.acodec.is_none());
        assert!(f.filesize.is_none());
    }

    fn id() -> VideoId {
        VideoId::new("dQw4w9WgXcQ").unwrap()
    }

    /// The shape a `bestvideo+bestaudio` selection actually prints: the top-level `url` is
    /// empty and the playable streams are the two entries in `requested_formats`, marked by
    /// which codec each one has. This is the payload the video view depends on.
    #[test]
    fn a_requested_formats_payload_splits_video_and_audio() {
        // No top-level `url` at all — the real shape of a `+` selection.
        let json = r#"{
            "duration": 213.5,
            "requested_formats": [
                {"url": "https://r1.googlevideo.com/videoplayback?expire=1757200000&itag=136",
                 "vcodec": "avc1.4d401f", "acodec": "none", "format_id": "136", "height": 720},
                {"url": "https://r1.googlevideo.com/videoplayback?expire=1757200000&itag=251",
                 "vcodec": "none", "acodec": "opus", "format_id": "251"}
            ]
        }"#;
        let parsed: YtDlpFormat = serde_json::from_str(json).unwrap();
        let stream = video_stream_from(&parsed, &id()).unwrap();
        assert!(stream.video.as_str().contains("itag=136"), "{stream:?}");
        assert!(
            stream
                .audio
                .as_ref()
                .is_some_and(|a| a.as_str().contains("itag=251")),
            "{stream:?}"
        );
        assert_eq!(stream.duration, Some(Duration::from_secs_f64(213.5)));
        assert!(stream.expires_at.is_some());
    }

    /// A muxed fallback has both codecs on the one top-level URL, so there is no external audio.
    #[test]
    fn a_progressive_payload_needs_no_external_audio() {
        let json = r#"{
            "url": "https://r1.googlevideo.com/videoplayback?expire=1757200000&itag=18",
            "vcodec": "avc1.42001E", "acodec": "mp4a.40.2"
        }"#;
        let parsed: YtDlpFormat = serde_json::from_str(json).unwrap();
        let stream = video_stream_from(&parsed, &id()).unwrap();
        assert!(stream.audio.is_none());
    }

    /// If the selector somehow lands on an audio-only stream, that is not a video and must be
    /// an error rather than a "video" that is really sound.
    #[test]
    fn an_audio_only_payload_is_not_a_video() {
        let json = r#"{
            "url": "https://r1.googlevideo.com/videoplayback?itag=251",
            "vcodec": "none", "acodec": "opus"
        }"#;
        let parsed: YtDlpFormat = serde_json::from_str(json).unwrap();
        assert!(video_stream_from(&parsed, &id()).is_err());
    }
}
