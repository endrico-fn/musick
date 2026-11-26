use std::collections::HashSet;
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
use crate::error::{CoreError, CoreResult};
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
struct L1CatalogEntry {
    json: String,
    expires_at: i64,
}

pub struct CacheManager {
    conn: Mutex<Connection>,
    images_dir: PathBuf,
    rustypipe_dir: PathBuf,
    settings: RwLock<CacheSettings>,
    l1_catalog: Mutex<LruCache<String, L1CatalogEntry>>,
    l1_lyrics: Mutex<LruCache<String, Lyrics>>,
    l1_images: Mutex<LruCache<String, Arc<Vec<u8>>>>,
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
        let rustypipe_dir = paths.rustypipe_cache_dir();
        ensure_private_dir(&rustypipe_dir)?;

        let conn = Connection::open(paths.cache_db_file())?;
        migrate_cache_db(&conn)?;

        Ok(Self {
            conn: Mutex::new(conn),
            images_dir,
            rustypipe_dir,
            settings: RwLock::new(settings),
            l1_catalog: Mutex::new(LruCache::new(non_zero(L1_CATALOG_CAP))),
            l1_lyrics: Mutex::new(LruCache::new(non_zero(L1_LYRICS_CAP))),
            l1_images: Mutex::new(LruCache::new(non_zero(L1_IMAGES_CAP))),
        })
    }

    pub fn update_settings(&self, settings: CacheSettings) {
        if let Ok(mut guard) = self.settings.write() {
            *guard = settings;
        }
    }

    pub fn current_settings(&self) -> CacheSettings {
        self.settings.read().map(|g| g.clone()).unwrap_or_default()
    }

    // ── Catalog JSON Cache ──────────────────────────────────────────────────

    pub fn get_catalog<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let now = now_unix_secs() as i64;

        // L1 lookup
        if let Ok(mut l1) = self.l1_catalog.lock()
            && let Some(entry) = l1.get(key)
        {
            if entry.expires_at > now {
                if let Ok(val) = serde_json::from_str::<T>(&entry.json) {
                    return Some(val);
                }
                // Self-heal corrupt L1 entry
                l1.pop(key);
            } else {
                l1.pop(key);
            }
        }

        let cfg = self.current_settings();
        if !cfg.enabled {
            return None;
        }

        // L2 lookup with self-healing on corrupt JSON
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
                // Self-heal: evict corrupt row instead of failing permanently.
                let _ = conn.execute("DELETE FROM catalog_cache WHERE cache_key = ?1", [key]);
                None
            }
        }
    }

    pub fn put_catalog<T: Serialize>(&self, key: &str, value: &T, ttl_secs: Option<u64>) {
        let Ok(json) = serde_json::to_string(value) else {
            return;
        };
        let cfg = self.current_settings();
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

        if !cfg.enabled {
            return;
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

    // ── Lyrics Cache ────────────────────────────────────────────────────────

    pub fn get_lyrics(&self, key: &str) -> Option<Lyrics> {
        if let Ok(mut l1) = self.l1_lyrics.lock()
            && let Some(hit) = l1.get(key)
        {
            return Some(hit.clone());
        }

        let cfg = self.current_settings();
        if !cfg.enabled {
            return None;
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
                // Self-heal corrupt lyrics cache row.
                let _ = conn.execute("DELETE FROM lyrics_cache WHERE cache_key = ?1", [key]);
                None
            }
        }
    }

    pub fn put_lyrics(&self, key: &str, lyrics: &Lyrics) {
        if lyrics.is_empty() {
            return;
        }
        if let Ok(mut l1) = self.l1_lyrics.lock() {
            l1.put(key.to_string(), lyrics.clone());
        }

        let cfg = self.current_settings();
        if !cfg.enabled {
            return;
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

    // ── Image Blob Cache ────────────────────────────────────────────────────

    pub fn get_image_bytes(&self, url: &str) -> Option<Arc<Vec<u8>>> {
        let hash = url_hash(url);
        if let Ok(mut l1) = self.l1_images.lock()
            && let Some(hit) = l1.get(&hash)
        {
            return Some(hit.clone());
        }

        let cfg = self.current_settings();
        if !cfg.enabled {
            return None;
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
                // Self-heal missing, empty, or truncated/corrupt image blob.
                let _ = conn.execute("DELETE FROM image_index WHERE url_hash = ?1", [&hash]);
                let _ = fs::remove_file(&blob_path);
                None
            }
        }
    }

    pub fn put_image_bytes(&self, url: &str, bytes: Vec<u8>) -> Arc<Vec<u8>> {
        let hash = url_hash(url);
        let arc = Arc::new(bytes);

        if let Ok(mut l1) = self.l1_images.lock() {
            l1.put(hash.clone(), arc.clone());
        }

        let cfg = self.current_settings();
        if !cfg.enabled || arc.is_empty() {
            return arc;
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

    // ── Visitor Data (24h TTL, rotated on Clear Cache or Logout) ────────────

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

    // ── Stats, Pruning & Clear Cache ────────────────────────────────────────

    pub fn stats(&self) -> CoreResult<CacheStats> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| CoreError::Config("CacheManager DB lock poisoned".into()))?;

        let (catalog_entries, catalog_bytes): (i64, i64) = conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(size_bytes), 0) FROM catalog_cache",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let (lyrics_entries, lyrics_bytes): (i64, i64) = conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(size_bytes), 0) FROM lyrics_cache",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let (image_files, image_bytes): (i64, i64) = conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(size_bytes), 0) FROM image_index",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;

        let cat_b = u64::try_from(catalog_bytes.max(0)).unwrap_or(0);
        let lyr_b = u64::try_from(lyrics_bytes.max(0)).unwrap_or(0);
        let img_b = u64::try_from(image_bytes.max(0)).unwrap_or(0);

        Ok(CacheStats {
            catalog_entries: usize::try_from(catalog_entries.max(0)).unwrap_or(0),
            catalog_bytes: cat_b,
            lyrics_entries: usize::try_from(lyrics_entries.max(0)).unwrap_or(0),
            lyrics_bytes: lyr_b,
            image_files: usize::try_from(image_files.max(0)).unwrap_or(0),
            image_bytes: img_b,
            total_bytes: cat_b.saturating_add(lyr_b).saturating_add(img_b),
        })
    }

    pub fn prune_all(&self) -> CoreResult<PruneReport> {
        let cfg = self.current_settings();
        self.prune_with_budgets(
            cfg.catalog_max_mib.saturating_mul(1024 * 1024),
            cfg.lyrics_max_mib.saturating_mul(1024 * 1024),
            cfg.images_max_mib.saturating_mul(1024 * 1024),
            cfg.images_max_files,
            cfg.image_ttl_days.saturating_mul(86_400),
        )
    }

    pub fn prune_with_budgets(
        &self,
        catalog_max_bytes: u64,
        lyrics_max_bytes: u64,
        images_max_bytes: u64,
        images_max_files: usize,
        image_ttl_secs: u64,
    ) -> CoreResult<PruneReport> {
        let now = now_unix_secs() as i64;
        let conn = self
            .conn
            .lock()
            .map_err(|_| CoreError::Config("CacheManager DB lock poisoned".into()))?;
        let mut report = PruneReport::default();

        // 1. Expired ephemeral KV
        let _ = conn.execute("DELETE FROM ephemeral_kv WHERE expires_at <= ?1", [now]);

        // 2. Catalog TTL + LRU byte cap
        let (cat_ev, cat_freed) = prune_kv_table(&conn, "catalog_cache", now, catalog_max_bytes)?;
        report.catalog_evicted = cat_ev;
        report.bytes_freed = report.bytes_freed.saturating_add(cat_freed);

        // 3. Lyrics TTL + LRU byte cap
        let (lyr_ev, lyr_freed) = prune_kv_table(&conn, "lyrics_cache", now, lyrics_max_bytes)?;
        report.lyrics_evicted = lyr_ev;
        report.bytes_freed = report.bytes_freed.saturating_add(lyr_freed);

        // 4. Image TTL + LRU byte & file-count cap
        let cutoff = now.saturating_sub(image_ttl_secs as i64);
        let expired_hashes: Vec<(String, i64)> = {
            let mut stmt =
                conn.prepare("SELECT url_hash, size_bytes FROM image_index WHERE created_at < ?1")?;
            let rows = stmt.query_map([cutoff], |r| Ok((r.get(0)?, r.get(1)?)))?;
            let mut v = Vec::new();
            for row in rows {
                v.push(row?);
            }
            v
        };
        for (hash, sz) in expired_hashes {
            conn.execute("DELETE FROM image_index WHERE url_hash = ?1", [&hash])?;
            let _ = fs::remove_file(self.images_dir.join(format!("{hash}.bin")));
            report.images_evicted += 1;
            report.bytes_freed = report
                .bytes_freed
                .saturating_add(u64::try_from(sz.max(0)).unwrap_or(0));
        }

        let ordered_imgs: Vec<(String, u64)> = {
            let mut stmt = conn.prepare(
                "SELECT url_hash, size_bytes FROM image_index ORDER BY last_accessed ASC",
            )?;
            let rows = stmt.query_map([], |r| {
                let sz: i64 = r.get(1)?;
                Ok((r.get(0)?, u64::try_from(sz.max(0)).unwrap_or(0)))
            })?;
            let mut v = Vec::new();
            for row in rows {
                v.push(row?);
            }
            v
        };

        let mut total_img_bytes: u64 = ordered_imgs.iter().map(|(_, s)| *s).sum();
        let mut total_img_files: usize = ordered_imgs.len();
        let mut kept_hashes: HashSet<String> = HashSet::with_capacity(ordered_imgs.len());

        for (hash, sz) in ordered_imgs {
            if total_img_bytes > images_max_bytes || total_img_files > images_max_files {
                conn.execute("DELETE FROM image_index WHERE url_hash = ?1", [&hash])?;
                let _ = fs::remove_file(self.images_dir.join(format!("{hash}.bin")));
                total_img_bytes = total_img_bytes.saturating_sub(sz);
                total_img_files = total_img_files.saturating_sub(1);
                report.images_evicted += 1;
                report.bytes_freed = report.bytes_freed.saturating_add(sz);
            } else {
                kept_hashes.insert(hash);
            }
        }

        // 5. Remove orphan files in images/ not tracked in image_index
        if let Ok(entries) = fs::read_dir(&self.images_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let stem = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default();
                if !kept_hashes.contains(stem) {
                    let len = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    if fs::remove_file(&path).is_ok() {
                        report.orphan_files_removed += 1;
                        report.bytes_freed = report.bytes_freed.saturating_add(len);
                    }
                }
            }
        }

        let _ = conn.execute_batch("PRAGMA incremental_vacuum;");
        Ok(report)
    }

    /// Wipe all L1 and L2 caches and rotate `visitor_data`. Strictly NEVER touches `musick.db`.
    pub fn clear_cache(&self) -> CoreResult<()> {
        if let Ok(mut l1) = self.l1_catalog.lock() {
            l1.clear();
        }
        if let Ok(mut l1) = self.l1_lyrics.lock() {
            l1.clear();
        }
        if let Ok(mut l1) = self.l1_images.lock() {
            l1.clear();
        }

        {
            let conn = self
                .conn
                .lock()
                .map_err(|_| CoreError::Config("CacheManager DB lock poisoned".into()))?;
            conn.execute_batch(
                "DELETE FROM catalog_cache;
                 DELETE FROM lyrics_cache;
                 DELETE FROM image_index;
                 DELETE FROM ephemeral_kv;
                 VACUUM;",
            )?;
        }

        clear_dir_contents(&self.images_dir);
        clear_dir_contents(&self.rustypipe_dir);
        Ok(())
    }
}

