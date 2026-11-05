package io.github.quezka.pillion;

import android.app.Activity;
import android.content.Intent;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;

/** Runs in its own process, so it outlives the app long enough to open it again. Never seen. */
public class Reopen extends Activity {
    private static final long AFTER_THE_OLD_ONE_IS_GONE_MS = 400;

    @Override
    protected void onCreate(Bundle saved) {
        super.onCreate(saved);
        new Handler(Looper.getMainLooper()).postDelayed(() -> {
            startActivity(new Intent(this, MainActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
            finish();
        }, AFTER_THE_OLD_ONE_IS_GONE_MS);
    }
}
