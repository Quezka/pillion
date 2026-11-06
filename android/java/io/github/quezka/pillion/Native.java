package io.github.quezka.pillion;

/** The one door between Java and the Rust engine. */
final class Native {
    static final char SEPARATOR = '\u001f';

    static {
        System.loadLibrary("pillion");
    }

    private Native() {}

    static native void send(String message);

    /** The next command for Android to carry out, or null when there's none. */
    static native String poll();

    /** The pairing code in a camera picture's brightness, or null when there's none to read. */
    static native String decodeQr(byte[] brightness, int width, int height);

    static void event(String... fields) {
        send(String.join(String.valueOf(SEPARATOR), fields));
    }
}
