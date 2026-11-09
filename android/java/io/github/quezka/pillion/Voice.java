package io.github.quezka.pillion;

import android.content.Context;
import android.media.AudioAttributes;
import android.media.AudioFocusRequest;
import android.media.AudioFormat;
import android.media.AudioManager;
import android.media.AudioTrack;
import android.speech.tts.TextToSpeech;
import android.speech.tts.UtteranceProgressListener;
import android.util.Log;
import com.k2fsa.sherpa.onnx.GeneratedAudio;
import com.k2fsa.sherpa.onnx.OfflineTts;
import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import java.security.MessageDigest;
import java.util.ArrayDeque;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import java.util.Locale;
import java.util.concurrent.LinkedBlockingQueue;
import java.util.concurrent.atomic.AtomicInteger;

/**
 * The voice. The rider picks it: the phone's own speech (with the best voice it has), the natural voice (runs on the phone
 * once downloaded, no internet), or "best", which takes the phone's voice when it's a good one and the natural voice otherwise.
 * If one engine fails on a sentence, the other one says it. Either way Android sends it to the connected earpiece, and music dips while it talks.
 */
final class Voice {
    private static final String UTTERANCE = "ride";
    private static final String HOST = "https://huggingface.co/csukuangfj/vits-piper-";
    /** Language, voice, the model's SHA-256, and its pronunciation dictionary. */
    private static final String[][] PACKS = {
        {"en", "en_GB-alba-medium", "c904d007a8047ab13628b021351b983d0a2627c0d7a81c64a6fe9ad661adb1cf", "en_dict"},
        {"it", "it_IT-paola-medium", "cbe315465f8a0be7db8b40f922fe906cf202996b0e15d388c540e9ac3d4b7fd7", "it_dict"},
        {"ru", "ru_RU-irina-medium", "c88536aaa9d823b20d1225e19759553bff546bc09cbf186d9d40919665be03bc", "ru_dict"},
    };
    /**
     * The speech rules live in one folder for all the languages. They're read once per run, from the first folder they're
     * given, and the list of languages is only looked at then, so all three have to be there from the start.
     */
    private static final String[] SHARED = {
        "phondata", "phonindex", "phontab", "intonations", "lang/gmw/en", "lang/gmw/en-GB-x-rp", "lang/roa/it", "lang/zle/ru",
    };
    private static final long TOTAL_BYTES_ROUGHLY = 64_000_000L;
    private static final int BYTES_PER_SAMPLE = 4;

    private final Context context;
    private final AudioManager audio;
    private final AudioAttributes attributes;
    private final AudioFocusRequest focus;
    private final TextToSpeech tts;
    private final ArrayDeque<String> waiting = new ArrayDeque<>();
    private final LinkedBlockingQueue<Spoken> queue = new LinkedBlockingQueue<>();
    private boolean ready;
    private Locale language = Locale.UK;
    private String code = "en";
    private float rate = 1f;
    /** "auto", "system" or "natural". */
    private volatile String engine = "auto";
    /** How good the phone's chosen voice is, from Android's quality scale; 0 when it has none for this language. */
    private volatile int systemQuality;
    private volatile boolean systemBroken;
    private final Map<String, String> pending = new ConcurrentHashMap<>();
    private final AtomicInteger nextId = new AtomicInteger();
    private volatile boolean downloading;
    private volatile int generation;
    private OfflineTts natural;
    private String loaded;
    private boolean naturalBroken;

    private static final class Spoken {
        final String text;
        final int generation;
        final String language;

        Spoken(String text, int generation, String language) {
            this.text = text;
            this.generation = generation;
            this.language = language;
        }
    }

