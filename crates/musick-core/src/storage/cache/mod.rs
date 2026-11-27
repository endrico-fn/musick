mod prune;

use std::fs;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use lru::LruCache;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};

use crate::auth::now_unix_secs;
use crate::config::CacheSettings;
use crate::domain::Lyrics;
use crate::error::CoreResult;
use crate::paths::{AppPaths, atomic_write_bytes, ensure_private_dir};
use crate::storage::migrations::migrate_cache_db;

const L1_CATALOG_CAP: usize = 256;
const L1_LYRICS_CAP: usize = 128;
const L1_IMAGES_CAP: usize = 96;
const VISITOR_DATA_KEY: &str = "visitor_data";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CacheStats {
    pub catalog_entries: usize,
    pub catalog_bytes: u64,
    pub lyrics_entries: usize,
    pub lyrics_bytes: u64,
    pub image_files: usize,
    pub image_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct PruneReport {
    pub catalog_evicted: usize,
    pub lyrics_evicted: usize,
    pub images_evicted: usize,
    pub orphan_files_removed: usize,
    pub bytes_freed: u64,
}

#[derive(Debug, Clone)]
pub(super) struct L1CatalogEntry {
    json: String,
    expires_at: i64,
}

pub struct CacheManager {
    pub(super) conn: Mutex<Connection>,
    pub(super) images_dir: PathBuf,
    pub(super) settings: RwLock<CacheSettings>,
    pub(super) l1_catalog: Mutex<LruCache<String, L1CatalogEntry>>,
    pub(super) l1_lyrics: Mutex<LruCache<String, Lyrics>>,
    pub(super) l1_images: Mutex<LruCache<String, Arc<Vec<u8>>>>,
}

fn non_zero(v: usize) -> NonZeroUsize {
    NonZeroUsize::new(v).unwrap_or(NonZeroUsize::MIN)
}

fn url_hash(url: &str) -> String {
    let mut h = Sha256::new();
    h.update(url.as_bytes());
    hex::encode(h.finalize())
}

impl CacheManager {
    pub fn open(paths: &AppPaths, settings: CacheSettings) -> CoreResult<Self> {
        ensure_private_dir(&paths.cache_dir)?;
        let images_dir = paths.images_cache_dir();
        ensure_private_dir(&images_dir)?;

        let conn = Connection::open(paths.cache_db_file())?;
        migrate_cache_db(&conn)?;

        Ok(Self {
            conn: Mutex::new(conn),
            images_dir,
            settings: RwLock::new(settings),
            l1_catalog: Mutex::new(LruCache::new(non_zero(L1_CATALOG_CAP))),
            l1_lyrics: Mutex::new(LruCache::new(non_zero(L1_LYRICS_CAP))),
            l1_images: Mutex::new(LruCache::new(non_zero(L1_IMAGES_CAP))),
        })
    }

    pub fn update_settings(&self, settings: CacheSettings) {
        let enabled = settings.enabled;
        if let Ok(mut guard) = self.settings.write() {
            *guard = settings;
        }
        if !enabled {
            if let Ok(mut l1) = self.l1_catalog.lock() {
                l1.clear();
            }
            if let Ok(mut l1) = self.l1_lyrics.lock() {
                l1.clear();
            }
            if let Ok(mut l1) = self.l1_images.lock() {
                l1.clear();
            }
        }
    }

    pub fn current_settings(&self) -> CacheSettings {
        self.settings.read().map(|g| g.clone()).unwrap_or_default()
    }

    pub fn get_catalog<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let cfg = self.current_settings();
        if !cfg.enabled {
            return None;
        }
        let now = now_unix_secs() as i64;

        if let Ok(mut l1) = self.l1_catalog.lock()
            && let Some(entry) = l1.get(key)
        {
            if entry.expires_at > now {
                if let Ok(val) = serde_json::from_str::<T>(&entry.json) {
                    return Some(val);
                }
                l1.pop(key);
            } else {
                l1.pop(key);
            }
        }

        let conn = self.conn.lock().ok()?;
        let row: Option<(String, i64)> = conn
            .query_row(
                "SELECT value_json, expires_at FROM catalog_cache WHERE cache_key = ?1",
                [key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .ok()?;

        let (raw_json, expires_at) = row?;
        if expires_at <= now {
            let _ = conn.execute("DELETE FROM catalog_cache WHERE cache_key = ?1", [key]);
            return None;
        }

        match serde_json::from_str::<T>(&raw_json) {
            Ok(val) => {
                let _ = conn.execute(
                    "UPDATE catalog_cache SET last_accessed = ?1 WHERE cache_key = ?2",
                    params![now, key],
                );
                if let Ok(mut l1) = self.l1_catalog.lock() {
                    l1.put(
                        key.to_string(),
                        L1CatalogEntry {
                            json: raw_json,
                            expires_at,
                        },
                    );
                }
                Some(val)
            }
            Err(_) => {
                let _ = conn.execute("DELETE FROM catalog_cache WHERE cache_key = ?1", [key]);
                None
            }
        }
    }

    pub fn put_catalog<T: Serialize>(&self, key: &str, value: &T, ttl_secs: Option<u64>) {
        let cfg = self.current_settings();
        if !cfg.enabled {
            return;
        }
        let Ok(json) = serde_json::to_string(value) else {
            return;
        };
        let ttl = ttl_secs.unwrap_or(cfg.catalog_ttl_secs) as i64;
        let now = now_unix_secs() as i64;
        let expires_at = now.saturating_add(ttl.max(30));

        if let Ok(mut l1) = self.l1_catalog.lock() {
            l1.put(
                key.to_string(),
                L1CatalogEntry {
                    json: json.clone(),
                    expires_at,
                },
            );
        }

        if let Ok(conn) = self.conn.lock() {
            let size = json.len() as i64;
            let _ = conn.execute(
                "INSERT OR REPLACE INTO catalog_cache
                    (cache_key, value_json, size_bytes, created_at, expires_at, last_accessed)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?4)",
                params![key, json, size, now, expires_at],
            );
        }
    }

    pub fn get_lyrics(&self, key: &str) -> Option<Lyrics> {
        let cfg = self.current_settings();
        if !cfg.enabled {
            return None;
        }
        if let Ok(mut l1) = self.l1_lyrics.lock()
            && let Some(hit) = l1.get(key)
        {
            return Some(hit.clone());
        }

        let now = now_unix_secs() as i64;
        let conn = self.conn.lock().ok()?;
        let row: Option<(String, i64)> = conn
            .query_row(
                "SELECT value_json, expires_at FROM lyrics_cache WHERE cache_key = ?1",
                [key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .ok()?;

        let (raw_json, expires_at) = row?;
        if expires_at <= now {
            let _ = conn.execute("DELETE FROM lyrics_cache WHERE cache_key = ?1", [key]);
            return None;
        }

        match serde_json::from_str::<Lyrics>(&raw_json) {
            Ok(lyrics) => {
                let _ = conn.execute(
                    "UPDATE lyrics_cache SET last_accessed = ?1 WHERE cache_key = ?2",
                    params![now, key],
                );
                if let Ok(mut l1) = self.l1_lyrics.lock() {
                    l1.put(key.to_string(), lyrics.clone());
                }
                Some(lyrics)
            }
            Err(_) => {
                let _ = conn.execute("DELETE FROM lyrics_cache WHERE cache_key = ?1", [key]);
                None
            }
        }
    }

    pub fn put_lyrics(&self, key: &str, lyrics: &Lyrics) {
        if lyrics.is_empty() {
            return;
        }
        let cfg = self.current_settings();
        if !cfg.enabled {
            return;
        }
        if let Ok(mut l1) = self.l1_lyrics.lock() {
            l1.put(key.to_string(), lyrics.clone());
        }

        let Ok(json) = serde_json::to_string(lyrics) else {
            return;
        };
        let now = now_unix_secs() as i64;
        let expires_at = now.saturating_add(30 * 86_400);
        if let Ok(conn) = self.conn.lock() {
            let _ = conn.execute(
                "INSERT OR REPLACE INTO lyrics_cache
                    (cache_key, value_json, size_bytes, created_at, expires_at, last_accessed)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?4)",
                params![key, json, json.len() as i64, now, expires_at],
            );
        }
    }

    pub fn get_image_bytes(&self, url: &str) -> Option<Arc<Vec<u8>>> {
        let cfg = self.current_settings();
        if !cfg.enabled {
            return None;
        }
        let hash = url_hash(url);
        if let Ok(mut l1) = self.l1_images.lock()
            && let Some(hit) = l1.get(&hash)
        {
            return Some(hit.clone());
        }

        let blob_path = self.images_dir.join(format!("{hash}.bin"));
        let now = now_unix_secs() as i64;
        let ttl_secs = (cfg.image_ttl_days as i64).saturating_mul(86_400);

        let conn = self.conn.lock().ok()?;
        let row: Option<i64> = conn
            .query_row(
                "SELECT created_at FROM image_index WHERE url_hash = ?1",
                [&hash],
                |r| r.get(0),
            )
            .optional()
            .ok()?;

        let created_at = row?;
        if now.saturating_sub(created_at) > ttl_secs {
            let _ = conn.execute("DELETE FROM image_index WHERE url_hash = ?1", [&hash]);
            let _ = fs::remove_file(&blob_path);
            return None;
        }

        match fs::read(&blob_path) {
            Ok(bytes) if !bytes.is_empty() && looks_like_image(&bytes) => {
                let _ = conn.execute(
                    "UPDATE image_index SET last_accessed = ?1 WHERE url_hash = ?2",
                    params![now, &hash],
                );
                let arc = Arc::new(bytes);
                if let Ok(mut l1) = self.l1_images.lock() {
                    l1.put(hash, arc.clone());
                }
                Some(arc)
            }
            _ => {
                let _ = conn.execute("DELETE FROM image_index WHERE url_hash = ?1", [&hash]);
                let _ = fs::remove_file(&blob_path);
                None
            }
        }
    }

    pub fn put_image_bytes(&self, url: &str, bytes: Vec<u8>) -> Arc<Vec<u8>> {
        let arc = Arc::new(bytes);
        let cfg = self.current_settings();
        if !cfg.enabled || arc.is_empty() {
            return arc;
        }
        let hash = url_hash(url);

        if let Ok(mut l1) = self.l1_images.lock() {
            l1.put(hash.clone(), arc.clone());
        }

        let blob_path = self.images_dir.join(format!("{hash}.bin"));
        if atomic_write_bytes(&blob_path, &arc).is_ok()
            && let Ok(conn) = self.conn.lock()
        {
            let now = now_unix_secs() as i64;
            let _ = conn.execute(
                "INSERT OR REPLACE INTO image_index (url_hash, url, size_bytes, created_at, last_accessed)
                 VALUES (?1, ?2, ?3, ?4, ?4)",
                params![hash, url, arc.len() as i64, now],
            );
        }
        arc
    }

    pub fn evict_image(&self, url: &str) {
        let hash = url_hash(url);
        if let Ok(mut l1) = self.l1_images.lock() {
            l1.pop(&hash);
        }
        if let Ok(conn) = self.conn.lock() {
            let _ = conn.execute("DELETE FROM image_index WHERE url_hash = ?1", [&hash]);
        }
        let _ = fs::remove_file(self.images_dir.join(format!("{hash}.bin")));
    }

    pub fn get_visitor_data(&self) -> Option<String> {
        let now = now_unix_secs() as i64;
        let conn = self.conn.lock().ok()?;
        let row: Option<(String, i64)> = conn
            .query_row(
                "SELECT value, expires_at FROM ephemeral_kv WHERE key = ?1",
                [VISITOR_DATA_KEY],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .ok()?;

        let (val, expires_at) = row?;
        if expires_at <= now || val.trim().is_empty() {
            let _ = conn.execute(
                "DELETE FROM ephemeral_kv WHERE key = ?1",
                [VISITOR_DATA_KEY],
            );
            return None;
        }
        Some(val)
    }

    pub fn set_visitor_data(&self, visitor_data: &str) {
        let clean = visitor_data.trim();
        if clean.is_empty() {
            return;
        }
        let ttl = self.current_settings().visitor_data_ttl_secs as i64;
        let now = now_unix_secs() as i64;
        if let Ok(conn) = self.conn.lock() {
            let _ = conn.execute(
                "INSERT OR REPLACE INTO ephemeral_kv (key, value, expires_at) VALUES (?1, ?2, ?3)",
                params![VISITOR_DATA_KEY, clean, now.saturating_add(ttl)],
            );
        }
    }

    pub fn clear_visitor_data(&self) {
        if let Ok(conn) = self.conn.lock() {
            let _ = conn.execute(
                "DELETE FROM ephemeral_kv WHERE key = ?1",
                [VISITOR_DATA_KEY],
            );
        }
    }
}

/// magic-byte check to reject truncated or html error responses saved as image blobs
fn looks_like_image(bytes: &[u8]) -> bool {
    if bytes.len() < 12 {
        return false;
    }
    if bytes[0] == 0xFF && bytes[1] == 0xD8 && bytes[2] == 0xFF {
        return true;
    }
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return true;
    }
    if bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return true;
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return true;
    }
    false
}
