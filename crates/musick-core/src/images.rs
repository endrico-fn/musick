use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use image::DynamicImage;
use lru::LruCache;
use tokio::sync::{Notify, Semaphore};
use tracing::debug;

use crate::storage::CacheManager;

const MAX_CONCURRENT_IMAGE_FETCHES: usize = 6;
const MAX_DECODED_IMAGES: usize = 96;
const IMAGE_USER_AGENT: &str = "musick/0.2.0";

fn non_zero(v: usize) -> NonZeroUsize {
    NonZeroUsize::new(v).unwrap_or(NonZeroUsize::MIN)
}

/// Center-crop image to 1:1 square so covers never stretch or distort.
pub fn crop_square(img: DynamicImage) -> DynamicImage {
    let w = img.width();
    let h = img.height();
    if w == h || w == 0 || h == 0 {
        return img;
    }
    let side = w.min(h);
    let x = (w - side) / 2;
    let y = (h - side) / 2;
    img.crop_imm(x, y, side, side)
}

/// Cookie-free concurrent cover image service with single-flight dedup and L1/L2 cache.
pub struct ImageService {
    http: reqwest::Client,
    cache: Arc<CacheManager>,
    sem: Arc<Semaphore>,
    decoded_lru: Mutex<LruCache<String, Arc<DynamicImage>>>,
    inflight: Mutex<HashMap<String, Arc<Notify>>>,
}

impl ImageService {
    pub fn new(http: reqwest::Client, cache: Arc<CacheManager>) -> Self {
        Self {
            http,
            cache,
            sem: Arc::new(Semaphore::new(MAX_CONCURRENT_IMAGE_FETCHES)),
            decoded_lru: Mutex::new(LruCache::new(non_zero(MAX_DECODED_IMAGES))),
            inflight: Mutex::new(HashMap::new()),
        }
    }

    pub fn clear_memory_cache(&self) {
        if let Ok(mut guard) = self.decoded_lru.lock() {
            guard.clear();
        }
    }

    pub fn get_cached_decoded(&self, url: &str) -> Option<Arc<DynamicImage>> {
        self.decoded_lru.lock().ok()?.get(url).cloned()
    }

    pub async fn fetch(&self, url: &str) -> Option<Arc<DynamicImage>> {
        let clean = url.trim();
        if clean.is_empty() {
            return None;
        }

        if let Some(hit) = self.get_cached_decoded(clean) {
            return Some(hit);
        }

        // Single-flight deduplication per URL
        loop {
            let waiter = {
                let mut map = self.inflight.lock().ok()?;
                if let Some(existing) = map.get(clean) {
                    Some(existing.clone())
                } else {
                    map.insert(clean.to_string(), Arc::new(Notify::new()));
                    None
                }
            };

            if let Some(notify) = waiter {
                notify.notified().await;
                if let Some(hit) = self.get_cached_decoded(clean) {
                    return Some(hit);
                }
                continue;
            }
            break;
        }

        let res = self.fetch_inner(clean).await;

        if let Ok(mut map) = self.inflight.lock()
            && let Some(notify) = map.remove(clean)
        {
            notify.notify_waiters();
        }

        res
    }

    async fn fetch_inner(&self, url: &str) -> Option<Arc<DynamicImage>> {
        // 1. Check L2 disk blob cache first; self-heal if cached blob fails to decode.
        if let Some(bytes) = self.cache.get_image_bytes(url) {
            if let Ok(img) = decode_square_async(bytes).await {
                let arc = Arc::new(img);
                if let Ok(mut guard) = self.decoded_lru.lock() {
                    guard.put(url.to_string(), arc.clone());
                }
                return Some(arc);
            }
            self.cache.evict_image(url);
        }

        // 2. Download cookie-free under concurrency semaphore
        let _permit = self.sem.acquire().await.ok()?;
        let resp = match self
            .http
            .get(url)
            .header("User-Agent", IMAGE_USER_AGENT)
            .timeout(Duration::from_secs(10))
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                debug!("Image fetch HTTP {} for {url}", r.status());
                return None;
            }
            Err(err) => {
                debug!("Image fetch error for {url}: {err}");
                return None;
            }
        };

        let raw_bytes = resp.bytes().await.ok()?.to_vec();
        if raw_bytes.is_empty() {
            return None;
        }

        let arc_bytes = Arc::new(raw_bytes.clone());
        let decoded = decode_square_async(arc_bytes).await.ok()?;
        let _ = self.cache.put_image_bytes(url, raw_bytes);

        let arc_img = Arc::new(decoded);
        if let Ok(mut guard) = self.decoded_lru.lock() {
            guard.put(url.to_string(), arc_img.clone());
        }
        Some(arc_img)
    }
}

async fn decode_square_async(bytes: Arc<Vec<u8>>) -> Result<DynamicImage, ()> {
    tokio::task::spawn_blocking(move || {
        image::load_from_memory(&bytes)
            .map(crop_square)
            .map_err(|_| ())
    })
    .await
    .map_err(|_| ())?
}