    Voice(Context context) {
        this.context = context;
        audio = context.getSystemService(AudioManager.class);
        attributes = new AudioAttributes.Builder()
                .setUsage(AudioAttributes.USAGE_ASSISTANCE_NAVIGATION_GUIDANCE)
                .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
                .build();
        focus = new AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN_TRANSIENT_MAY_DUCK)
                .setAudioAttributes(attributes)
                .build();
        tts = new TextToSpeech(context, status -> {
            if (status != TextToSpeech.SUCCESS) {
                systemBroken = true;
                Native.event("voice_missing");
                // Whatever was waiting goes to the natural voice, if there is one.
                while (!waiting.isEmpty()) {
                    sayNaturally(waiting.poll());
                }
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
                pending.remove(id);
                release();
            }

            @Override
            public void onError(String id) {
                String text = pending.remove(id);
                release();
                // The phone's speech couldn't say it: let the natural voice try.
                if (text != null && packReady(code) && !naturalBroken) {
                    sayNaturally(text);
                }
            }

            private void release() {
                if (!tts.isSpeaking()) {
                    audio.abandonAudioFocusRequest(focus);
                }
            }
        });
        Thread worker = new Thread(this::speakNaturally, "natural-voice");
        worker.setDaemon(true);
        worker.start();
        reportPack();
    }

    private void flushWaiting() {
        useLanguage();
        while (!waiting.isEmpty()) {
            speak(waiting.poll(), TextToSpeech.QUEUE_ADD);
        }
    }

    private void speak(String text, int mode) {
        audio.requestAudioFocus(focus);
        String id = UTTERANCE + nextId.getAndIncrement();
        pending.put(id, text);
        tts.speak(text, mode, null, id);
    }

    /** Whether this sentence goes to the natural voice rather than the phone's own speech. */
    private boolean useNatural() {
        boolean naturalOk = !naturalBroken && packReady(code);
        switch (engine) {
            case "natural":
                return naturalOk || systemBroken;
            case "system":
                return systemBroken && naturalOk;
            default:
                // The phone's voice is good enough when Android rates it high (Google's voices are) and it's installed.
                boolean systemGood = !systemBroken && systemQuality >= android.speech.tts.Voice.QUALITY_HIGH;
                return naturalOk && !systemGood;
        }
    }

    /** Urgent speech, such as a turn or a warning, cuts in ahead of what's being said, whichever voice is saying it. */
    void say(String text, boolean urgent) {
        if (urgent) {
            generation++;
            queue.clear();
            if (ready) {
                tts.stop();
            }
            pending.clear();
        }
        if (useNatural()) {
            sayNaturally(text);
            return;
        }
        if (!ready) {
            waiting.add(text);
            return;
        }
        speak(text, urgent ? TextToSpeech.QUEUE_FLUSH : TextToSpeech.QUEUE_ADD);
    }

    private void sayNaturally(String text) {
        // A sentence at a time, so that something urgent never waits long.
        for (String sentence : text.split("(?<=[.!?])\\s+")) {
            queue.add(new Spoken(sentence, generation, code));
        }
    }

    void setLanguage(String next) {
        code = next;
        language = "en".equals(next) ? Locale.UK : new Locale(next);
        if (ready) {
            useLanguage();
        }
        reportPack();
    }

    private void useLanguage() {
        int result = tts.setLanguage(language);
        android.speech.tts.Voice best = bestAndroidVoice();
        if (best != null) {
            tts.setVoice(best);
        }
        boolean missing = result == TextToSpeech.LANG_MISSING_DATA || result == TextToSpeech.LANG_NOT_SUPPORTED;
        systemQuality = missing || best == null ? 0 : best.getQuality();
    }

    /** The clearest voice Android has for this language, preferring one that works without internet. */
    private android.speech.tts.Voice bestAndroidVoice() {
        android.speech.tts.Voice best = tts.getVoice();
        int bestScore = Integer.MIN_VALUE;
        try {
            for (android.speech.tts.Voice voice : tts.getVoices()) {
                Locale locale = voice.getLocale();
                if (!locale.getLanguage().equals(language.getLanguage())
                        || voice.getFeatures().contains(TextToSpeech.Engine.KEY_FEATURE_NOT_INSTALLED)) {
                    continue;
                }
                int score = voice.getQuality() - voice.getLatency() / 4;
                score += locale.getCountry().equals(language.getCountry()) ? 40 : 0;
                score -= voice.isNetworkConnectionRequired() ? 150 : 0;
                if (score > bestScore) {
                    bestScore = score;
                    best = voice;
                }
            }
        } catch (RuntimeException noList) {
            // Some speech engines have no list of voices. Their default is what there is.
        }
        return best;
    }

    void setRate(float next) {
        rate = next;
        tts.setSpeechRate(next);
    }

    void setEngine(String next) {
        engine = next;
        // A voice installed since the last look gets picked up now.
        if (ready) {
            useLanguage();
        }
    }

    // ----- The natural voice -----

    private void speakNaturally() {
        while (true) {
            try {
                Spoken next = queue.take();
                if (next.generation != generation) {
                    continue;
                }
                if (!load(next.language)) {
                    if (ready) {
                        speak(next.text, TextToSpeech.QUEUE_ADD);
                    }
                    continue;
                }
                audio.requestAudioFocus(focus);
                GeneratedAudio sound = natural.generate(next.text, rate);
                if (next.generation == generation && sound.samples.length > 0) {
                    play(sound, next.generation);
                }
                if (queue.isEmpty()) {
                    audio.abandonAudioFocusRequest(focus);
                }
            } catch (InterruptedException stop) {
                return;
            } catch (RuntimeException failed) {
                Log.w(UTTERANCE, "The natural voice failed", failed);
                naturalBroken = true;
                Native.event("voice_pack", "failed", "0");
            }
        }
    }

    private boolean load(String language) {
        if (language.equals(loaded) && natural != null) {
            return true;
        }
        if (natural != null) {
            natural.release();
            natural = null;
            loaded = null;
        }
        try {
            natural = new OfflineTts(packFolder(language).getPath(), speechRules().getPath());
            loaded = language;
            return true;
        } catch (RuntimeException | UnsatisfiedLinkError failed) {
            Log.w(UTTERANCE, "The natural voice wouldn't load", failed);
            naturalBroken = true;
            Native.event("voice_pack", "failed", "0");
            return false;
        }
    }

    private void play(GeneratedAudio sound, int spokenGeneration) throws InterruptedException {
        AudioTrack track = new AudioTrack.Builder()
                .setAudioAttributes(attributes)
                .setAudioFormat(new AudioFormat.Builder()
                        .setEncoding(AudioFormat.ENCODING_PCM_FLOAT)
                        .setSampleRate(sound.sampleRate)
                        .setChannelMask(AudioFormat.CHANNEL_OUT_MONO)
                        .build())
                .setTransferMode(AudioTrack.MODE_STREAM)
                .setBufferSizeInBytes(sound.sampleRate / 2 * BYTES_PER_SAMPLE)
                .build();
        final int chunk = sound.sampleRate / 10;
        try {
            track.play();
            for (int at = 0; at < sound.samples.length && spokenGeneration == generation; at += chunk) {
                int count = Math.min(chunk, sound.samples.length - at);
                track.write(sound.samples, at, count, AudioTrack.WRITE_BLOCKING);
            }
            // The last of it is still playing, and stopping now would clip the end of the word.
            while (spokenGeneration == generation && track.getPlaybackHeadPosition() < sound.samples.length) {
                Thread.sleep(20);
            }
        } finally {
            track.pause();
            track.flush();
            track.release();
        }
    }

    // ----- Getting a voice -----

    private static String[] packOf(String language) {
        for (String[] pack : PACKS) {
            if (pack[0].equals(language)) {
                return pack;
            }
        }
        return null;
    }

    private File packFolder(String language) {
        return new File(new File(context.getFilesDir(), "voices"), language);
    }

    private File speechRules() {
        return new File(new File(context.getFilesDir(), "voices"), "espeak-ng-data");
    }

    private boolean packReady(String language) {
        return new File(packFolder(language), "complete").exists();
    }

    private void reportPack() {
        if (!downloading) {
            Native.event("voice_pack", packReady(code) ? "ready" : "none", "0");
        }
    }

    void downloadPack() {
        if (downloading) {
            return;
        }
        final String language = code;
        final String[] pack = packOf(language);
        if (pack == null) {
            return;
        }
        downloading = true;
        Native.event("voice_pack", "downloading", "0");
        Thread thread = new Thread(() -> {
            try {
                File folder = packFolder(language);
                String host = HOST + pack[1] + "/resolve/main/";
                MessageDigest digest = MessageDigest.getInstance("SHA-256");
                fetch(host + pack[1] + ".onnx", new File(folder, "model.onnx"), digest);
                StringBuilder sum = new StringBuilder();
                for (byte b : digest.digest()) {
                    sum.append(String.format(Locale.ROOT, "%02x", b));
                }
                if (!sum.toString().equals(pack[2])) {
                    throw new IOException("The voice didn't arrive intact");
                }
                fetch(host + "tokens.txt", new File(folder, "tokens.txt"), null);
                for (String name : SHARED) {
                    fetch(host + "espeak-ng-data/" + name, new File(speechRules(), name), null);
                }
                fetch(host + "espeak-ng-data/" + pack[3], new File(speechRules(), pack[3]), null);
                if (!new File(folder, "complete").createNewFile() && !packReady(language)) {
                    throw new IOException("The voice couldn't be saved");
                }
                naturalBroken = false;
                downloading = false;
                Native.event("voice_pack", "ready", "0");
            } catch (Exception failed) {
                Log.w(UTTERANCE, "The voice download failed", failed);
                downloading = false;
                Native.event("voice_pack", "failed", "0");
            }
        }, "voice-download");
        thread.setDaemon(true);
        thread.start();
    }

    private void fetch(String address, File target, MessageDigest digest) throws IOException {
        File parent = target.getParentFile();
        if (parent != null && !parent.isDirectory() && !parent.mkdirs()) {
            throw new IOException("No room for the voice");
        }
        HttpURLConnection connection = (HttpURLConnection) new URL(address).openConnection();
        connection.setConnectTimeout(20_000);
        connection.setReadTimeout(30_000);
        connection.setRequestProperty("User-Agent", "Pillion/0.1 (personal riding app)");
        try (InputStream in = connection.getInputStream(); FileOutputStream out = new FileOutputStream(target)) {
            byte[] buffer = new byte[64 * 1024];
            long total = 0;
            int lastPercent = -1;
            for (int read = in.read(buffer); read >= 0; read = in.read(buffer)) {
                out.write(buffer, 0, read);
                if (digest != null) {
                    digest.update(buffer, 0, read);
                    total += read;
                    int percent = (int) Math.min(99, total * 100 / TOTAL_BYTES_ROUGHLY);
                    if (percent != lastPercent) {
                        lastPercent = percent;
                        Native.event("voice_pack", "downloading", String.valueOf(percent));
                    }
                }
            }
        } finally {
            connection.disconnect();
        }
    }

    void removePack() {
        final String language = code;
        generation++;
        queue.clear();
        Thread thread = new Thread(() -> {
            new File(packFolder(language), "complete").delete();
            deleteInside(packFolder(language));
            reportPack();
        }, "voice-remove");
        thread.setDaemon(true);
        thread.start();
    }

    private static void deleteInside(File folder) {
        File[] inside = folder.listFiles();
        if (inside == null) {
            return;
        }
        for (File file : inside) {
            if (file.isDirectory()) {
                deleteInside(file);
            }
            file.delete();
        }
    }

    void close() {
        generation++;
        queue.clear();
        tts.stop();
        tts.shutdown();
        audio.abandonAudioFocusRequest(focus);
    }
}
