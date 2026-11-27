use tracing::debug;

use musick_core::{Lyrics, PositionTick, RepeatMode, Track};

use crate::backend::BackendEvent;
use crate::engine::{InternalMsg, PlayerEngine};

impl PlayerEngine {
    pub(super) async fn load_track(&mut self, track: Track, is_retry: bool) {
        self.cancel_inflight_loads();
        self.load_gen += 1;
        let epoch = self.load_gen;
        self.pending_autofill_advance = false;
        if !is_retry {
            self.retried_video_id = None;
        }
        self.gapless_appended_video_id = None;
        self.active_tracking = None;
        self.watchtime_reported = false;

        self.snapshot.track = Some(track.clone());
        self.snapshot.track_rev += 1;
        self.snapshot.resolving = true;
        self.snapshot.playing = false;
        self.snapshot.restored = false;
        self.snapshot.error = None;
        self.snapshot.audio_info.clear();
        self.position = PositionTick {
            position_secs: 0.0,
            duration_secs: track.duration.map(f64::from).unwrap_or(0.0),
            buffering: true,
            active_lyric_idx: None,
        };
        let _ = self.pos_tx.send(self.position);
        self.publish_snapshot();

        if !is_retry {
            self.snapshot.lyrics_offset_secs = 0.0;
            self.snapshot.loading_lyrics = self.settings.lyrics.enabled;
            self.publish_lyrics(Some(track.video_id.clone()), Lyrics::default());
            if self.settings.lyrics.enabled {
                self.spawn_lyrics_load(epoch, track.clone());
            }
        }

        if self.settings.privacy.save_history && !is_retry {
            let _ = self.store.record_play(&track);
        }

        let force_fallback = is_retry && self.last_stream_from_fast;
        let resolver = self.resolver.clone();
        let tx = self.internal_tx.clone();
        let jh = tokio::spawn(async move {
            let res = if force_fallback {
                resolver.resolve_fallback_only(&track.video_id).await
            } else {
                resolver.resolve(&track.video_id).await
            }
            .map_err(|e| e.to_string());
            let _ = tx.send(InternalMsg::StreamResolved {
                epoch,
                track,
                result: res,
            });
        });
        self.resolve_task = Some(jh.abort_handle());
    }

    /// evict stale or fast-path url and retry once via fallback on playback failure
    pub(super) async fn handle_load_or_playback_error(&mut self, track: &Track, err: String) {
        if self.retried_video_id.as_deref() != Some(track.video_id.as_str()) {
            debug!(
                "Playback error on {} (fast={}), invalidating URL and retrying once: {err}",
                track.video_id, self.last_stream_from_fast
            );
            self.retried_video_id = Some(track.video_id.clone());
            self.resolver
                .report_playback_failure(&track.video_id, self.last_stream_from_fast);
            Box::pin(self.load_track(track.clone(), true)).await;
            return;
        }
        self.position.buffering = false;
        let _ = self.pos_tx.send(self.position);
        self.snapshot.resolving = false;
        self.snapshot.playing = false;
        self.snapshot.error = Some(err);
        self.publish_snapshot();
    }

