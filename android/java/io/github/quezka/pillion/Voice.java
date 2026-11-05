package io.github.quezka.pillion;

import android.content.Context;
import android.media.AudioAttributes;
import android.media.AudioFocusRequest;
import android.media.AudioManager;
import android.speech.tts.TextToSpeech;
import android.speech.tts.UtteranceProgressListener;
import java.util.ArrayDeque;
import java.util.Locale;

/** The synthesised voice. Android sends it to the connected earpiece, and music dips while it talks. */
final class Voice {
    private static final String UTTERANCE = "ride";

    private final AudioManager audio;
    private final AudioFocusRequest focus;
    private final TextToSpeech tts;
    private final ArrayDeque<String> waiting = new ArrayDeque<>();
    private boolean ready;
    private Locale language = Locale.UK;

    Voice(Context context) {
        audio = context.getSystemService(AudioManager.class);
        AudioAttributes attributes = new AudioAttributes.Builder()
                .setUsage(AudioAttributes.USAGE_ASSISTANCE_NAVIGATION_GUIDANCE)
                .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
                .build();
        focus = new AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN_TRANSIENT_MAY_DUCK)
                .setAudioAttributes(attributes)
                .build();
        tts = new TextToSpeech(context, status -> {
            if (status != TextToSpeech.SUCCESS) {
                Native.event("voice_missing");
                return;
            }
            ready = true;
            flushWaiting();
        });
        tts.setAudioAttributes(attributes);
        tts.setOnUtteranceProgressListener(new UtteranceProgressListener() {
            @Override
            public void onStart(String id) {}

            @Override
            public void onDone(String id) {
                release();
            }

            @Override
            public void onError(String id) {
                release();
            }

            private void release() {
                if (!tts.isSpeaking()) {
                    audio.abandonAudioFocusRequest(focus);
                }
            }
        });
    }

    private void flushWaiting() {
        tts.setLanguage(language);
        while (!waiting.isEmpty()) {
            speak(waiting.poll(), TextToSpeech.QUEUE_ADD);
        }
    }

    private void speak(String text, int mode) {
        audio.requestAudioFocus(focus);
        tts.speak(text, mode, null, UTTERANCE);
    }

    /** Urgent speech, such as a turn or a warning, cuts in ahead of what's being said. */
    void say(String text, boolean urgent) {
        if (!ready) {
            waiting.add(text);
            return;
        }
        speak(text, urgent ? TextToSpeech.QUEUE_FLUSH : TextToSpeech.QUEUE_ADD);
    }

    void setLanguage(String code) {
        language = "en".equals(code) ? Locale.UK : new Locale(code);
        if (ready) {
            tts.setLanguage(language);
        }
    }

    void setRate(float rate) {
        tts.setSpeechRate(rate);
    }

    void close() {
        tts.stop();
        tts.shutdown();
        audio.abandonAudioFocusRequest(focus);
    }
}
