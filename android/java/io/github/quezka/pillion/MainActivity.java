package io.github.quezka.pillion;

import android.Manifest;
import android.app.AlertDialog;
import android.app.NativeActivity;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.os.Build;
import android.os.Bundle;
import android.view.WindowManager;
import android.widget.EditText;
import java.util.ArrayList;
import java.util.List;

/** The screen. The drawing is in Rust; this asks for permissions and shows text dialogs. */
public class MainActivity extends NativeActivity {
    private static final int PERMISSION_REQUEST = 1;
    private static MainActivity current;

    @Override
    protected void onCreate(Bundle saved) {
        super.onCreate(saved);
        current = this;
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        // The drawing surface runs under Android's bars, so the app needs to know how much room to leave.
        getWindow().getDecorView().setOnApplyWindowInsetsListener((view, insets) -> {
            Native.event("insets",
                    String.valueOf(insets.getSystemWindowInsetTop()),
                    String.valueOf(insets.getSystemWindowInsetBottom()));
            return insets;
        });
        List<String> missing = missingPermissions();
        if (missing.isEmpty()) {
            startRideService();
        } else {
            requestPermissions(missing.toArray(new String[0]), PERMISSION_REQUEST);
        }
    }

    /** Back keeps the ride going in the background, like the Home button. */
    @Override
    public void onBackPressed() {
        moveTaskToBack(true);
    }

    /**
     * The window library's thread never ends, and NativeActivity's own onDestroy waits for it
     * forever, which freezes the whole app. So closing the screen ends the app cleanly. Android also
     * rebuilds the screen on its own, for one when the wallpaper's colours change the system theme,
     * and then Pillion comes straight back: it reconnects to the moped by itself.
     */
    @Override
    protected void onDestroy() {
        if (!isFinishing()) {
            startActivity(new Intent(this, Reopen.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
        }
        android.os.Process.killProcess(android.os.Process.myPid());
        super.onDestroy();
    }

    private List<String> missingPermissions() {
        List<String> wanted = new ArrayList<>();
        wanted.add(Manifest.permission.ACCESS_FINE_LOCATION);
        wanted.add(Manifest.permission.READ_PHONE_STATE);
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            wanted.add(Manifest.permission.BLUETOOTH_SCAN);
            wanted.add(Manifest.permission.BLUETOOTH_CONNECT);
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            wanted.add(Manifest.permission.POST_NOTIFICATIONS);
        }
        List<String> missing = new ArrayList<>();
        for (String permission : wanted) {
            if (checkSelfPermission(permission) != PackageManager.PERMISSION_GRANTED) {
                missing.add(permission);
            }
        }
        return missing;
    }

    @Override
    public void onRequestPermissionsResult(int request, String[] permissions, int[] results) {
        super.onRequestPermissionsResult(request, permissions, results);
        startRideService();
    }

    private void startRideService() {
        startForegroundService(new Intent(this, RideService.class));
    }

    /** A QR reader app usually copies what it reads, which saves typing 24 characters by hand. */
    private static String pairingCodeOnClipboard(MainActivity activity) {
        ClipboardManager clipboard = activity.getSystemService(ClipboardManager.class);
        ClipData clip = clipboard == null ? null : clipboard.getPrimaryClip();
        if (clip == null || clip.getItemCount() == 0 || clip.getItemAt(0).getText() == null) {
            return "";
        }
        String copied = clip.getItemAt(0).getText().toString().trim();
        return copied.length() == 24 || copied.length() == 20 ? copied : "";
    }

    /** Asks for text with Android's own dialog, since its keyboard can't type into the Rust window. */
    static void askText(String key, String title, String hint) {
        MainActivity activity = current;
        if (activity == null) {
            return;
        }
        activity.runOnUiThread(() -> {
            EditText input = new EditText(activity);
            input.setHint(hint);
            input.setSingleLine(true);
            if ("pair".equals(key)) {
                input.setText(pairingCodeOnClipboard(activity));
            }
            AlertDialog dialog = new AlertDialog.Builder(activity, android.R.style.Theme_Material_Dialog_Alert)
                    .setTitle(title)
                    .setView(input)
                    .setPositiveButton("OK", (d, w) -> Native.event("text", key, input.getText().toString()))
                    .setNegativeButton("Cancel", null)
                    .create();
            dialog.getWindow().setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_STATE_ALWAYS_VISIBLE);
            dialog.show();
        });
    }
}
