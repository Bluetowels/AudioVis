package com.bluetowel.audiovis;

import android.app.Activity;
import android.app.NotificationManager;
import android.content.ActivityNotFoundException;
import android.content.ComponentName;
import android.content.Context;
import android.content.Intent;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.media.MediaMetadata;
import android.media.session.MediaController;
import android.media.session.MediaSessionManager;
import android.media.session.PlaybackState;
import android.os.Build;
import android.os.Handler;
import android.os.Looper;
import android.os.SystemClock;
import android.provider.Settings;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;

/**
 * Watches what other apps are playing: the title, artist, album, cover and
 * position that players give Android for the media controls in the
 * notification shade. Lyrics, the track card and colours from the cover all
 * come from this. Android only allows it once the user has switched on
 * "notification access" for the app (see MediaListener). Everything here
 * runs on the main thread, and what it learns goes to the Rust side through
 * MainActivity.nativeNowPlaying and nativeCover.
 */
final class MediaWatch implements MediaSessionManager.OnActiveSessionsChangedListener {
    /** Side of the square a cover is shrunk to, as in nowplaying.rs. */
    static final int COVER = 256;

    private final Context context;
    private final ComponentName listener;
    private final Handler main = new Handler(Looper.getMainLooper());

    /** Whether the Rust side has asked to be told. */
    private boolean wanted;
    private MediaSessionManager manager;
    /** Every player Android knows of, most important first, and what listens to each. */
    private final List<MediaController> players = new ArrayList<>();
    private final List<MediaController.Callback> callbacks = new ArrayList<>();
    /** The track and cover last passed on, so a cover is only sent when it is new. */
    private String lastTrack = "";
    private int lastCover;

    MediaWatch(Context context) {
        this.context = context.getApplicationContext();
        listener = new ComponentName(this.context, MediaListener.class);
    }

    void start() {
        wanted = true;
        lastTrack = "";
        connect();
    }

    void stop() {
        wanted = false;
        disconnect();
    }

    private boolean allowed() {
        NotificationManager notifications = (NotificationManager) context.getSystemService(Context.NOTIFICATION_SERVICE);
        return notifications != null && notifications.isNotificationListenerAccessGranted(listener);
    }

    /** Start listening if that is wanted and allowed. Called again each time the app comes back into view, in case the user has just allowed it. */
    void connect() {
        if (!wanted) {
            return;
        }
        if (!allowed()) {
            disconnect();
            MainActivity.nativeMediaAccess(false);
            return;
        }
        MainActivity.nativeMediaAccess(true);
        if (manager != null) {
            look();
            return;
        }
        try {
            MediaSessionManager sessions = (MediaSessionManager) context.getSystemService(Context.MEDIA_SESSION_SERVICE);
            sessions.addOnActiveSessionsChangedListener(this, listener);
            manager = sessions;
            onActiveSessionsChanged(sessions.getActiveSessions(listener));
        } catch (SecurityException e) {
            disconnect();
            MainActivity.nativeMediaAccess(false);
        }
    }

    private void disconnect() {
        if (manager != null) {
            manager.removeOnActiveSessionsChangedListener(this);
            manager = null;
        }
        forget();
    }

    private void forget() {
        for (int i = 0; i < players.size(); i++) {
            players.get(i).unregisterCallback(callbacks.get(i));
        }
        players.clear();
        callbacks.clear();
    }

