//! Ordered fallback across several resolvers, with a circuit breaker.
//!
//! The breaker matters more than it looks. When YouTube changes the player, the native
//! resolver does not fail once, it fails for every track for the rest of the session. Without
//! a breaker each play would pay the full native timeout before falling back, making a
//! degraded app feel broken. After a few consecutive failures we stop asking until a cooldown
//! elapses, so the cost of a dead fast path is bounded.
//!
//! Two limits keep it from doing harm. A failure that belongs to one video — YouTube wanting a
//! sign-in for it, the video being gone or private — says nothing about the resolver and is not
//! counted. And the last resolver is never skipped: there is nothing after it to route to, so
//! skipping it (the whole chain, by default) only turned one bad track into ten minutes of
//! failures for every track after it, which is what the log showed on 2026-09-22 and -23.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use tracing::{debug, warn};

#[cfg(feature = "native-resolver")]
use crate::NativeResolver;
use crate::{AudioStream, StreamResolver, VideoId, YtDlpResolver, is_sign_in_gate};

/// Consecutive failures before a resolver is skipped.
const FAILURES_BEFORE_TRIP: u32 = 3;
/// How long a tripped resolver is skipped before being retried once.
const COOLDOWN: Duration = Duration::from_secs(600);

/// Whether a failure is about this one video rather than the resolver: the next video has no
/// reason to fail the same way, so the breaker does not count it.
fn is_video_specific(error: &anyhow::Error) -> bool {
    if is_sign_in_gate(error) {
        return true;
    }
    let text = format!("{error:#}");
    [
        "Video unavailable",
        "Private video",
        "This video has been removed",
    ]
    .iter()
    .any(|reason| text.contains(reason))
}

#[derive(Debug, Default)]
struct Breaker {
    consecutive_failures: u32,
    tripped_at: Option<Instant>,
}

impl Breaker {
    fn should_skip(&self) -> bool {
        match self.tripped_at {
            Some(at) => at.elapsed() < COOLDOWN,
            None => false,
        }
    }

    fn record_success(&mut self) {
        self.consecutive_failures = 0;
        self.tripped_at = None;
    }

    fn record_failure(&mut self) -> bool {
        self.consecutive_failures += 1;
        if self.consecutive_failures >= FAILURES_BEFORE_TRIP && self.tripped_at.is_none() {
            self.tripped_at = Some(Instant::now());
            return true;
        }
        // A tripped breaker whose cooldown has expired gets one more chance; if that fails
        // too, restart the cooldown rather than hammering.
        if let Some(at) = self.tripped_at
            && at.elapsed() >= COOLDOWN
        {
            self.tripped_at = Some(Instant::now());
        }
        false
    }
}

pub struct ChainResolver {
    /// Kept alongside the boxed chain so an audio-quality change can retune it without rebuilding
    /// the chain, which lives behind an `Arc` shared with in-flight resolves.
    ytdlp: std::sync::Arc<YtDlpResolver>,
    resolvers: Vec<Box<dyn StreamResolver>>,
    breakers: Mutex<HashMap<&'static str, Breaker>>,
}

impl ChainResolver {
    /// Resolvers are tried in the order given: fastest first, most reliable last.
    pub fn new(resolvers: Vec<Box<dyn StreamResolver>>) -> Self {
        Self {
            ytdlp: std::sync::Arc::new(YtDlpResolver::new()),
            resolvers,
            breakers: Mutex::new(HashMap::new()),
        }
    }

