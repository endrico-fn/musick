use musick_core::{AppCommand, Lyrics, PlayerEvent, PositionTick};

use crate::engine::{InternalMsg, PlayerEngine};

impl PlayerEngine {
    pub(super) async fn handle_command(&mut self, cmd: AppCommand) {
        match cmd {
            AppCommand::PlayTracks {
                tracks,
                start_index,
                autoplay,
            } => {
                self.radio_gen += 1;
                self.radio_inflight = false;
                self.radio_continuation = None;
                self.radio_playlist_id = None;
                self.snapshot.radio_active = false;
                self.snapshot.autoplay_active = autoplay && self.settings.player.autoplay;
                self.queue.set_tracks(tracks, start_index);
                self.publish_queue();
                if let Some(cur) = self.queue.current().cloned() {
                    self.load_track(cur, false).await;
                    if self.snapshot.autoplay_active && self.queue.upcoming_tracks().is_empty() {
                        self.maybe_autofill_radio().await;
                    }
                }
            }
            AppCommand::PlayNext { tracks } => {
                if tracks.is_empty() {
                    return;
                }
                let was_empty = self.queue.is_empty();
                let count = tracks.len();
                let first_title = tracks.first().map(|t| t.title.clone());
                self.queue.play_next_many(tracks);
                self.invalidate_gapless_tail().await;
                self.publish_queue();
                if was_empty && let Some(cur) = self.queue.current().cloned() {
                    self.load_track(cur, false).await;
                } else {
                    self.schedule_prefetch_and_gapless();
                    let msg = if count == 1
                        && let Some(title) = first_title
                    {
                        format!("Playing next: {title}")
                    } else {
                        format!("Playing {count} tracks next")
                    };
                    let _ = self.events_tx.send(PlayerEvent::StatusToast(msg));
                }
            }
            AppCommand::AppendTracks { tracks } => {
                if tracks.is_empty() {
                    return;
                }
                let was_empty = self.queue.is_empty();
                let count = tracks.len();
                let first_title = tracks.first().map(|t| t.title.clone());
                self.queue.enqueue(tracks);
                self.invalidate_gapless_tail().await;
                self.publish_queue();
                if was_empty && let Some(cur) = self.queue.current().cloned() {
                    self.load_track(cur, false).await;
                } else {
                    self.schedule_prefetch_and_gapless();
                    let msg = if count == 1
                        && let Some(title) = first_title
                    {
                        format!("Added to queue: {title}")
                    } else {
                        format!("Added {count} tracks to queue")
                    };
                    let _ = self.events_tx.send(PlayerEvent::StatusToast(msg));
                }
            }
            AppCommand::StartRadio {
                video_id,
                playlist_id,
                seed_track,
            } => {
                self.cancel_inflight_loads();
                self.load_gen += 1;
                let epoch = self.load_gen;
                self.snapshot.resolving = true;
                self.snapshot.restored = false;
                self.snapshot.error = None;
                self.publish_snapshot();

                let catalog = self.catalog.clone();
                let tx = self.internal_tx.clone();
                let pid = playlist_id.clone();
                let jh = tokio::spawn(async move {
                    let res = catalog
                        .radio(video_id.as_deref(), pid.as_deref(), None, 25)
                        .await
                        .map_err(|e| e.to_string());
                    let _ = tx.send(InternalMsg::RadioStarted {
                        epoch,
                        playlist_id,
                        seed_track,
                        result: res,
                    });
                });
                self.radio_start_task = Some(jh.abort_handle());
            }
            AppCommand::RestoreTrack { track } => {
                if !self.queue.is_empty() || self.snapshot.track.is_some() {
                    return;
                }
                self.cancel_inflight_loads();
                let dur = track.duration.map(f64::from).unwrap_or(0.0);
                self.queue.set_tracks(vec![track.clone()], 0);
                self.snapshot.track = Some(track);
                self.snapshot.track_rev += 1;
                self.snapshot.restored = true;
                self.snapshot.playing = false;
                self.snapshot.paused = true;
                self.snapshot.resolving = false;
                self.snapshot.loading_lyrics = false;
                self.snapshot.lyrics_offset_secs = 0.0;
                self.snapshot.error = None;
                self.snapshot.audio_info.clear();
                self.pending_restore_seek = None;
                self.position = PositionTick {
                    position_secs: 0.0,
                    duration_secs: dur,
                    buffering: false,
                    active_lyric_idx: None,
                };
                let _ = self.pos_tx.send(self.position);
                self.publish_queue();
            }
            AppCommand::RestoreSession {
                tracks,
                index,
                position_secs,
                shuffle,
                repeat,
            } => {
                if !self.queue.is_empty() || self.snapshot.track.is_some() || tracks.is_empty() {
                    return;
                }
                self.cancel_inflight_loads();
                let Some(track) = self.queue.restore_session(tracks, index, shuffle, repeat) else {
                    return;
                };
                let dur = track.duration.map(f64::from).unwrap_or(0.0);
                let clamped_pos = if dur > 0.0 {
                    position_secs.clamp(0.0, (dur - 2.0).max(0.0))
                } else {
                    position_secs.max(0.0)
                };
                self.snapshot.track = Some(track);
                self.snapshot.track_rev += 1;
                self.snapshot.restored = true;
                self.snapshot.playing = false;
                self.snapshot.paused = true;
                self.snapshot.resolving = false;
                self.snapshot.loading_lyrics = false;
                self.snapshot.lyrics_offset_secs = 0.0;
                self.snapshot.error = None;
                self.snapshot.audio_info.clear();
                self.pending_restore_seek = (clamped_pos > 1.0).then_some(clamped_pos);
                self.position = PositionTick {
                    position_secs: clamped_pos,
                    duration_secs: dur,
                    buffering: false,
                    active_lyric_idx: None,
                };
                let _ = self.pos_tx.send(self.position);
                self.publish_queue();
            }
            AppCommand::StopPlayback => {
                self.cancel_inflight_loads();
                self.load_gen += 1;
                self.invalidate_gapless_tail().await;
                let _ = self.backend.stop_track().await;
                self.snapshot.playing = false;
                self.snapshot.paused = true;
                self.snapshot.resolving = false;
                self.snapshot.loading_lyrics = false;
                self.snapshot.restored = self.snapshot.track.is_some();
                self.snapshot.error = None;
                self.pending_restore_seek = None;
                self.position.position_secs = 0.0;
                self.position.buffering = false;
                self.position.active_lyric_idx = None;
                let _ = self.pos_tx.send(self.position);
                self.publish_snapshot();
            }
            AppCommand::TogglePause => {
                if (self.snapshot.restored
                    || self.snapshot.track.is_none()
                    || self.snapshot.error.is_some())
                    && let Some(cur) = self.queue.current().cloned()
                {
                    self.load_track(cur, false).await;
                    return;
                }
                if let Ok(paused) = self.backend.toggle_pause().await {
                    self.snapshot.paused = paused;
                    self.snapshot.playing = !paused && self.snapshot.track.is_some();
                    self.publish_snapshot();
                }
            }
            AppCommand::ToggleMute => {
                if self.snapshot.muted {
                    let restore = self
                        .pre_mute_volume
                        .take()
                        .unwrap_or_else(|| self.settings.player.volume.max(10));
                    if let Ok(v) = self.backend.set_volume(restore).await {
                        self.snapshot.muted = false;
                        self.snapshot.volume = v;
                        self.settings.player.volume = v;
                        self.publish_snapshot();
                    }
                } else {
                    if self.snapshot.volume > 0 {
                        self.pre_mute_volume = Some(self.snapshot.volume);
                    }
                    if let Ok(v) = self.backend.set_volume(0).await {
                        self.snapshot.muted = true;
                        self.snapshot.volume = v;
                        self.publish_snapshot();
                    }
                }
            }
            AppCommand::Next => {
                self.flush_watchtime_if_needed().await;
                self.invalidate_gapless_tail().await;
                if let Some(next) = self.queue.advance(true) {
                    self.pending_autofill_advance = false;
                    self.publish_queue();
                    self.load_track(next, false).await;
                } else if self.snapshot.autoplay_active && self.settings.player.autoplay {
                    self.pending_autofill_advance = true;
                    self.snapshot.resolving = true;
                    self.publish_snapshot();
                    self.maybe_autofill_radio().await;
                }
            }
            AppCommand::Previous => {
                if self.position.position_secs > 4.0 {
                    let _ = self.backend.seek(0.0, false).await;
                    let _ = self
                        .events_tx
                        .send(PlayerEvent::Seeked { position_secs: 0.0 });
                    return;
                }
                if let Some(prev) = self.queue.retreat() {
                    self.invalidate_gapless_tail().await;
                    self.publish_queue();
                    self.load_track(prev, false).await;
                } else if self.snapshot.track.is_some() && !self.snapshot.restored {
                    let _ = self.backend.seek(0.0, false).await;
                    let _ = self
                        .events_tx
                        .send(PlayerEvent::Seeked { position_secs: 0.0 });
                }
            }
            AppCommand::SeekRelative { seconds } => {
                let _ = self.backend.seek(seconds, true).await;
                let max_pos = if self.position.duration_secs > 0.0 {
                    self.position.duration_secs
                } else {
                    f64::MAX
                };
                let target = (self.position.position_secs + seconds).clamp(0.0, max_pos);
                let _ = self.events_tx.send(PlayerEvent::Seeked {
                    position_secs: target,
                });
            }
            AppCommand::SeekAbsolute { seconds } => {
                let target = seconds.max(0.0);
                let _ = self.backend.seek(target, false).await;
                let _ = self.events_tx.send(PlayerEvent::Seeked {
                    position_secs: target,
                });
            }
            AppCommand::AdjustLyricsOffset { delta_secs } => {
                let next = if delta_secs == 0.0 {
                    0.0
                } else {
                    let raw = (self.snapshot.lyrics_offset_secs + delta_secs).clamp(-10.0, 10.0);
                    (raw * 20.0).round() / 20.0
                };
                self.snapshot.lyrics_offset_secs = next;
                self.position.active_lyric_idx = self
                    .current_lyrics
                    .1
                    .active_line_index((self.position.position_secs + next).max(0.0));
                let _ = self.pos_tx.send(self.position);
                self.publish_snapshot();
                let label = if next.abs() < 1e-6 {
                    "Lyrics offset reset (0.00s)".to_string()
                } else {
                    format!("Lyrics offset: {next:+.2}s")
                };
                let _ = self.events_tx.send(PlayerEvent::StatusToast(label));
            }
            AppCommand::SetVolume { volume } => {
                if let Ok(v) = self.backend.set_volume(volume).await {
                    self.snapshot.volume = v;
                    self.snapshot.muted = v == 0;
                    if v > 0 {
                        self.pre_mute_volume = None;
                        self.settings.player.volume = v;
                    }
                    self.publish_snapshot();
                }
            }
            AppCommand::AdjustVolume { delta } => {
                let base = if self.snapshot.muted {
                    self.pre_mute_volume.unwrap_or(0)
                } else {
                    self.snapshot.volume
                };
                let target = (i16::from(base) + delta).clamp(0, 130) as u8;
                if let Ok(v) = self.backend.set_volume(target).await {
                    self.snapshot.volume = v;
                    self.snapshot.muted = v == 0;
                    if v > 0 {
                        self.pre_mute_volume = None;
                        self.settings.player.volume = v;
                    }
                    self.publish_snapshot();
                }
            }
            AppCommand::ToggleShuffle => {
                self.queue.toggle_shuffle();
                self.invalidate_gapless_tail().await;
                self.publish_queue();
                self.schedule_prefetch_and_gapless();
            }
            AppCommand::CycleRepeat => {
                self.queue.cycle_repeat();
                self.invalidate_gapless_tail().await;
                self.publish_queue();
                self.schedule_prefetch_and_gapless();
            }
            AppCommand::JumpToQueueIndex { index } => {
                if let Some(track) = self.queue.jump(index) {
                    self.invalidate_gapless_tail().await;
                    self.publish_queue();
                    self.load_track(track, false).await;
                }
            }
            AppCommand::MoveQueueItem { from, to } => {
                if self.queue.move_item(from, to) {
                    self.invalidate_gapless_tail().await;
                    self.publish_queue();
                    self.schedule_prefetch_and_gapless();
                }
            }
            AppCommand::RemoveQueueItem { index } => {
                let removed_active = self.queue.index() == Some(index);
                if self.queue.remove(index) {
                    self.invalidate_gapless_tail().await;
                    self.publish_queue();
                    if removed_active {
                        if let Some(cur) = self.queue.current().cloned() {
                            self.load_track(cur, false).await;
                        } else {
                            self.cancel_inflight_loads();
                            self.load_gen += 1;
                            let _ = self.backend.stop_track().await;
                            self.snapshot.track = None;
                            self.snapshot.playing = false;
                            self.snapshot.resolving = false;
                            self.snapshot.restored = false;
                            self.position.buffering = false;
                            let _ = self.pos_tx.send(self.position);
                            self.publish_snapshot();
                        }
                    } else {
                        self.schedule_prefetch_and_gapless();
                    }
                }
            }
            AppCommand::PromoteQueueItemToNext { index } => {
                if self.queue.move_to_next(index) {
                    self.invalidate_gapless_tail().await;
                    self.publish_queue();
                    self.schedule_prefetch_and_gapless();
                    let _ = self
                        .events_tx
                        .send(PlayerEvent::StatusToast("Moved to play next".into()));
                }
            }
            AppCommand::UndoQueue => {
                let prev_vid = self.snapshot.track.as_ref().map(|t| t.video_id.clone());
                if self.queue.undo() {
                    self.invalidate_gapless_tail().await;
                    self.publish_queue();
                    match self.queue.current().cloned() {
                        Some(cur) if prev_vid.as_deref() != Some(cur.video_id.as_str()) => {
                            self.load_track(cur, false).await;
                        }
                        None if prev_vid.is_some() => {
                            self.cancel_inflight_loads();
                            self.load_gen += 1;
                            let _ = self.backend.stop_track().await;
                            self.snapshot.track = None;
                            self.snapshot.playing = false;
                            self.snapshot.resolving = false;
                            self.snapshot.restored = false;
                            self.position.buffering = false;
                            let _ = self.pos_tx.send(self.position);
                            self.publish_snapshot();
                        }
                        _ => self.schedule_prefetch_and_gapless(),
                    }
                    let _ = self
                        .events_tx
                        .send(PlayerEvent::StatusToast("Undid queue change".into()));
                }
            }
            AppCommand::DeduplicateQueue => {
                let removed = self.queue.deduplicate();
                if removed > 0 {
                    self.invalidate_gapless_tail().await;
                    self.publish_queue();
                    let _ = self.events_tx.send(PlayerEvent::StatusToast(format!(
                        "Removed {removed} duplicate track(s)"
                    )));
                }
            }
            AppCommand::ClearUpcomingQueue => {
                let removed = self.queue.clear_upcoming();
                if removed > 0 {
                    self.radio_gen += 1;
                    self.radio_inflight = false;
                    self.invalidate_gapless_tail().await;
                    self.publish_queue();
                    let _ = self.events_tx.send(PlayerEvent::StatusToast(format!(
                        "Cleared {removed} upcoming track(s)"
                    )));
                }
            }
            AppCommand::ClearQueue => {
                self.cancel_inflight_loads();
                self.load_gen += 1;
                self.radio_gen += 1;
                self.radio_inflight = false;
                self.radio_continuation = None;
                self.radio_playlist_id = None;
                self.queue.clear();
                self.invalidate_gapless_tail().await;
                let _ = self.backend.stop_track().await;
                self.snapshot.track = None;
                self.snapshot.playing = false;
                self.snapshot.paused = false;
                self.snapshot.resolving = false;
                self.snapshot.loading_lyrics = false;
                self.snapshot.translating_lyrics = false;
                self.snapshot.lyrics_offset_secs = 0.0;
                self.snapshot.restored = false;
                self.snapshot.autoplay_active = false;
                self.snapshot.radio_active = false;
                self.snapshot.error = None;
                self.pending_restore_seek = None;
                self.position = PositionTick::default();
                let _ = self.pos_tx.send(self.position);
                self.publish_queue();
                self.publish_lyrics(None, Lyrics::default());
                let _ = self
                    .events_tx
                    .send(PlayerEvent::StatusToast("Queue cleared".into()));
            }
            AppCommand::ToggleLyricsTranslation => {
                self.snapshot.show_translation = !self.snapshot.show_translation;
                self.settings.lyrics.translate_to_id = self.snapshot.show_translation;
                self.lyrics_svc.update_settings(self.settings.clone());

                if self.snapshot.show_translation
                    && !self.current_lyrics.1.is_empty()
                    && self.current_lyrics.1.translation_lang != "id"
                    && let Some(vid) = self.current_lyrics.0.clone()
                {
                    self.spawn_lyrics_translation(vid);
                } else {
                    self.publish_snapshot();
                }
            }
            AppCommand::ApplySettings { settings } => {
                self.apply_settings_command(settings).await;
            }
            AppCommand::Shutdown => {}
        }
    }

