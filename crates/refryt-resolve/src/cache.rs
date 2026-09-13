//! In-memory cache of resolved URLs.
//!
//! YouTube's stream URLs carry an `expire` parameter, typically about six hours out, so
//! within a listening session a track resolved once need not be resolved again. This turns
//! replays and queue reordering into zero-latency operations and takes load off the fragile
//! resolver layer.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Result;

use crate::{AudioStream, StreamResolver, VideoId};

/// Re-resolve this long before actual expiry: a URL that dies mid-track is worse than one
/// refreshed slightly early.
const FRESHNESS_MARGIN: Duration = Duration::from_secs(10 * 60);

pub struct StreamCache<R> {
    inner: R,
    entries: Mutex<HashMap<VideoId, AudioStream>>,
}

impl<R: StreamResolver> StreamCache<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// The resolver underneath, so callers can retune it.
    pub fn inner(&self) -> &R {
        &self.inner
    }

    /// Forget every cached URL. Needed when the *kind* of stream we want changes — a cached
    /// high-quality URL would otherwise keep playing after switching to a lower one.
    pub fn clear(&self) {
        self.entries.lock().unwrap().clear();
    }

    pub fn get_cached(&self, id: &VideoId) -> Option<AudioStream> {
        let entries = self.entries.lock().unwrap();
        entries
            .get(id)
            .filter(|s| s.is_fresh(FRESHNESS_MARGIN))
            .cloned()
    }

    /// Drop expired entries. Cheap; call when convenient.
    pub fn evict_stale(&self) {
        let mut entries = self.entries.lock().unwrap();
        entries.retain(|_, s| s.is_fresh(FRESHNESS_MARGIN));
    }

    pub fn len(&self) -> usize {
        self.entries.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[async_trait::async_trait]
impl<R: StreamResolver> StreamResolver for StreamCache<R> {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    async fn resolve(&self, id: &VideoId) -> Result<AudioStream> {
        if let Some(hit) = self.get_cached(id) {
            return Ok(hit);
        }
        let stream = self.inner.resolve(id).await?;
        self.entries
            .lock()
            .unwrap()
            .insert(id.clone(), stream.clone());
        Ok(stream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Codec;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::SystemTime;
    use url::Url;

    struct Counting {
        calls: AtomicUsize,
        expires_in: Option<Duration>,
    }

    #[async_trait::async_trait]
    impl StreamResolver for std::sync::Arc<Counting> {
        fn name(&self) -> &'static str {
            "counting"
        }
        async fn resolve(&self, _id: &VideoId) -> Result<AudioStream> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(AudioStream {
                url: Url::parse("https://example.com/a").unwrap(),
                itag: 251,
                codec: Codec::Opus,
                bitrate_kbps: 160,
                content_length: None,
                duration: None,
                expires_at: self.expires_in.map(|d| SystemTime::now() + d),
                loudness_db: None,
                resolved_by: "counting",
            })
        }
    }

    fn id() -> VideoId {
        VideoId::new("dQw4w9WgXcQ").unwrap()
    }

    #[tokio::test]
    async fn second_resolve_is_served_from_cache() {
        let inner = std::sync::Arc::new(Counting {
            calls: AtomicUsize::new(0),
            expires_in: Some(Duration::from_secs(6 * 3600)),
        });
        let cache = StreamCache::new(inner.clone());

        cache.resolve(&id()).await.unwrap();
        cache.resolve(&id()).await.unwrap();

        assert_eq!(inner.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_url_near_expiry_is_re_resolved() {
        let inner = std::sync::Arc::new(Counting {
            calls: AtomicUsize::new(0),
            // Inside the freshness margin, so it must not be reused.
            expires_in: Some(Duration::from_secs(60)),
        });
        let cache = StreamCache::new(inner.clone());

        cache.resolve(&id()).await.unwrap();
        cache.resolve(&id()).await.unwrap();

        assert_eq!(inner.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn evict_stale_drops_expired_entries() {
        let inner = std::sync::Arc::new(Counting {
            calls: AtomicUsize::new(0),
            expires_in: Some(Duration::from_secs(60)),
        });
        let cache = StreamCache::new(inner);

        cache.resolve(&id()).await.unwrap();
        assert_eq!(cache.len(), 1);
        cache.evict_stale();
        assert!(cache.is_empty());
    }
}
