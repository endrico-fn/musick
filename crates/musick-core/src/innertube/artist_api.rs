use std::collections::{HashMap, HashSet};

use tracing::debug;

use crate::domain::{AlbumSummary, Artist, ArtistRef, Track};
use crate::error::CoreResult;

use super::{CatalogApi, ClientKind, InnerTubeClient, parsers};

impl InnerTubeClient {
    pub(super) async fn artist_impl(&self, browse_id: &str) -> CoreResult<Artist> {
        let clean_bid = browse_id.strip_prefix("MPLA").unwrap_or(browse_id);
        let cache_key = format!("artist:{}:{clean_bid}", self.region());
        if let Some(cached) = self.cache.get_catalog::<Artist>(&cache_key) {
            return Ok(cached);
        }

        let raw = self
            .browse(clean_bid, None, None, ClientKind::WebRemix, None)
            .await?;
        let mut art = parsers::parse_artist(clean_bid, &raw);
        let mut plays_map = parsers::extract_plays_from_browse(&raw);

        let songs_pl_id = art.songs_playlist_id.clone();
        let videos_pl_id = art.videos_playlist_id.clone();
        let albums_bid = art
            .albums_browse_id
            .clone()
            .unwrap_or_else(|| clean_bid.to_string());
        let albums_params = art.albums_params.clone();
        let singles_bid = art
            .singles_browse_id
            .clone()
            .unwrap_or_else(|| clean_bid.to_string());
        let singles_params = art.singles_params.clone();

        let (songs_pl, videos_pl, full_albums, full_singles) = tokio::join!(
            async {
                if let Some(pid) = songs_pl_id.as_deref() {
                    self.playlist(pid, 150).await.ok()
                } else {
                    None
                }
            },
            async {
                if let Some(pid) = videos_pl_id.as_deref() {
                    self.playlist(pid, 100).await.ok()
                } else {
                    None
                }
            },
            async {
                if albums_params.is_some() || albums_bid != clean_bid {
                    self.artist_releases(&albums_bid, albums_params.as_deref(), "Album")
                        .await
                        .unwrap_or_default()
                } else {
                    Vec::new()
                }
            },
            async {
                if singles_params.is_some() || singles_bid != clean_bid {
                    self.artist_releases(&singles_bid, singles_params.as_deref(), "Single")
                        .await
                        .unwrap_or_default()
                } else {
                    Vec::new()
                }
            },
        );

        let fallback_artists = vec![ArtistRef {
            name: art.summary.name.clone(),
            id: Some(clean_bid.to_string()),
        }];

        if let Some(pl) = songs_pl
            && !pl.tracks.is_empty()
        {
            for t in &pl.tracks {
                if let Some(p) = &t.plays {
                    plays_map
                        .entry(t.video_id.clone())
                        .or_insert_with(|| p.clone());
                }
            }
            let orig_map: HashMap<String, Track> = art
                .songs
                .iter()
                .map(|t| (t.video_id.clone(), t.clone()))
                .collect();
            let mut seen = HashSet::new();
            let mut enriched = Vec::new();
            for mut t in pl.tracks {
                seen.insert(t.video_id.clone());
                if let Some(orig) = orig_map.get(&t.video_id) {
                    if t.artists.is_empty() {
                        t.artists = orig.artists.clone();
                    }
                    if t.album.is_none() {
                        t.album = orig.album.clone();
                    }
                    if t.album_id.is_none() {
                        t.album_id = orig.album_id.clone();
                    }
                    if t.duration.is_none() {
                        t.duration = orig.duration;
                    }
                    if t.thumbs.is_empty() {
                        t.thumbs = orig.thumbs.clone();
                    }
                    if t.plays.is_none() {
                        t.plays = orig.plays.clone();
                    }
                    if t.video_type.is_none() {
                        t.video_type = orig.video_type.clone();
                    }
                }
                if t.artists.is_empty() {
                    t.artists = fallback_artists.clone();
                }
                if let Some(p) = plays_map.get(&t.video_id) {
                    t.plays = Some(p.clone());
                }
                enriched.push(t);
            }
            for mut orig in art.songs {
                if seen.insert(orig.video_id.clone()) {
                    if orig.artists.is_empty() {
                        orig.artists = fallback_artists.clone();
                    }
                    if let Some(p) = plays_map.get(&orig.video_id) {
                        orig.plays = Some(p.clone());
                    }
                    enriched.push(orig);
                }
            }
            art.songs = enriched;
        } else {
            for t in &mut art.songs {
                if t.artists.is_empty() {
                    t.artists = fallback_artists.clone();
                }
                if let Some(p) = plays_map.get(&t.video_id) {
                    t.plays = Some(p.clone());
                }
            }
        }

        if let Some(vpl) = videos_pl
            && !vpl.tracks.is_empty()
        {
            let mut seen = HashSet::new();
            let mut vids = Vec::new();
            for mut t in vpl.tracks {
                if seen.insert(t.video_id.clone()) {
                    if t.artists.is_empty() {
                        t.artists = fallback_artists.clone();
                    }
                    if t.video_type.is_none() {
                        t.video_type = Some("MUSIC_VIDEO_TYPE_OMV".into());
                    }
                    vids.push(t);
                }
            }
            for orig in art.videos {
                if seen.insert(orig.video_id.clone()) {
                    vids.push(orig);
                }
            }
            art.videos = vids;
        }

        if !full_albums.is_empty() {
            art.albums = full_albums;
        }
        for al in &mut art.albums {
            if al.artists.is_empty() {
                al.artists = fallback_artists.clone();
            }
        }

        if !full_singles.is_empty() {
            art.singles = full_singles;
        }
        for s in &mut art.singles {
            if s.artists.is_empty() {
                s.artists = fallback_artists.clone();
            }
        }

        self.cache.put_catalog(&cache_key, &art, None);
        Ok(art)
    }

    pub(super) async fn artist_releases_impl(
        &self,
        browse_id: &str,
        params: Option<&str>,
        kind: &str,
    ) -> CoreResult<Vec<AlbumSummary>> {
        let p_key = params.unwrap_or("");
        let cache_key = format!("releases:{}:{browse_id}:{p_key}:{kind}", self.region());
        if let Some(cached) = self.cache.get_catalog::<Vec<AlbumSummary>>(&cache_key) {
            return Ok(cached);
        }
        let raw = self
            .browse(browse_id, params, None, ClientKind::WebRemix, None)
            .await?;
        let (mut items, mut cont) = parsers::parse_artist_releases_with_continuation(&raw, kind);
        let mut seen: HashSet<String> = items.iter().map(|a| a.browse_id.clone()).collect();
        let mut pages = 0usize;
        while let Some(token) = cont.take()
            && pages < 4
        {
            pages += 1;
            match self
                .browse(browse_id, None, Some(&token), ClientKind::WebRemix, None)
                .await
            {
                Ok(page_raw) => {
                    let (page_items, next_cont) =
                        parsers::parse_artist_releases_with_continuation(&page_raw, kind);
                    let before = items.len();
                    for al in page_items {
                        if seen.insert(al.browse_id.clone()) {
                            items.push(al);
                        }
                    }
                    if items.len() == before {
                        break;
                    }
                    cont = next_cont;
                }
                Err(err) => {
                    debug!("Artist releases pagination stopped for {browse_id}: {err}");
                    break;
                }
            }
        }
        self.cache.put_catalog(&cache_key, &items, None);
        Ok(items)
    }
}
