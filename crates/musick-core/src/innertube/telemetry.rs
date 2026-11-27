use serde_json::json;

use crate::error::CoreResult;

use super::{
    ClientKind, InnerTubeClient, PlaybackTelemetryEvent, PlaybackTracking, endpoints, parsers,
};

impl InnerTubeClient {
    pub(super) async fn fetch_playback_tracking_impl(
        &self,
        video_id: &str,
        report_enabled: bool,
    ) -> CoreResult<Option<PlaybackTracking>> {
        // hard privacy gate: no-op in anonymous mode or when opt-in toggle is off
        if !report_enabled || !self.auth.is_authenticated() {
            return Ok(None);
        }
        let mut map = serde_json::Map::new();
        map.insert("videoId".into(), json!(video_id));
        let raw = self
            .post_json("player", ClientKind::WebRemix, None, map)
            .await?;
        Ok(parsers::parse_player_tracking(video_id, &raw))
    }

    pub(super) async fn report_playback_impl(
        &self,
        tracking: &PlaybackTracking,
        cpn: &str,
        event: PlaybackTelemetryEvent,
        report_enabled: bool,
    ) -> CoreResult<()> {
        if !report_enabled || !self.auth.is_authenticated() {
            return Ok(());
        }

        let (base_url, query_pairs): (&str, Vec<(&str, String)>) = match event {
            PlaybackTelemetryEvent::Start => {
                let Some(url) = tracking.playback_url.as_deref() else {
                    return Ok(());
                };
                (
                    url,
                    vec![
                        ("cpn", cpn.to_string()),
                        ("ver", "2".to_string()),
                        ("c", endpoints::WEB_REMIX_CLIENT_NAME.to_string()),
                        ("cver", endpoints::WEB_REMIX_CLIENT_VERSION.to_string()),
                        ("docid", tracking.video_id.clone()),
                    ],
                )
            }
            PlaybackTelemetryEvent::Watchtime {
                elapsed_secs,
                duration_secs,
                playing,
            } => {
                let Some(url) = tracking.watchtime_url.as_deref() else {
                    return Ok(());
                };
                let len = if duration_secs > 0.0 {
                    duration_secs
                } else {
                    f64::from(tracking.length_secs)
                };
                (
                    url,
                    vec![
                        ("cpn", cpn.to_string()),
                        ("ver", "2".to_string()),
                        ("c", endpoints::WEB_REMIX_CLIENT_NAME.to_string()),
                        ("cver", endpoints::WEB_REMIX_CLIENT_VERSION.to_string()),
                        ("docid", tracking.video_id.clone()),
                        ("cmt", format!("{elapsed_secs:.3}")),
                        ("et", format!("{elapsed_secs:.3}")),
                        ("st", "0.000".to_string()),
                        ("len", format!("{len:.3}")),
                        (
                            "state",
                            if playing { "playing" } else { "paused" }.to_string(),
                        ),
                    ],
                )
            }
        };

        let req = self
            .http
            .get(base_url)
            .header("User-Agent", endpoints::WEB_USER_AGENT)
            .query(&query_pairs);
        let req = self.auth.apply_headers(req).await?;
        let _ = req.send().await?;
        Ok(())
    }
}
