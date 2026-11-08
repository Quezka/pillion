package io.github.quezka.pillion;

import android.bluetooth.BluetoothAdapter;
import android.bluetooth.BluetoothDevice;
import android.bluetooth.BluetoothGatt;
import android.bluetooth.BluetoothGattCallback;
import android.bluetooth.BluetoothGattCharacteristic;
import android.bluetooth.BluetoothGattDescriptor;
import android.bluetooth.BluetoothGattService;
import android.bluetooth.BluetoothManager;
import android.bluetooth.BluetoothProfile;
import android.bluetooth.le.BluetoothLeScanner;
import android.bluetooth.le.ScanCallback;
import android.bluetooth.le.ScanResult;
import android.bluetooth.le.ScanSettings;
import android.content.Context;
import android.os.Handler;
import android.os.Looper;
import java.util.ArrayDeque;
import java.util.UUID;

/** Bluetooth Low Energy link to the moped, in the order the original app does it. */
final class Ble {
    private static final UUID SERVICE = UUID.fromString("6e400001-b5a3-f393-e0a9-e50e24dcca9e");
    private static final UUID WRITE = UUID.fromString("6e400002-b5a3-f393-e0a9-e50e24dcca9e");
    private static final UUID NOTIFY = UUID.fromString("6e400003-b5a3-f393-e0a9-e50e24dcca9e");
    private static final UUID SWITCH = UUID.fromString("00002902-0000-1000-8000-00805f9b34fb");
    private static final int MTU = 512;
    private static final long WRITE_GAP_MS = 100;
    private static final long WRITE_STALL_MS = 500;

    private final Context context;
    private final Handler main = new Handler(Looper.getMainLooper());
    private final ArrayDeque<byte[]> writes = new ArrayDeque<>();
    private BluetoothGatt gatt;
    private BluetoothGattCharacteristic writer;
    private boolean writing;

    private final ScanCallback scanner = new ScanCallback() {
        @Override
        public void onScanResult(int type, ScanResult result) {
            BluetoothDevice device = result.getDevice();
            String name = device.getName();
            if (name == null && result.getScanRecord() != null) {
                name = result.getScanRecord().getDeviceName();
            }
            if (name != null) {
                Native.event("found", name, device.getAddress());
            }
        }
    };

    private final Runnable stalledWrite = () -> {
        writing = false;
        sendNext();
    };

    private final BluetoothGattCallback callback = new BluetoothGattCallback() {
        @Override
        public void onConnectionStateChange(BluetoothGatt g, int status, int state) {
            if (state == BluetoothProfile.STATE_CONNECTED) {
                g.discoverServices();
            } else if (state == BluetoothProfile.STATE_DISCONNECTED) {
                main.post(() -> {
                    writes.clear();
                    writing = false;
                    writer = null;
                });
                Native.event("gone");
            }
        }

        @Override
        public void onServicesDiscovered(BluetoothGatt g, int status) {
            BluetoothGattService service = g.getService(SERVICE);
            BluetoothGattCharacteristic notify = service == null ? null : service.getCharacteristic(NOTIFY);
            BluetoothGattCharacteristic write = service == null ? null : service.getCharacteristic(WRITE);
            BluetoothGattDescriptor on = notify == null ? null : notify.getDescriptor(SWITCH);
            if (write == null || on == null) {
                g.disconnect();
                return;
            }
            main.post(() -> writer = write);
            g.setCharacteristicNotification(notify, true);
            on.setValue(BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE);
            g.writeDescriptor(on);
        }

        @Override
        public void onDescriptorWrite(BluetoothGatt g, BluetoothGattDescriptor descriptor, int status) {
            g.requestMtu(MTU);
        }

        @Override
        public void onMtuChanged(BluetoothGatt g, int mtu, int status) {
            Native.event("ready");
        }

        @Override
        public void onCharacteristicChanged(BluetoothGatt g, BluetoothGattCharacteristic c) {
            if (NOTIFY.equals(c.getUuid())) {
                Native.event("rx", toHex(c.getValue()));
            }
        }

        @Override
        public void onCharacteristicWrite(BluetoothGatt g, BluetoothGattCharacteristic c, int status) {
            main.post(() -> {
                main.removeCallbacks(stalledWrite);
                main.postDelayed(() -> {
                    writing = false;
                    sendNext();
                }, WRITE_GAP_MS);
            });
        }
    };