    fn spawn_lyrics_translation(&mut self, vid: String) {
        self.snapshot.translating_lyrics = true;
        self.publish_snapshot();
        let lyr = self.current_lyrics.1.clone();
        let lyr_svc = self.lyrics_svc.clone();
        let tx = self.internal_tx.clone();
        let epoch = self.load_gen;
        tokio::spawn(async move {
            let translated = lyr_svc.translate_lyrics(lyr, Some(&vid), "id").await;
            let _ = tx.send(InternalMsg::LyricsTranslated {
                epoch,
                video_id: vid,
                lyrics: translated,
            });
        });
    }

    async fn apply_settings_command(&mut self, settings: musick_core::Settings) {
        let prev = self.settings.clone();
        self.settings = settings.clone();
        self.resolver.set_quality(settings.player.audio_quality);
        self.lyrics_svc.update_settings(settings.clone());
        self.snapshot.normalize_audio = settings.player.normalize_audio;
        if !self.snapshot.muted {
            self.snapshot.volume = settings.player.volume;
        }
        self.snapshot.show_translation = settings.lyrics.translate_to_id;

        if !settings.player.autoplay {
            self.snapshot.autoplay_active = false;
        }

        if prev.player.use_mpv_config != settings.player.use_mpv_config
            && !self.backend.properties().core_idle
        {
            let _ = self.events_tx.send(PlayerEvent::StatusToast(
                "mpv.conf change applies after playback stops".into(),
            ));
        }

        let mut effective = settings.clone();
        if self.snapshot.muted {
            effective.player.volume = 0;
        }
        let _ = self.backend.apply_settings(&effective).await;

        if (!settings.player.gapless && prev.player.gapless)
            || prev.player.audio_quality != settings.player.audio_quality
        {
            self.invalidate_gapless_tail().await;
        }
        if prev.player.gapless != settings.player.gapless
            || prev.player.prefetch != settings.player.prefetch
            || prev.player.audio_quality != settings.player.audio_quality
        {
            self.schedule_prefetch_and_gapless();
        }

        if let Some(cur) = self.snapshot.track.clone() {
            let lyrics_changed = prev.lyrics.enabled != settings.lyrics.enabled
                || prev.lyrics.source != settings.lyrics.source
                || prev.lyrics.synced_only != settings.lyrics.synced_only;
            if !settings.lyrics.enabled {
                if prev.lyrics.enabled || !self.current_lyrics.1.is_empty() {
                    self.snapshot.loading_lyrics = false;
                    self.snapshot.translating_lyrics = false;
                    self.publish_lyrics(Some(cur.video_id), Lyrics::default());
                } else {
                    self.publish_snapshot();
                }
            } else if lyrics_changed {
                self.snapshot.loading_lyrics = true;
                self.snapshot.translating_lyrics = false;
                self.publish_lyrics(Some(cur.video_id.clone()), Lyrics::default());
                self.spawn_lyrics_load(self.load_gen, cur);
            } else if settings.lyrics.translate_to_id
                && !prev.lyrics.translate_to_id
                && !self.current_lyrics.1.is_empty()
                && self.current_lyrics.1.translation_lang != "id"
            {
                self.spawn_lyrics_translation(cur.video_id);
            } else {
                self.publish_snapshot();
            }
        } else {
            self.publish_snapshot();
        }
    }
}
