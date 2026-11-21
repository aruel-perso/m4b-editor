# Testing

## Application and fixtures

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all -- --check
python3 scripts/generate-fixtures.py --check
```

The synthetic corpus includes legacy RAR, ZIP, AES ZIP, ZIP64, multiple audio
streams, and legacy project data. The manifest records provenance, expected
properties and hashes. Run the generator without flags to rebuild fixtures, or
with `--check` to verify the hashes. The AES fixture is regenerated only when
missing or when `--regenerate-encrypted` is passed. Native tests use generated
RAR5 archives to check passwords, integrity, metadata limits and rollback.
Legacy RAR preflight tests cover the solid-member memory limit and reject
packed offsets that the decoder's legacy scan cannot represent.
Runtime tests also cover portable copies, stale saves, chapters, metadata,
formats, preview and process cleanup.

Run the opt-in 15-minute synthetic export workflow with FFmpeg and at least
450 MiB of free temporary space. The Linux CI job runs this workflow. It
checks monotonic progress, validated AAC and chapter timings, full audio
decode, mid-job cancellation and removal of staged files.

```sh
cargo test --locked -p m4b-editor-core --test ffmpeg long_form_export_lifecycle -- --ignored
```

For Windows Rust tests, cross-compile with `--no-run --message-format=json` and
pass the output file to `scripts/verify-windows-tests.py`. It stages the Windows
FFmpeg/ffprobe and all fixtures into an isolated Wine prefix. The
`M4B_EDITOR_TEST_FIXTURES` override supplies a real Windows fixture path; tests
must use that path on Wine. macOS CI also runs the workspace tests with the
native FFmpeg/ffprobe builds.

Fast release-tool tests (no Rust/media compilation or network required):

```sh
python3 scripts/test-source-fetch.py
python3 scripts/test-release.py --scratch /tmp
python3 -m compileall -q scripts
for script in scripts/*.sh; do bash -n "$script"; done
```

The suites cover nullable Cargo license_file, private-note exclusion from source,
MSI identity, media-cache exact checksums and source pins, canonical and legacy
Ed25519 envelopes, wrong keys/platforms/names, payload tampering and a flat exact
asset allowlist. Test signatures use temporary keys, never the release private key.
Transport fixtures prove rejection behavior, not live upstream availability.

CI builds Windows GNU MSI/ZIP and native Mac ARM64 DMG with fresh binaries and
verified media caches. `test-release.py --msi FILE --scratch DIR` inspects MSI
tables, extracts files, checks ZIP CRC and corresponding source hashes.
`verify-windows-bundle.py` runs installed-layout media operations under Wine;
Wine is not a native Windows GUI/upgrade test. `test-macos-release.py` mounts the
DMG, copies the app into a writable directory and exercises the installed CLI
with a system-only PATH. It does not test GUI behavior or live app replacement.

Before publication, exercise the exact signed release payload: install over 0.3,
retain projects/preferences, confirm the canonical Windows shortcut, project
association and both legacy relaunch paths, export/decode AAC/MP3/FLAC offline, uninstall without deleting user
data. On physical Mac ARM64, test Gatekeeper, GUI playback, canonical and legacy
update helpers and replacement/relaunch from both app roots. No live native Mac
upgrade is claimed by the automated tests. Authenticode and Apple Developer ID /
notarization are outside this release scope; ad hoc sealing is not publisher signing.
