package io.github.quezka.pillion;

import android.Manifest;
import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.bluetooth.BluetoothAdapter;
import android.bluetooth.BluetoothManager;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.content.pm.PackageManager;
import android.content.pm.ServiceInfo;
import android.content.res.Configuration;
import android.location.Location;
import android.location.LocationListener;
import android.location.LocationManager;
import android.media.AudioAttributes;
import android.media.AudioFormat;
import android.media.AudioTrack;
import android.media.session.MediaSession;
import android.media.session.PlaybackState;
import android.net.Uri;
import android.os.BatteryManager;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.IBinder;
import android.os.Looper;
import android.provider.Settings;
import android.telephony.TelephonyManager;
import android.view.KeyEvent;
import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import java.util.Calendar;
import java.util.Locale;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/** Keeps the ride going with the screen off: carries out the engine's commands and feeds it events. */
public class RideService extends Service {
    private static final String CHANNEL = "ride";
    private static final String ACTION_STOP = "io.github.quezka.pillion.STOP";
    private static final int NOTIFICATION_ID = 1;
    private static final long POLL_MS = 50;
    private static final long SECOND_MS = 1000;
    private static final long GPS_INTERVAL_MS = 1000;
    private static final float GPS_MIN_METRES = 1f;
    private static final int HTTP_TIMEOUT_MS = 15000;
    private static final long TILE_KEEP_MS = 7L * 24 * 60 * 60 * 1000;
    private final ExecutorService tiles = Executors.newFixedThreadPool(2);
    private static final String USER_AGENT = "Pillion/0.1 (personal riding app)";
    private static final int SILENCE_RATE_HZ = 8000;

    private final Handler main = new Handler(Looper.getMainLooper());
    private final ExecutorService network = Executors.newSingleThreadExecutor();
    private Ble ble;
    private Voice voice;
    private MediaSession media;
    private LocationManager location;
    private boolean started;

    private final Runnable pump = new Runnable() {
        @Override
        public void run() {
            String command;
            while ((command = Native.poll()) != null) {
                carryOut(command);
            }
            main.postDelayed(this, POLL_MS);
        }
    };

    private final Runnable everySecond = new Runnable() {
        @Override
        public void run() {
            sendClockAndBattery();
            main.postDelayed(this, SECOND_MS);
        }
    };

    private final LocationListener gps = new LocationListener() {
        @Override
        public void onLocationChanged(Location at) {
            String speed = at.hasSpeed() ? String.valueOf(at.getSpeed()) : "-1";
            Native.event("gps", String.valueOf(at.getLatitude()), String.valueOf(at.getLongitude()), speed);
        }

        @Override
        public void onStatusChanged(String provider, int status, Bundle extras) {}

        @Override
        public void onProviderEnabled(String provider) {}

        @Override
        public void onProviderDisabled(String provider) {}
    };

