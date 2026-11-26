#[cfg(target_os = "linux")]
mod linux_impl {
    use mpris_server::{
        LoopStatus, Metadata, PlaybackRate, PlaybackStatus, PlayerInterface, Property,
        RootInterface, Server, Time, TrackId, Volume, zbus::fdo,
    };
    use tokio::task::JoinHandle;
    use tracing::debug;

    use musick_core::{AppCommand, PlaybackSnapshot, RepeatMode};

    use crate::engine::PlayerEngineHandle;

    struct MusickMpris {
        engine: PlayerEngineHandle,
    }

    fn build_metadata(snap: &PlaybackSnapshot) -> Metadata {
        let mut meta = Metadata::new();
        let Some(ref tr) = snap.track else {
            return meta;
        };
        let track_path = format!(
            "/io/musick/track/{}",
            tr.video_id
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .collect::<String>()
        );
        if let Ok(tid) = TrackId::try_from(track_path) {
            meta.set_trackid(Some(tid));
        }
        meta.set_title(Some(tr.title.clone()));
        let artists: Vec<String> = tr.artists.iter().map(|a| a.name.clone()).collect();
        if !artists.is_empty() {
            meta.set_artist(Some(artists));
        }
        if let Some(ref album) = tr.album {
            meta.set_album(Some(album.clone()));
        }
        if let Some(art_url) = tr.cover_url() {
            meta.set_art_url(Some(art_url));
        }
        if let Some(dur) = tr.duration {
            meta.set_length(Some(Time::from_secs(i64::from(dur))));
        }
        meta
    }

    fn playback_status_for(snap: &PlaybackSnapshot) -> PlaybackStatus {
        if snap.track.is_none() {
            PlaybackStatus::Stopped
        } else if snap.paused || !snap.playing {
            PlaybackStatus::Paused
        } else {
            PlaybackStatus::Playing
        }
    }

    fn loop_status_for(mode: RepeatMode) -> LoopStatus {
        match mode {
            RepeatMode::Off => LoopStatus::None,
            RepeatMode::All => LoopStatus::Playlist,
            RepeatMode::One => LoopStatus::Track,
        }
    }

    impl RootInterface for MusickMpris {
        async fn raise(&self) -> fdo::Result<()> {
            Ok(())
        }

        async fn quit(&self) -> fdo::Result<()> {
            self.engine.send(AppCommand::Shutdown).await;
            Ok(())
        }

        async fn can_quit(&self) -> fdo::Result<bool> {
            Ok(true)
        }

        async fn fullscreen(&self) -> fdo::Result<bool> {
            Ok(false)
        }

        async fn set_fullscreen(&self, _fullscreen: bool) -> mpris_server::zbus::Result<()> {
            Ok(())
        }

        async fn can_set_fullscreen(&self) -> fdo::Result<bool> {
            Ok(false)
        }

        async fn can_raise(&self) -> fdo::Result<bool> {
            Ok(false)
        }

        async fn has_track_list(&self) -> fdo::Result<bool> {
            Ok(false)
        }

        async fn identity(&self) -> fdo::Result<String> {
            Ok("musick".to_string())
        }

        async fn desktop_entry(&self) -> fdo::Result<String> {
            Ok("musick".to_string())
        }

        async fn supported_uri_schemes(&self) -> fdo::Result<Vec<String>> {
            Ok(vec!["https".to_string()])
        }

        async fn supported_mime_types(&self) -> fdo::Result<Vec<String>> {
            Ok(vec!["audio/ogg".to_string(), "audio/mp4".to_string()])
        }
    }

    impl PlayerInterface for MusickMpris {
        async fn next(&self) -> fdo::Result<()> {
            self.engine.send(AppCommand::Next).await;
            Ok(())
        }

        async fn previous(&self) -> fdo::Result<()> {
            self.engine.send(AppCommand::Previous).await;
            Ok(())
        }

        async fn pause(&self) -> fdo::Result<()> {
            let snap = self.engine.snapshot();
            if !snap.paused && snap.playing {
                self.engine.send(AppCommand::TogglePause).await;
            }
            Ok(())
        }

        async fn play_pause(&self) -> fdo::Result<()> {
            self.engine.send(AppCommand::TogglePause).await;
            Ok(())
        }

        async fn stop(&self) -> fdo::Result<()> {
            self.engine.send(AppCommand::ClearQueue).await;
            Ok(())
        }

        async fn play(&self) -> fdo::Result<()> {
            let snap = self.engine.snapshot();
            if snap.paused || !snap.playing {
                self.engine.send(AppCommand::TogglePause).await;
            }
            Ok(())
        }

        async fn seek(&self, offset: Time) -> fdo::Result<()> {
            self.engine
                .send(AppCommand::SeekRelative {
                    seconds: offset.as_micros() as f64 / 1_000_000.0,
                })
                .await;
            Ok(())
        }

        async fn set_position(&self, _track_id: TrackId, position: Time) -> fdo::Result<()> {
            self.engine
                .send(AppCommand::SeekAbsolute {
                    seconds: (position.as_micros() as f64 / 1_000_000.0).max(0.0),
                })
                .await;
            Ok(())
        }

        async fn open_uri(&self, _uri: String) -> fdo::Result<()> {
            Ok(())
        }

