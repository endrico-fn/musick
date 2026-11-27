use std::collections::HashSet;

use serde_json::{Value, json};

use crate::domain::{AlbumSummary, ArtistSummary, PlaylistSummary, Track};
use crate::error::{CoreError, CoreResult};

use super::{
    CatalogApi, ClientKind, InnerTubeClient, PlaylistEditAction, PlaylistEditResult, SongRating,
    parsers,
};

impl InnerTubeClient {
    async fn paginate_library<T>(
        &self,
        browse_id: &str,
        parse_page: fn(&Value) -> (Vec<T>, Option<String>),
        key_of: fn(&T) -> &str,
    ) -> CoreResult<Vec<T>> {
        let raw = self
            .browse(browse_id, None, None, ClientKind::WebRemix, None)
            .await?;
        let (mut out, mut cont) = parse_page(&raw);
        let mut seen: HashSet<String> = out.iter().map(|i| key_of(i).to_string()).collect();
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
                    let (page, next_cont) = parse_page(&page_raw);
                    let before = out.len();
                    for item in page {
                        if seen.insert(key_of(&item).to_string()) {
                            out.push(item);
                        }
                    }
                    if out.len() == before {
                        break;
                    }
                    cont = next_cont;
                }
                Err(_) => break,
            }
        }
        Ok(out)
    }

    pub(super) async fn library_playlists_impl(&self) -> CoreResult<Vec<PlaylistSummary>> {
        self.require_auth("library_playlists")?;
        self.paginate_library(
            "FEmusic_liked_playlists",
            parsers::parse_library_playlists_with_continuation,
            |p| p.browse_id.as_str(),
        )
        .await
    }

    pub(super) async fn library_liked_songs_impl(&self, limit: usize) -> CoreResult<Vec<Track>> {
        self.require_auth("library_liked_songs")?;
        let pl = self.playlist_uncached("LM", limit).await?;
        Ok(pl.tracks)
    }

    pub(super) async fn library_albums_impl(&self) -> CoreResult<Vec<AlbumSummary>> {
        self.require_auth("library_albums")?;
        self.paginate_library(
            "FEmusic_liked_albums",
            parsers::parse_library_albums_with_continuation,
            |a| a.browse_id.as_str(),
        )
        .await
    }

    pub(super) async fn library_artists_impl(&self) -> CoreResult<Vec<ArtistSummary>> {
        self.require_auth("library_artists")?;
        self.paginate_library(
            "FEmusic_library_corpus_artists",
            parsers::parse_library_artists_with_continuation,
            |a| a.browse_id.as_str(),
        )
        .await
    }

    pub(super) async fn create_playlist_impl(
        &self,
        title: &str,
        description: &str,
        video_ids: &[String],
    ) -> CoreResult<String> {
        self.require_auth("create_playlist")?;
        let mut map = serde_json::Map::new();
        map.insert("title".into(), json!(title));
        map.insert("description".into(), json!(description));
        map.insert("privacyStatus".into(), json!("PRIVATE"));
        if !video_ids.is_empty() {
            map.insert("videoIds".into(), json!(video_ids));
        }
        let raw = self
            .post_json("playlist/create", ClientKind::WebRemix, None, map)
            .await?;
        raw.get("playlistId")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| CoreError::Catalog("playlist/create response missing playlistId".into()))
    }

    pub(super) async fn edit_playlist_impl(
        &self,
        remote_playlist_id: &str,
        actions: &[PlaylistEditAction],
    ) -> CoreResult<PlaylistEditResult> {
        self.require_auth("edit_playlist")?;
        if actions.is_empty() {
            return Ok(PlaylistEditResult::default());
        }
        let clean_pid = remote_playlist_id
            .strip_prefix("VL")
            .unwrap_or(remote_playlist_id);
        let mut map = serde_json::Map::new();
        map.insert("playlistId".into(), json!(clean_pid));
        map.insert(
            "actions".into(),
            Value::Array(
                actions
                    .iter()
                    .map(PlaylistEditAction::to_innertube_json)
                    .collect(),
            ),
        );
        let raw = self
            .post_json("browse/edit_playlist", ClientKind::WebRemix, None, map)
            .await?;
        Ok(parsers::parse_playlist_edit_results(&raw))
    }

    pub(super) async fn delete_playlist_impl(&self, remote_playlist_id: &str) -> CoreResult<()> {
        self.require_auth("delete_playlist")?;
        let clean_pid = remote_playlist_id
            .strip_prefix("VL")
            .unwrap_or(remote_playlist_id);
        let mut map = serde_json::Map::new();
        map.insert("playlistId".into(), json!(clean_pid));
        let _ = self
            .post_json("playlist/delete", ClientKind::WebRemix, None, map)
            .await?;
        Ok(())
    }

    pub(super) async fn rate_song_impl(
        &self,
        video_id: &str,
        rating: SongRating,
    ) -> CoreResult<()> {
        self.require_auth("rate_song")?;
        let mut map = serde_json::Map::new();
        map.insert("target".into(), json!({ "videoId": video_id }));
        let _ = self
            .post_json(rating.endpoint(), ClientKind::WebRemix, None, map)
            .await?;
        Ok(())
    }

    pub(super) async fn set_artist_subscription_impl(
        &self,
        channel_id: &str,
        subscribe: bool,
    ) -> CoreResult<()> {
        self.require_auth("set_artist_subscription")?;
        let mut map = serde_json::Map::new();
        map.insert("channelIds".into(), json!([channel_id]));
        let ep = if subscribe {
            "subscription/subscribe"
        } else {
            "subscription/unsubscribe"
        };
        let _ = self.post_json(ep, ClientKind::WebRemix, None, map).await?;
        Ok(())
    }

    pub(super) async fn set_album_library_status_impl(
        &self,
        browse_id: &str,
        save: bool,
    ) -> CoreResult<()> {
        self.require_auth("set_album_library_status")?;
        // youtube music `like/like` requires the album's audioPlaylistId (`OLAK5uy_...`), not `MPREb_...`
        let playlist_id = if browse_id.starts_with("MPRE") {
            match self.album(browse_id).await {
                Ok(alb) => alb
                    .audio_playlist_id
                    .unwrap_or_else(|| browse_id.to_string()),
                Err(_) => browse_id.to_string(),
            }
        } else {
            browse_id.to_string()
        };
        let mut map = serde_json::Map::new();
        map.insert("target".into(), json!({ "playlistId": playlist_id }));
        let ep = if save { "like/like" } else { "like/removelike" };
        let _ = self.post_json(ep, ClientKind::WebRemix, None, map).await?;
        Ok(())
    }

    pub(super) async fn set_playlist_library_status_impl(
        &self,
        playlist_id: &str,
        save: bool,
    ) -> CoreResult<()> {
        self.require_auth("set_playlist_library_status")?;
        let clean_pid = playlist_id.strip_prefix("VL").unwrap_or(playlist_id);
        let mut map = serde_json::Map::new();
        map.insert("target".into(), json!({ "playlistId": clean_pid }));
        let ep = if save { "like/like" } else { "like/removelike" };
        let _ = self.post_json(ep, ClientKind::WebRemix, None, map).await?;
        Ok(())
    }
}
