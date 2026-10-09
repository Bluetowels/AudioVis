//! Prints what Windows' media controls report about the playing track, four
//! times a second for about twenty seconds. Used to see how well each music
//! app fills in the position: `cargo run --example smtc_probe`.

#[cfg(windows)]
fn main() -> windows::core::Result<()> {
    use std::time::{Duration, Instant};
    use windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager as Manager;

    let seconds: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(20);
    let manager = Manager::RequestAsync()?.join()?;
    let start = Instant::now();
    let mut last_updated = 0i64;
    while start.elapsed() < Duration::from_secs(seconds) {
        let t = start.elapsed().as_secs_f32();
        match manager.GetCurrentSession() {
            Ok(session) => {
                let app = session.SourceAppUserModelId().map(|s| s.to_string()).unwrap_or_default();
                let (title, artist, album) = match session.TryGetMediaPropertiesAsync().and_then(|op| op.join()) {
                    Ok(p) => (
                        p.Title().map(|s| s.to_string()).unwrap_or_default(),
                        p.Artist().map(|s| s.to_string()).unwrap_or_default(),
                        p.AlbumTitle().map(|s| s.to_string()).unwrap_or_default(),
                    ),
                    Err(e) => (format!("<no properties: {e}>"), String::new(), String::new()),
                };
                let status = session.GetPlaybackInfo().and_then(|i| i.PlaybackStatus()).map(|s| s.0).unwrap_or(-1);
                let tl = session.GetTimelineProperties()?;
                // Times are in 100 ns steps; LastUpdatedTime counts from 1601.
                let position = tl.Position()?.Duration as f64 / 1e7;
                let end = tl.EndTime()?.Duration as f64 / 1e7;
                let updated = tl.LastUpdatedTime()?.UniversalTime;
                let changed = if updated != last_updated { " *UPDATED*" } else { "" };
                last_updated = updated;
                // How long ago the app last reported its position.
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64() + 11_644_473_600.0;
                let age = now - updated as f64 / 1e7;
                println!(
                    "{t:6.2}s app={app} status={status} pos={position:8.3} end={end:8.3} age={age:8.3} est={:8.3}{changed} | {title} / {artist} / {album}",
                    position + age
                );
            }
            Err(_) => println!("{t:6.2}s no media session"),
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Ok(())
}

#[cfg(not(windows))]
fn main() {
    println!("Windows only.");
}
