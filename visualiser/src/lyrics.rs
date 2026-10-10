//! Time-synced lyrics: reading the LRC format, and fetching it for a track
//! from LRCLIB (https://lrclib.net), a free, crowd-sourced lyrics database.
//! Nothing here runs in the render loop except `Lookup::want` and
//! `Lookup::state`, which only compare and copy; files and the network are
//! left to a background thread.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// One line of a song and when it starts, in seconds from the start of the track.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub start: f64,
    pub text: String,
    /// When each word starts, if the file gives word timings. The pieces
    /// join up to make `text`.
    pub words: Vec<(f64, String)>,
}

/// A time such as `01:23.45`, `1:23` or `01:23:45`, in seconds.
fn timestamp(tag: &str) -> Option<f64> {
    let mut parts = tag.trim().split(':');
    let minutes: u32 = parts.next()?.trim().parse().ok()?;
    let seconds: f64 = parts.next()?.trim().replace(',', ".").parse().ok()?;
    let hundredths: f64 = match parts.next() {
        Some(h) => h.trim().parse::<u32>().ok()? as f64 / 100.0,
        None => 0.0,
    };
    (parts.next().is_none() && seconds < 60.0).then_some(minutes as f64 * 60.0 + seconds + hundredths)
}

/// Split `<mm:ss.xx>` word timings out of the text of a line.
fn words(text: &str, line_start: f64) -> (String, Vec<(f64, String)>) {
    let mut pieces: Vec<(f64, String)> = Vec::new();
    let mut at = line_start;
    let mut timed = false;
    let mut rest = text;
    while let Some(open) = rest.find('<') {
        let tag = rest[open + 1..].find('>').and_then(|close| Some((timestamp(&rest[open + 1..open + 1 + close])?, close)));
        let Some((time, close)) = tag else {
            // An ordinary "<" in the words of the song.
            let (before, after) = rest.split_at(open + 1);
            pieces.push((at, before.to_string()));
            rest = after;
            continue;
        };
        if open > 0 {
            pieces.push((at, rest[..open].to_string()));
        }
        (at, timed) = (time, true);
        rest = &rest[open + 2 + close..];
    }
    if !rest.is_empty() {
        pieces.push((at, rest.to_string()));
    }
    // Pieces that share a start (text split round a stray "<") are one word.
    let mut merged: Vec<(f64, String)> = Vec::new();
    for (start, piece) in pieces {
        match merged.last_mut() {
            Some((last, text)) if *last == start => text.push_str(&piece),
            _ => merged.push((start, piece)),
        }
    }
    if let Some((_, first)) = merged.first_mut() {
        *first = first.trim_start().to_string();
    }
    if let Some((_, last)) = merged.last_mut() {
        *last = last.trim_end().to_string();
    }
    let whole: String = merged.iter().map(|(_, piece)| piece.as_str()).collect();
    (whole, if timed { merged } else { Vec::new() })
}

/// Read LRC text: `[mm:ss.xx]words` per line, optionally with a time in
/// angle brackets before each word. Lines without a time are left out.
pub fn parse_lrc(text: &str) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut offset = 0.0;
    for raw in text.lines() {
        let mut rest = raw.trim().trim_start_matches('\u{feff}');
        let mut starts = Vec::new();
        while let Some((tag, after)) = rest.strip_prefix('[').and_then(|r| r.split_once(']')) {
            match timestamp(tag) {
                Some(time) => starts.push(time),
                // "[Chorus]" after a time is part of the words.
                None if !starts.is_empty() => break,
                None => {
                    // A positive offset means the lyrics should come sooner.
                    if let Some(ms) = tag.trim().strip_prefix("offset:") {
                        offset = ms.trim().parse::<f64>().unwrap_or(0.0) / 1000.0;
                    }
                }
            }
            rest = after;
        }
        // A line sung more than once can carry several times.
        for start in starts {
            let (text, words) = words(rest.trim(), start);
            lines.push(Line { start, text, words });
        }
    }
    for line in &mut lines {
        line.start = (line.start - offset).max(0.0);
        line.words.iter_mut().for_each(|(start, _)| *start = (*start - offset).max(0.0));
    }
    lines.sort_by(|a, b| a.start.total_cmp(&b.start));
    lines
}

