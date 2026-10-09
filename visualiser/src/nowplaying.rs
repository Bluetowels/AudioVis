//! What is playing in other apps, and how far through it is, from Windows'
//! media controls (the same information as the volume flyout shows). Spotify,
//! Tidal, browsers and most players report there. Other systems have no
//! equivalent wired up, so there nothing is ever reported as playing.

use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Track {
    /// The player's id, such as "Spotify.exe".
    pub app: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// Length in seconds, 0 if the player does not say.
    pub duration: f64,
}

/// What is playing at this moment.
pub struct Now {
    pub track: Track,
    /// Seconds into the track, if the player has ever said.
    pub position: Option<f64>,
}

#[derive(Default)]
struct Shared {
    track: Option<Track>,
    playing: bool,
    /// The position when it was last worked out, and when that was. While
    /// playing, the position now is that plus the time since.
    position: Option<(f64, Instant)>,
}

pub struct NowPlaying {
    shared: Arc<Mutex<Shared>>,
}

impl NowPlaying {
    /// Start watching, on a thread that stops when this is dropped.
    pub fn start() -> Self {
        let shared = Arc::new(Mutex::new(Shared::default()));
        #[cfg(windows)]
        {
            let weak = Arc::downgrade(&shared);
            std::thread::spawn(move || {
                let _ = windows_media::watch(weak);
            });
        }
        Self { shared }
    }

    pub fn now(&self) -> Option<Now> {
        let shared = self.shared.lock().unwrap();
        let track = shared.track.clone()?;
        let position = shared.position.map(|(position, at)| {
            let position = if shared.playing { position + at.elapsed().as_secs_f64() } else { position };
            if track.duration > 0.0 { position.min(track.duration) } else { position }
        });
        Some(Now { track, position })
    }
}

#[cfg(windows)]
mod windows_media {
    use super::{Shared, Track};
    use std::sync::mpsc::{Sender, channel};
    use std::sync::{Mutex, Weak};
    use std::time::{Duration, Instant};
    use windows::Foundation::TypedEventHandler;
    use windows::Media::Control::{
        GlobalSystemMediaTransportControlsSession as Session, GlobalSystemMediaTransportControlsSessionManager as Manager,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
    };
    use windows::core::Result;

    /// Windows counts time in steps of 100 ns.
    const TICKS: f64 = 1e7;
    /// Seconds from 1601, where Windows' clock starts, to 1970.
    const TO_1970_S: f64 = 11_644_473_600.0;

    /// The session being listened to and its three event registrations.
    struct Listening {
        session: Session,
        tokens: [i64; 3],
    }

    impl Listening {
        fn start(session: Session, wake: &Sender<bool>) -> Result<Self> {
            let (a, b, c) = (wake.clone(), wake.clone(), wake.clone());
            let tokens = [
                session.MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
                    let _ = a.send(false);
                    Ok(())
                }))?,
                session.TimelinePropertiesChanged(&TypedEventHandler::new(move |_, _| {
                    let _ = b.send(false);
                    Ok(())
                }))?,
                session.PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
                    let _ = c.send(false);
                    Ok(())
                }))?,
            ];
            Ok(Self { session, tokens })
        }
    }

    impl Drop for Listening {
        fn drop(&mut self) {
            let _ = self.session.RemoveMediaPropertiesChanged(self.tokens[0]);
            let _ = self.session.RemoveTimelinePropertiesChanged(self.tokens[1]);
            let _ = self.session.RemovePlaybackInfoChanged(self.tokens[2]);
        }
    }

    /// What one look at a session gave.
    struct Seen {
        track: Track,
        playing: bool,
        /// Seconds into the track when the player last reported, and when
        /// that was (Windows clock, 0 if it never has).
        position: f64,
        reported: i64,
    }

    fn look(session: &Session) -> Result<Seen> {
        let properties = session.TryGetMediaPropertiesAsync()?.join()?;
        let timeline = session.GetTimelineProperties()?;
        let start = timeline.StartTime()?.Duration;
        Ok(Seen {
            track: Track {
                app: session.SourceAppUserModelId()?.to_string(),
                title: properties.Title()?.to_string(),
                artist: properties.Artist()?.to_string(),
                album: properties.AlbumTitle()?.to_string(),
                duration: (timeline.EndTime()?.Duration - start) as f64 / TICKS,
            },
            playing: session.GetPlaybackInfo()?.PlaybackStatus()? == Status::Playing,
            position: (timeline.Position()?.Duration - start) as f64 / TICKS,
            reported: timeline.LastUpdatedTime()?.UniversalTime,
        })
    }

    /// Runs until the `NowPlaying` that started it is dropped.
    pub fn watch(shared: Weak<Mutex<Shared>>) -> Result<()> {
        let manager = Manager::RequestAsync()?.join()?;
        // `true` means a different app has become the one in control.
        let (wake, woken) = channel::<bool>();
        let changed = wake.clone();
        manager.CurrentSessionChanged(&TypedEventHandler::new(move |_, _| {
            let _ = changed.send(true);
            Ok(())
        }))?;

        let mut listening: Option<Listening> = None;
        let mut reconnect = true;
        // The last report from the player that the position was taken from.
        let mut reported = 0i64;
        loop {
            let Some(shared) = shared.upgrade() else { return Ok(()) };
            if reconnect || listening.is_none() {
                listening = manager.GetCurrentSession().ok().and_then(|session| Listening::start(session, &wake).ok());
                reconnect = false;
            }
            let seen = listening.as_ref().and_then(|l| look(&l.session).ok());
            {
                let mut shared = shared.lock().unwrap();
                let now = Instant::now();
                match seen {
                    None => *shared = Shared::default(),
                    Some(seen) => {
                        let same_track = shared.track.as_ref().is_some_and(|t| {
                            t.app == seen.track.app && t.title == seen.track.title && t.artist == seen.track.artist
                        });
                        // Position is only reported now and then (on play, pause
                        // and seek, and at the start of a track), so between
                        // reports it is counted forward from the last one.
                        if seen.reported != reported || shared.track.is_none() {
                            let wall = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_secs_f64() + TO_1970_S)
                                .unwrap_or(0.0);
                            let since = if seen.playing { (wall - seen.reported as f64 / TICKS).max(0.0) } else { 0.0 };
                            shared.position = (seen.reported > 0).then_some((seen.position + since, now));
                            reported = seen.reported;
                        } else if !same_track {
                            // A new track with no new report: take it as just started.
                            shared.position = shared.position.map(|_| (0.0, now));
                        } else if seen.playing != shared.playing {
                            // Paused or resumed without a report: hold, or carry on from, where it was.
                            shared.position = shared.position.map(|(position, at)| {
                                (if shared.playing { position + (now - at).as_secs_f64() } else { position }, now)
                            });
                        }
                        shared.playing = seen.playing;
                        shared.track = Some(seen.track);
                    }
                }
            }
            drop(shared);
            // Wait for news, looking again every second in case an event was missed.
            match woken.recv_timeout(Duration::from_secs(1)) {
                Ok(session_changed) => reconnect = session_changed || woken.try_iter().any(|changed| changed),
                Err(_) => {}
            }
        }
    }
}
