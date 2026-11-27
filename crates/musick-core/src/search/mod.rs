mod ranker;
mod url;

use std::collections::HashSet;
use std::sync::{Arc, RwLock};

use crate::config::Settings;
use crate::domain::{SearchCategory, SearchItem, Track};
use crate::error::CoreResult;
use crate::innertube::CatalogApi;
use crate::storage::UserStore;

pub use ranker::{
    is_semantic_result_set, norm_text, parse_play_count, rank_search_items, score_search_item,
    token_set_ratio, token_sort_ratio, wratio,
};
pub use url::{YoutubeTarget, extract_youtube_target, parse_query_prefix};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub text: String,
    pub kind_hint: String,
    pub from_history: bool,
}

pub struct SearchService {
    catalog: Arc<dyn CatalogApi>,
    store: Arc<UserStore>,
    settings: RwLock<Settings>,
}

impl SearchService {
    pub fn new(catalog: Arc<dyn CatalogApi>, store: Arc<UserStore>, settings: Settings) -> Self {
        Self {
            catalog,
            store,
            settings: RwLock::new(settings),
        }
    }

    pub fn update_settings(&self, settings: Settings) {
        if let Ok(mut guard) = self.settings.write() {
            *guard = settings;
        }
    }

    fn current_settings(&self) -> Settings {
        self.settings.read().map(|g| g.clone()).unwrap_or_default()
    }

    pub async fn search(
        &self,
        category: SearchCategory,
        raw_query: &str,
        limit: usize,
    ) -> CoreResult<Vec<SearchItem>> {
        let settings = self.current_settings();
        let trimmed = raw_query.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }

        if settings.privacy.save_history {
            let _ = self.store.record_search_query(trimmed);
        }

        if let Some(target) = extract_youtube_target(trimmed) {
            return match target {
                YoutubeTarget::Track(vid) => {
                    if let Ok(batch) = self.catalog.radio(Some(&vid), None, None, 5).await {
                        let mut out = Vec::new();
                        if let Some(exact) =
                            batch.tracks.iter().find(|t| t.video_id == vid).cloned()
                        {
                            out.push(SearchItem::Track(exact));
                            for t in batch.tracks {
                                if t.video_id != vid {
                                    out.push(SearchItem::Track(t));
                                }
                            }
                            return Ok(out);
                        }
                        if !batch.tracks.is_empty() {
                            out.push(SearchItem::Track(Track::minimal(
                                vid.clone(),
                                format!("YouTube Track ({vid})"),
                            )));
                            out.extend(batch.tracks.into_iter().map(SearchItem::Track));
                            return Ok(out);
                        }
                    }
                    Ok(vec![SearchItem::Track(Track::minimal(
                        vid.clone(),
                        format!("YouTube Track ({vid})"),
                    ))])
                }
                YoutubeTarget::Playlist(pid) => {
                    let pl = self.catalog.playlist(&pid, 50).await?;
                    let mut out = vec![SearchItem::Playlist(pl.summary)];
                    out.extend(pl.tracks.into_iter().take(15).map(SearchItem::Track));
                    Ok(out)
                }
                YoutubeTarget::Album(aid) => {
                    let alb = self.catalog.album(&aid).await?;
                    let mut out = vec![SearchItem::Album(alb.summary)];
                    out.extend(alb.tracks.into_iter().map(SearchItem::Track));
                    Ok(out)
                }
                YoutubeTarget::Artist(cid) => {
                    let art = self.catalog.artist(&cid).await?;
                    let mut out = vec![SearchItem::Artist(art.summary)];
                    out.extend(art.songs.into_iter().take(15).map(SearchItem::Track));
                    Ok(out)
                }
                YoutubeTarget::Handle(handle) => {
                    let hits = self
                        .catalog
                        .search(SearchCategory::Artists, &handle, 5)
                        .await?;
                    if let Some(SearchItem::Artist(ar)) = hits.first()
                        && let Ok(art) = self.catalog.artist(&ar.browse_id).await
                    {
                        let mut out = vec![SearchItem::Artist(art.summary)];
                        out.extend(art.songs.into_iter().take(15).map(SearchItem::Track));
                        return Ok(out);
                    }
                    Ok(hits)
                }
            };
        }

        let (resolved_cat, clean_query) = parse_query_prefix(trimmed, category);
        if clean_query.is_empty() {
            return Ok(Vec::new());
        }
        let raw_items = self
            .catalog
            .search(resolved_cat, &clean_query, limit)
            .await?;
        let mut ranked = rank_search_items(raw_items, &clean_query, resolved_cat);
        ranked.truncate(limit);
        Ok(ranked)
    }

    pub fn fuzzy_history(&self, query: &str, limit: usize) -> Vec<Suggestion> {
        let settings = self.current_settings();
        if !settings.privacy.save_history {
            return Vec::new();
        }
        let hist = self.store.search_history(40).unwrap_or_default();
        if hist.is_empty() {
            return Vec::new();
        }
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return hist
                .into_iter()
                .take(limit)
                .map(|text| Suggestion {
                    text,
                    kind_hint: "History".to_string(),
                    from_history: true,
                })
                .collect();
        }

        let mut scored: Vec<(f64, String)> = hist
            .into_iter()
            .filter_map(|h| {
                let s = wratio(&q, &h.to_lowercase());
                if s >= 55.0 { Some((s, h)) } else { None }
            })
            .collect();
        scored.sort_by(|a, b| b.0.total_cmp(&a.0));
        scored
            .into_iter()
            .take(limit)
            .map(|(_, text)| Suggestion {
                text,
                kind_hint: "History".to_string(),
                from_history: true,
            })
            .collect()
    }

    pub async fn autocomplete(&self, raw_query: &str, limit: usize) -> Vec<Suggestion> {
        let settings = self.current_settings();
        let (cat, clean_q) = parse_query_prefix(raw_query, SearchCategory::All);
        let hist = self.fuzzy_history(&clean_q, 3);
        if clean_q.is_empty() || !settings.privacy.remote_suggestions {
            return hist.into_iter().take(limit).collect();
        }

        let remote = self.catalog.suggestions(&clean_q).await.unwrap_or_default();

        let mut seen: HashSet<String> = hist.iter().map(|s| s.text.to_lowercase()).collect();
        let hint = if cat == SearchCategory::All {
            String::new()
        } else {
            cat.label().trim_end_matches('s').to_string()
        };

        let mut out = hist;
        for item in remote {
            if seen.insert(item.to_lowercase()) {
                out.push(Suggestion {
                    text: item,
                    kind_hint: hint.clone(),
                    from_history: false,
                });
                if out.len() >= limit {
                    break;
                }
            }
        }
        out
    }
}
