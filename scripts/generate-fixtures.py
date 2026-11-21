#!/usr/bin/env python3
"""Generate redistributable synthetic fixtures; --long opts into a 1h WAV.
No downloads. FFmpeg and native ZIP versions are recorded; exact hashes describe the
checked-in corpus, while audio comparisons use explicit duration tolerances.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import struct
import subprocess
import wave
import zipfile
import zlib

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures"

def run(*args):
    subprocess.run(args, check=True, stdout=subprocess.DEVNULL)

def wav(path, frequency, rate=8000, seconds=0.25):
    with wave.open(str(path), "wb") as output:
        output.setparams((1, 2, rate, 0, "NONE", "not compressed"))
        total = round(rate * seconds)
        for start in range(0, total, rate):
            output.writeframesraw(b"".join(struct.pack("<h", round(4000 * math.sin(2 * math.pi * frequency * i / rate))) for i in range(start, min(total,start+rate))))

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--long", action="store_true")
    parser.add_argument("--check", action="store_true", help="Verify the frozen corpus hashes without regenerating encodings")
    parser.add_argument("--regenerate-encrypted", action="store_true", help="Replace the encrypted ZIP fixture (random salt changes the hash)")
    args = parser.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    if args.check:
        manifest = json.loads((OUT / "manifest.json").read_text())
        for entry in manifest["files"]:
            assert hashlib.sha256((OUT / entry["path"]).read_bytes()).hexdigest() == entry["sha256"], entry["path"]
        print(f"Verified {len(manifest['files'])} fixture hashes")
        return
    wav(OUT / "tone440.wav", 440)
    wav(OUT / "tone880.wav", 880, 16000)
    wav(OUT / "silence.wav", 0)
    wav(OUT / "very-short.wav", 440, seconds=0.01)
    # PNG encoder using only the standard library; no image package required.
    def chunk(kind, data):
        return struct.pack(">I",len(data)) + kind + data + struct.pack(">I",zlib.crc32(kind+data))
    (OUT / "cover.png").write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR",struct.pack(">IIBBBBB",2,2,8,2,0,0,0)) + chunk(b"IDAT",zlib.compress((b"\x00"+b"\xff\x00\x00"*2)*2)) + chunk(b"IEND",b""))
    (OUT / "cover.ppm").write_bytes(b"P6\n2 2\n255\n" + b"\xff\x00\x00" * 4)
    (OUT / "corrupt.png").write_bytes(b"not an image")
    common = ["ffmpeg", "-v", "error", "-nostdin", "-y", "-threads", "1"]
    run(*common, "-i", str(OUT / "tone440.wav"), "-c:a", "aac", "-fflags", "+bitexact", "-flags:a", "+bitexact", "-metadata", "title=Écoute synthétique", str(OUT / "tagged.m4b"))
    run(*common, "-i", str(OUT / "tone440.wav"), "-i", str(OUT / "tone880.wav"), "-map", "0:a", "-map", "1:a", "-c:a", "flac", "-fflags", "+bitexact", "-flags:a", "+bitexact", str(OUT / "multistream.mka"))
    run(*common, "-i", str(OUT / "cover.png"), "-frames:v", "1", "-threads", "1", "-flags:v", "+bitexact", str(OUT / "cover.jpg"))
    run(*common, "-i", str(OUT / "tone440.wav"), "-ac", "2", "-ar", "44100", "-c:a", "pcm_s16le", str(OUT / "stereo.wav"))
    run(*common, "-i", str(OUT / "tone440.wav"), "-c:a", "flac", "-fflags", "+bitexact", "-flags:a", "+bitexact", str(OUT / "tone.flac"))
    with zipfile.ZipFile(OUT / "audio.zip", "w") as archive:
        archive.writestr(zipfile.ZipInfo("tone440.wav", (2000,1,1,0,0,0)), (OUT / "tone440.wav").read_bytes())
    with zipfile.ZipFile(OUT / "zip64.zip", "w") as archive:
        with archive.open(zipfile.ZipInfo("tone440.wav", (2000,1,1,0,0,0)), "w", force_zip64=True) as entry:
            entry.write((OUT / "tone440.wav").read_bytes())
    if args.regenerate_encrypted or not (OUT / "encrypted.zip").exists():
        subprocess.run(["cargo", "run", "--locked", "-p", "m4b-editor-core",
                        "--example", "encrypted_fixture"], cwd=ROOT, check=True)
    # A stored RAR 2.x member with CRC-protected headers and a synthetic WAV.
    def rar_header(kind, flags, body):
        data = struct.pack("<BHH", kind, flags, 7 + len(body)) + body
        return struct.pack("<H", zlib.crc32(data) & 65535) + data
    data = (OUT / "tone440.wav").read_bytes()
    name = b"tone440.wav"
    rar = b"Rar!\x1a\x07\x00" + rar_header(0x73, 0, b"\0" * 6)
    rar += rar_header(0x74, 0x8000, struct.pack("<IIBIIBBHI", len(data), len(data), 3, zlib.crc32(data), 0x28210000, 20, 0x30, len(name), 0o100644) + name)
    (OUT / "audio.rar").write_bytes(rar + data + rar_header(0x7b, 0, b""))
    with zipfile.ZipFile(OUT / "hostile-traversal.zip", "w") as archive:
        archive.writestr(zipfile.ZipInfo("../escape.wav", (2000,1,1,0,0,0)), data)
    (OUT / "corrupt.zip").write_bytes(b"PK invalid")
    project = {"version": 1, "metadata": {"title": "Ancien livre"}, "tracks": [{"path": "tone440.wav", "duration_ms": 250, "codec": "pcm_s16le", "sample_rate": 8000, "channels": 1}], "chapters": [{"start_ms": 0, "title": "Début"}, {"start_ms": 100, "title": "Personnalisé"}], "cover": "cover.ppm", "settings": {"language": "fr", "theme": "system", "bitrate_kbps": 96}}
    (OUT / "legacy.json").write_text(json.dumps(project, ensure_ascii=False, indent=2) + "\n")
    project["chapters"][1]["start_ms"] = 9999
    (OUT / "invalid-chapters.m4bproj").write_text(json.dumps(project, ensure_ascii=False, indent=2) + "\n")
    manifest = {"provenance": "Generated mathematical tones and red pixels; CC0-1.0 dedication by project contributors", "password": "fixture (public test data)", "tools": {"ffmpeg": subprocess.check_output(["ffmpeg", "-version"], text=True).splitlines()[0], "zip": "Rust zip 9.8.1 (AES fixture)"}, "files": []}
    for path in sorted(OUT.iterdir()):
        if path.name in ["manifest.json", "long-opt-in.wav"]:
            continue
        properties = {"expected": "reject as corrupt image"} if path.name == "corrupt.png" else {"expected": "reject hostile/corrupt archive" if path.name in ["corrupt.zip", "hostile-traversal.zip", "invalid-chapters.m4bproj"] else "valid synthetic input"}
        if path.suffix in [".wav", ".m4b", ".mka", ".flac"]:
            properties.update(duration_ms=10 if path.name == "very-short.wav" else 250, duration_tolerance_ms=150 if path.suffix == ".m4b" else 2, audio_streams=2 if path.suffix == ".mka" else 1)
        if path.suffix in [".zip", ".rar"]:
            properties.update(container=path.suffix[1:], encrypted=path.name.startswith("encrypted"), member_count=1)
        if path.suffix in [".png", ".jpg", ".ppm"] and not path.name.startswith("corrupt"):
            properties.update(width=2, height=2)
        manifest["files"].append({"path": path.name, "sha256": hashlib.sha256(path.read_bytes()).hexdigest(), **properties})
    original_mp3 = ROOT / "crates/core/tests/fixtures/one-second-tone.mp3"
    manifest["files"].append({"path": "../../crates/core/tests/fixtures/one-second-tone.mp3", "sha256": hashlib.sha256(original_mp3.read_bytes()).hexdigest(), "expected": "existing synthetic MP3 preserved", "duration_ms": 1000, "duration_tolerance_ms": 150})
    manifest["reproduction"] = "Python standard library for WAV/ZIP/RAR/project; FFmpeg bitexact modes for media. Exact hashes pin the corpus; encoder version differences use property tolerances. The AES ZIP uses a random salt and is retained unless --regenerate-encrypted. Public password: fixture."
    (OUT / "manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n")
    if args.long:
        wav(OUT / "long-opt-in.wav", 440, seconds=3600)

if __name__ == "__main__":
    main()
