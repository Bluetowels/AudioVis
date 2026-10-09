package com.bluetowel.audiovis;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.Service;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.media.AudioAttributes;
import android.media.AudioFormat;
import android.media.AudioPlaybackCaptureConfiguration;
import android.media.AudioRecord;
import android.media.projection.MediaProjection;
import android.media.projection.MediaProjectionManager;
import android.os.Handler;
import android.os.IBinder;
import android.os.Looper;

/**
 * Records what other apps are playing. Android only allows that from a
 * service that shows a notification for as long as it runs, and only after
 * the user has said yes to its own "start recording?" question, whose
 * answer MainActivity passes in here.
 */
public class CaptureService extends Service {
    static final String RESULT = "result";
    static final String DATA = "data";
    static final String ID = "id";

    private static final String CHANNEL = "capture";

    private MediaProjection projection;
    private MediaProjection.Callback stopped;
    private volatile boolean running;

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }

    @Override
    @SuppressWarnings("deprecation")
    public int onStartCommand(Intent intent, int flags, int startId) {
        if (intent == null) {
            stopSelf();
            return START_NOT_STICKY;
        }
        getSystemService(NotificationManager.class).createNotificationChannel(
                new NotificationChannel(CHANNEL, "Listening to other apps", NotificationManager.IMPORTANCE_LOW));
        Notification notification = new Notification.Builder(this, CHANNEL)
                .setContentTitle("AudioVis is listening to other apps")
                .setSmallIcon(R.drawable.ic_notification)
                .setOngoing(true)
                .build();
        startForeground(1, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION);

        final int id = intent.getIntExtra(ID, -1);
        Intent answer = intent.getParcelableExtra(DATA);
        try {
            projection = getSystemService(MediaProjectionManager.class)
                    .getMediaProjection(intent.getIntExtra(RESULT, 0), answer);
        } catch (RuntimeException e) {
            projection = null;
        }
        if (projection == null) {
            MainActivity.nativeStatus("Android would not start the capture");
            stopSelf();
            return START_NOT_STICKY;
        }
        // Android can end the capture itself, from the notification shade for one.
        stopped = new MediaProjection.Callback() {
            @Override
            public void onStop() {
                MainActivity.nativeStatus("Android stopped the capture. Choose the source again to restart it.");
                stopSelf();
            }
        };
        projection.registerCallback(stopped, new Handler(Looper.getMainLooper()));

        running = true;
        final MediaProjection granted = projection;
        new Thread(() -> record(granted, id), "audiovis-capture").start();
        return START_NOT_STICKY;
    }

    private void record(MediaProjection granted, int id) {
        AudioRecord recorder = null;
        try {
            // Music, games and anything unlabelled. Apps can opt out of being
            // captured, and calls and alarms are never included.
            AudioPlaybackCaptureConfiguration what = new AudioPlaybackCaptureConfiguration.Builder(granted)
                    .addMatchingUsage(AudioAttributes.USAGE_MEDIA)
                    .addMatchingUsage(AudioAttributes.USAGE_GAME)
                    .addMatchingUsage(AudioAttributes.USAGE_UNKNOWN)
                    .build();
            int size = Math.max(
                    AudioRecord.getMinBufferSize(MainActivity.RATE, AudioFormat.CHANNEL_IN_STEREO, AudioFormat.ENCODING_PCM_16BIT),
                    MainActivity.RATE / 5 * 4);
            recorder = new AudioRecord.Builder()
                    .setAudioFormat(new AudioFormat.Builder()
                            .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                            .setSampleRate(MainActivity.RATE)
                            .setChannelMask(AudioFormat.CHANNEL_IN_STEREO)
                            .build())
                    .setBufferSizeInBytes(size)
                    .setAudioPlaybackCaptureConfig(what)
                    .build();
            recorder.startRecording();
            MainActivity.nativeStatus("2 ch, " + MainActivity.RATE + " Hz");
            short[] block = new short[2 * MainActivity.RATE / 100]; // ten milliseconds of stereo
            while (running && MainActivity.current.get() == id) {
                int read = recorder.read(block, 0, block.length);
                if (read < 0) {
                    MainActivity.nativeStatus("The capture stopped (error " + read + ")");
                    break;
                }
                if (read > 0) {
                    MainActivity.nativeAudio(block, read, 2, MainActivity.RATE);
                }
            }
        } catch (SecurityException | UnsupportedOperationException | IllegalArgumentException | IllegalStateException e) {
            MainActivity.nativeStatus("Other apps could not be captured: " + e.getMessage());
        } finally {
            if (recorder != null) {
                recorder.release();
            }
            stopSelf();
        }
    }

    @Override
    public void onDestroy() {
        running = false;
        if (projection != null) {
            // Stopping it ourselves is not news for the panel.
            projection.unregisterCallback(stopped);
            projection.stop();
        }
        super.onDestroy();
    }
}
