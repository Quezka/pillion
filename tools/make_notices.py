"""Writes THIRD_PARTY_NOTICES.txt: the licences of everything that ships in the APK. Run it after changing dependencies.

With --check it changes nothing and fails if the file is out of date, so a release can't ship stale notices.
Only the Android build's crates count, without the ones that only run while building.
"""
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUTPUT = ROOT / "THIRD_PARTY_NOTICES.txt"
TARGET = "aarch64-linux-android"
LICENCE_FILE = re.compile(r"^(licen[cs]e|copying|notice|unlicense|copyright)", re.I)

HEADER = """Third-party notices for Pillion
===============================

Pillion is Copyright (C) 2026 Quezka, under the GNU General Public License version 3 (see LICENSE). That licence covers
Pillion's own work only. Everything below keeps its own licence. Pillion isn't affiliated with or endorsed by Yamaha, and
product names belong to their owners.

Speech engine
-------------
The APK contains libsherpa-onnx-jni.so, taken unchanged from the sherpa-onnx 1.13.8 Android release. It includes:

- sherpa-onnx 1.13.8 (Apache-2.0) https://github.com/k2-fsa/sherpa-onnx/tree/v1.13.8
- espeak-ng (GPL-3.0-or-later, whose text is in LICENSE) at commit ed530aa113046142eb5115cf2fc9157854d0ffe1 of
  https://github.com/csukuangfj/espeak-ng
- piper-phonemize at commit f3ff95afc03640bc1399e113e83361192a2fafb4 of https://github.com/csukuangfj/piper-phonemize
- ONNX Runtime 1.28.2 (MIT) https://github.com/microsoft/onnxruntime/tree/v1.28.2
- The smaller libraries pinned in sherpa-onnx's cmake folder, whose licences follow where they could be read.

Source code: each release since 0.2.1 carries the sherpa-onnx, espeak-ng and piper-phonemize sources as files beside the
APK. Pillion's own source is in https://github.com/Quezka/pillion, under the tag of the release.

android/java/com/k2fsa/sherpa/onnx/ holds Pillion's own classes, with the names and fields sherpa-onnx's library expects.

Voices
------
Voices aren't in the APK. Pillion downloads one only when you ask, from https://huggingface.co/csukuangfj, and each keeps
its own terms: en_GB-alba-medium is CC BY 4.0; for it_IT-paola-medium and ru_RU-irina-medium see their model cards.

Map data
--------
Routes, places and map tiles come from online services and are (C) OpenStreetMap contributors, under the ODbL.
"""

# Files in this repository, in the order they appear.
LOCAL = [
    ("Manrope font", "assets/Manrope-OFL.txt"),
    ("Material Symbols", "assets/MaterialSymbols-LICENSE.txt"),
    ("sherpa-onnx", "licenses/sherpa-onnx.txt"),
    ("ONNX Runtime", "licenses/onnxruntime.txt"),
    ("ONNX Runtime's third-party notices", "licenses/onnxruntime-third-party.txt"),
    ("piper-phonemize", "licenses/piper-phonemize.txt"),
    ("kaldi-native-fbank", "licenses/kaldi-native-fbank.txt"),
    ("kaldi-decoder", "licenses/kaldi-decoder.txt"),
    ("OpenFst", "licenses/openfst.txt"),
    ("nlohmann/json", "licenses/nlohmann-json.txt"),
    ("Eigen (MPL-2.0)", "licenses/eigen-mpl2.txt"),
]


def run(*args):
    return subprocess.run(args, cwd=ROOT, check=True, capture_output=True, text=True).stdout


def shipped_crates():
    """The (name, version) of every crate linked into the Android library, except Pillion."""
    tree = run("cargo", "tree", "--locked", "--target", TARGET, "-e", "normal,no-proc-macro",
               "--prefix", "none", "--no-dedupe", "-f", "{p}")
    found = set()
    for line in tree.splitlines():
        parts = line.split()
        if len(parts) >= 2 and parts[0] != "pillion":
            found.add((parts[0], parts[1].lstrip("v")))
    return found


def read(path):
    return path.read_text(encoding="utf-8", errors="replace").replace("\r\n", "\n").strip()


def section(title, body):
    return f"\n\n{'=' * 100}\n{title}\n{'=' * 100}\n\n{body}"


def build():
    out = [HEADER.strip()]
    for title, path in LOCAL:
        out.append(section(title, read(ROOT / path)))

    metadata = json.loads(run("cargo", "metadata", "--format-version", "1", "--locked"))
    packages = {(p["name"], p["version"]): p for p in metadata["packages"]}
    # Crates often share the same licence text, so each text appears once with every crate that uses it.
    texts = {}
    for key in sorted(shipped_crates()):
        package = packages[key]
        label = f"{key[0]} {key[1]} ({package.get('license') or 'see its files'})"
        folder = Path(package["manifest_path"]).parent
        files = sorted(f for f in folder.iterdir() if f.is_file() and LICENCE_FILE.match(f.name))
        if not files:
            source = package.get("repository") or f"https://crates.io/crates/{key[0]}"
            texts.setdefault(f"Licensed as stated; this crate's package has no licence file. See {source}", []).append(label)
        for file in files:
            texts.setdefault(read(file), []).append(label)
    for text, labels in sorted(texts.items(), key=lambda item: item[1][0]):
        out.append(section("Rust crates:\n" + "\n".join(dict.fromkeys(labels)), text))
    return "\n".join(out) + "\n"


def main():
    wanted = build()
    if "--check" in sys.argv:
        if not OUTPUT.exists() or OUTPUT.read_text(encoding="utf-8") != wanted:
            sys.exit("THIRD_PARTY_NOTICES.txt is out of date: run tools/make_notices.py")
        return
    OUTPUT.write_text(wanted, encoding="utf-8")


if __name__ == "__main__":
    main()
