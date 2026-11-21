# M4B Editor release procedure

Merge the code and Cargo version before building. The 0.4.1 candidate uses
a new MSI product version for major upgrades from 0.4.0. The canonical repository is `aruel-perso/m4b-editor`.
No publication or repository rename is performed by the local tooling.

## Build and review

Run the checks in [testing.md](testing.md). Fetch pinned media sources,
build platform media caches, then run `package-release.py` on the Windows GNU
build host and `package-macos.py` on native Mac ARM64. New assets start with
`M4B-Editor-VERSION-PLATFORM`. The app is `M4B Editor.app`, with
`Contents/MacOS/m4b-editor` and adjacent `m4b-editor-cli`, ffmpeg and ffprobe.
Windows installs `m4b-editor.exe` and `m4b-editor-cli.exe` with adjacent media tools.
Media source and license ZIP sidecars, application source, generated media configuration and license
notices are mandatory. Do not distribute private planning/session notes.

Collect only installer/portable/source ZIP assets and SHA-256 sidecars in a flat
directory; run `verify-release-assets.py DIRECTORY --version VERSION` before signing.
Run it again with `--signed` afterward; use `--legacy --signed` for the old channel.
Keep generated MSI payload/WiX inspection files outside that upload directory.
The workflow uploads explicit extensions and validates the exact asset set.

Sign with `sign-update.py --key EXTERNAL.pem --installer FILE --version VERSION
--platform PLATFORM --out update-v2-PLATFORM.json --scratch DIRECTORY`.
The signer derives the public key and rejects anything other than the original
pinned Ed25519 key. Never rotate the key to get a release through. Installer
filenames, version, size, digest, platform and exact canonical GitHub URL are
bound by the signature. SHA-256 sidecars alone do not authenticate updates.

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
filename and the separately built Mac image plus source sidecars and checksums.
Sign each legacy installer with `sign-update.py --legacy` and the same external
key, writing `update-PLATFORM.json`. The signed URLs point strictly to
`https://github.com/aruel-perso/m4b-craft/releases/download/vVERSION/m4b-Craft-VERSION-windows-x86_64.msi`
and the corresponding `m4b-Craft-VERSION-macos-aarch64.dmg` URL.
The compatibility repository must serve the actual files and manifests without
redirection. Copy GPL source/license sidecars with both channels. Publish only
after the reviewer checks exact filenames, hashes, signatures and native tests.
No live native Mac upgrade has been verified by this packaging work.

## Validation scope

Hosted Windows and native ARM64 Mac builds exercise the bundled FFmpeg codecs
and native ZIP/RAR import, including password paths in the workspace tests. Both canonical and compatibility DMGs are mounted and
their installed CLI is exercised. Historical 0.4.0 MSI acceptance included a Wine upgrade from 0.3, actual
AAC/MP3/FLAC exports, portable-copy and data-preserving uninstall. The 0.4.1
candidate requires its own installed upgrade and uninstall acceptance.
Wine is not native Windows certification. Physical GUI, audible playback and
live in-app replacement remain unverified; ad hoc Mac sealing is not Developer
ID/notarization. Real release-key signing is mandatory at publication.
CI publishing targets only the canonical repository; compatibility publication
is a reviewer-controlled separate operation with its own repository permissions.
