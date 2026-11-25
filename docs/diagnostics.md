# Diagnostic logs

Installed and portable builds use the same per-user `log/` directory:

| Platform | Location |
| --- | --- |
| Windows | `%LOCALAPPDATA%\m4b-editor\log` |
| macOS | `~/Library/Logs/m4b-editor/log` |
| Linux | `$XDG_STATE_HOME/m4b-editor/log`, or `~/.local/state/m4b-editor/log` |

A private temporary directory is used when the usual home or state location is
unavailable. On Unix the editor reuses the per-user directory only if it owns
the directory and its permissions are 0700. On startup the editor attempts to move the
previous `logs/` directory into the new location if that location does not exist.
The logs can be read after a crash. In the editor use **Help → Open logs folder**
or **Help → Export diagnostic log…**. Export writes the retained records,
oldest first, to a new file you choose. Sharing the copy is manual.

There are up to three 256 KiB logs. `diagnostic-0.log` is current. Records are
written as the job runs and include Unix milliseconds in UTC, process and job
numbers, elapsed milliseconds, application version/platform at startup, and
the import or export settings relevant to the job. For export, the encoder
records include the phase, media timestamp, byte counts for each pipe,
time since either pipe produced data, and exit code where available.
The source and track counters describe positions in the current operation.

The persistent logs use a fixed set of fields. FFmpeg stderr is summarized
from its bounded tail as a known category, such as invalid input or access
denied, a muxer allocation failure, or as redacted. An exit status or an
allocation category alone cannot distinguish a per-allocation limit from
operating-system memory exhaustion. Diagnostic files exclude raw stderr, commands, paths,
filenames, title/author tags, passwords, credentials, cover art and audio.
The in-memory journal in the application can contain private details. Export
the diagnostic copy when sharing a problem.

If export appears to stop, record which phase is shown, how long it has been
there, and whether the most recent `media_time_us` or byte counts are still
changing. `idle_ms` measures silence on the pipes, so disk work can sometimes
continue while that number grows. Note the job number and whether Cancel
returned control. The progress bar estimates progress across export stages.
100% means the output was validated and published. The elapsed time measures
wall-clock execution. The total uses observed progress in track preparation,
encoding and audio verification. The total shows **Unknown** at startup, at
phase changes and after ten seconds without forward progress. The final elapsed time remains visible when the job
completes, is cancelled or fails.

Logging is best effort. A forced termination, operating-system crash or native
fault can leave the last record without an outcome. A Rust panic records a
generic event when the log is available. The updater's separate installer
handoff error file is outside this diagnostic export.

## Windows 0.4.1 export investigation

Hermes reviewed the private Windows x86_64 diagnostics for job 1. The
55-track, 160 kbps AAC transcode had 52,607,198 ms of source audio and entered
encoding at 65,337 ms elapsed. Progress was advancing five seconds before
the job failed at 485,809 ms elapsed, progress 0.6389 and media time
19,879,253,333 µs. FFmpeg exited with code -12 and the log classified its
750 stderr bytes as redacted. With staged 48 kHz audio, the reported media
time is close to the 64 MiB threshold for FFmpeg 8.0.1's MOV packet index.
The exit code and synthetic reproduction are consistent with reaching that
ceiling. The retained diagnostic cannot confirm the cause.

## Recovery

Only a valid autosave from an interrupted instance is offered at startup.
Saving or explicitly discarding the active project clears that instance's
snapshot. Dismissal marks a snapshot resolved. Restoring opens a separate
unsaved project and keeps the snapshot eligible until the restored project is
saved or discarded. Recovery never replaces the original project file.

Versions before 0.4 could leave a fixed `m4b-Craft/recovery.autosave.json`
after a normal close. Legacy ownerless snapshots are kept on disk and need
to be opened manually, as they contain no record of an interrupted session.
Use **Save As** to give a manually opened snapshot its own project file.
