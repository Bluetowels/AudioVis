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

/// A track's cover picture, shrunk to a small square.
pub struct Cover {
    /// Pixels along each side.
    pub size: usize,
    /// Red, green, blue and an unused byte for each pixel, row by row from the top.
    pub rgba: Vec<u8>,
}

/// What is playing at this moment.
pub struct Now {
    pub track: Track,
    pub cover: Option<Arc<Cover>>,
    /// Seconds into the track, if the player has ever said.
    pub position: Option<f64>,
}

#[derive(Default)]
struct Shared {
    track: Option<Track>,
    cover: Option<Arc<Cover>>,
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
        Some(Now { track, cover: shared.cover.clone(), position })
    }
}

#[cfg(windows)]
mod windows_media {
    use super::{Cover, Shared, Track};
    use std::sync::mpsc::{Sender, channel};
    use std::sync::{Arc, Mutex, Weak};
    use std::time::{Duration, Instant};
    use windows::Foundation::TypedEventHandler;
    use windows::Graphics::Imaging::{
        BitmapAlphaMode, BitmapDecoder, BitmapInterpolationMode, BitmapPixelFormat, BitmapTransform, ColorManagementMode,
        ExifOrientationMode,
    };
    use windows::Media::Control::GlobalSystemMediaTransportControlsSessionMediaProperties as Properties;
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

    /// Side of the square the cover is shrunk to, in pixels.
    const COVER_SIZE: u32 = 96;
    /// Players often hand over the cover a moment after the title, so it is
    /// asked for this many times as a track starts.
    const COVER_TRIES: u8 = 3;

    /// The track's cover, decoded by Windows and shrunk to a small square.
    fn cover(properties: &Properties) -> Result<Cover> {
        let stream = properties.Thumbnail()?.OpenReadAsync()?.join()?;
        let decoder = BitmapDecoder::CreateAsync(&stream)?.join()?;
        let shrink = BitmapTransform::new()?;
        shrink.SetScaledWidth(COVER_SIZE)?;
        shrink.SetScaledHeight(COVER_SIZE)?;
        shrink.SetInterpolationMode(BitmapInterpolationMode::Fant)?;
        let pixels = decoder
            .GetPixelDataTransformedAsync(
                BitmapPixelFormat::Rgba8,
                BitmapAlphaMode::Ignore,
                &shrink,
                ExifOrientationMode::IgnoreExifOrientation,
                ColorManagementMode::DoNotColorManage,
            )?
            .join()?
            .DetachPixelData()?;
        Ok(Cover { size: COVER_SIZE as usize, rgba: pixels.to_vec() })
    }

    /// What one look at a session gave.
    struct Seen {
        track: Track,
        playing: bool,
        /// Seconds into the track when the player last reported, and when
        /// that was (Windows clock, 0 if it never has).
        position: f64,
        reported: i64,
        /// The cover, if it was asked for and the player has one.
        cover: Option<Cover>,
    }

    fn look(session: &Session, want_cover: impl Fn(&Track) -> bool) -> Result<Seen> {
        let properties = session.TryGetMediaPropertiesAsync()?.join()?;
        let timeline = session.GetTimelineProperties()?;
        let start = timeline.StartTime()?.Duration;
        let track = Track {
            app: session.SourceAppUserModelId()?.to_string(),
            title: properties.Title()?.to_string(),
            artist: properties.Artist()?.to_string(),
            album: properties.AlbumTitle()?.to_string(),
            duration: (timeline.EndTime()?.Duration - start) as f64 / TICKS,
        };
        let cover = if want_cover(&track) { cover(&properties).ok() } else { None };
        Ok(Seen {
            track,
            playing: session.GetPlaybackInfo()?.PlaybackStatus()? == Status::Playing,
            position: (timeline.Position()?.Duration - start) as f64 / TICKS,
            reported: timeline.LastUpdatedTime()?.UniversalTime,
            cover,
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
        // The track the cover was last asked for, and how many times.
        let mut cover_asked: (Option<Track>, u8) = (None, 0);
        loop {
            let Some(shared) = shared.upgrade() else { return Ok(()) };
            if reconnect || listening.is_none() {
                listening = manager.GetCurrentSession().ok().and_then(|session| Listening::start(session, &wake).ok());
                reconnect = false;
            }
            let seen = listening
                .as_ref()
                .and_then(|l| look(&l.session, |track| cover_asked.0.as_ref() != Some(track) || cover_asked.1 < COVER_TRIES).ok());
            if let Some(seen) = &seen {
                cover_asked = if cover_asked.0.as_ref() == Some(&seen.track) { (cover_asked.0, cover_asked.1 + 1) } else { (Some(seen.track.clone()), 1) };
            }
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
                        if seen.cover.is_some() {
                            shared.cover = seen.cover.map(Arc::new);
                        } else if !same_track {
                            shared.cover = None;
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
