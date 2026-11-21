# Diagnostic logs

On Windows, open `%LOCALAPPDATA%\m4b-editor\logs` in File Explorer, including
after a crash or when the editor cannot start. In the editor, use **Help →
Open logs folder** or **Help → Export diagnostic log…**. The export dialog
writes one copy of the retained records, oldest first, to the file you choose.
Choose a new filename if the destination already exists. The editor sends no
diagnostic data. Share the copy manually if you want someone to investigate.

There are up to three 256 KiB logs. `diagnostic-0.log` is current. Records are
written as the job runs and include Unix milliseconds in UTC, process and job
numbers, elapsed milliseconds, application version/platform at startup, and
the import or export settings relevant to the job. For export, the encoder
records include the phase, media timestamp, byte counts for each pipe,
time since either pipe produced data, and exit code where available.
The source and track counters describe positions in the current operation.

The persistent logs use a fixed set of fields. FFmpeg stderr is summarized
from its bounded tail as a known category, such as invalid input or access
denied, or as redacted. Diagnostic files exclude raw stderr, commands, paths,
filenames, title/author tags, passwords, credentials, cover art and audio.
The in-memory journal in the application can contain private details. Export
the diagnostic copy when sharing a problem.

If export appears to stop, record which phase is shown, how long it has been
there, and whether the most recent `media_time_us` or byte counts are still
changing. `idle_ms` measures silence on the pipes, so disk work can sometimes
continue while that number grows. Note the job number and whether Cancel
returned control. The progress bar estimates progress across export stages.
100% means the output was validated and published.

Logging is best effort. A forced termination, operating-system crash or native
fault can leave the last record without an outcome. A Rust panic records a
generic event when the log is available. The updater's separate installer
handoff error file is outside this diagnostic export.
