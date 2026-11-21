# Development

M4B Editor uses Rust workspace packages `m4b-editor-core`, `m4b-editor-desktop`
and `m4b-editor-cli`. The binaries are `m4b-editor` and `m4b-editor-cli`.
Use the toolchain declared by CI and Python 3.11 or newer. The workspace version
is read from Cargo.toml; release tooling never substitutes a hardcoded version.
Keep the workspace version, application code and packaging at the same reviewed commit.

```sh
cargo fetch --locked
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Use `CARGO_BUILD_JOBS=1` on constrained hosts. One owner builds Rust/media at a
time. Packaging scripts build fresh binaries and fail on a failed build.
Do not package stale executables. See [testing](testing.md) and [release](release.md).

## Application icon

Edit `packaging/icons/app.svg` and run `python3 scripts/generate-icons.py` to
refresh the PNG, ICO and ICNS. The generator requires FFmpeg with the
`librsvg` decoder. Use `--check` for a byte-for-byte comparison with the
committed assets. The reference renderer is FFmpeg 6.1.1 with librsvg 2.58.0.
Regular builds use the committed assets.

## Media backend

`scripts/fetch-media-sources.sh DIRECTORY` fetches pinned FFmpeg 8.0.1,
zlib 1.3.1 and LAME 3.100 sources from upstream release locations. Every download
must match its committed SHA-256. An HTTP-success challenge page is rejected;
never update a digest to make a failing download pass. Verified cached archives
work offline. The source fetch regression suite uses local transport fixtures.

Pass all three archives, in that order, to `build-ffmpeg-windows.sh` or
`build-ffmpeg-macos.sh`. Windows requires the GNU cross toolchain; Mac requires
native ARM64 clang. LAME is built statically without its frontend, then linked
into FFmpeg as libmp3lame. The build enables AAC, MP3 and FLAC encoders and the
matching MP4, MP3 and FLAC muxers. Capability claims require actual packaged-tool
encoding tests. FFmpeg networking and automatic library discovery are disabled.

Installers contain application source, build recipes, media configuration,
Rust license notices and media licenses. Each platform's media-sources ZIP
contains the exact pinned upstream archives. Retain these sidecars with each
release so static-library modifications can be rebuilt with the supplied recipe.

## Native archive import

The core uses `zip 9.8.1` (MIT) with stored/deflate, ZipCrypto and AES support,
and `rar-rs 0.15.3` for single-volume RAR. RAR's original code uses BSD-2-Clause.
The upstream `NOTICE` and `THIRD_PARTY_LICENSES.md` credit `rars` ports under MIT
or Apache-2.0 and the BSD-2-Clause libarchive RAR5 reference. Installers and
license sidecars carry those notices and the `LICENSES` directory.

RAR preflight accepts RAR5/RAR7 containers and legacy RAR 1.5–4.x containers
with clear headers. RAR5 header and member encryption are supported within the
password derivation limit. Multi-volume, self-extracting or header-encrypted
legacy RARs and RAR alternate streams are rejected. RAR5 headers are limited
to 2 MiB each and RAR preflight reads at most 16 MiB of headers. The modern
decoder caps the dictionary at 64 MiB. RAR members whose packed input the
decoder buffers are limited to 32 MiB packed. For legacy archives marked solid
at the archive or member level, the expanded limit is 64 MiB per member because
the decoder materializes solid members in memory. General output limits are
2 GiB per member and 8 GiB per batch. ZIP supports ZIP64 within the same
general entry and expanded-size limits and caps packed member data at
8 GiB plus 32 MiB per archive. Unsupported methods report an error.

Windows packaging can reuse a complete verified historical media cache with
`scripts/package-release.py --media /path/to/cache`. The cache must include
the FFmpeg build with LAME and the corresponding pinned sources. The packagers
use explicit FFmpeg, zlib and LAME file lists for payloads and source archives.
Compatible historical caches work directly.

### Windows MSI presentation wizard

The Windows GNU release packager requires `wixl` and `msibuild` (from
msitools). `package-release.py` builds the per-user MSI with `wixl` and imports
the French wizard tables with `msibuild` before the MSI is checksummed. Use
`--wixl` and `--msibuild` to point to extracted tools when they are outside
`PATH`. The retained WiX source describes the payload. Rebuild through the
packager to include the wizard.

In a full-UI install, **Suivant** opens the confirmation page and shows the
resolved `INSTALLDIR`, normally `%LOCALAPPDATA%\Programs\m4b-Craft`. **Installer**
starts installation and **Terminer** closes the completion screen. Check that
the welcome icon appears, **Retour** returns to welcome, and **Annuler** offers
a choice between continuing and seeing the cancellation screen. The MSI also
provides a failure screen. The installer uses the same execute sequence for
the wizard, `/qn` and `/passive`. Installed-product maintenance and removal
bypass the presentation wizard.

With `wixl` and msitools available, `test-release.py --scratch DIRECTORY`
builds a small MSI and checks its wizard tables against the compiled package
without the UI import. Add `--msi FILE` to inspect the release MSI and sidecars.
For a Windows smoke test, run the MSI interactively, repeat with an older
version installed using a disposable Windows account, and check install and
uninstall with `msiexec /i FILE /qn`, `msiexec /i FILE /passive` and
`msiexec /x FILE /qn`. Verify the app and shortcuts before and after removal.
