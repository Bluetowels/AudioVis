//! The join between the app and Android. Android only lets Java code ask
//! for permissions, capture what other apps are playing or open a file the
//! user picks, so that part of the app is Java (android/app/src/main/java).
//! This file asks it to start and stop a source, and takes in the sound it
//! sends back.

use crate::audio::Ring;
use jni::JNIEnv;
use jni::objects::{JClass, JObject, JShortArray, JString, JValue};
use jni::sys::jint;
use std::ffi::{CString, c_char, c_int};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};

/// The Java virtual machine and the app's screen (its activity), as Android handed them over.
static JAVA: Mutex<Option<(usize, usize)>> = Mutex::new(None);
static DATA_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);
/// Where arriving sound goes, and a count of how many sources have been started.
static FEED: Mutex<(u64, Option<Weak<Mutex<Ring>>>)> = Mutex::new((0, None));
/// What the Java side last said about the source: what is playing, or what went wrong.
static STATUS: Mutex<String> = Mutex::new(String::new());

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
