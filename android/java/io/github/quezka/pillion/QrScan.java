package io.github.quezka.pillion;

import android.Manifest;
import android.app.Activity;
import android.content.pm.PackageManager;
import android.graphics.Canvas;
import android.graphics.Color;
import android.graphics.Paint;
import android.graphics.Path;
import android.graphics.RectF;
import android.hardware.Camera;
import android.os.Bundle;
import android.view.Gravity;
import android.view.HapticFeedbackConstants;
import android.view.SurfaceHolder;
import android.view.SurfaceView;
import android.view.View;
import android.view.WindowManager;
import android.widget.Button;
import android.widget.FrameLayout;
import android.widget.LinearLayout;
import android.widget.TextView;
import java.io.IOException;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/** Reads the moped's QR sticker with the camera. The older camera interface is all this needs: pictures to look at. */
public class QrScan extends Activity implements SurfaceHolder.Callback {
    static final String TITLE = "title";
    static final String TORCH = "torch";
    static final String TYPE_IT = "type";
    static final String CANCEL = "cancel";
    private static final int CAMERA_REQUEST = 7;
    private static final int WANTED_WIDTH = 1280;

    private final ExecutorService reader = Executors.newSingleThreadExecutor();
    private Camera camera;
    private SurfaceView preview;
    private boolean surfaceReady;
    private boolean permitted;
    private volatile boolean busy;
    private volatile boolean found;
    private boolean torchOn;

    /** A dimmed screen with a clear window in the middle, to aim with. */
    private static final class Viewfinder extends View {
        private final Paint dim = new Paint();
        private final Paint edge = new Paint(Paint.ANTI_ALIAS_FLAG);
        private final Path hole = new Path();

        Viewfinder(Activity activity) {
            super(activity);
            dim.setColor(0x99000000);
            edge.setColor(Color.WHITE);
            edge.setStyle(Paint.Style.STROKE);
            edge.setStrokeWidth(6f);
        }

        @Override
        protected void onDraw(Canvas canvas) {
            float side = getWidth() * 0.72f;
            float left = (getWidth() - side) / 2f;
            float top = getHeight() * 0.42f - side / 2f;
            RectF window = new RectF(left, top, left + side, top + side);
            hole.reset();
            hole.setFillType(Path.FillType.EVEN_ODD);
            hole.addRect(0, 0, getWidth(), getHeight(), Path.Direction.CW);
            hole.addRoundRect(window, 42f, 42f, Path.Direction.CW);
            canvas.drawPath(hole, dim);
            canvas.drawRoundRect(window, 42f, 42f, edge);
        }
    }

    @Override
    protected void onCreate(Bundle saved) {
        super.onCreate(saved);
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        FrameLayout root = new FrameLayout(this);
        root.setBackgroundColor(Color.BLACK);
        preview = new SurfaceView(this);
        root.addView(preview, new FrameLayout.LayoutParams(-1, -1, Gravity.CENTER));
        preview.getHolder().addCallback(this);
        root.addView(new Viewfinder(this), new FrameLayout.LayoutParams(-1, -1));

        LinearLayout controls = new LinearLayout(this);
        controls.setOrientation(LinearLayout.VERTICAL);
        controls.setGravity(Gravity.CENTER_HORIZONTAL);
        int gap = (int) (24 * getResources().getDisplayMetrics().density);
        controls.setPadding(gap, gap, gap, gap * 2);
        TextView title = new TextView(this);
        title.setText(getIntent().getStringExtra(TITLE));
        title.setTextColor(Color.WHITE);
        title.setTextSize(17f);
        title.setGravity(Gravity.CENTER);
        title.setPadding(0, 0, 0, gap);
        controls.addView(title);
        LinearLayout buttons = new LinearLayout(this);
        buttons.setGravity(Gravity.CENTER);
        buttons.addView(button(getIntent().getStringExtra(TORCH), v -> toggleTorch()));
        buttons.addView(button(getIntent().getStringExtra(TYPE_IT), v -> {
            Native.event("qr_type");
            finish();
        }));
        buttons.addView(button(getIntent().getStringExtra(CANCEL), v -> finish()));
        controls.addView(buttons);
        root.addView(controls, new FrameLayout.LayoutParams(-1, -2, Gravity.BOTTOM));
        setContentView(root);

        permitted = checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED;
        if (!permitted) {
            requestPermissions(new String[] {Manifest.permission.CAMERA}, CAMERA_REQUEST);
        }
    }