/// The lyrics file kept beside an audio file: the same name ending `.lrc`.
pub fn sidecar(audio: &Path) -> Option<Vec<Line>> {
    let text = std::fs::read_to_string(audio.with_extension("lrc")).ok()?;
    Some(parse_lrc(&text)).filter(|lines| !lines.is_empty())
}

/// The track lyrics are wanted for.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Query {
    pub title: String,
    pub artist: String,
    pub album: String,
    /// Length in seconds, 0 if not known.
    pub duration: f64,
}

#[derive(Clone)]
pub enum State {
    Looking,
    Synced(Arc<[Line]>),
    /// Why there are no lyrics to show.
    Missing(String),
}

/// What was found for a track, as kept on disk so each track is asked about once.
#[derive(Serialize, Deserialize)]
struct Entry {
    title: String,
    artist: String,
    album: String,
    duration: f64,
    /// The synced lyrics as LRC text, if there are any.
    synced: Option<String>,
    /// Why not, if not.
    note: String,
    /// When this was fetched, in seconds since 1970.
    fetched: u64,
}

/// A track with no lyrics is asked about again after this long, in case
/// someone has added them since.
const RETRY_MISS_S: u64 = 7 * 24 * 3600;
/// A new track is only looked up once its details have stopped changing;
/// players often announce the title a moment before the artist.
const SETTLE: Duration = Duration::from_millis(400);

/// Finds lyrics for one track at a time, on a background thread.
pub struct Lookup {
    cache: PathBuf,
    shared: Arc<Mutex<(Query, State)>>,
    /// A track seen but not yet asked about, and when it was first seen.
    pending: Option<(Query, Instant)>,
}

impl Lookup {
    /// `cache` is the folder results are kept in.
    pub fn new(cache: PathBuf) -> Self {
        Self { cache, shared: Arc::new(Mutex::new((Query::default(), State::Looking))), pending: None }
    }

    /// Say which track is playing. Call every frame; it starts a search when the track changes.
    pub fn want(&mut self, query: &Query) {
        if self.shared.lock().unwrap().0 == *query {
            self.pending = None;
            return;
        }
        match &self.pending {
            Some((pending, since)) if pending == query => {
                if since.elapsed() < SETTLE {
                    return;
                }
            }
            _ => {
                self.pending = Some((query.clone(), Instant::now()));
                return;
            }
        }
        self.pending = None;
        *self.shared.lock().unwrap() = (query.clone(), State::Looking);
        let (shared, cache, query) = (self.shared.clone(), self.cache.clone(), query.clone());
        std::thread::spawn(move || {
            let found = find(&query, &cache);
            let mut shared = shared.lock().unwrap();
            // The track may have changed while this one was being fetched.
            if shared.0 == query {
                shared.1 = found;
            }
        });
    }

    /// What is known about `query`. A track that has only just come on
    /// counts as being looked for.
    pub fn state(&self, query: &Query) -> State {
        let shared = self.shared.lock().unwrap();
        if shared.0 == *query { shared.1.clone() } else { State::Looking }
    }
}

fn now_s() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// A file name for a track's cache entry (FNV-1a of its details).
fn cache_name(query: &Query) -> String {
    let key = format!("{}\n{}\n{}\n{:.0}", query.artist, query.title, query.album, query.duration).to_lowercase();
    let hash = key.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3));
    format!("{hash:016x}.json")
}

fn from_entry(entry: &Entry) -> State {
    match entry.synced.as_deref().map(parse_lrc) {
        Some(lines) if !lines.is_empty() => State::Synced(lines.into()),
        _ => State::Missing(entry.note.clone()),
    }
}

