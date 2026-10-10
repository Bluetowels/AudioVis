//! The join between the app and Android. Android only lets Java code ask
//! for permissions, capture what other apps are playing or open a file the
//! user picks, so that part of the app is Java (android/app/src/main/java).
//! This file asks it to start and stop a source, and takes in the sound it
//! sends back. It also passes on the two things lyrics need from Android:
//! leave to see what other apps are playing, and a way to fetch a web page.

use crate::audio::Ring;
use jni::JNIEnv;
use jni::objects::{JClass, JObject, JShortArray, JString, JValue};
use jni::sys::jint;
use std::ffi::{CString, c_char, c_int};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

/// The Java virtual machine and the app's screen (its activity), as Android handed them over.
static JAVA: Mutex<Option<(usize, usize)>> = Mutex::new(None);
static DATA_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);
/// Where arriving sound goes, and a count of how many sources have been started.
static FEED: Mutex<(u64, Option<Weak<Mutex<Ring>>>)> = Mutex::new((0, None));
/// What the Java side last said about the source: what is playing, or what went wrong.
static STATUS: Mutex<String> = Mutex::new(String::new());
/// Whether the user has let the app see what other apps are playing.
static MEDIA_ACCESS: AtomicBool = AtomicBool::new(false);
/// The text of a lyrics file the user has picked for the audio file, until
/// the app takes it. Empty text means there is none any more.
static LYRICS_FILE: Mutex<Option<String>> = Mutex::new(None);

/// What the Java side calls the app's own audio file where it reports what
/// is playing. Shared with `MainActivity.java`.
pub const FILE_APP: &str = "AudioVis file";

/// Called once each time Android opens the app's screen.
pub fn attach(app: &android_activity::AndroidApp) {
    *JAVA.lock().unwrap() = Some((app.vm_as_ptr() as usize, app.activity_as_ptr() as usize));
    *DATA_DIR.lock().unwrap() = app.internal_data_path();
}

/// The screen is closing: nothing more may be asked of it.
pub fn detach() {
    *JAVA.lock().unwrap() = None;
}

pub fn data_dir() -> PathBuf {
    DATA_DIR.lock().unwrap().clone().unwrap_or_else(|| PathBuf::from("."))
}

#[link(name = "log")]
unsafe extern "C" {
    fn __android_log_write(priority: c_int, tag: *const c_char, text: *const c_char) -> c_int;
}

/// Write an error to the system log (`adb logcat -s AudioVis`).
pub fn log(text: &str) {
    const ERROR: c_int = 6;
    let text = CString::new(text.replace('\0', " ")).unwrap_or_default();
    // SAFETY: both strings are NUL-terminated and outlive the call.
    unsafe { __android_log_write(ERROR, c"AudioVis".as_ptr(), text.as_ptr()) };
}

/// Call a method of the app's Java activity. Each one hands the work to
/// Android's main thread and returns at once.
fn call(method: &str, signature: &str, arguments: &[JValue]) {
    let Some((vm, activity)) = *JAVA.lock().unwrap() else { return };
    // SAFETY: both pointers came from Android in `attach`, and `detach`
    // forgets them before the screen they belong to goes away.
    let Ok(vm) = (unsafe { jni::JavaVM::from_raw(vm as *mut jni::sys::JavaVM) }) else { return };
    let Ok(mut env) = vm.attach_current_thread() else { return };
    let activity = unsafe { JObject::from_raw(activity as jni::sys::jobject) };
    if env.call_method(&activity, method, signature, arguments).is_err() {
        let _ = env.exception_clear();
        log(&format!("could not call {method}"));
    }
}

/// Where the Java side gets the sound from. The numbers are shared with
/// `MainActivity.java`.
#[derive(Clone, Copy, PartialEq)]
pub enum Feed {
    /// Nothing from Java: silence, or the test signal made in Rust.
    None = 0,
    /// What other apps are playing.
    Playback = 1,
    Microphone = 2,
    /// An audio file, played out loud by the app itself.
    File = 3,
}

/// A running source. Dropping it stops the source, unless a newer one has
/// already taken its place.
pub struct Stream {
    started: u64,
}

impl Stream {
    pub fn start(feed: Feed, ring: &Arc<Mutex<Ring>>) -> Self {
        let started = {
            let mut current = FEED.lock().unwrap();
            current.0 += 1;
            current.1 = (feed != Feed::None).then(|| Arc::downgrade(ring));
            current.0
        };
        STATUS.lock().unwrap().clear();
        call("startSource", "(I)V", &[JValue::Int(feed as jint)]);
        Self { started }
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        let mut current = FEED.lock().unwrap();
        if current.0 == self.started {
            current.1 = None;
            drop(current);
            call("startSource", "(I)V", &[JValue::Int(Feed::None as jint)]);
        }
    }
}

/// Ask the user to pick an audio file; it starts playing once chosen.
pub fn pick_file() {
    call("pickFile", "()V", &[]);
}

