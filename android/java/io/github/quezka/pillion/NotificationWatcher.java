package io.github.quezka.pillion;

import android.app.Notification;
import android.content.pm.ApplicationInfo;
import android.content.pm.PackageManager;
import android.service.notification.NotificationListenerService;
import android.service.notification.StatusBarNotification;
import java.util.ArrayDeque;
import java.util.Objects;

/** Passes new phone notifications to the engine. Nothing here leaves the phone. */
public class NotificationWatcher extends NotificationListenerService {
    private static final int RECENT = 64;

    private final ArrayDeque<String> recent = new ArrayDeque<>();
    private long connectedAt;

    @Override
    public void onListenerConnected() {
        connectedAt = System.currentTimeMillis();
    }

    @Override
    public void onNotificationPosted(StatusBarNotification posted) {
        Notification n = posted.getNotification();
        boolean summary = (n.flags & Notification.FLAG_GROUP_SUMMARY) != 0;
        if (posted.isOngoing() && !Notification.CATEGORY_CALL.equals(n.category)) {
            return;
        }
        if (summary || n.when < connectedAt || getPackageName().equals(posted.getPackageName())) {
            return;
        }
        String key = posted.getPackageName() + '/' + posted.getTag() + '/' + posted.getId() + '/' + n.when;
        if (recent.contains(key)) {
            return;
        }
        recent.addLast(key);
        if (recent.size() > RECENT) {
            recent.removeFirst();
        }
        CharSequence title = n.extras.getCharSequence(Notification.EXTRA_TITLE);
        CharSequence text = n.extras.getCharSequence(Notification.EXTRA_BIG_TEXT);
        if (text == null) {
            text = n.extras.getCharSequence(Notification.EXTRA_TEXT);
        }
        boolean call = Notification.CATEGORY_CALL.equals(n.category);
        Native.event("notif",
                posted.getPackageName(),
                label(posted.getPackageName(), posted.getNotification()),
                Objects.toString(title, ""),
                Objects.toString(text, ""),
                call ? "1" : "0");
    }

    @Override
    public void onNotificationRemoved(StatusBarNotification removed) {
        for (StatusBarNotification still : getActiveNotifications()) {
            if (still.getPackageName().equals(removed.getPackageName()) && !still.isOngoing()) {
                return;
            }
        }
        Native.event("notif_gone", removed.getPackageName());
    }

    /** The app's name, not its package id. Android 11+ hides other apps from the package manager unless
     *  the manifest asks to see them, but every notification carries its app's info: use that first. */
    private String label(String packageName, android.app.Notification notification) {
        PackageManager packages = getPackageManager();
        if (notification != null) {
            ApplicationInfo carried = notification.extras.getParcelable("android.appInfo");
            if (carried != null) {
                CharSequence name = carried.loadLabel(packages);
                if (name != null && name.length() > 0 && !name.toString().equals(packageName)) {
                    return name.toString();
                }
            }
        }
        try {
            ApplicationInfo info = packages.getApplicationInfo(packageName, 0);
            return packages.getApplicationLabel(info).toString();
        } catch (PackageManager.NameNotFoundException gone) {
            return packageName;
        }
    }
}
