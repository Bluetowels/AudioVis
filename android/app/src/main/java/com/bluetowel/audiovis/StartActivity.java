package com.bluetowel.audiovis;

import android.app.Activity;
import android.app.ActivityManager;
import android.app.ApplicationExitInfo;
import android.content.Context;
import android.content.Intent;
import android.content.SharedPreferences;
import android.graphics.Typeface;
import android.os.Build;
import android.os.Bundle;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;

import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileInputStream;
import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.util.List;

/**
 * Where the app opens. Normally it goes straight on to MainActivity. If the
 * app stopped by itself the last time it ran, it first shows what is known
 * about why, with a button to send that on, because a phone gives its user
 * no other way to see it. What is known comes from three places: a note the
 * app left as it failed (crash.txt), Android's own record of how the app's
 * last run ended, and the app's notes on how far that run got
 * (last-run.txt; see android.rs).
 */
public class StartActivity extends Activity {
    static final String CRASH_FILE = "crash.txt";
    static final String PROGRESS_FILE = "last-run.txt";

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        String report = null;
        try {
            report = report();
        } catch (RuntimeException e) {
            // A report that cannot be put together must not stop the app opening.
        }
        if (report == null) {
            carryOn();
        } else {
            show(report);
        }
    }

    private void carryOn() {
        startActivity(new Intent(this, MainActivity.class));
        finish();
    }

    /** What is known about why the last run stopped, or null if it did not stop by itself. */
    private String report() {
        File crashFile = new File(getFilesDir(), CRASH_FILE);
        String crash = read(crashFile, 16 << 10);
        crashFile.delete();
        String ending = Build.VERSION.SDK_INT >= 30 ? ending() : null;
        if (crash == null && ending == null) {
            return null;
        }

        StringBuilder all = new StringBuilder();
        String version = "?";
        try {
            version = getPackageManager().getPackageInfo(getPackageName(), 0).versionName;
        } catch (Exception e) {
            // Left as "?".
        }
        all.append("AudioVis ").append(version)
                .append(" on ").append(Build.MANUFACTURER).append(' ').append(Build.MODEL)
                .append(", Android ").append(Build.VERSION.RELEASE).append(" (API ").append(Build.VERSION.SDK_INT).append(")")
                .append(", chip ").append(Build.VERSION.SDK_INT >= 31 ? Build.SOC_MANUFACTURER + " " + Build.SOC_MODEL : Build.HARDWARE)
                .append("\n\n");
        if (crash != null) {
            all.append(crash.trim()).append("\n\n");
        }
        if (ending != null) {
            all.append(ending.trim()).append("\n\n");
        }
        String progress = read(new File(getFilesDir(), PROGRESS_FILE), 16 << 10);
        all.append("How far that run got:\n").append(progress == null ? "(nothing was noted)" : progress.trim()).append('\n');
        return all.toString();
    }

    /** Android's record of how the app's last run ended, if it ended badly and has not been shown before. */
    @SuppressWarnings("NewApi")
    private String ending() {
        ActivityManager manager = (ActivityManager) getSystemService(Context.ACTIVITY_SERVICE);
        List<ApplicationExitInfo> endings = manager.getHistoricalProcessExitReasons(null, 0, 1);
        if (endings.isEmpty()) {
            return null;
        }
        ApplicationExitInfo last = endings.get(0);
        SharedPreferences seen = getPreferences(MODE_PRIVATE);
        if (seen.getLong("ending shown", 0) == last.getTimestamp()) {
            return null;
        }
        seen.edit().putLong("ending shown", last.getTimestamp()).apply();

        String how;
        switch (last.getReason()) {
            case ApplicationExitInfo.REASON_CRASH:
                how = "an error in the Java side of the app";
                break;
            case ApplicationExitInfo.REASON_CRASH_NATIVE:
                how = "a crash in the app's library or the graphics driver";
                break;
            case ApplicationExitInfo.REASON_ANR:
                how = "the app stopped responding";
                break;
            case ApplicationExitInfo.REASON_INITIALIZATION_FAILURE:
                how = "the app could not be started";
                break;
            default:
                // Closed by the user or by Android making room: not a fault.
                return null;
        }
        StringBuilder text = new StringBuilder("Android's record of how it ended: ").append(how);
        if (last.getDescription() != null) {
            text.append(" (").append(last.getDescription()).append(')');
        }
        text.append(", status ").append(last.getStatus()).append('\n');
        if (Build.VERSION.SDK_INT >= 31) {
            try (InputStream trace = last.getTraceInputStream()) {
                if (trace != null) {
                    text.append("\nFrom Android's crash record:\n").append(readable(readAll(trace, 256 << 10), 6000));
                }
            } catch (IOException | RuntimeException e) {
                // The record is a bonus; go without.
            }
        }
        return text.toString();
    }

    /**
     * The readable text in Android's crash record, which is otherwise in a
     * packed form: the signal, any message the app died with and the names
     * of the functions it was in come out as words, one to a line.
     */
    private static String readable(byte[] record, int limit) {
        StringBuilder out = new StringBuilder();
        StringBuilder word = new StringBuilder();
        for (int i = 0; i <= record.length && out.length() < limit; i++) {
            char c = i < record.length ? (char) (record[i] & 0xFF) : 0;
            if (c >= 32 && c < 127) {
                word.append(c);
            } else {
                if (word.length() >= 5) {
                    out.append(word).append('\n');
                }
                word.setLength(0);
            }
        }
        return out.toString();
    }

    private static byte[] readAll(InputStream in, int limit) throws IOException {
        ByteArrayOutputStream all = new ByteArrayOutputStream();
        byte[] block = new byte[16384];
        int read;
        while (all.size() < limit && (read = in.read(block)) > 0) {
            all.write(block, 0, read);
        }
        return all.toByteArray();
    }

    /** A file's text, or null if there is no such file or it is empty. */
    private static String read(File file, int limit) {
        try (InputStream in = new FileInputStream(file)) {
            String text = new String(readAll(in, limit), StandardCharsets.UTF_8);
            return text.trim().isEmpty() ? null : text;
        } catch (IOException e) {
            return null;
        }
    }

    private void show(final String report) {
        int gap = (int) (16 * getResources().getDisplayMetrics().density);
        LinearLayout page = new LinearLayout(this);
        page.setOrientation(LinearLayout.VERTICAL);
        page.setPadding(gap, gap, gap, gap);

        TextView heading = new TextView(this);
        heading.setText("AudioVis stopped by itself last time");
        heading.setTextSize(20);
        page.addView(heading);

        TextView what = new TextView(this);
        what.setText("This is what is known about why. Sending it on (or a screenshot of it) helps get it fixed. It holds nothing about you beyond the make and model of this device.");
        what.setPadding(0, gap / 2, 0, gap / 2);
        page.addView(what);

        LinearLayout buttons = new LinearLayout(this);
        buttons.setOrientation(LinearLayout.HORIZONTAL);
        Button send = new Button(this);
        send.setText("Send it");
        send.setOnClickListener(view -> startActivity(Intent.createChooser(
                new Intent(Intent.ACTION_SEND)
                        .setType("text/plain")
                        .putExtra(Intent.EXTRA_SUBJECT, "AudioVis crash report")
                        .putExtra(Intent.EXTRA_TEXT, report),
                "Send the report")));
        buttons.addView(send);
        Button open = new Button(this);
        open.setText("Open AudioVis");
        open.setOnClickListener(view -> carryOn());
        buttons.addView(open);
        page.addView(buttons);

        TextView text = new TextView(this);
        text.setText(report);
        text.setTextSize(12);
        text.setTypeface(Typeface.MONOSPACE);
        text.setTextIsSelectable(true);
        text.setPadding(0, gap / 2, 0, 0);
        ScrollView scroll = new ScrollView(this);
        scroll.addView(text);
        page.addView(scroll);

        setContentView(page);
    }
}