fn find(query: &Query, cache: &Path) -> State {
    let path = cache.join(cache_name(query));
    let saved: Option<Entry> = std::fs::read_to_string(&path).ok().and_then(|text| serde_json::from_str(&text).ok());
    if let Some(entry) = saved.filter(|e| e.synced.is_some() || now_s().saturating_sub(e.fetched) < RETRY_MISS_S) {
        return from_entry(&entry);
    }
    match fetch(query) {
        Ok((synced, note)) => {
            let entry = Entry {
                title: query.title.clone(),
                artist: query.artist.clone(),
                album: query.album.clone(),
                duration: query.duration,
                synced,
                note,
                fetched: now_s(),
            };
            if std::fs::create_dir_all(cache).is_ok() {
                if let Ok(text) = serde_json::to_string_pretty(&entry) {
                    let _ = std::fs::write(&path, text);
                }
            }
            from_entry(&entry)
        }
        // Not kept, so it is tried again the next time the track comes round.
        Err(e) => State::Missing(format!("couldn't reach lrclib.net ({e})")),
    }
}

/// The title without "(feat. …)", "[Live]", "- Remastered 2011" and the like.
fn plain_title(title: &str) -> &str {
    let cut = [" (", " [", " - "].iter().filter_map(|mark| title.find(mark)).min().unwrap_or(title.len());
    title[..cut].trim()
}

/// Synced lyrics in one of LRCLIB's records, or why it has none.
#[cfg(any(windows, target_os = "android"))]
fn synced_in(record: &serde_json::Value) -> Result<String, &'static str> {
    match record.get("syncedLyrics").and_then(|v| v.as_str()) {
        Some(text) if !text.trim().is_empty() => Ok(text.to_string()),
        _ if record.get("instrumental").and_then(|v| v.as_bool()) == Some(true) => Err("LRCLIB lists this track as instrumental"),
        _ => Err("LRCLIB has the words for this track but no timings"),
    }
}

#[cfg(any(windows, target_os = "android"))]
const SITE: &str = "https://lrclib.net/api";
/// LRCLIB asks apps to say who they are.
#[cfg(any(windows, target_os = "android"))]
const USER_AGENT: &str = concat!("AudioVis/", env!("CARGO_PKG_VERSION"), " (https://github.com/Bluetowels/AudioVis)");

/// Something that asks LRCLIB one question: the page under `SITE` and the
/// fields to send, giving the status code and the answer.
#[cfg(any(windows, target_os = "android"))]
type Get = Box<dyn Fn(&str, &[(&str, &str)]) -> Result<(u16, serde_json::Value), String>>;

#[cfg(windows)]
fn client() -> Get {
    use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .user_agent(USER_AGENT)
        .http_status_as_error(false)
        // Windows' own TLS and certificates.
        .tls_config(TlsConfig::builder().provider(TlsProvider::NativeTls).root_certs(RootCerts::PlatformVerifier).build())
        .build()
        .into();
    Box::new(move |path: &str, fields: &[(&str, &str)]| -> Result<(u16, serde_json::Value), String> {
        let mut request = agent.get(format!("{SITE}/{path}"));
        for (name, value) in fields.iter().filter(|(_, value)| !value.is_empty()) {
            request = request.query(name, value);
        }
        let mut response = request.call().map_err(|e| e.to_string())?;
        let status = response.status().as_u16();
        let body = response.body_mut().read_to_string().map_err(|e| e.to_string())?;
        Ok((status, serde_json::from_str(&body).unwrap_or(serde_json::Value::Null)))
    })
}