        async fn playback_status(&self) -> fdo::Result<PlaybackStatus> {
            Ok(playback_status_for(&self.engine.snapshot()))
        }

        async fn loop_status(&self) -> fdo::Result<LoopStatus> {
            Ok(loop_status_for(self.engine.snapshot().repeat))
        }

        async fn set_loop_status(&self, loop_status: LoopStatus) -> mpris_server::zbus::Result<()> {
            let cur = self.engine.snapshot().repeat;
            let want = match loop_status {
                LoopStatus::None => RepeatMode::Off,
                LoopStatus::Playlist => RepeatMode::All,
                LoopStatus::Track => RepeatMode::One,
            };
            let mut mode = cur;
            for _ in 0..3 {
                if mode == want {
                    break;
                }
                self.engine.send(AppCommand::CycleRepeat).await;
                mode = mode.cycle();
            }
            Ok(())
        }

        async fn rate(&self) -> fdo::Result<PlaybackRate> {
            Ok(1.0)
        }

        async fn set_rate(&self, _rate: PlaybackRate) -> mpris_server::zbus::Result<()> {
            Ok(())
        }

        async fn shuffle(&self) -> fdo::Result<bool> {
            Ok(self.engine.snapshot().shuffle)
        }

        async fn set_shuffle(&self, shuffle: bool) -> mpris_server::zbus::Result<()> {
            if self.engine.snapshot().shuffle != shuffle {
                self.engine.send(AppCommand::ToggleShuffle).await;
            }
            Ok(())
        }

        async fn metadata(&self) -> fdo::Result<Metadata> {
            Ok(build_metadata(&self.engine.snapshot()))
        }

        async fn volume(&self) -> fdo::Result<Volume> {
            Ok(f64::from(self.engine.snapshot().volume) / 100.0)
        }

        async fn set_volume(&self, volume: Volume) -> mpris_server::zbus::Result<()> {
            let pct = (volume * 100.0).round().clamp(0.0, 130.0) as u8;
            self.engine
                .send(AppCommand::SetVolume { volume: pct })
                .await;
            Ok(())
        }

        async fn position(&self) -> fdo::Result<Time> {
            let secs = self.engine.position().position_secs;
            Ok(Time::from_micros((secs * 1_000_000.0) as i64))
        }

        async fn minimum_rate(&self) -> fdo::Result<PlaybackRate> {
            Ok(1.0)
        }

        async fn maximum_rate(&self) -> fdo::Result<PlaybackRate> {
            Ok(1.0)
        }

        async fn can_go_next(&self) -> fdo::Result<bool> {
            Ok(true)
        }

        async fn can_go_previous(&self) -> fdo::Result<bool> {
            Ok(true)
        }

        async fn can_play(&self) -> fdo::Result<bool> {
            Ok(true)
        }

        async fn can_pause(&self) -> fdo::Result<bool> {
            Ok(true)
        }

        async fn can_seek(&self) -> fdo::Result<bool> {
            Ok(true)
        }

        async fn can_control(&self) -> fdo::Result<bool> {
            Ok(true)
        }
    }

    pub struct MprisService {
        task: JoinHandle<()>,
    }

    impl MprisService {
        pub fn spawn(engine: PlayerEngineHandle) -> Self {
            let task = tokio::spawn(async move {
                let suffix = format!("musick.instance{}", std::process::id());
                let server = match Server::new(
                    &suffix,
                    MusickMpris {
                        engine: engine.clone(),
                    },
                )
                .await
                {
                    Ok(s) => s,
                    Err(err) => {
                        debug!("MPRIS D-Bus unavailable (non-fatal): {err}");
                        return;
                    }
                };

                let mut snap_rx = engine.snapshot_rx.clone();
                let mut prev = snap_rx.borrow().clone();
                while snap_rx.changed().await.is_ok() {
                    let cur = snap_rx.borrow().clone();
                    let mut changed = Vec::new();
                    if cur.playing != prev.playing
                        || cur.paused != prev.paused
                        || cur.track.is_some() != prev.track.is_some()
                    {
                        changed.push(Property::PlaybackStatus(playback_status_for(&cur)));
                    }
                    if cur.track_rev != prev.track_rev {
                        changed.push(Property::Metadata(build_metadata(&cur)));
                    }
                    if cur.volume != prev.volume {
                        changed.push(Property::Volume(f64::from(cur.volume) / 100.0));
                    }
                    if cur.shuffle != prev.shuffle {
                        changed.push(Property::Shuffle(cur.shuffle));
                    }
                    if cur.repeat != prev.repeat {
                        changed.push(Property::LoopStatus(loop_status_for(cur.repeat)));
                    }
                    if !changed.is_empty() {
                        let _ = server.properties_changed(changed).await;
                    }
                    prev = cur;
                }
            });
            Self { task }
        }

        pub fn shutdown(self) {
            self.task.abort();
        }
    }
}

#[cfg(target_os = "linux")]
pub use linux_impl::MprisService;

#[cfg(not(target_os = "linux"))]
pub struct MprisService;

#[cfg(not(target_os = "linux"))]
impl MprisService {
    pub fn spawn(_engine: crate::engine::PlayerEngineHandle) -> Self {
        Self
    }

    pub fn shutdown(self) {}
}
