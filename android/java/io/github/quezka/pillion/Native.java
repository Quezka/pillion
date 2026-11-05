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

    static void event(String... fields) {
        send(String.join(String.valueOf(SEPARATOR), fields));
    }
}
