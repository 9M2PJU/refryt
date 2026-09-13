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

use crate::{AudioStream, Codec, StreamResolver, VideoId, parse_expiry};

/// Prefer Opus, falling back to whatever the best audio-only stream is.
const DEFAULT_FORMAT: &str = "bestaudio[acodec=opus]/bestaudio";

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
        tokio::process::Command::new(&self.program)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
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
    url: String,
    #[serde(default)]
    acodec: Option<String>,
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
}

#[async_trait::async_trait]
impl StreamResolver for YtDlpResolver {
    fn name(&self) -> &'static str {
        "yt-dlp"
    }

    async fn resolve(&self, id: &VideoId) -> Result<AudioStream> {
        let mut cmd = tokio::process::Command::new(&self.program);
        let format = self
            .format
            .lock()
            .map(|f| f.clone())
            .unwrap_or_else(|_| DEFAULT_FORMAT.to_owned());
        cmd.args(["-j", "--no-warnings", "--no-playlist", "-f", &format])
            .args(&self.extra_args)
            .arg(id.watch_url())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let output = tokio::time::timeout(self.timeout, cmd.output())
            .await
            .with_context(|| format!("yt-dlp timed out resolving {id}"))?
            .with_context(|| format!("failed to run {}", self.program))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("yt-dlp failed for {id}: {}", stderr.trim());
        }

        let parsed: YtDlpFormat =
            serde_json::from_slice(&output.stdout).context("could not parse yt-dlp JSON output")?;

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
}