    /** Open the page of Android's settings where the user allows this. */
    void ask(Activity from) {
        if (Build.VERSION.SDK_INT >= 30) {
            // Straight to this app's own switch, where Android has such a page.
            try {
                from.startActivity(new Intent(Settings.ACTION_NOTIFICATION_LISTENER_DETAIL_SETTINGS)
                        .putExtra(Settings.EXTRA_NOTIFICATION_LISTENER_COMPONENT_NAME, listener.flattenToString()));
                return;
            } catch (ActivityNotFoundException | SecurityException e) {
                // Fall through to the list of every app.
            }
        }
        try {
            from.startActivity(new Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS));
        } catch (ActivityNotFoundException | SecurityException e) {
            MainActivity.nativeStatus("This device has no setting for notification access");
        }
    }

    @Override
    public void onActiveSessionsChanged(List<MediaController> active) {
        forget();
        if (active != null) {
            for (MediaController player : active) {
                MediaController.Callback callback = new MediaController.Callback() {
                    @Override
                    public void onPlaybackStateChanged(PlaybackState state) {
                        look();
                    }

                    @Override
                    public void onMetadataChanged(MediaMetadata metadata) {
                        look();
                    }

                    @Override
                    public void onSessionDestroyed() {
                        look();
                    }
                };
                player.registerCallback(callback, main);
                players.add(player);
                callbacks.add(callback);
            }
        }
        look();
    }

    private static String text(MediaMetadata data, String... keys) {
        for (String key : keys) {
            CharSequence value = data.getText(key);
            if (value != null && value.length() > 0) {
                return value.toString();
            }
        }
        return "";
    }

    /** Tell the Rust side what is playing now: the first player that is playing, or failing that the first one. */
    void look() {
        // The app's own audio file speaks for itself (MainActivity.playFile).
        if (manager == null || MainActivity.fileReports) {
            return;
        }
        MediaController chosen = null;
        PlaybackState state = null;
        MediaMetadata data = null;
        try {
            for (MediaController player : players) {
                PlaybackState its = player.getPlaybackState();
                if (its != null && its.getState() == PlaybackState.STATE_PLAYING) {
                    chosen = player;
                    break;
                }
            }
            if (chosen == null && !players.isEmpty()) {
                chosen = players.get(0);
            }
            if (chosen != null) {
                state = chosen.getPlaybackState();
                data = chosen.getMetadata();
            }
        } catch (RuntimeException e) {
            // A player that has just gone away; the next change says what is left.
            data = null;
        }
        if (chosen == null || data == null) {
            lastTrack = "";
            MainActivity.nativeNowPlaying(null, null, null, null, 0, -1, false, 0);
            return;
        }

        String app = chosen.getPackageName();
        String title = text(data, MediaMetadata.METADATA_KEY_TITLE, MediaMetadata.METADATA_KEY_DISPLAY_TITLE);
        String artist = text(data, MediaMetadata.METADATA_KEY_ARTIST, MediaMetadata.METADATA_KEY_ALBUM_ARTIST,
                MediaMetadata.METADATA_KEY_DISPLAY_SUBTITLE);
        String album = text(data, MediaMetadata.METADATA_KEY_ALBUM);
        double length = Math.max(0, data.getLong(MediaMetadata.METADATA_KEY_DURATION)) / 1000.0;

        boolean playing = state != null && state.getState() == PlaybackState.STATE_PLAYING;
        double position = -1;
        long reported = 0;
        if (state != null && state.getPosition() >= 0) {
            position = state.getPosition() / 1000.0;
            reported = state.getLastPositionUpdateTime();
            if (reported <= 0) {
                // A player that does not say when: any new position counts as news.
                reported = -1 - state.getPosition();
            } else if (playing) {
                // Bring it up to this moment.
                position += (SystemClock.elapsedRealtime() - reported) / 1000.0 * state.getPlaybackSpeed();
            }
        }
        MainActivity.nativeNowPlaying(app, title, artist, album, length, Math.max(position, -1), playing, reported);

        // Players often hand over the cover a moment after the title.
        Bitmap art = data.getBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART);
        if (art == null) {
            art = data.getBitmap(MediaMetadata.METADATA_KEY_ART);
        }
        if (art == null) {
            art = data.getBitmap(MediaMetadata.METADATA_KEY_DISPLAY_ICON);
        }
        String track = app + "\n" + title + "\n" + artist;
        int[] cover = shrink(art);
        int coverHash = cover == null ? 0 : Arrays.hashCode(cover);
        if (cover != null && (!track.equals(lastTrack) || coverHash != lastCover)) {
            MainActivity.nativeCover(cover);
        }
        if (cover != null || !track.equals(lastTrack)) {
            lastCover = coverHash;
        }
        lastTrack = track;
    }

    /** A picture as COVER by COVER pixels, each a number holding alpha, red, green and blue; null if there is none. */
    static int[] shrink(Bitmap picture) {
        if (picture == null) {
            return null;
        }
        try {
            if (picture.getConfig() != Bitmap.Config.ARGB_8888) {
                // Some players hand over a picture kept in the graphics chip, whose pixels cannot be read.
                picture = picture.copy(Bitmap.Config.ARGB_8888, false);
            }
            Bitmap small = Bitmap.createScaledBitmap(picture, COVER, COVER, true);
            int[] pixels = new int[COVER * COVER];
            small.getPixels(pixels, 0, COVER, 0, 0, COVER, COVER);
            return pixels;
        } catch (RuntimeException | OutOfMemoryError e) {
            return null;
        }
    }

    /** The same from a picture file's bytes, such as the cover kept inside an audio file. */
    static int[] shrink(byte[] file) {
        if (file == null) {
            return null;
        }
        try {
            BitmapFactory.Options size = new BitmapFactory.Options();
            size.inJustDecodeBounds = true;
            BitmapFactory.decodeByteArray(file, 0, file.length, size);
            BitmapFactory.Options options = new BitmapFactory.Options();
            // Read a large picture at a half, a quarter and so on, down to no smaller than is wanted.
            options.inSampleSize = Math.max(1, Integer.highestOneBit(Math.max(1, Math.min(size.outWidth, size.outHeight) / COVER)));
            return shrink(BitmapFactory.decodeByteArray(file, 0, file.length, options));
        } catch (RuntimeException | OutOfMemoryError e) {
            return null;
        }
    }
}
