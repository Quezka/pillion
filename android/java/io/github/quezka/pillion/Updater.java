package io.github.quezka.pillion;

import android.app.PendingIntent;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageInfo;
import android.content.pm.PackageInstaller;
import android.net.Uri;
import android.provider.Settings;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import java.security.MessageDigest;
import java.util.Locale;

/**
 * Downloads a new version and hands it to Android, which asks the rider to confirm. Android refuses it unless it's signed with
 * the same key as the installed app, so nobody else's build can replace this one.
 */
final class Updater {
    private static final String USER_AGENT = "Pillion/0.2 (personal riding app)";
    private static volatile boolean busy;

    private Updater() {}

    static void install(Context context, String address, String expectedSum) {
        if (busy) {
            return;
        }
        if (!context.getPackageManager().canRequestPackageInstalls()) {
            Native.event("update_state", "permission", "0");
            Intent allow = new Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:" + context.getPackageName()));
            context.startActivity(allow.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
            return;
        }
        busy = true;
        Native.event("update_state", "downloading", "0");
        Thread thread = new Thread(() -> {
            File file = new File(context.getCacheDir(), "update.apk");
            try {
                String sum = download(address, file);
                if (!expectedSum.isEmpty() && !expectedSum.equalsIgnoreCase(sum)) {
                    throw new IOException("The update didn't arrive intact");
                }
                check(context, file);
                Native.event("update_state", "installing", "0");
                send(context, file);
            } catch (Exception failed) {
                android.util.Log.w("update", "The update failed", failed);
                Native.event("update_state", "failed", "0");
            } finally {
                file.delete();
                busy = false;
            }
        }, "update");
        thread.setDaemon(true);
        thread.start();
    }

    private static String download(String address, File file) throws Exception {
        HttpURLConnection connection = (HttpURLConnection) new URL(address).openConnection();
        connection.setConnectTimeout(20_000);
        connection.setReadTimeout(30_000);
        connection.setRequestProperty("User-Agent", USER_AGENT);
        MessageDigest digest = MessageDigest.getInstance("SHA-256");
        try (InputStream in = connection.getInputStream(); FileOutputStream out = new FileOutputStream(file)) {
            long size = connection.getContentLengthLong();
            long total = 0;
            int lastPercent = -1;
            byte[] buffer = new byte[64 * 1024];
            for (int read = in.read(buffer); read >= 0; read = in.read(buffer)) {
                out.write(buffer, 0, read);
                digest.update(buffer, 0, read);
                total += read;
                int percent = size > 0 ? (int) Math.min(99, total * 100 / size) : 0;
                if (percent != lastPercent) {
                    lastPercent = percent;
                    Native.event("update_state", "downloading", String.valueOf(percent));
                }
            }
        } finally {
            connection.disconnect();
        }
        StringBuilder sum = new StringBuilder();
        for (byte b : digest.digest()) {
            sum.append(String.format(Locale.ROOT, "%02x", b));
        }
        return sum.toString();
    }

    /** It has to be this app, and newer. The signature is left to Android, which checks it when installing. */
    @SuppressWarnings("deprecation")
    private static void check(Context context, File file) throws Exception {
        PackageInfo update = context.getPackageManager().getPackageArchiveInfo(file.getPath(), 0);
        if (update == null || !context.getPackageName().equals(update.packageName)) {
            throw new IOException("That isn't Pillion");
        }
        PackageInfo installed = context.getPackageManager().getPackageInfo(context.getPackageName(), 0);
        if (update.versionCode <= installed.versionCode) {
            throw new IOException("That isn't newer");
        }
    }

    private static void send(Context context, File file) throws IOException {
        PackageInstaller installer = context.getPackageManager().getPackageInstaller();
        PackageInstaller.SessionParams params = new PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL);
        params.setAppPackageName(context.getPackageName());
        int id = installer.createSession(params);
        try (PackageInstaller.Session session = installer.openSession(id)) {
            try (InputStream in = new FileInputStream(file); OutputStream out = session.openWrite("pillion.apk", 0, file.length())) {
                byte[] buffer = new byte[64 * 1024];
                for (int read = in.read(buffer); read >= 0; read = in.read(buffer)) {
                    out.write(buffer, 0, read);
                }
                session.fsync(out);
            }
            Intent result = new Intent(context, UpdateResult.class);
            // Android fills in the outcome, so this one has to stay changeable.
            int flags = PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_MUTABLE;
            session.commit(PendingIntent.getBroadcast(context, id, result, flags).getIntentSender());
        }
    }

    /** Android's answer: it wants the rider's confirmation, or it has an outcome. On success this app is replaced and closes. */
    public static final class UpdateResult extends BroadcastReceiver {
        @Override
        @SuppressWarnings("deprecation")
        public void onReceive(Context context, Intent intent) {
            int status = intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE);
            if (status == PackageInstaller.STATUS_PENDING_USER_ACTION) {
                Intent confirm = intent.getParcelableExtra(Intent.EXTRA_INTENT);
                if (confirm != null) {
                    context.startActivity(confirm.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
                }
            } else if (status != PackageInstaller.STATUS_SUCCESS) {
                android.util.Log.w("update", "Android refused: " + intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE));
                Native.event("update_state", "failed", "0");
            }
        }
    }
}
