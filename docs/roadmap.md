# Roadmap

0.4.0 implements stable M4B Editor naming, portable projects, preserved custom
chapters, bounded/cancellable media jobs, selected audio streams, metadata,
AAC/MP3/FLAC exports, encoded previews, isolated recovery and FR/EN UI.
Regression suites exercise the actual core and CLI paths, both UI languages,
stale-save protection, read-only project loading and large MP4 metadata.

Automated acceptance covers Linux, Windows tests under Wine and native Mac ARM64,
bundled media tools and native archive import, MSI structure and mounted DMG installed CLI operation.
The exact hosted MSI is also exercised through 0.3-to-0.4 upgrade, exports and
uninstall while preserving external data. Physical Windows/Mac GUI, audible
playback and live in-app replacement remain separate manual acceptance work.

Publication requires signed manifests with the original trust key. The old-slug
compatibility channel retains historical release assets/source tags and supports
old clients without weakening origin or signature checks.

## Local UI development pass (approved)

The user approved implementation of the UI requirements below for a local-only
Windows MSI. The no-project title is `M4B Editor <version>`. UI code is now
implemented, 71 workspace tests pass, and Windows desktop/CLI release builds
succeed. Packaging and installed acceptance are tracked in the local delivery
report; native Windows appearance remains a user validation gate. No commit,
push or GitHub publication is authorized. The single-EXE portable version is
explicitly deferred. RAR fixes, security audit, end-to-end updater acceptance
and the installer's launch checkbox are excluded from this UI-only pass.

The following requirement descriptions retain their original planning context;
UI implementation status is given above. Further UI redesign beyond this
approved list still requires a recap and explicit approval. Preserve existing
application capabilities and user projects.

User requirement — title bar: display exactly
`M4B Editor <version> - <Projet chargé>` when a project is loaded, substituting
the application version and loaded project name. The no-project title remains
to be specified. Recorded for the UI redesign; not yet implemented.

User requirement — remove the secondary/submenu row containing the text
"M4B Editor", which duplicates the application identity in the title bar.
This targets that row, not the main menu bar. Any other controls currently
hosted there must be inventoried before implementation so no functionality is
silently removed. Recorded for the UI redesign; not yet implemented.

User requirement — add an "Aide" / "Help" menu, localized to the UI language,
with an update entry inside it. Proposed entry labels: "Rechercher des mises à
jour…" / "Check for updates…". Reuse the existing integrated updater without
changing the mandatory startup check or installation-consent policy. Recorded
for the UI redesign; not yet implemented.

User requirement — replace the inline "Récupération disponible" row with a
recovery popup shown when a recoverable autosave is detected. Explain that an
unsaved project can be recovered, show its name and autosave date when available,
and offer localized Restore / Dismiss actions. Preserve recovery safety: no
automatic overwrite of the original project, and dismissing must not delete the
recovery file. Recorded for the next UI development pass; not yet implemented.

User requirement — move "Fermer projet" / "Close project" into the "Projet" /
"Project" menu, removing the separate top-level button. Preserve unsaved-change
confirmation and busy-operation guards.

User requirement — clicking "Réglage" opens a popup containing all
application-wide settings, not project settings. Inventory existing settings
and distinguish global preferences from project-specific settings; retain the
latter in the project UI without silently changing their scope or persistence.
These requirements are recorded for the next UI pass; not yet implemented.

Assistant recommendations, not yet approved: clarify the import/edit/export
workflow, group related controls, improve visual hierarchy, and review mockups
before implementation. Specific pain points and preferred layout remain open.

Previously deferred user scope: security audit and real integrated-update tests.
These remain for the next development pass, subject to its approval recap; they
are not implied complete by installer work.

## Next UI version — approved requirements

The local UI pass implements the requirements below. Private archive import and
Windows installer acceptance are tracked separately from the synthetic tests.

1. Recent projects: show a thumbnail on the left using the cover image in the
   project's metadata when available. Gracefully handle absent, missing or invalid
   covers without preventing project access; bound image decoding and avoid
   probing media repeatedly during UI redraws.
2. Recent projects: move the remove-from-recents button to the right of its row.
   Show a confirmation popup before removing the entry. This only removes the
   recent-list entry, never the project, its media or its cover on disk.
3. Project > Import: accept only an M4B file. Enforce this beyond the picker filter
   and preserve unsaved-change confirmation. This requirement targets the menu
   action; keep the separate audio/archive track import workflow available.
4. Project > Export: open an export popup instead of starting export immediately.
   Let the user choose output audio encoding and bitrate, then explicitly confirm
   or cancel. Offer only encoder/container combinations supported by the bundled
   tools; handle codecs without a bitrate setting appropriately. Preserve output
   path selection and overwrite consent, job cancellation and FR/EN localization.

5. Main project navigation: remove Export from the sequence currently labelled
   Tracks / Metadata / Chapters / Export. Export remains accessible through
   Project > Export and its planned confirmation/settings popup; preserve its
   encoder/bitrate controls there rather than losing them with the page.
6. Rename the Tracks page to Project (FR: Pistes -> Projet; EN: Tracks -> Project).
   This is a page label change, distinct from the existing Project menu. Keep
   audio/archive imports on this page and include the reproduced private RAR
   defect below in this next-version scope. Validate with the private archive in
   `tests-local/` while preserving path and link protections and keeping the
   audiobook private. Project > Import remains M4B-only.

