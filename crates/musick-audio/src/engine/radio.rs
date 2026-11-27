use std::collections::HashSet;

use tracing::debug;

use musick_core::{RadioBatch, Track};

use crate::engine::{InternalMsg, PlayerEngine};

const RADIO_AUTOFILL_THRESHOLD: usize = 3;

impl PlayerEngine {
    pub(super) async fn handle_radio_started(
        &mut self,
        epoch: u64,
        playlist_id: Option<String>,
        seed_track: Option<Track>,
        result: Result<RadioBatch, String>,
    ) {
        if epoch != self.load_gen {
            return;
        }
        self.radio_start_task = None;
        match result {
            Ok(batch) => {
                let mut tracks = batch.tracks;
                if let Some(seed) = seed_track
                    && !tracks.iter().any(|t| t.video_id == seed.video_id)
                {
                    tracks.insert(0, seed);
                }
                self.radio_gen += 1;
                self.radio_inflight = false;
                self.radio_continuation = batch.continuation;
                self.radio_playlist_id = batch.playlist_id.or(playlist_id);
                self.snapshot.autoplay_active = true;
                self.snapshot.radio_active = true;
                self.queue.set_tracks(tracks, 0);
                self.publish_queue();
                if let Some(cur) = self.queue.current().cloned() {
                    self.load_track(cur, false).await;
                } else {
                    self.snapshot.resolving = false;
                    self.publish_snapshot();
                }
            }
            Err(err) => {
                self.snapshot.resolving = false;
                self.snapshot.error = Some(format!("Radio failed: {err}"));
                self.publish_snapshot();
            }
        }
    }

    pub(super) async fn handle_radio_autofill(
        &mut self,
        radio_gen: u64,
        tracks: Vec<Track>,
        continuation: Option<String>,
        playlist_id: Option<String>,
    ) {
        if radio_gen != self.radio_gen {
            return;
        }
        self.radio_inflight = false;
        if continuation.is_some() {
            self.radio_continuation = continuation;
        }
        if playlist_id.is_some() {
            self.radio_playlist_id = playlist_id;
        }
        let existing: HashSet<String> = self
            .queue
            .tracks()
            .iter()
            .map(|t| t.video_id.clone())
            .collect();
        let fresh: Vec<Track> = tracks
            .into_iter()
            .filter(|t| !existing.contains(&t.video_id))
            .collect();
        if !fresh.is_empty() {
            self.snapshot.radio_active = true;
            self.queue.append(fresh);
            self.publish_queue();
            if self.pending_autofill_advance {
                self.pending_autofill_advance = false;
                if let Some(next) = self.queue.advance(false) {
                    self.publish_queue();
                    self.load_track(next, false).await;
                    return;
                }
            }
            self.schedule_prefetch_and_gapless();
        } else if self.pending_autofill_advance {
            self.pending_autofill_advance = false;
            self.snapshot.resolving = false;
            self.publish_snapshot();
        }
    }

    pub(super) async fn maybe_autofill_radio(&mut self) {
        if !self.snapshot.autoplay_active
            || !self.settings.player.autoplay
            || self.radio_inflight
            || self.queue.upcoming_tracks().len() > RADIO_AUTOFILL_THRESHOLD
        {
            return;
        }
        let Some(seed_vid) = self
            .queue
            .tracks()
            .last()
            .or_else(|| self.queue.current())
            .map(|t| t.video_id.clone())
        else {
            return;
        };

        self.radio_inflight = true;
        let radio_gen = self.radio_gen;
        let catalog = self.catalog.clone();
        let tx = self.internal_tx.clone();
        let pid = self.radio_playlist_id.clone();
        let cont = self.radio_continuation.clone();

        tokio::spawn(async move {
            let res = catalog
                .radio(Some(&seed_vid), pid.as_deref(), cont.as_deref(), 20)
                .await;
            match res {
                Ok(batch) => {
                    let _ = tx.send(InternalMsg::RadioAutofill {
                        radio_gen,
                        tracks: batch.tracks,
                        continuation: batch.continuation,
                        playlist_id: batch.playlist_id,
                    });
                }
                Err(err) => {
                    debug!("Radio autofill failed: {err}");
                    let _ = tx.send(InternalMsg::RadioAutofill {
                        radio_gen,
                        tracks: Vec::new(),
                        continuation: None,
                        playlist_id: None,
                    });
                }
            }
        });
    }
}