fn clear_dir_contents(dir: &PathBuf) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let _ = fs::remove_dir_all(&path);
            } else {
                let _ = fs::remove_file(&path);
            }
        }
    }
}

fn prune_kv_table(
    conn: &Connection,
    table: &str,
    now: i64,
    max_bytes: u64,
) -> CoreResult<(usize, u64)> {
    let mut evicted = 0usize;
    let mut freed = 0u64;

    let expired_sql = format!("SELECT cache_key, size_bytes FROM {table} WHERE expires_at <= ?1");
    let expired: Vec<(String, u64)> = {
        let mut stmt = conn.prepare(&expired_sql)?;
        let rows = stmt.query_map([now], |r| {
            let sz: i64 = r.get(1)?;
            Ok((r.get(0)?, u64::try_from(sz.max(0)).unwrap_or(0)))
        })?;
        let mut v = Vec::new();
        for row in rows {
            v.push(row?);
        }
        v
    };

    let del_sql = format!("DELETE FROM {table} WHERE cache_key = ?1");
    for (k, sz) in expired {
        conn.execute(&del_sql, [&k])?;
        evicted += 1;
        freed = freed.saturating_add(sz);
    }

    let ordered_sql =
        format!("SELECT cache_key, size_bytes FROM {table} ORDER BY last_accessed ASC");
    let rows: Vec<(String, u64)> = {
        let mut stmt = conn.prepare(&ordered_sql)?;
        let mapped = stmt.query_map([], |r| {
            let sz: i64 = r.get(1)?;
            Ok((r.get(0)?, u64::try_from(sz.max(0)).unwrap_or(0)))
        })?;
        let mut v = Vec::new();
        for row in mapped {
            v.push(row?);
        }
        v
    };

    let mut total: u64 = rows.iter().map(|(_, s)| *s).sum();
    for (k, sz) in rows {
        if total <= max_bytes {
            break;
        }
        conn.execute(&del_sql, [&k])?;
        total = total.saturating_sub(sz);
        evicted += 1;
        freed = freed.saturating_add(sz);
    }

    Ok((evicted, freed))
}

/// Quick magic-byte check to catch truncated/HTML error pages saved as image blobs.
fn looks_like_image(bytes: &[u8]) -> bool {
    if bytes.len() < 12 {
        return false;
    }
    // JPEG
    if bytes[0] == 0xFF && bytes[1] == 0xD8 && bytes[2] == 0xFF {
        return true;
    }
    // PNG
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return true;
    }
    // WEBP (RIFF....WEBP)
    if bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return true;
    }
    // GIF
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return true;
    }
    false
}
