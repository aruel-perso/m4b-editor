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
450 MiB of free temporary space. The Windows build job runs this workflow on its Linux host. It
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

CI builds Windows x86_64 MSI/ZIP, native Mac ARM64 DMG/ZIP and Linux x86_64
DEB/ZIP with fresh binaries and
verified media caches. `test-release.py --msi FILE --scratch DIR` inspects MSI
tables, extracts files, checks ZIP CRC and corresponding source hashes.
`verify-windows-bundle.py` runs installed-layout media operations under Wine;
Wine is not a native Windows GUI/upgrade test. `test-macos-release.py` mounts the
DMG, copies the app into a writable directory and exercises the installed CLI
with a system-only PATH. The portable Mac ZIP is unpacked and its bundle seal
is verified. The Linux job installs the DEB and runs `test-linux-release.py`
against both it and the extracted ZIP. Physical desktop interaction and live
app replacement remain manual checks.

Version 0.4.2 adds headless FR/EN welcome layout tests at two window sizes,
recovery lifecycle tests, running and terminal timer rendering, and persistent
log privacy, rotation and export coverage. The startup log test exercises
production path resolution with injected LOCALAPPDATA, HOME and XDG_STATE_HOME
values and verifies log creation and rotation. On each installed and portable build, verify
that **Help → Open logs folder** opens the documented directory and that
**Export diagnostic log…** creates a readable copy after a restart.

The AAC muxer regression uses seven minutes of generated silence and a reduced
per-allocation ceiling, so no long PCM staging file is needed. It demonstrates
FFmpeg’s MOV sample index allocation failure and verifies that a duration-based
ceiling permits the same encode to complete. A separate export regression
remuxes 62,000 seconds of synthetic AAC, decodes the entire output, and
asserts that the verified M4B was published. The Windows Rust test job runs
this regression under Wine with the bundled FFmpeg and ffprobe.

Before publishing 0.4.2, install the exact candidate MSI over 0.4.1 and
record the MSI hash and results. Check retained projects, media, preferences
and logs, the Windows shortcut, project association, both legacy relaunch
paths, and offline AAC/MP3/FLAC export and decode. Uninstall and confirm
that external data is still present. MSI inspection and the portable ZIP
smoke test do not satisfy this acceptance gate. On physical Mac ARM64, test Gatekeeper, GUI playback, canonical and legacy
update helpers and replacement/relaunch from both app roots. No live native Mac
upgrade is claimed by the automated tests. Authenticode and Apple Developer ID /
notarization are outside this release scope; ad hoc sealing is not publisher signing.
