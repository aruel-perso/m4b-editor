# M4B Editor release procedure

Branch CI builds and tests packages for the reviewed Cargo version. Publishing
requires separate approval to create the version tag on the canonical repository,
`aruel-perso/m4b-editor`, and access to the protected release environment.

## Build and review

| Target | Installer | Portable |
| --- | --- | --- |
| Windows x86_64 | MSI | ZIP |
| macOS ARM64 | DMG | ZIP |
| Linux x86_64 | DEB | ZIP |

Run the checks in [testing.md](testing.md). Fetch pinned media sources, build
the per-platform media cache and run `package-release.py` on the Windows
GNU cross-build host, `package-macos.py` on native ARM64 macOS, and
`package-linux.py` on Ubuntu 24.04 x86_64. Linux ZIP users need the same
runtime dependencies as the DEB. The media tools are adjacent to the GUI and
CLI in every package. The macOS executables live under `Contents/MacOS`;
the Linux DEB payload lives under `/usr/lib/m4b-editor`.

Each installer and portable ZIP includes the application's source, license
notices, FFmpeg/zlib/LAME archives with their pinned hashes, and generated
media build configuration. The macOS bundle stores these under
`Contents/Resources`. Keep private media and planning notes outside the
source payload.

Collect the six installer and portable files and their SHA-256 sidecars in a
flat CI staging directory. Run `verify-release-assets.py DIRECTORY --version
VERSION` before signing and again with `--signed` afterward. Keep generated
MSI payload/WiX inspection files and legacy compatibility assets outside this
directory. The release attaches the six packages and the technical
`update-v2-PLATFORM.json` manifests. SHA-256 hashes appear in the release
notes and in the CI staging artifacts.

Sign with `sign-update.py --key EXTERNAL.pem --installer FILE --version VERSION
--platform PLATFORM --out update-v2-PLATFORM.json --scratch DIRECTORY`.
The signer requires the original pinned Ed25519 key and binds the installer
filename, version, size, digest, platform and canonical GitHub URL. The
Windows and macOS updaters consume their respective manifests. Linux users
install updates manually and can verify the signed DEB manifest.

## Legacy 0.3 channel: explicit rebrand exceptions

Renaming the repository does not update 0.3 clients: their API redirect host and
release-origin checks reject the redirects. After renaming, the reviewer must
create **aruel-perso/m4b-craft** as a release-only compatibility repository.
Preserve historical tags and original release assets there too, so old download
URLs keep working. Disable Actions in that repository before pushing old tags;
historical workflows must not run as new release jobs.
Serve each approved compatibility release there under its versioned tag with the original trust key.
Keep canonical manifests named `update-v2-PLATFORM.json`; old clients use
`update-PLATFORM.json` at the old slug.

The Windows MSI preserves UpgradeCode `790fce8b-e564-43b1-9701-d427fdd80c6c`.
Starting with 0.4.1, the application installs in `Programs\M4B Editor` and
creates the canonical Start menu shortcut and a per-user Open With registration.
Two small launch shims at the previous MSI paths handle older PowerShell
helpers that relaunch their exact former executable paths. They resolve the
current executable from its MSI-owned HKCU registration. Canonical and
old-named releases use the same MSI bytes. Upgrade and uninstall target the
MSI's components. External projects, media, settings and recovery live
outside this migration.

Build an additional DMG with `package-macos.py --legacy --out LEGACY-BUILD-DIR`.
It contains `m4b-Craft.app/Contents/MacOS/m4b-Craft` for the exact old helper;
its executable is the same new application. Canonical downloads retain
`M4B Editor.app`. The legacy build preserves the old bundle identifier/project
UTI, as does the canonical app, to retain project association identity.
The core updater must recognize both installation roots. Do not simply rename
the canonical DMG: its internal layout is incompatible with the old helper.

Run `prepare-legacy-release.py --canonical CANONICAL-DIR --legacy-dmg FILE
--out EMPTY-DIR --version VERSION`. This copies the Windows payload under its old
filename and the separately built Mac image. It reconstructs the Windows
compatibility sidecars from the canonical ZIP and copies the Mac sidecars
from the legacy build.
Sign each legacy installer with `sign-update.py --legacy` and the same external
key, writing `update-PLATFORM.json`. The signed URLs point strictly to
`https://github.com/aruel-perso/m4b-craft/releases/download/vVERSION/m4b-Craft-VERSION-windows-x86_64.msi`
and the corresponding `m4b-Craft-VERSION-macos-aarch64.dmg` URL.
The compatibility repository must serve the actual files and manifests without
redirection. Include the source/license sidecars on the legacy channel. Publish only
after the reviewer checks exact filenames, hashes, signatures and native tests.
No live native Mac upgrade has been verified by this packaging work.

## Validation scope

Hosted Windows and native ARM64 Mac builds exercise the bundled FFmpeg codecs
and native ZIP/RAR import, including password paths in the workspace tests. Both canonical and compatibility DMGs are mounted and
their installed CLI is exercised. CI installs the Linux DEB and exercises the
bundled CLI and media tools from both the installed and portable layouts. Historical 0.4.0 MSI acceptance included a Wine upgrade from 0.3, actual
AAC/MP3/FLAC exports, portable-copy and data-preserving uninstall. Each release candidate requires its own installed upgrade and uninstall acceptance.
Wine is not native Windows certification. Physical GUI, audible playback and
live in-app replacement remain unverified; ad hoc Mac sealing is not Developer
ID/notarization. Real release-key signing is mandatory at publication.
CI publishing targets only the canonical repository; compatibility publication
is a reviewer-controlled separate operation with its own repository permissions.
