# 0.4.1 Windows development validation

This is a development candidate, not a published or native-Windows-certified
release. Portable single-EXE distribution remains deferred.

## Executed locally

- `cargo fmt --all -- --check`: passed.
- `cargo test --workspace --offline`: 85 tests passed; the long-form test is
  normally ignored and was executed separately successfully.
- `cargo clippy --workspace --all-targets --offline -- -D warnings`: passed.
- Release/source-fetch packaging checks passed, including compilation and
  inspection of a synthetic MSI wizard. macOS structural checks ran on Linux;
  this is not a native macOS build or mounted-DMG acceptance.
- Windows GNU release desktop and CLI build: passed.
- Complete private RAR import through the real Linux CLI: 55 tracks, saved
  project. The original archive's SHA-256 remains unchanged, and the input
  remains ignored/untracked and absent from packaged source.
- Exact locally generated MSI installed under Wine over the previous 0.4.0
  UI candidate. Installed CLI reports 0.4.1; desktop executable bytes match
  the payload. Canonical installation and Start menu names are M4B Editor.
  Old paths retain only the intended compatibility launchers, not old media
  binaries. The new payload contains no external 7-Zip executable or DLL.
- Installed Windows CLI: synthetic audio import, AAC/M4B export, ffprobe
  duration/codec checks and full FFmpeg decode all passed.
- Installed Windows CLI: complete private RAR import and saved project with
  55 tracks passed without an external archive decoder.
- Silent uninstall removed the application and compatibility launchers while
  preserving hashed external projects, imported assets, source media and the
  private original. Temporary private test outputs were removed afterward.

A real-input regression exposed a language tag incompatible with the project's
three-lowercase-letter language contract. Automatic tag import now leaves that
field unset instead of blocking project saving; it does not guess conversions
or overwrite manually entered values. A synthetic tagged-archive regression
failed before the correction and passes afterward.

## Security review and limits

Archive preflight rejects unsafe names, links/redirections, special files,
collisions, split archives, excessive metadata and oversized catalogs before
staged extraction. Decoders enforce memory, expanded-byte and cancellation
limits; imports remain transactional. RAR5 skippable main-header flags are
accepted without accepting split volumes. Legacy solid RAR members are capped
at 64 MiB expanded because that decoder path buffers complete members.

The dependency advisory review found informational unmaintained dependencies
and quick-xml 0.30.0 denial-of-service advisories (RUSTSEC-2026-0194/0195) in
the Linux accessibility dependency chain. `cargo tree --target
x86_64-pc-windows-gnu -i quick-xml@0.30.0` reports no Windows dependency path.
This is not a claim of a universally clean audit; migrating that Linux chain
and following maintenance advisories remain follow-up work.

Updater signatures, exact origins, explicit consent, saved-project and busy
checks are retained. The handoff helper rechecks installer bytes after editor
exit. Those code checks and tests are not proof of a live signed replacement.

## Still requiring acceptance

- Native Windows GUI appearance, activation/focus, crash relaunch and file
  association behavior; the installed CLI tests do not establish these.
- Actual installer finish-checkbox clicks and no-launch behavior for failed,
  cancelled and passive installations (compiled MSI conditions are tested).
- Other historical installer upgrade baselines.
- Native ARM64 macOS DMG and GUI acceptance for the new archive backend.
- End-to-end signed in-app upgrade/install/relaunch using release assets.
- The reported 63% export incident: its original cause is not established.
  Independent FFmpeg pipe draining and synthetic long export/cancellation
  tests improve robustness but do not reproduce that user's Windows incident.

No release tag or public release is authorized by code-push approval.