    pub(super) async fn handle_backend_event(&mut self, ev: BackendEvent) {
        match ev {
            BackendEvent::Properties(props) => {
                if self.snapshot.resolving {
                    return;
                }
                let active_idx = self.current_lyrics.1.active_line_index(
                    (props.position + self.snapshot.lyrics_offset_secs).max(0.0),
                );
                self.position = PositionTick {
                    position_secs: props.position,
                    duration_secs: if props.duration > 0.0 {
                        props.duration
                    } else {
                        self.position.duration_secs
                    },
                    buffering: props.buffering,
                    active_lyric_idx: active_idx,
                };
                let _ = self.pos_tx.send(self.position);

                let should_update_play_state =
                    self.snapshot.error.is_none() && !self.snapshot.resolving;
                if props.paused != self.snapshot.paused || props.volume != self.snapshot.volume {
                    self.snapshot.paused = props.paused;
                    self.snapshot.volume = props.volume;
                    if should_update_play_state {
                        self.snapshot.playing = !props.paused && self.snapshot.track.is_some();
                    }
                    self.publish_snapshot();
                }

                if !self.watchtime_reported && props.position >= 30.0 {
                    self.flush_watchtime_if_needed().await;
                }
            }
            BackendEvent::Eof => {
                if self.snapshot.resolving {
                    return;
                }
                self.flush_watchtime_if_needed().await;
                if let Some(next) = self.queue.advance(false) {
                    self.pending_autofill_advance = false;
                    self.publish_queue();
                    let was_gapless =
                        self.gapless_appended_video_id.as_deref() == Some(next.video_id.as_str());
                    if was_gapless && self.queue.repeat_mode() != RepeatMode::One {
                        self.transition_metadata_after_gapless(next).await;
                    } else {
                        self.load_track(next, false).await;
                    }
                } else if self.snapshot.autoplay_active && self.settings.player.autoplay {
                    self.pending_autofill_advance = true;
                    self.snapshot.playing = false;
                    self.snapshot.resolving = true;
                    self.publish_snapshot();
                    self.maybe_autofill_radio().await;
                } else {
                    self.snapshot.playing = false;
                    self.publish_snapshot();
                }
            }
            BackendEvent::FileError(err) => {
                if self.snapshot.resolving {
                    return;
                }
                if let Some(cur) = self.snapshot.track.clone() {
                    self.handle_load_or_playback_error(&cur, err).await;
                } else {
                    self.position.buffering = false;
                    let _ = self.pos_tx.send(self.position);
                    self.snapshot.error = Some(err);
                    self.publish_snapshot();
                }
            }
            BackendEvent::Disconnected(err) => {
                self.position.buffering = false;
                let _ = self.pos_tx.send(self.position);
                self.snapshot.playing = false;
                self.snapshot.error = Some(err);
                self.publish_snapshot();
            }
        }
    }

    async fn transition_metadata_after_gapless(&mut self, track: Track) {
        self.cancel_inflight_loads();
        self.load_gen += 1;
        let epoch = self.load_gen;
        self.retried_video_id = None;
        self.gapless_appended_video_id = None;
        self.active_tracking = None;
        self.watchtime_reported = false;

        self.snapshot.track = Some(track.clone());
        self.snapshot.track_rev += 1;
        self.snapshot.playing = true;
        self.snapshot.paused = false;
        self.snapshot.resolving = false;
        self.snapshot.restored = false;
        self.snapshot.error = None;
        if let Some(cached) = self.resolver.get_cached(&track.video_id) {
            self.snapshot.audio_info = cached.quality_label();
            self.last_stream_from_fast = cached.from_fast_path;
        } else {
            self.last_stream_from_fast = false;
        }
        self.position = PositionTick {
            position_secs: 0.0,
            duration_secs: track.duration.map(f64::from).unwrap_or(0.0),
            buffering: false,
            active_lyric_idx: None,
        };
        let _ = self.pos_tx.send(self.position);
        self.publish_snapshot();

        if self.settings.privacy.save_history {
            let _ = self.store.record_play(&track);
        }
        self.snapshot.lyrics_offset_secs = 0.0;
        self.snapshot.loading_lyrics = self.settings.lyrics.enabled;
        self.publish_lyrics(Some(track.video_id.clone()), Lyrics::default());
        if self.settings.lyrics.enabled {
            self.spawn_lyrics_load(epoch, track.clone());
        }
        self.spawn_telemetry_start(epoch, track.video_id);
        self.schedule_prefetch_and_gapless();
        self.maybe_autofill_radio().await;
    }