    private final BroadcastReceiver phoneState = new BroadcastReceiver() {
        @Override
        public void onReceive(Context context, Intent intent) {
            String state = intent.getStringExtra(TelephonyManager.EXTRA_STATE);
            if (TelephonyManager.EXTRA_STATE_RINGING.equals(state)) {
                Native.event("call", "ring");
            } else if (TelephonyManager.EXTRA_STATE_OFFHOOK.equals(state)) {
                Native.event("call", "offhook");
            } else if (TelephonyManager.EXTRA_STATE_IDLE.equals(state)) {
                Native.event("call", "idle");
            }
        }
    };

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        if (intent != null && ACTION_STOP.equals(intent.getAction())) {
            stopForeground(STOP_FOREGROUND_REMOVE);
            stopSelf();
            return START_NOT_STICKY;
        }
        enterForeground();
        if (!started) {
            started = true;
            begin();
        }
        return START_NOT_STICKY;
    }

    private void begin() {
        ble = new Ble(this);
        voice = new Voice(this);
        Native.event("dir", getFilesDir().getAbsolutePath());
        startGps();
        registerReceiver(phoneState, new IntentFilter(TelephonyManager.ACTION_PHONE_STATE_CHANGED));
        startEarpieceButton();
        main.post(pump);
        main.post(everySecond);
    }

    private void enterForeground() {
        NotificationManager manager = getSystemService(NotificationManager.class);
        manager.createNotificationChannel(
                new NotificationChannel(CHANNEL, getString(R.string.channel_riding), NotificationManager.IMPORTANCE_LOW));
        PendingIntent open = PendingIntent.getActivity(
                this, 0, new Intent(this, MainActivity.class), PendingIntent.FLAG_IMMUTABLE);
        PendingIntent stop = PendingIntent.getService(
                this, 1, new Intent(this, RideService.class).setAction(ACTION_STOP), PendingIntent.FLAG_IMMUTABLE);
        Notification notification = new Notification.Builder(this, CHANNEL)
                .setContentTitle(getString(R.string.running_title))
                .setContentText(getString(R.string.running_text))
                .setSmallIcon(R.drawable.ic_status)
                .setContentIntent(open)
                .addAction(new Notification.Action.Builder(null, getString(R.string.stop), stop).build())
                .setOngoing(true)
                .build();
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) {
            startForeground(NOTIFICATION_ID, notification);
            return;
        }
        int bluetoothOnly = ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE;
        try {
            startForeground(NOTIFICATION_ID, notification, bluetoothOnly | ServiceInfo.FOREGROUND_SERVICE_TYPE_LOCATION);
        } catch (SecurityException noLocationPermission) {
            try {
                startForeground(NOTIFICATION_ID, notification, bluetoothOnly);
            } catch (RuntimeException notAllowed) {
                // Permissions were declined: it can still work while the screen is open.
            }
        } catch (RuntimeException notAllowed) {
            // Android restarted this in the background, where it may not show itself until reopened.
        }
    }

    private void startGps() {
        if (checkSelfPermission(Manifest.permission.ACCESS_FINE_LOCATION) != PackageManager.PERMISSION_GRANTED) {
            return;
        }
        location = getSystemService(LocationManager.class);
        if (location != null && location.getAllProviders().contains(LocationManager.GPS_PROVIDER)) {
            location.requestLocationUpdates(LocationManager.GPS_PROVIDER, GPS_INTERVAL_MS, GPS_MIN_METRES, gps, Looper.getMainLooper());
        }
    }

    /** The earpiece's button asks for a spoken status report. */
    private void startEarpieceButton() {
        media = new MediaSession(this, "ride");
        media.setCallback(new MediaSession.Callback() {
            @Override
            public boolean onMediaButtonEvent(Intent intent) {
                KeyEvent key = intent.getParcelableExtra(Intent.EXTRA_KEY_EVENT);
                if (key != null && key.getAction() == KeyEvent.ACTION_UP) {
                    int code = key.getKeyCode();
                    if (code == KeyEvent.KEYCODE_HEADSETHOOK
                            || code == KeyEvent.KEYCODE_MEDIA_PLAY_PAUSE
                            || code == KeyEvent.KEYCODE_MEDIA_PLAY
                            || code == KeyEvent.KEYCODE_MEDIA_PAUSE) {
                        Native.event("button");
                    }
                }
                return true;
            }
        });
        media.setPlaybackState(new PlaybackState.Builder()
                .setActions(PlaybackState.ACTION_PLAY_PAUSE | PlaybackState.ACTION_PLAY | PlaybackState.ACTION_PAUSE)
                .setState(PlaybackState.STATE_PLAYING, 0, 1f)
                .build());
        media.setActive(true);
        playSilence();
    }

    /** Android hands the media button to whichever app played audio last, so play a moment of silence. */
    private void playSilence() {
        byte[] silence = new byte[SILENCE_RATE_HZ / 2];
        AudioTrack track = new AudioTrack.Builder()
                .setAudioAttributes(new AudioAttributes.Builder()
                        .setUsage(AudioAttributes.USAGE_MEDIA)
                        .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
                        .build())
                .setAudioFormat(new AudioFormat.Builder()
                        .setEncoding(AudioFormat.ENCODING_PCM_8BIT)
                        .setSampleRate(SILENCE_RATE_HZ)
                        .setChannelMask(AudioFormat.CHANNEL_OUT_MONO)
                        .build())
                .setBufferSizeInBytes(silence.length)
                .setTransferMode(AudioTrack.MODE_STATIC)
                .build();
        track.write(silence, 0, silence.length);
        track.play();
        main.postDelayed(track::release, SECOND_MS);
    }

    private void sendClockAndBattery() {
        Calendar now = Calendar.getInstance();
        Native.event("time",
                String.valueOf(now.get(Calendar.YEAR)),
                String.valueOf(now.get(Calendar.MONTH) + 1),
                String.valueOf(now.get(Calendar.DAY_OF_MONTH)),
                String.valueOf(now.get(Calendar.HOUR_OF_DAY)),
                String.valueOf(now.get(Calendar.MINUTE)),
                String.valueOf(now.get(Calendar.SECOND)));
        BatteryManager battery = getSystemService(BatteryManager.class);
        if (battery != null) {
            Native.event("phone_batt", String.valueOf(battery.getIntProperty(BatteryManager.BATTERY_PROPERTY_CAPACITY)));
        }
        String enabled = Settings.Secure.getString(getContentResolver(), "enabled_notification_listeners");
        boolean allowed = enabled != null && enabled.contains(getPackageName());
        Native.event("access", allowed ? "1" : "0");
        sendPhoneState();
    }

    /** What the phone is set to, and what's switched off that the rider can fix. */
    private void sendPhoneState() {
        boolean night = (getResources().getConfiguration().uiMode & Configuration.UI_MODE_NIGHT_MASK)
                == Configuration.UI_MODE_NIGHT_YES;
        Native.event("system", Locale.getDefault().getLanguage(), night ? "1" : "0");
        Native.event("bt", bluetoothState());
        LocationManager manager = getSystemService(LocationManager.class);
        boolean located = manager != null && manager.isProviderEnabled(LocationManager.GPS_PROVIDER);
        Native.event("location", located ? "1" : "0");
    }

    private String bluetoothState() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S
                && checkSelfPermission(Manifest.permission.BLUETOOTH_CONNECT) != PackageManager.PERMISSION_GRANTED) {
            return "denied";
        }
        BluetoothManager manager = getSystemService(BluetoothManager.class);
        BluetoothAdapter adapter = manager == null ? null : manager.getAdapter();
        return adapter != null && adapter.isEnabled() ? "on" : "off";
    }

    private void open(Intent intent) {
        try {
            startActivity(intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
        } catch (RuntimeException unavailable) {
            // Some phones lack a settings page, or Android refused: the banner stays up to try again.
        }
    }

    private void carryOut(String command) {
        String[] f = command.split(String.valueOf(Native.SEPARATOR), -1);
        switch (f[0]) {
            case "scan":
                ble.scan();
                break;
            case "stop_scan":
                ble.stopScan();
                break;
            case "connect":
                ble.connect(f[1]);
                break;
            case "disconnect":
                ble.disconnect();
                break;
            case "write":
                ble.write(Ble.fromHex(f[1]));
                break;
            case "speak":
                voice.say(f[2], "1".equals(f[1]));
                break;
            case "rate":
                voice.setRate(Float.parseFloat(f[1]));
                break;
            case "route":
                fetch(f[1], "route_ok", "route_err", f[2]);
                break;
            case "search":
                fetch(f[1], "search_ok", "search_err", null);
                break;
            case "ask":
                MainActivity.askText(f[1], f[2], f[3], "1".equals(f[4]));
                break;
            case "haptic":
                MainActivity.haptic();
                break;
            case "update_check":
                fetch(f[1], "update_ok", "update_err", null);
                break;
            case "update_install":
                Updater.install(this, f[1], f[2]);
                break;
            case "scan_qr":
                MainActivity.scanQr(f[1], f[2], f[3], f[4]);
                break;
            case "voice_download":
                voice.downloadPack();
                break;
            case "voice_remove":
                voice.removePack();
                break;
            case "voice_natural":
                voice.setNaturalWanted("1".equals(f[1]));
                break;
            case "tile":
                fetchTile(f[1], f[2], f[3]);
                break;
            case "share":
                share(f[1]);
                break;
            case "notif_access":
                open(new Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS));
                break;
            case "lang":
                voice.setLanguage(f[1]);
                break;
            case "bt_enable":
                open(new Intent(BluetoothAdapter.ACTION_REQUEST_ENABLE));
                break;
            case "location_settings":
                open(new Intent(Settings.ACTION_LOCATION_SOURCE_SETTINGS));
                break;
            case "app_settings":
                open(new Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS,
                        Uri.fromParts("package", getPackageName(), null)));
                break;
            default:
                break;
        }
    }

    /** Map pictures are kept for a week, as the map's provider asks, and so that riding the same way costs no data. */
    private void fetchTile(String z, String x, String y) {
        tiles.execute(() -> {
            File folder = new File(getCacheDir(), "tiles");
            File file = new File(folder, z + "_" + x + "_" + y + ".png");
            if (file.length() > 0 && System.currentTimeMillis() - file.lastModified() < TILE_KEEP_MS) {
                Native.event("tile", z, x, y, file.getPath());
                return;
            }
            File partial = new File(folder, file.getName() + ".part");
            try {
                if (!folder.isDirectory() && !folder.mkdirs()) {
                    throw new IOException("No room for the map");
                }
                HttpURLConnection connection = (HttpURLConnection) new URL(
                        "https://tile.openstreetmap.org/" + z + "/" + x + "/" + y + ".png").openConnection();
                connection.setConnectTimeout(10_000);
                connection.setReadTimeout(15_000);
                connection.setRequestProperty("User-Agent", USER_AGENT);
                try (InputStream in = connection.getInputStream(); FileOutputStream out = new FileOutputStream(partial)) {
                    byte[] buffer = new byte[16 * 1024];
                    for (int read = in.read(buffer); read >= 0; read = in.read(buffer)) {
                        out.write(buffer, 0, read);
                    }
                } finally {
                    connection.disconnect();
                }
                if (!partial.renameTo(file)) {
                    throw new IOException("The map couldn't be saved");
                }
                Native.event("tile", z, x, y, file.getPath());
            } catch (IOException failed) {
                Native.event("tile_failed", z, x, y);
            }
        });
    }

    private void share(String text) {
        Intent send = new Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, text);
        open(Intent.createChooser(send, null));
    }

    private void fetch(String url, String okEvent, String errorEvent, String destination) {
        network.execute(() -> {
            try {
                String body = download(url);
                if (destination == null) {
                    Native.event(okEvent, body);
                } else {
                    Native.event(okEvent, destination, body);
                }
            } catch (IOException | RuntimeException failure) {
                String message = String.valueOf(failure.getMessage());
                if (destination == null) {
                    Native.event(errorEvent, message);
                } else {
                    Native.event(errorEvent, destination, message);
                }
            }
        });
    }

    private static String download(String address) throws IOException {
        HttpURLConnection connection = (HttpURLConnection) new URL(address).openConnection();
        try {
            connection.setRequestProperty("User-Agent", USER_AGENT);
            connection.setConnectTimeout(HTTP_TIMEOUT_MS);
            connection.setReadTimeout(HTTP_TIMEOUT_MS);
            int code = connection.getResponseCode();
            InputStream in = code < 400 ? connection.getInputStream() : connection.getErrorStream();
            if (in == null) {
                throw new IOException("the server answered " + code);
            }
            ByteArrayOutputStream out = new ByteArrayOutputStream();
            byte[] buffer = new byte[8192];
            int read;
            while ((read = in.read(buffer)) != -1) {
                out.write(buffer, 0, read);
            }
            return new String(out.toByteArray(), StandardCharsets.UTF_8);
        } finally {
            connection.disconnect();
        }
    }

    @Override
    public void onDestroy() {
        main.removeCallbacksAndMessages(null);
        network.shutdownNow();
        if (started) {
            unregisterReceiver(phoneState);
            if (location != null) {
                location.removeUpdates(gps);
            }
            if (media != null) {
                media.release();
            }
            ble.disconnect();
            voice.close();
        }
        super.onDestroy();
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }
}