/// What the Java side last reported about the running source.
pub fn status() -> String {
    STATUS.lock().unwrap().clone()
}

/// Start or stop being told what other apps are playing. The news arrives
/// in `nowplaying.rs`. Nothing comes until the user has allowed it.
pub fn watch_media(on: bool) {
    call("watchMedia", "(Z)V", &[JValue::Bool(on as u8)]);
}

/// Whether Android lets the app see what other apps are playing.
pub fn media_access() -> bool {
    MEDIA_ACCESS.load(Ordering::Relaxed)
}

/// Open the page of Android's settings where the user allows that.
pub fn ask_media_access() {
    call("askMediaAccess", "()V", &[]);
}

/// Ask the user to pick a lyrics (.lrc) file for the audio file that is playing.
pub fn pick_lyrics() {
    call("pickLyrics", "()V", &[]);
}

/// A lyrics file picked since this was last asked: its text, or empty text
/// if the audio file playing now has none.
pub fn take_lyrics_file() -> Option<String> {
    LYRICS_FILE.lock().unwrap().take()
}

/// Fetch a web page with Android's own networking and certificates. Gives
/// the status code and the body. Waits for the answer, so not for the
/// thread that draws.
pub fn http_get(url: &str, user_agent: &str) -> Result<(u16, String), String> {
    const NO_JAVA: &str = "the app could not reach Android's networking";
    let java = JAVA.lock().unwrap();
    let Some((vm, activity)) = *java else { return Err("the app is closing".to_string()) };
    // SAFETY: as in `call`. The activity is only used while `JAVA` is held,
    // so it cannot go away meanwhile; the class found through it lasts.
    let vm = unsafe { jni::JavaVM::from_raw(vm as *mut jni::sys::JavaVM) }.map_err(|_| NO_JAVA)?;
    let mut env = vm.attach_current_thread().map_err(|_| NO_JAVA)?;
    let activity = unsafe { JObject::from_raw(activity as jni::sys::jobject) };
    let class = env.get_object_class(&activity).map_err(|_| NO_JAVA)?;
    drop(java);

    let answer = (|| {
        let url = env.new_string(url)?;
        let user_agent = env.new_string(user_agent)?;
        let answer = env
            .call_static_method(
                &class,
                "httpGet",
                "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;",
                &[JValue::Object(&url), JValue::Object(&user_agent)],
            )?
            .l()?;
        let answer: String = env.get_string(&JString::from(answer))?.into();
        Ok::<String, jni::errors::Error>(answer)
    })();
    let Ok(answer) = answer else {
        let _ = env.exception_clear();
        return Err(NO_JAVA.to_string());
    };
    // The status code on the first line and the page after it, or an empty
    // first line and why it could not be fetched.
    let (status, body) = answer.split_once('\n').unwrap_or(("", answer.as_str()));
    match status.parse() {
        Ok(status) => Ok((status, body.to_string())),
        Err(_) => Err(body.to_string()),
    }
}

/// A block of sound from the Java side: 16-bit samples, channels interleaved.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_bluetowel_audiovis_MainActivity_nativeAudio<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
    pcm: JShortArray<'local>,
    samples: jint,
    channels: jint,
    sample_rate: jint,
) {
    let Some(ring) = FEED.lock().unwrap().1.as_ref().and_then(Weak::upgrade) else { return };
    let (samples, channels) = (samples.max(0) as usize, channels.max(1) as usize);
    let mut block = vec![0i16; samples];
    if env.get_short_array_region(&pcm, 0, &mut block).is_err() {
        let _ = env.exception_clear();
        return;
    }
    let mut ring = ring.lock().unwrap();
    // A file plays at its own rate; the analysis follows the ring's.
    ring.sample_rate = sample_rate.max(8_000) as u32;
    let mut frame = [0.0f32; crate::audio::MAX_METERED_CHANNELS];
    let used = channels.min(frame.len());
    for raw in block.chunks_exact(channels) {
        for (f, s) in frame.iter_mut().zip(raw) {
            *f = *s as f32 / 32768.0;
        }
        ring.push_frame(&frame[..used]);
    }
}

/// A line about the source for the panel: what is playing, or why nothing is.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_bluetowel_audiovis_MainActivity_nativeStatus<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    text: JString<'local>,
) {
    let text: String = env.get_string(&text).map(Into::into).unwrap_or_default();
    *STATUS.lock().unwrap() = text;
}

/// Whether the user has allowed the app to see what other apps are playing.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_bluetowel_audiovis_MainActivity_nativeMediaAccess<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    allowed: jni::sys::jboolean,
) {
    MEDIA_ACCESS.store(allowed != 0, Ordering::Relaxed);
}

/// The text of the lyrics file that goes with the audio file now playing,
/// or empty text if it has none.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_bluetowel_audiovis_MainActivity_nativeLyricsFile<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    text: JString<'local>,
) {
    let text: String = env.get_string(&text).map(Into::into).unwrap_or_default();
    *LYRICS_FILE.lock().unwrap() = Some(text);
}
