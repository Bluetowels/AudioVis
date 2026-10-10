package com.bluetowel.audiovis;

import android.Manifest;
import android.app.NativeActivity;
import android.content.ActivityNotFoundException;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.database.Cursor;
import android.media.AudioAttributes;
import android.media.AudioFormat;
import android.media.AudioRecord;
import android.media.AudioTrack;
import android.media.MediaCodec;
import android.media.MediaExtractor;
import android.media.MediaFormat;
import android.media.MediaMetadataRetriever;
import android.media.MediaRecorder;
import android.media.projection.MediaProjectionManager;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.SystemClock;
import android.provider.OpenableColumns;
import android.view.View;
import android.view.WindowInsets;
import android.view.WindowInsetsController;
import android.view.WindowManager;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.FloatBuffer;
import java.nio.ShortBuffer;
import java.nio.charset.StandardCharsets;
import java.util.ArrayDeque;
import java.util.concurrent.atomic.AtomicInteger;

/**
 * The app's one screen. The picture, the panel and the analysis are all in
 * the Rust library, which NativeActivity loads and runs. This class does the
 * things Android only lets Java do: ask for permissions, record the
 * microphone, ask to capture what other apps are playing, and open and play
 * an audio file. Sound goes to the Rust side through nativeAudio. For lyrics
 * it also says what is playing (see MediaWatch for other apps' music) and
 * fetches web pages for the Rust side.
 */
public class MainActivity extends NativeActivity {
    static {
        // NativeActivity loads the library too, but only this makes the
        // native methods below findable.
        System.loadLibrary("audiovis");
    }

    /** A block of 16-bit samples, channels interleaved, for the picture. */
    static native void nativeAudio(short[] pcm, int samples, int channels, int sampleRate);

    /** A line for the panel: what is playing, or why nothing is. */
    static native void nativeStatus(String text);

    /**
     * What is playing, for lyrics and the track card; a null app means
     * nothing is. Position is seconds into the track at this moment, below
     * zero if not known, and reported changes with each new position.
     */
    static native void nativeNowPlaying(String app, String title, String artist, String album,
            double duration, double position, boolean playing, long reported);

    /** The cover of the track just reported, as MediaWatch.shrink makes it. */
    static native void nativeCover(int[] pixels);

    /** Whether the user has let the app see what other apps are playing. */
    static native void nativeMediaAccess(boolean allowed);

    /** The text of the lyrics file chosen for the audio file now playing, or empty text for none. */
    static native void nativeLyricsFile(String text);

    /** What the app's own audio file is called where it reports what is playing, as in android.rs. */
    static final String FILE_APP = "AudioVis file";

    // Sources, as numbered in android.rs.
    static final int NONE = 0;
    static final int PLAYBACK = 1;
    static final int MICROPHONE = 2;
    static final int FILE = 3;

    private static final int ASK_MICROPHONE = 1;
    private static final int ASK_CAPTURE = 2;
    private static final int ASK_FILE = 3;
    private static final int ASK_LYRICS = 4;

    /** Recording rate. Android converts whatever the device really uses. */
    static final int RATE = 48000;

    /** Goes up each time a source starts or stops; a worker carries on only while its own number is the newest. */
    static final AtomicInteger current = new AtomicInteger();

    /** The source asked for, which may still be waiting on a permission. */
    private int wanted = NONE;

    /** File playback stops while the app is out of sight. */
    private volatile boolean visible = true;

    /** True while the audio file is what is playing, so other apps' music is not reported over it. */
    static volatile boolean fileReports;

    /** Set when the file's cover should be sent (again) with its next report. */
    private volatile boolean coverDue;