    Ble(Context context) {
        this.context = context;
    }

    private BluetoothAdapter adapter() {
        BluetoothManager manager = context.getSystemService(BluetoothManager.class);
        return manager == null ? null : manager.getAdapter();
    }

    void scan() {
        BluetoothAdapter adapter = adapter();
        BluetoothLeScanner le = adapter == null ? null : adapter.getBluetoothLeScanner();
        if (le == null) {
            return;
        }
        try {
            le.stopScan(scanner);
            le.startScan(null, new ScanSettings.Builder().setReportDelay(0).build(), scanner);
        } catch (SecurityException noPermission) {
            // The engine keeps retrying, and it'll work once the permission is granted.
        }
    }

    void stopScan() {
        BluetoothAdapter adapter = adapter();
        BluetoothLeScanner le = adapter == null ? null : adapter.getBluetoothLeScanner();
        if (le == null) {
            return;
        }
        try {
            le.stopScan(scanner);
        } catch (SecurityException noPermission) {
            // Nothing was scanning without the permission.
        }
    }

    void connect(String address) {
        BluetoothAdapter adapter = adapter();
        if (adapter == null) {
            return;
        }
        try {
            closeGatt();
            BluetoothDevice device = adapter.getRemoteDevice(address);
            device.createBond();
            gatt = device.connectGatt(context, true, callback, BluetoothDevice.TRANSPORT_LE);
        } catch (SecurityException | IllegalArgumentException cannotConnect) {
            // The engine retries after its timeout.
        }
    }

    /**
     * Leaves one request open for the moped, without scanning. Android hands it to the Bluetooth chip, which connects
     * by itself when the moped is switched on: almost no battery while waiting. An open request for the same moped is
     * kept as it is.
     */
    void waitFor(String address) {
        if (gatt != null && gatt.getDevice() != null && address.equals(gatt.getDevice().getAddress())) {
            return;
        }
        BluetoothAdapter adapter = adapter();
        if (adapter == null) {
            return;
        }
        try {
            closeGatt();
            gatt = adapter.getRemoteDevice(address).connectGatt(context, true, callback, BluetoothDevice.TRANSPORT_LE);
        } catch (SecurityException | IllegalArgumentException cannotConnect) {
            // Without the permission there's nothing to wait with; "Look for it now" tries again.
        }
    }

    void disconnect() {
        try {
            closeGatt();
        } catch (SecurityException noPermission) {
            gatt = null;
        }
    }

    private void closeGatt() {
        writes.clear();
        writing = false;
        writer = null;
        if (gatt != null) {
            gatt.disconnect();
            gatt.close();
            gatt = null;
        }
    }

    void write(byte[] bytes) {
        writes.add(bytes);
        sendNext();
    }

    private void sendNext() {
        if (writing || writer == null || gatt == null || writes.isEmpty()) {
            return;
        }
        byte[] bytes = writes.poll();
        writing = true;
        writer.setValue(bytes);
        try {
            if (!gatt.writeCharacteristic(writer)) {
                writing = false;
                return;
            }
        } catch (SecurityException noPermission) {
            writing = false;
            return;
        }
        main.postDelayed(stalledWrite, WRITE_STALL_MS);
    }

    static byte[] fromHex(String hex) {
        byte[] out = new byte[hex.length() / 2];
        for (int i = 0; i < out.length; i++) {
            out[i] = (byte) Integer.parseInt(hex.substring(i * 2, i * 2 + 2), 16);
        }
        return out;
    }

    static String toHex(byte[] bytes) {
        StringBuilder out = new StringBuilder(bytes.length * 2);
        for (byte b : bytes) {
            out.append(Character.forDigit((b >> 4) & 0xF, 16));
            out.append(Character.forDigit(b & 0xF, 16));
        }
        return out.toString();
    }
}
