package com.k2fsa.sherpa.onnx;

/**
 * The speech engine's native library reads these classes by name, field by field, so the names here have to stay as they
 * are. They match sherpa-onnx 1.13.8. Only the Piper voice settings are used; the rest must merely exist and not be null.
 */
public final class OfflineTts {
    static {
        System.loadLibrary("sherpa-onnx-jni");
    }

    private long ptr;

    /**
     * @param folder where the voice's model.onnx and tokens.txt are
     * @param speechRules the espeak-ng-data folder
     */
    public OfflineTts(String folder, String speechRules) {
        OfflineTtsVitsModelConfig vits = new OfflineTtsVitsModelConfig();
        vits.model = folder + "/model.onnx";
        vits.tokens = folder + "/tokens.txt";
        vits.dataDir = speechRules;
        OfflineTtsModelConfig model = new OfflineTtsModelConfig();
        model.vits = vits;
        OfflineTtsConfig config = new OfflineTtsConfig();
        config.model = model;
        ptr = newFromFile(config);
    }

    public GeneratedAudio generate(String text, float speed) {
        return generateImpl(ptr, text, 0, speed);
    }

    public void release() {
        if (ptr != 0) {
            delete(ptr);
            ptr = 0;
        }
    }

    private native long newFromFile(OfflineTtsConfig config);

    private native void delete(long ptr);

    private native GeneratedAudio generateImpl(long ptr, String text, int speaker, float speed);
}

final class OfflineTtsConfig {
    OfflineTtsModelConfig model = new OfflineTtsModelConfig();
    String ruleFsts = "";
    String ruleFars = "";
    int maxNumSentences = 1;
    float silenceScale = 0.2f;
}

final class OfflineTtsModelConfig {
    OfflineTtsVitsModelConfig vits = new OfflineTtsVitsModelConfig();
    OfflineTtsMatchaModelConfig matcha = new OfflineTtsMatchaModelConfig();
    OfflineTtsKokoroModelConfig kokoro = new OfflineTtsKokoroModelConfig();
    OfflineTtsZipVoiceModelConfig zipvoice = new OfflineTtsZipVoiceModelConfig();
    OfflineTtsKittenModelConfig kitten = new OfflineTtsKittenModelConfig();
    OfflineTtsPocketModelConfig pocket = new OfflineTtsPocketModelConfig();
    OfflineTtsSupertonicModelConfig supertonic = new OfflineTtsSupertonicModelConfig();
    int numThreads = 2;
    boolean debug = false;
    String provider = "cpu";
}

final class OfflineTtsVitsModelConfig {
    String model = "";
    String lexicon = "";
    String tokens = "";
    String dataDir = "";
    String dictDir = "";
    float noiseScale = 0.667f;
    float noiseScaleW = 0.8f;
    float lengthScale = 1.0f;
}

final class OfflineTtsMatchaModelConfig {
    String acousticModel = "";
    String vocoder = "";
    String lexicon = "";
    String tokens = "";
    String dataDir = "";
    String dictDir = "";
    float noiseScale = 1.0f;
    float lengthScale = 1.0f;
}

final class OfflineTtsKokoroModelConfig {
    String model = "";
    String voices = "";
    String tokens = "";
    String dataDir = "";
    String lexicon = "";
    String lang = "";
    String dictDir = "";
    float lengthScale = 1.0f;
}

final class OfflineTtsZipVoiceModelConfig {
    String tokens = "";
    String encoder = "";
    String decoder = "";
    String vocoder = "";
    String dataDir = "";
    String lexicon = "";
    float featScale = 0.1f;
    float tShift = 0.5f;
    float targetRms = 0.1f;
    float guidanceScale = 1.0f;
}

final class OfflineTtsKittenModelConfig {
    String model = "";
    String voices = "";
    String tokens = "";
    String dataDir = "";
    float lengthScale = 1.0f;
    String lexicon = "";
}

final class OfflineTtsPocketModelConfig {
    String lmFlow = "";
    String lmMain = "";
    String encoder = "";
    String decoder = "";
    String textConditioner = "";
    String vocabJson = "";
    String tokenScoresJson = "";
    int voiceEmbeddingCacheCapacity = 50;
}

final class OfflineTtsSupertonicModelConfig {
    String durationPredictor = "";
    String textEncoder = "";
    String vectorEstimator = "";
    String vocoder = "";
    String ttsJson = "";
    String unicodeIndexer = "";
    String voiceStyle = "";
}
