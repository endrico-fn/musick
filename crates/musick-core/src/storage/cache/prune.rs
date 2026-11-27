use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use rusqlite::Connection;

use crate::auth::now_unix_secs;
use crate::error::{CoreError, CoreResult};
use crate::storage::cache::{CacheManager, CacheStats, PruneReport};

impl CacheManager {
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

        let _ = conn.execute("DELETE FROM ephemeral_kv WHERE expires_at <= ?1", [now]);

        let (cat_ev, cat_freed) = prune_kv_table(&conn, "catalog_cache", now, catalog_max_bytes)?;
        report.catalog_evicted = cat_ev;
        report.bytes_freed = report.bytes_freed.saturating_add(cat_freed);

        let (lyr_ev, lyr_freed) = prune_kv_table(&conn, "lyrics_cache", now, lyrics_max_bytes)?;
        report.lyrics_evicted = lyr_ev;
        report.bytes_freed = report.bytes_freed.saturating_add(lyr_freed);

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

    /// wipe all l1 and l2 caches and rotate visitor_data without touching musick.db
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
