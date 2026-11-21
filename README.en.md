# M4B Editor

[Français](README.md) · English

Desktop audiobook editor with French/English UI and a CLI sharing the Rust core.
Current development candidate: **0.4.1**.

[Official downloads](https://github.com/aruel-perso/m4b-editor/releases)

Choose the Windows x86_64 MSI or macOS Apple Silicon ARM64 DMG. Retain the
corresponding source archives and checksum sidecars. Windows installs the app
and media tools together; extract the entire portable ZIP. On Mac, copy
**M4B Editor.app** into Applications. FFmpeg/ffprobe are bundled; no PATH changes
are needed. Publisher signing and notarization are outside this release scope.

Create or open a `.m4bproj`, add tracks, edit ordering, chapters, metadata and
cover, then export the book. Legacy `.json` projects remain compatibility formats;
keep their referenced media. A project and an exported audiobook are different
files. The CLI is `m4b-editor-cli`; see `m4b-editor-cli --help`.

AAC/M4B, MP3 and FLAC exports use the bundled encoders. ZIP and RAR imports
work offline. See [Diagnostic logs](docs/diagnostics.md) for troubleshooting.

Updates require Ed25519 signatures and content verification. Migration from 0.3
uses a compatibility repository and deliberate old technical names, explained in
the [release procedure](docs/release.md).

[Development](docs/development.md) · [Testing](docs/testing.md) ·
[Roadmap](docs/roadmap.md) · [GPL-3.0 license](LICENSE)

Distributions must retain dependency licenses and corresponding sources.
No DRM circumvention or advanced waveform editing is included.

Audio import accepts ZIP and RAR. Use the archive password field for supported encrypted archives.