/// Text made safe to put in a web address.
#[cfg(any(target_os = "android", test))]
fn url_encoded(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Android's own networking and certificates, by way of the Java side of the app.
#[cfg(target_os = "android")]
fn client() -> Get {
    Box::new(|path: &str, fields: &[(&str, &str)]| -> Result<(u16, serde_json::Value), String> {
        let mut url = format!("{SITE}/{path}");
        for (i, (name, value)) in fields.iter().filter(|(_, value)| !value.is_empty()).enumerate() {
            url.push(if i == 0 { '?' } else { '&' });
            url.push_str(&format!("{name}={}", url_encoded(value)));
        }
        let (status, body) = crate::android::http_get(&url, USER_AGENT)?;
        Ok((status, serde_json::from_str(&body).unwrap_or(serde_json::Value::Null)))
    })
}

/// Ask LRCLIB. Returns the synced lyrics as LRC text, or the reason there are none.
#[cfg(any(windows, target_os = "android"))]
fn fetch(query: &Query) -> Result<(Option<String>, String), String> {
    let get = client();

    let duration = if query.duration > 0.0 { format!("{:.0}", query.duration) } else { String::new() };
    let mut note = "LRCLIB has no lyrics for this track";

    // An exact match on title, artist, album and length first.
    let exact = [
        ("track_name", query.title.as_str()),
        ("artist_name", query.artist.as_str()),
        ("album_name", query.album.as_str()),
        ("duration", duration.as_str()),
    ];
    match get("get", &exact)? {
        (200, record) => match synced_in(&record) {
            Ok(text) => return Ok((Some(text), String::new())),
            Err(why) => note = why,
        },
        (404, _) => {}
        (status, _) => return Err(format!("it answered with error {status}")),
    }

    // Then a search, taking the synced version closest in length. A version
    // of a different length (a live take, an edit) would not stay in time.
    let mut titles = vec![query.title.as_str()];
    if plain_title(&query.title) != query.title {
        titles.push(plain_title(&query.title));
    }
    for title in titles {
        let (status, results) = get("search", &[("track_name", title), ("artist_name", query.artist.as_str())])?;
        if status != 200 {
            return Err(format!("it answered with error {status}"));
        }
        let best = results
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|record| {
                let apart = match record.get("duration").and_then(|v| v.as_f64()) {
                    Some(length) if query.duration > 0.0 => (length - query.duration).abs(),
                    _ => 0.0,
                };
                Some((apart, synced_in(record).ok()?)).filter(|(apart, _)| *apart <= 3.0)
            })
            .min_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((_, text)) = best {
            return Ok((Some(text), String::new()));
        }
    }
    Ok((None, note.to_string()))
}

/// Only the Windows and Android builds know what is playing, so only they fetch lyrics.
#[cfg(not(any(windows, target_os = "android")))]
fn fetch(_query: &Query) -> Result<(Option<String>, String), String> {
    Err("not available on this system".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_lines() {
        let lines = parse_lrc("[ar:Someone]\n[00:12.50]First\n[01:02.03] Second line \n\nno time\n[00:20]\n");
        assert_eq!(lines.len(), 3);
        assert_eq!((lines[0].start, lines[0].text.as_str()), (12.5, "First"));
        assert_eq!((lines[1].start, lines[1].text.as_str()), (20.0, ""));
        assert_eq!((lines[2].start, lines[2].text.as_str()), (62.03, "Second line"));
        assert!(lines[0].words.is_empty());
    }

    #[test]
    fn repeats_offset_and_sections() {
        let lines = parse_lrc("[offset:+500]\n[00:10.00][00:30.00][Chorus] la <3\n");
        assert_eq!(lines.len(), 2);
        assert_eq!((lines[0].start, lines[1].start), (9.5, 29.5));
        assert_eq!(lines[0].text, "[Chorus] la <3");
    }

    #[test]
    fn word_timings() {
        let lines = parse_lrc("[00:10.00]<00:10.00>Hello <00:10.50>big <00:11.25>world<00:12.00>");
        assert_eq!(lines[0].text, "Hello big world");
        assert_eq!(lines[0].words, vec![(10.0, "Hello ".to_string()), (10.5, "big ".to_string()), (11.25, "world".to_string())]);
    }

    /// Asks the real site, so it is only run by hand:
    /// `cargo test --release --lib live -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live() {
        let query = Query { title: "Woman In Chains".into(), artist: "Tears For Fears".into(), album: "The Seeds Of Love".into(), duration: 390.0 };
        let started = Instant::now();
        let found = fetch(&query);
        println!("{:.1} s: {:?}", started.elapsed().as_secs_f32(), found.as_ref().map(|(text, note)| (text.as_ref().map(|t| parse_lrc(t).len()), note)));
        assert!(found.is_ok());
    }

    #[test]
    fn web_addresses() {
        assert_eq!(url_encoded("Beyoncé & Jay-Z: 4.44~"), "Beyonc%C3%A9%20%26%20Jay-Z%3A%204.44~");
    }

    #[test]
    fn titles() {
        assert_eq!(plain_title("Song (feat. X) - Remastered"), "Song");
        assert_eq!(plain_title("Song"), "Song");
    }
}
