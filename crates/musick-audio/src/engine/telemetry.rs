use std::time::Duration;

use rand::RngExt;

use musick_core::{PlaybackTelemetryEvent, Track};

use crate::engine::{InternalMsg, PlayerEngine};

const CPN_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn generate_cpn() -> String {
    let mut rng = rand::rng();
    (0..16)
        .map(|_| {
            let idx = rng.random_range(0..CPN_ALPHABET.len());
            CPN_ALPHABET[idx] as char
        })
        .collect()
}

impl PlayerEngine {
    pub(super) fn spawn_lyrics_load(&self, epoch: u64, track: Track) {
        let lyr_svc = self.lyrics_svc.clone();
        let tx = self.internal_tx.clone();
        tokio::spawn(async move {
            let lyr = lyr_svc.fetch(&track).await;
            let _ = tx.send(InternalMsg::LyricsLoaded {
                epoch,
                video_id: track.video_id,
                lyrics: lyr,
            });
        });
    }

    pub(super) fn spawn_telemetry_start(&self, epoch: u64, video_id: String) {
        // hard privacy gate: strictly off in anonymous mode or when report_playback_history is false
        if !self.settings.privacy.report_playback_history || !self.auth.is_authenticated() {
            return;
        }
        let catalog = self.catalog.clone();
        let tx = self.internal_tx.clone();
        let cpn = generate_cpn();
        tokio::spawn(async move {
            if let Ok(Some(tracking)) = catalog.fetch_playback_tracking(&video_id, true).await {
                let _ = catalog
                    .report_playback(&tracking, &cpn, PlaybackTelemetryEvent::Start, true)
                    .await;
                let _ = tx.send(InternalMsg::TrackingLoaded {
                    epoch,
                    video_id,
                    cpn,
                    tracking,
                });
            }
        });
    }

    pub(super) async fn flush_watchtime_if_needed(&mut self) {
        if self.watchtime_reported
            || !self.settings.privacy.report_playback_history
            || !self.auth.is_authenticated()
        {
            return;
        }
        let Some((_, ref cpn, ref tracking)) = self.active_tracking else {
            return;
        };
        if self.position.position_secs < 5.0 {
            return;
        }
        self.watchtime_reported = true;
        let catalog = self.catalog.clone();
        let tracking = tracking.clone();
        let cpn = cpn.clone();
        let elapsed = self.position.position_secs;
        let duration = self.position.duration_secs;
        let playing = self.snapshot.playing && !self.snapshot.paused;
        tokio::spawn(async move {
            let _ = tokio::time::timeout(
                Duration::from_secs(5),
                catalog.report_playback(
                    &tracking,
                    &cpn,
                    PlaybackTelemetryEvent::Watchtime {
                        elapsed_secs: elapsed,
                        duration_secs: duration,
                        playing,
                    },
                    true,
                ),
            )
            .await;
        });
    }
}