    /// The order to use today.
    ///
    /// yt-dlp goes first, which inverts the intended design. As of the M1 gate the native
    /// resolver cannot produce playable URLs at all (see `native.rs`), so putting it first
    /// would make every track pay its latency before falling through.
    ///
    /// Without the `native-resolver` feature this is a chain of one. That feature is off by
    /// default because the native path costs the Boa JS engine — the largest dependency in the
    /// workspace — to sit second in a chain and fail its own validity check.
    pub fn default_order(ytdlp: YtDlpResolver) -> Self {
        let ytdlp = std::sync::Arc::new(ytdlp);
        #[allow(unused_mut)]
        let mut resolvers: Vec<Box<dyn StreamResolver>> =
            vec![Box::new(std::sync::Arc::clone(&ytdlp))];
        #[cfg(feature = "native-resolver")]
        resolvers.push(Box::new(
            NativeResolver::new(crate::NativeConfig::default()),
        ));
        Self {
            ytdlp,
            resolvers,
            breakers: Mutex::new(HashMap::new()),
        }
    }

    /// Retune what `yt-dlp` is asked for, for the audio-quality preference.
    pub fn set_format(&self, format: &str) {
        self.ytdlp.set_format(format);
    }

    /// The intended order, for when the native resolver is healthy again.
    #[cfg(feature = "native-resolver")]
    pub fn native_first(native: NativeResolver, ytdlp: YtDlpResolver) -> Self {
        Self::new(vec![Box::new(native), Box::new(ytdlp)])
    }

    fn should_skip(&self, name: &'static str) -> bool {
        self.breakers
            .lock()
            .unwrap()
            .get(name)
            .map(Breaker::should_skip)
            .unwrap_or(false)
    }

    fn record_success(&self, name: &'static str) {
        self.breakers
            .lock()
            .unwrap()
            .entry(name)
            .or_default()
            .record_success();
    }

    fn record_failure(&self, name: &'static str) {
        let newly_tripped = {
            let mut breakers = self.breakers.lock().unwrap();
            breakers.entry(name).or_default().record_failure()
        };
        if newly_tripped {
            warn!(
                resolver = name,
                cooldown_secs = COOLDOWN.as_secs(),
                "resolver failed repeatedly; skipping it until the cooldown elapses"
            );
        }
    }
}