    private MediaWatch media;

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        media = new MediaWatch(this);
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
    }

    @Override
    public void onWindowFocusChanged(boolean focused) {
        super.onWindowFocusChanged(focused);
        if (focused) {
            hideBars();
        }
    }

    /** Fill the whole screen; a swipe from the edge brings the bars back for a moment. */
    @SuppressWarnings("deprecation")
    private void hideBars() {
        if (Build.VERSION.SDK_INT >= 30) {
            WindowInsetsController bars = getWindow().getInsetsController();
            if (bars != null) {
                bars.hide(WindowInsets.Type.systemBars());
                bars.setSystemBarsBehavior(WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
            }
        } else {
            getWindow().getDecorView().setSystemUiVisibility(View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY
                    | View.SYSTEM_UI_FLAG_FULLSCREEN
                    | View.SYSTEM_UI_FLAG_HIDE_NAVIGATION
                    | View.SYSTEM_UI_FLAG_LAYOUT_STABLE
                    | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
                    | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION);
        }
    }

    @Override
    protected void onStart() {
        super.onStart();
        visible = true;
        // The user may be coming back from allowing notification access.
        media.connect();
    }

    @Override
    protected void onStop() {
        visible = false;
        super.onStop();
    }

    @Override
    protected void onDestroy() {
        current.incrementAndGet();
        media.stop();
        stopService(new Intent(this, CaptureService.class));
        super.onDestroy();
    }

    /** Called from Rust, on its own thread: switch to a source, or to none. */
    public void startSource(final int kind) {
        runOnUiThread(() -> begin(kind));
    }

    /** Called from Rust: let the user pick an audio file, then play it. */
    public void pickFile() {
        runOnUiThread(this::chooseFile);
    }

    /** Called from Rust: start or stop saying what is playing. */
    public void watchMedia(final boolean on) {
        runOnUiThread(() -> {
            coverDue = true;
            if (on) {
                media.start();
            } else {
                media.stop();
            }
        });
    }

    /** Called from Rust: show the setting that lets the app see what other apps are playing. */
    public void askMediaAccess() {
        runOnUiThread(() -> media.ask(this));
    }

    /** Called from Rust: let the user pick a lyrics file to go with the audio file. */
    public void pickLyrics() {
        runOnUiThread(() -> {
            Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT)
                    .addCategory(Intent.CATEGORY_OPENABLE)
                    // Android has no file type for lyrics, so every file is offered.
                    .setType("*/*");
            try {
                startActivityForResult(intent, ASK_LYRICS);
            } catch (ActivityNotFoundException e) {
                nativeStatus("This device has no file picker");
            }
        });
    }

    /**
     * Called from Rust, on a thread of its own: fetch a web page. Returns
     * the status code, a line break and the page; or, if the site could not
     * be reached, an empty first line and why.
     */
    public static String httpGet(String address, String userAgent) {
        HttpURLConnection link = null;
        try {
            link = (HttpURLConnection) new URL(address).openConnection();
            link.setConnectTimeout(10_000);
            link.setReadTimeout(10_000);
            link.setRequestProperty("User-Agent", userAgent);
            link.setRequestProperty("Accept", "application/json");
            int status = link.getResponseCode();
            InputStream in = status >= 400 ? link.getErrorStream() : link.getInputStream();
            String page = in == null ? "" : new String(readAll(in, 8 << 20), StandardCharsets.UTF_8);
            return status + "\n" + page;
        } catch (IOException | RuntimeException e) {
            return "\n" + (e.getMessage() == null ? e.toString() : e.getMessage());
        } finally {
            if (link != null) {
                link.disconnect();
            }
        }
    }

    /** Everything in a stream, up to a limit. */
    private static byte[] readAll(InputStream in, int limit) throws IOException {
        try (InputStream stream = in) {
            ByteArrayOutputStream all = new ByteArrayOutputStream();
            byte[] block = new byte[16384];
            int read;
            while (all.size() < limit && (read = stream.read(block)) > 0) {
                all.write(block, 0, read);
            }
            return all.toByteArray();
        }
    }

    private void begin(int kind) {
        current.incrementAndGet();
        stopService(new Intent(this, CaptureService.class));
        wanted = kind;
        if (kind != FILE) {
            // Back to saying what other apps are playing, with no lyrics file.
            fileReports = false;
            nativeLyricsFile("");
            media.look();
        }
        if (kind == MICROPHONE || kind == PLAYBACK) {
            // Android counts capturing other apps as recording, so both need this.
            if (checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
                requestPermissions(new String[] {Manifest.permission.RECORD_AUDIO}, ASK_MICROPHONE);
            } else if (kind == MICROPHONE) {
                startMicrophone();
            } else {
                askToCapture();
            }
        } else if (kind == FILE) {
            Uri last = lastFile();
            if (last == null) {
                chooseFile();
            } else {
                startFile(last);
            }
        }
    }

    @Override
    public void onRequestPermissionsResult(int code, String[] permissions, int[] results) {
        super.onRequestPermissionsResult(code, permissions, results);
        if (code != ASK_MICROPHONE) {
            return;
        }
        if (results.length == 0 || results[0] != PackageManager.PERMISSION_GRANTED) {
            nativeStatus("Android refused the recording permission. It can be allowed in Settings > Apps > AudioVis > Permissions.");
        } else if (wanted == MICROPHONE) {
            startMicrophone();
        } else if (wanted == PLAYBACK) {
            askToCapture();
        }
    }

    /** Android shows its own "start recording or casting?" question; the answer comes to onActivityResult. */
    private void askToCapture() {
        MediaProjectionManager manager = (MediaProjectionManager) getSystemService(Context.MEDIA_PROJECTION_SERVICE);
        startActivityForResult(manager.createScreenCaptureIntent(), ASK_CAPTURE);
    }

    private void chooseFile() {
        Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT)
                .addCategory(Intent.CATEGORY_OPENABLE)
                .setType("audio/*")
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
        try {
            startActivityForResult(intent, ASK_FILE);
        } catch (ActivityNotFoundException e) {
            nativeStatus("This device has no file picker");
        }
    }

    @Override
    protected void onActivityResult(int code, int result, Intent data) {
        super.onActivityResult(code, result, data);
        if (code == ASK_CAPTURE) {
            if (result != RESULT_OK || data == null) {
                nativeStatus("Capture was not allowed");
            } else if (wanted == PLAYBACK) {
                // Android only lets a capture run inside a service that shows a notification.
                startForegroundService(new Intent(this, CaptureService.class)
                        .putExtra(CaptureService.RESULT, result)
                        .putExtra(CaptureService.DATA, data)
                        .putExtra(CaptureService.ID, current.get()));
            }
        } else if (code == ASK_FILE && result == RESULT_OK && data != null && data.getData() != null) {
            Uri file = data.getData();
            try {
                // Keeps the file readable the next time the app starts.
                getContentResolver().takePersistableUriPermission(file, Intent.FLAG_GRANT_READ_URI_PERMISSION);
            } catch (SecurityException e) {
                // Not every file provider offers that; the file still plays now.
            }
            getPreferences(MODE_PRIVATE).edit().putString("file", file.toString()).apply();
            if (wanted == FILE) {
                startFile(file);
            }
        } else if (code == ASK_LYRICS && result == RESULT_OK && data != null && data.getData() != null) {
            Uri audio = lastFile();
            if (audio == null) {
                return;
            }
            String lyrics;
            try (InputStream in = getContentResolver().openInputStream(data.getData())) {
                lyrics = in == null ? "" : decode(readAll(in, 512 << 10));
            } catch (IOException | RuntimeException e) {
                nativeStatus("Could not read the lyrics file: " + e.getMessage());
                return;
            }
            // Kept with the audio file's address, so it comes back whenever that file plays.
            getPreferences(MODE_PRIVATE).edit().putString(LYRICS_FOR + audio, lyrics).apply();
            if (wanted == FILE) {
                nativeLyricsFile(lyrics);
            }
        }
    }

    /** Where the lyrics chosen for an audio file are kept in the app's preferences. */
    private static final String LYRICS_FOR = "lyrics for ";

    /** Text from a file's bytes: UTF-8 unless it starts with the mark of UTF-16. */
    private static String decode(byte[] bytes) {
        if (bytes.length >= 2 && ((bytes[0] == (byte) 0xFF && bytes[1] == (byte) 0xFE)
                || (bytes[0] == (byte) 0xFE && bytes[1] == (byte) 0xFF))) {
            return new String(bytes, StandardCharsets.UTF_16);
        }
        return new String(bytes, StandardCharsets.UTF_8);
    }

    private Uri lastFile() {
        String saved = getPreferences(MODE_PRIVATE).getString("file", null);
        return saved == null ? null : Uri.parse(saved);
    }

    private String nameOf(Uri file) {
        try (Cursor cursor = getContentResolver().query(file, new String[] {OpenableColumns.DISPLAY_NAME}, null, null, null)) {
            if (cursor != null && cursor.moveToFirst()) {
                return cursor.getString(0);
            }
        } catch (RuntimeException e) {
            // Fall through to the plain name.
        }
        return "the file";
    }

    private void startMicrophone() {
        final int id = current.incrementAndGet();
        new Thread(() -> {
            AudioRecord recorder = null;
            try {
                int size = Math.max(
                        AudioRecord.getMinBufferSize(RATE, AudioFormat.CHANNEL_IN_STEREO, AudioFormat.ENCODING_PCM_16BIT),
                        RATE / 5 * 4);
                // Unprocessed: without the phone's noise reduction and levelling, where it has that.
                recorder = new AudioRecord(MediaRecorder.AudioSource.UNPROCESSED, RATE,
                        AudioFormat.CHANNEL_IN_STEREO, AudioFormat.ENCODING_PCM_16BIT, size);
                if (recorder.getState() != AudioRecord.STATE_INITIALIZED) {
                    recorder.release();
                    recorder = new AudioRecord(MediaRecorder.AudioSource.MIC, RATE,
                            AudioFormat.CHANNEL_IN_STEREO, AudioFormat.ENCODING_PCM_16BIT, size);
                }
                if (recorder.getState() != AudioRecord.STATE_INITIALIZED) {
                    nativeStatus("The microphone could not be opened");
                    return;
                }
                recorder.startRecording();
                nativeStatus("2 ch, " + RATE + " Hz");
                pump(recorder, id);
            } catch (SecurityException | IllegalArgumentException | IllegalStateException e) {
                nativeStatus("The microphone could not be opened: " + e.getMessage());
            } finally {
                if (recorder != null) {
                    recorder.release();
                }
            }
        }, "audiovis-microphone").start();
    }

    /** Hand a recorder's sound to the picture until a newer source takes over. */
    static void pump(AudioRecord recorder, int id) {
        short[] block = new short[2 * RATE / 100]; // ten milliseconds of stereo
        while (current.get() == id) {
            int read = recorder.read(block, 0, block.length);
            if (read < 0) {
                nativeStatus("The recording stopped (error " + read + ")");
                return;
            }
            if (read > 0) {
                nativeAudio(block, read, 2, RATE);
            }
        }
    }

    private void startFile(final Uri file) {
        final int id = current.incrementAndGet();
        final String name = nameOf(file);
        fileReports = true;
        coverDue = true;
        nativeLyricsFile(getPreferences(MODE_PRIVATE).getString(LYRICS_FOR + file, ""));
        new Thread(() -> {
            try {
                playFile(file, name, id);
            } catch (Exception e) {
                if (current.get() == id) {
                    nativeStatus("Could not play " + name + ": " + e.getMessage());
                    nativeNowPlaying(null, null, null, null, 0, -1, false, 0);
                }
            }
        }, "audiovis-file").start();
    }

    /** What an audio file says about itself. */
    private static final class Tags {
        String title = "";
        String artist = "";
        String album = "";
        /** Length in seconds, 0 if not known. */
        double length;
        /** The cover kept inside the file, as MediaWatch.shrink makes it, or null. */
        int[] cover;
    }

    private static String tag(MediaMetadataRetriever from, int key) {
        String value = from.extractMetadata(key);
        return value == null ? "" : value.trim();
    }

    /** The title, artist, album, length and cover written inside an audio file, with the file's name standing in for a missing title. */
    private Tags tagsOf(Uri file, String name) {
        Tags tags = new Tags();
        MediaMetadataRetriever reader = new MediaMetadataRetriever();
        try {
            reader.setDataSource(this, file);
            tags.title = tag(reader, MediaMetadataRetriever.METADATA_KEY_TITLE);
            tags.artist = tag(reader, MediaMetadataRetriever.METADATA_KEY_ARTIST);
            if (tags.artist.isEmpty()) {
                tags.artist = tag(reader, MediaMetadataRetriever.METADATA_KEY_ALBUMARTIST);
            }
            tags.album = tag(reader, MediaMetadataRetriever.METADATA_KEY_ALBUM);
            String length = tag(reader, MediaMetadataRetriever.METADATA_KEY_DURATION);
            tags.length = length.isEmpty() ? 0 : Long.parseLong(length) / 1000.0;
            tags.cover = MediaWatch.shrink(reader.getEmbeddedPicture());
        } catch (RuntimeException e) {
            // A file with no tags it can read still plays.
        } finally {
            try {
                reader.release();
            } catch (Exception e) {
                // Nothing to be done about it.
            }
        }
        if (tags.title.isEmpty()) {
            // "Artist - Title.mp3" is a common way to name a file.
            String plain = name.contains(".") ? name.substring(0, name.lastIndexOf('.')) : name;
            int dash = plain.indexOf(" - ");
            if (dash > 0 && tags.artist.isEmpty()) {
                tags.artist = plain.substring(0, dash).trim();
                tags.title = plain.substring(dash + 3).trim();
            } else {
                tags.title = plain;
            }
        }
        return tags;
    }

    /** Decode a file, play it through the speaker and show it, round and round, until a newer source takes over. */
    private void playFile(Uri file, String name, int id) throws Exception {
        MediaExtractor extractor = new MediaExtractor();
        MediaCodec codec = null;
        AudioTrack speaker = null;
        try {
            Tags tags = tagsOf(file, name);
            extractor.setDataSource(this, file, null);
            MediaFormat format = null;
            for (int i = 0; i < extractor.getTrackCount() && format == null; i++) {
                MediaFormat track = extractor.getTrackFormat(i);
                String mime = track.getString(MediaFormat.KEY_MIME);
                if (mime != null && mime.startsWith("audio/")) {
                    extractor.selectTrack(i);
                    format = track;
                }
            }
            if (format == null) {
                throw new IOException("it has no sound in it");
            }
            codec = MediaCodec.createDecoderByType(format.getString(MediaFormat.KEY_MIME));
            codec.configure(format, null, null, 0);
            codec.start();

            MediaCodec.BufferInfo info = new MediaCodec.BufferInfo();
            int channels = 2;
            int rate = RATE;
            boolean floats = false;
            boolean endOfFile = false;
            boolean paused = false;
            // The speaker plays what it is given a moment later, so the
            // picture is held back by the same amount to stay in step.
            ArrayDeque<short[]> held = new ArrayDeque<>();
            // How far into the file each held block is, in millionths of a second.
            ArrayDeque<Long> heldTimes = new ArrayDeque<>();
            int heldFrames = 0;
            int delayFrames = 0;
            // How far into the file the picture has got, and when that was last passed on.
            long shown = 0;
            long toldAt = 0;

            while (current.get() == id) {
                if (!visible) {
                    if (speaker != null && !paused) {
                        speaker.pause();
                        paused = true;
                        nativeNowPlaying(FILE_APP, tags.title, tags.artist, tags.album, tags.length, shown / 1e6, false,
                                SystemClock.elapsedRealtime());
                        toldAt = 0;
                    }
                    Thread.sleep(50);
                    continue;
                }
                if (paused) {
                    speaker.play();
                    paused = false;
                }
                if (!endOfFile) {
                    int in = codec.dequeueInputBuffer(10_000);
                    if (in >= 0) {
                        int size = extractor.readSampleData(codec.getInputBuffer(in), 0);
                        if (size < 0) {
                            codec.queueInputBuffer(in, 0, 0, 0, MediaCodec.BUFFER_FLAG_END_OF_STREAM);
                            endOfFile = true;
                        } else {
                            codec.queueInputBuffer(in, 0, size, extractor.getSampleTime(), 0);
                            extractor.advance();
                        }
                    }
                }
                int out = codec.dequeueOutputBuffer(info, 10_000);
                if (out == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED) {
                    MediaFormat decoded = codec.getOutputFormat();
                    channels = decoded.getInteger(MediaFormat.KEY_CHANNEL_COUNT);
                    rate = decoded.getInteger(MediaFormat.KEY_SAMPLE_RATE);
                    floats = decoded.containsKey(MediaFormat.KEY_PCM_ENCODING)
                            && decoded.getInteger(MediaFormat.KEY_PCM_ENCODING) == AudioFormat.ENCODING_PCM_FLOAT;
                    if (speaker != null) {
                        speaker.release();
                    }
                    speaker = openSpeaker(rate);
                    delayFrames = speaker.getBufferSizeInFrames();
                    speaker.play();
                    nativeStatus(name + ": " + channels + " ch, " + rate + " Hz");
                } else if (out >= 0) {
                    if (info.size > 0 && speaker != null) {
                        ByteBuffer decoded = codec.getOutputBuffer(out);
                        decoded.position(info.offset).limit(info.offset + info.size);
                        short[] stereo = toStereo(decoded.order(ByteOrder.nativeOrder()), channels, floats);
                        // Waits for room in the speaker's buffer, which is what keeps time.
                        speaker.write(stereo, 0, stereo.length);
                        held.add(stereo);
                        heldTimes.add(info.presentationTimeUs);
                        heldFrames += stereo.length / 2;
                        while (!held.isEmpty() && heldFrames - held.peek().length / 2 >= delayFrames) {
                            short[] due = held.poll();
                            long time = heldTimes.poll();
                            heldFrames -= due.length / 2;
                            nativeAudio(due, due.length, 2, rate);
                            // Say where the file has got to every second, and at once
                            // when it starts, resumes or goes back to the top.
                            long clock = SystemClock.elapsedRealtime();
                            if (current.get() == id && (clock - toldAt >= 1000 || time < shown)) {
                                nativeNowPlaying(FILE_APP, tags.title, tags.artist, tags.album, tags.length, time / 1e6, true, clock);
                                toldAt = clock;
                                if (coverDue) {
                                    coverDue = false;
                                    if (tags.cover != null) {
                                        nativeCover(tags.cover);
                                    }
                                }
                            }
                            shown = time;
                        }
                    }
                    codec.releaseOutputBuffer(out, false);
                    if ((info.flags & MediaCodec.BUFFER_FLAG_END_OF_STREAM) != 0) {
                        // Back to the top.
                        extractor.seekTo(0, MediaExtractor.SEEK_TO_CLOSEST_SYNC);
                        codec.flush();
                        endOfFile = false;
                    }
                }
            }
        } finally {
            if (speaker != null) {
                speaker.release();
            }
            if (codec != null) {
                codec.release();
            }
            extractor.release();
            // If the file is no longer the source, other apps' music is what is playing again.
            runOnUiThread(() -> {
                if (wanted != FILE) {
                    fileReports = false;
                    media.look();
                }
            });
        }
    }

    private static AudioTrack openSpeaker(int rate) {
        int size = AudioTrack.getMinBufferSize(rate, AudioFormat.CHANNEL_OUT_STEREO, AudioFormat.ENCODING_PCM_16BIT);
        return new AudioTrack.Builder()
                .setAudioAttributes(new AudioAttributes.Builder()
                        .setUsage(AudioAttributes.USAGE_MEDIA)
                        .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
                        .build())
                .setAudioFormat(new AudioFormat.Builder()
                        .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                        .setSampleRate(rate)
                        .setChannelMask(AudioFormat.CHANNEL_OUT_STEREO)
                        .build())
                .setBufferSizeInBytes(size)
                .setTransferMode(AudioTrack.MODE_STREAM)
                .build();
    }

    /** The front left and right channels of decoded sound, as 16-bit samples. */
    private static short[] toStereo(ByteBuffer decoded, int channels, boolean floats) {
        int right = channels > 1 ? 1 : 0;
        if (floats) {
            FloatBuffer samples = decoded.asFloatBuffer();
            int frames = samples.remaining() / channels;
            short[] stereo = new short[frames * 2];
            for (int f = 0; f < frames; f++) {
                stereo[2 * f] = toShort(samples.get(f * channels));
                stereo[2 * f + 1] = toShort(samples.get(f * channels + right));
            }
            return stereo;
        }
        ShortBuffer samples = decoded.asShortBuffer();
        int frames = samples.remaining() / channels;
        short[] stereo = new short[frames * 2];
        for (int f = 0; f < frames; f++) {
            stereo[2 * f] = samples.get(f * channels);
            stereo[2 * f + 1] = samples.get(f * channels + right);
        }
        return stereo;
    }

    private static short toShort(float sample) {
        return (short) Math.max(-32768, Math.min(32767, Math.round(sample * 32767f)));
    }
}
