package com.k2fsa.sherpa.onnx;

/** What the speech engine hands back: its native code builds this. */
public final class GeneratedAudio {
    public final float[] samples;
    public final int sampleRate;

    public GeneratedAudio(float[] samples, int sampleRate) {
        this.samples = samples;
        this.sampleRate = sampleRate;
    }
}