#[async_trait::async_trait]
impl StreamResolver for ChainResolver {
    fn name(&self) -> &'static str {
        "chain"
    }

    async fn resolve(&self, id: &VideoId) -> Result<AudioStream> {
        let mut errors = Vec::new();

        let last = self.resolvers.len().saturating_sub(1);
        for (i, resolver) in self.resolvers.iter().enumerate() {
            let name = resolver.name();
            if i < last && self.should_skip(name) {
                debug!(
                    resolver = name,
                    "skipping resolver, circuit breaker is open"
                );
                continue;
            }

            match resolver.resolve(id).await {
                Ok(stream) => {
                    self.record_success(name);
                    debug!(
                        resolver = name,
                        itag = stream.itag,
                        codec = %stream.codec,
                        bitrate_kbps = stream.bitrate_kbps,
                        "resolved"
                    );
                    return Ok(stream);
                }
                Err(e) => {
                    warn!(resolver = name, error = format!("{e:#}"), "resolver failed");
                    if !is_video_specific(&e) {
                        self.record_failure(name);
                    }
                    errors.push(format!("{name}: {e:#}"));
                }
            }
        }

        // Only an empty chain gets here with no errors: the last resolver is never skipped.
        if errors.is_empty() {
            bail!("every resolver is in cooldown after repeated failures; cannot resolve {id}");
        }
        bail!("all resolvers failed for {id}:\n  {}", errors.join("\n  "));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Codec;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use url::Url;

    struct Stub {
        name: &'static str,
        fail: bool,
        error: &'static str,
        calls: AtomicUsize,
    }

    impl Stub {
        fn new(name: &'static str, fail: bool) -> Self {
            Self {
                name,
                fail,
                error: "stub failure",
                calls: AtomicUsize::new(0),
            }
        }

        fn failing_with(name: &'static str, error: &'static str) -> Self {
            Self {
                error,
                ..Self::new(name, true)
            }
        }
    }

    #[async_trait::async_trait]
    impl StreamResolver for std::sync::Arc<Stub> {
        fn name(&self) -> &'static str {
            self.name
        }
        async fn resolve(&self, _id: &VideoId) -> Result<AudioStream> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                bail!("{}", self.error);
            }
            Ok(AudioStream {
                url: Url::parse("https://example.com/a").unwrap(),
                itag: 251,
                codec: Codec::Opus,
                bitrate_kbps: 160,
                content_length: None,
                duration: None,
                expires_at: None,
                loudness_db: None,
                resolved_by: "stub",
            })
        }
    }

    fn id() -> VideoId {
        VideoId::new("dQw4w9WgXcQ").unwrap()
    }

    #[tokio::test]
    async fn falls_through_to_the_next_resolver() {
        let bad = std::sync::Arc::new(Stub::new("bad", true));
        let good = std::sync::Arc::new(Stub::new("good", false));
        let chain = ChainResolver::new(vec![Box::new(bad.clone()), Box::new(good.clone())]);

        let s = chain.resolve(&id()).await.unwrap();
        assert_eq!(s.codec, Codec::Opus);
        assert_eq!(bad.calls.load(Ordering::SeqCst), 1);
        assert_eq!(good.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn breaker_stops_calling_a_persistently_failing_resolver() {
        let bad = std::sync::Arc::new(Stub::new("bad", true));
        let good = std::sync::Arc::new(Stub::new("good", false));
        let chain = ChainResolver::new(vec![Box::new(bad.clone()), Box::new(good.clone())]);

        for _ in 0..10 {
            chain.resolve(&id()).await.unwrap();
        }

        // Tripped after FAILURES_BEFORE_TRIP, then skipped for the rest of the run.
        assert_eq!(
            bad.calls.load(Ordering::SeqCst),
            FAILURES_BEFORE_TRIP as usize
        );
        assert_eq!(good.calls.load(Ordering::SeqCst), 10);
    }

    #[tokio::test]
    async fn success_resets_the_failure_count() {
        let flaky = std::sync::Arc::new(Stub::new("flaky", true));
        let chain = ChainResolver::new(vec![Box::new(flaky.clone())]);

        assert!(chain.resolve(&id()).await.is_err());
        chain.record_success("flaky");

        let breakers = chain.breakers.lock().unwrap();
        assert_eq!(breakers["flaky"].consecutive_failures, 0);
    }

    #[tokio::test]
    async fn reports_every_error_when_all_fail() {
        let a = std::sync::Arc::new(Stub::new("a", true));
        let b = std::sync::Arc::new(Stub::new("b", true));
        let chain = ChainResolver::new(vec![Box::new(a), Box::new(b)]);

        let err = chain.resolve(&id()).await.unwrap_err().to_string();
        assert!(err.contains("a: "), "{err}");
        assert!(err.contains("b: "), "{err}");
    }

    /// With nothing after it to route to, the last resolver is asked every time however often
    /// it has failed — skipping it would only fail every track after one bad one.
    #[tokio::test]
    async fn the_last_resolver_is_never_skipped() {
        let only = std::sync::Arc::new(Stub::new("only", true));
        let chain = ChainResolver::new(vec![Box::new(only.clone())]);

        for _ in 0..10 {
            let err = chain.resolve(&id()).await.unwrap_err().to_string();
            assert!(err.contains("only: "), "{err}");
        }
        assert_eq!(only.calls.load(Ordering::SeqCst), 10);
    }

    /// A video YouTube will not hand over anonymously says nothing about the resolver, so a run
    /// of them does not trip its breaker.
    #[tokio::test]
    async fn failures_about_one_video_do_not_trip_the_breaker() {
        let gated = std::sync::Arc::new(Stub::failing_with(
            "gated",
            "ERROR: [youtube] x: Sign in to confirm you’re not a bot.",
        ));
        let good = std::sync::Arc::new(Stub::new("good", false));
        let chain = ChainResolver::new(vec![Box::new(gated.clone()), Box::new(good)]);

        for _ in 0..10 {
            chain.resolve(&id()).await.unwrap();
        }
        assert_eq!(gated.calls.load(Ordering::SeqCst), 10);
    }
}