    pub(super) async fn handle_internal(&mut self, msg: InternalMsg) {
        match msg {
            InternalMsg::StreamResolved {
                epoch,
                track,
                result,
            } => {
                if epoch != self.load_gen {
                    return;
                }
                self.resolve_task = None;
                match result {
                    Ok(stream) => {
                        self.last_stream_from_fast = stream.from_fast_path;
                        self.snapshot.audio_info = stream.quality_label();
                        self.snapshot.resolving = false;
                        if let Err(err) = self
                            .backend
                            .load_url_with_ua(&stream.url, stream.user_agent.as_deref())
                            .await
                        {
                            self.handle_load_or_playback_error(&track, err.to_string())
                                .await;
                            return;
                        }
                        if let Some(seek_pos) = self.pending_restore_seek.take() {
                            let _ = self.backend.seek(seek_pos, false).await;
                        }
                        self.snapshot.playing = true;
                        self.snapshot.paused = false;
                        self.publish_snapshot();

                        self.spawn_telemetry_start(epoch, track.video_id.clone());
                        self.schedule_prefetch_and_gapless();
                        self.maybe_autofill_radio().await;
                    }
                    Err(err) => {
                        self.position.buffering = false;
                        let _ = self.pos_tx.send(self.position);
                        self.snapshot.resolving = false;
                        self.snapshot.playing = false;
                        self.snapshot.error = Some(err);
                        self.publish_snapshot();
                    }
                }
            }
            InternalMsg::RadioStarted {
                epoch,
                playlist_id,
                seed_track,
                result,
            } => {
                self.handle_radio_started(epoch, playlist_id, seed_track, result)
                    .await;
            }
            InternalMsg::LyricsLoaded {
                epoch,
                video_id,
                lyrics,
            } => {
                if epoch == self.load_gen
                    && self
                        .snapshot
                        .track
                        .as_ref()
                        .is_some_and(|t| t.video_id == video_id)
                {
                    self.snapshot.loading_lyrics = false;
                    self.publish_lyrics(Some(video_id), lyrics);
                }
            }
            InternalMsg::LyricsTranslated {
                epoch,
                video_id,
                lyrics,
            } => {
                self.snapshot.translating_lyrics = false;
                if epoch == self.load_gen
                    && self
                        .snapshot
                        .track
                        .as_ref()
                        .is_some_and(|t| t.video_id == video_id)
                {
                    self.publish_lyrics(Some(video_id), lyrics);
                } else {
                    self.publish_snapshot();
                }
            }
            InternalMsg::RadioAutofill {
                tracks,
                continuation,
                playlist_id,
            } => {
                self.handle_radio_autofill(tracks, continuation, playlist_id)
                    .await;
            }
            InternalMsg::TrackingLoaded {
                epoch,
                video_id,
                cpn,
                tracking,
            } => {
                if epoch == self.load_gen {
                    self.active_tracking = Some((video_id, cpn, tracking));
                }
            }
            InternalMsg::GaplessResolved {
                epoch,
                video_id,
                url,
                user_agent,
            } => {
                if epoch == self.load_gen
                    && self.settings.player.gapless
                    && self.queue.repeat_mode() != RepeatMode::One
                    && self
                        .queue
                        .peek_next()
                        .is_some_and(|t| t.video_id == video_id)
                    && self.gapless_appended_video_id.as_deref() != Some(video_id.as_str())
                    && self
                        .backend
                        .append_gapless_with_ua(&url, user_agent.as_deref())
                        .await
                        .is_ok()
                {
                    self.gapless_appended_video_id = Some(video_id);
                }
            }
        }
    }

    pub(super) async fn invalidate_gapless_tail(&mut self) {
        if self.gapless_appended_video_id.take().is_some() {
            let _ = self.backend.clear_gapless_tail().await;
        }
    }

    pub(super) fn schedule_prefetch_and_gapless(&self) {
        if !self.settings.player.prefetch {
            return;
        }
        let Some(next_track) = self.queue.peek_next().cloned() else {
            return;
        };
        if self.settings.player.gapless
            && self.queue.repeat_mode() != RepeatMode::One
            && self.gapless_appended_video_id.as_deref() != Some(next_track.video_id.as_str())
        {
            let resolver = self.resolver.clone();
            let tx = self.internal_tx.clone();
            let vid = next_track.video_id;
            let epoch = self.load_gen;
            tokio::spawn(async move {
                if let Ok(stream) = resolver.resolve(&vid).await {
                    let _ = tx.send(InternalMsg::GaplessResolved {
                        epoch,
                        video_id: vid,
                        url: stream.url,
                        user_agent: stream.user_agent,
                    });
                }
            });
        } else {
            self.resolver.prefetch(next_track.video_id);
        }
    }
}
