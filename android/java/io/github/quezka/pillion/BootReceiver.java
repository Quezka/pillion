package io.github.quezka.pillion;

import android.bluetooth.BluetoothAdapter;
import android.bluetooth.BluetoothDevice;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;

/**
 * Starts Pillion on its own: after the phone boots, after an update, when Bluetooth comes on and when a Bluetooth
 * device connects. The ride service then waits for the moped (one auto-connect request), so opening the app
 * first is never needed. The Stop button in its notification still stops it until the next of these.
 */
public final class BootReceiver extends BroadcastReceiver {
    @Override
    public void onReceive(Context context, Intent intent) {
        String action = intent.getAction();
        if (action == null) {
            return;
        }
        boolean wanted = Intent.ACTION_BOOT_COMPLETED.equals(action)
                || Intent.ACTION_LOCKED_BOOT_COMPLETED.equals(action)
                || Intent.ACTION_MY_PACKAGE_REPLACED.equals(action)
                || BluetoothDevice.ACTION_ACL_CONNECTED.equals(action)
                || (BluetoothAdapter.ACTION_STATE_CHANGED.equals(action)
                    && intent.getIntExtra(BluetoothAdapter.EXTRA_STATE, -1) == BluetoothAdapter.STATE_ON);
        if (!wanted || !context.getSharedPreferences("pillion", Context.MODE_PRIVATE).getBoolean("autostart", true)) {
            return;
        }
        try {
            context.startForegroundService(new Intent(context, RideService.class));
        } catch (RuntimeException notAllowed) {
            // Android refused a background start (rare for these broadcasts): the next one tries again.
        }
    }
}