    private Button button(String label, View.OnClickListener action) {
        Button button = new Button(this, null, android.R.attr.borderlessButtonStyle);
        button.setText(label);
        button.setTextColor(Color.WHITE);
        button.setAllCaps(false);
        button.setOnClickListener(action);
        return button;
    }

    @Override
    public void onRequestPermissionsResult(int request, String[] permissions, int[] results) {
        permitted = results.length > 0 && results[0] == PackageManager.PERMISSION_GRANTED;
        if (permitted) {
            start();
        } else {
            Native.event("qr_type");
            finish();
        }
    }

    @Override
    protected void onResume() {
        super.onResume();
        start();
    }

    @Override
    protected void onPause() {
        stop();
        super.onPause();
    }

    @Override
    protected void onDestroy() {
        reader.shutdownNow();
        super.onDestroy();
    }

    @Override
    public void surfaceCreated(SurfaceHolder holder) {
        surfaceReady = true;
        start();
    }

    @Override
    public void surfaceChanged(SurfaceHolder holder, int format, int width, int height) {}

    @Override
    public void surfaceDestroyed(SurfaceHolder holder) {
        surfaceReady = false;
        stop();
    }

    private void start() {
        if (camera != null || !surfaceReady || !permitted) {
            return;
        }
        try {
            camera = Camera.open();
            if (camera == null) {
                failed();
                return;
            }
            Camera.Parameters settings = camera.getParameters();
            Camera.Size size = settings.getPreviewSize();
            for (Camera.Size option : settings.getSupportedPreviewSizes()) {
                if (Math.abs(option.width - WANTED_WIDTH) < Math.abs(size.width - WANTED_WIDTH)) {
                    size = option;
                }
            }
            settings.setPreviewSize(size.width, size.height);
            if (settings.getSupportedFocusModes().contains(Camera.Parameters.FOCUS_MODE_CONTINUOUS_PICTURE)) {
                settings.setFocusMode(Camera.Parameters.FOCUS_MODE_CONTINUOUS_PICTURE);
            }
            camera.setParameters(settings);
            camera.setDisplayOrientation(90);
            // The camera's picture lies on its side, so its width runs down the screen.
            int screen = getResources().getDisplayMetrics().widthPixels;
            preview.setLayoutParams(new FrameLayout.LayoutParams(screen, screen * size.width / size.height, Gravity.CENTER));
            final int width = size.width;
            final int height = size.height;
            camera.addCallbackBuffer(new byte[width * height * 3 / 2]);
            camera.setPreviewCallbackWithBuffer((frame, source) -> look(frame, source, width, height));
            camera.setPreviewDisplay(preview.getHolder());
            camera.startPreview();
        } catch (IOException | RuntimeException noCamera) {
            failed();
        }
    }

    private void failed() {
        Native.event("qr_type");
        finish();
    }

    private void look(byte[] frame, Camera source, int width, int height) {
        if (busy || found) {
            source.addCallbackBuffer(frame);
            return;
        }
        busy = true;
        // The first part of the frame is its brightness, which is all a QR code needs.
        final byte[] brightness = new byte[width * height];
        System.arraycopy(frame, 0, brightness, 0, brightness.length);
        source.addCallbackBuffer(frame);
        reader.execute(() -> {
            String code = Native.decodeQr(brightness, width, height);
            busy = false;
            if (code != null && !found) {
                found = true;
                runOnUiThread(() -> {
                    getWindow().getDecorView().performHapticFeedback(HapticFeedbackConstants.CONFIRM);
                    Native.event("text", "pair", code);
                    finish();
                });
            }
        });
    }

    private void toggleTorch() {
        if (camera == null) {
            return;
        }
        try {
            torchOn = !torchOn;
            Camera.Parameters settings = camera.getParameters();
            settings.setFlashMode(torchOn ? Camera.Parameters.FLASH_MODE_TORCH : Camera.Parameters.FLASH_MODE_OFF);
            camera.setParameters(settings);
        } catch (RuntimeException noTorch) {
            torchOn = false;
        }
    }

    private void stop() {
        if (camera != null) {
            camera.setPreviewCallbackWithBuffer(null);
            camera.stopPreview();
            camera.release();
            camera = null;
        }
    }
}