Verification to plan: covers present/absent/broken; recent-removal confirm/cancel
and persistence with disk files unchanged; M4B acceptance and non-M4B rejection;
export popup confirm/cancel, codec/bitrate validation and real output probing;
renamed navigation without an Export page, and actual audio/archive imports
including the private RAR with synthetic security regressions.

## Reproduced private RAR import defect

Initial diagnosis: local CLI import of the private test archive failed with
`Archive links forbidden` before creating a project. It contains 55 MP3 tracks
and one directory in a single-volume, unencrypted RAR5 archive. Known empty
link fields were mistaken for actual links. Further Windows readback revealed
that the shipped 7-Zip lists nested paths with backslashes, requiring safe
platform-aware normalization while preserving exact extraction selectors.

The Windows-local build used the corrected 7-Zip listing and extraction path. Synthetic regressions preserve rejection of real links, reparse fields,
traversal, reserved paths, normalized duplicate aliases and path conflicts. The
archive's integrity passes with official 7-Zip 26.06; the host distro 7-Zip can
list it but cannot decode its newer compression method. Real importer/MSI
acceptance belongs in the local delivery report, not inferred from unit tests.
Never commit or publish the user audiobook.

## Following version — approved Windows development pass

1. **Track ordering.** The Project page uses directional buttons with localized
   tooltips and accessibility labels. Existing chapter-anchor logic still
   controls reorder behavior.
2. **Imported metadata.** For the first batch, fill blank book fields from
   agreeing nonempty tags. Track-based formats prefer album and album artist.
   A single embedded cover, or matching bounded fingerprints across the
   covers present, supplies the cover when the project has none. Conflicting
   values remain blank. Subsequent batches preserve metadata, cover and
   existing chapters. Synthetic tagged ZIP tests exercise these rules.
3. **Import feedback.** Listing shows an indeterminate indicator. Extraction,
   probing and cover comparison show completed file counts. Cancellation
   rolls back the staged import.
4. **Export feedback.** The running export has a separate modal with phase,
   progress, cancellation and a persistent terminal result. The export
   settings dialog retains its nested dropdown behavior.
5. **Windows GUI instance.** The per-user, per-Windows-session OS file lock
   releases with the process. Later launches request activation and can offer
   a project through the existing unsaved-change confirmation. The CLI has
   no GUI lock. Native focus, crash relaunch and file association behavior
   require installed acceptance.
6. **Archive scope.** ZIP and RAR import use pinned Rust decoders in the app.
   Preflight bounds the archives' catalogs before opening them, and staged writes
   enforce declared sizes, batch quotas and integrity checks. Password entry is
   available for ZipCrypto, AES ZIP and supported RAR encryption. Packages
   include the decoders' notices and license texts. Linux tests cover legacy
   RAR import and RAR5 password, corruption, cancellation, path conflicts and
   catalog limits. RAR5 preflight accepts the standard skippable main-header
   flag. Legacy solid RARs have a 64 MiB expanded member limit because the
   decoder buffers whole members. The complete import passes through the real
   Linux CLI and the exact MSI-installed Windows CLI under Wine: 55 tracks and
   a saved project. Keep the archive private. Invalid imported language tags
   are left unset rather than making the project unsaveable; existing user
   metadata is preserved.
7. **Windows upgrade naming.** The MSI installs the app under
   `Programs\M4B Editor`, with the canonical Start menu entry and a per-user
   `.m4bproj` Open With registration. The existing UpgradeCode and
   old-slug signed release channel are retained. The old directory contains
   only two MSI-owned compatibility launchers for older updater helpers.
   Windows Installer removes existing owned components during a major upgrade.
   The candidate moves to 0.4.1 because MSI needs a new three-field version
   to upgrade the previous 0.4.0 candidate and stable product. The upgrade from
   the installed 0.4.0 UI candidate, real installed export/import and uninstall
   preserving external data pass under Wine. Other baselines and native
   Windows remain separate acceptance gates. See [local validation](validation-0.4.1.md).
8. **Export incident.** FFmpeg pipes now drain independently of progress
   delivery. Core tests exercise a stderr flood, a bounded tail, a nonzero
   exit, deadline and cancellation. The 15-minute synthetic export test
   verifies the full output and rollback during cancellation. The reported
   63% Windows incident has no confirmed cause.
9. **Diagnostics.** The persistent log is per-user, rotated and limited to
   typed fields. See [Diagnostic logs](diagnostics.md) for locations, export,
   sharing and interpretation.

## Windows installer launch checkbox

The successful full-UI completion dialog offers a default-off
“Lancer M4B Editor” checkbox. Its launch action is attached to Finish only and
guarded by the full interactive-install condition plus the checkbox property.
The compiled MSI tables are covered locally. Actual clicks, cancellation,
failure, passive/silent setup and uninstall still need installed acceptance.

The updater's existing signature, origin, consent and saved-project checks
remain in use. The GUI rechecks cancellation, playback and dirty state at
download completion. The Windows helper rechecks the staged installer hash
after the running app exits. Signed live replacement and relaunch require
native Windows acceptance with the exact release asset and matching channel.
Portable single-EXE remains deferred.
