# v0.2.6: snapshot management

Both tray backends show snapshot names and original creation date/time in local time.
Creation time comes from the existing immutable timestamp ID, so old files need no migration.
Names are optional in schema 1; rename changes only the name field, preserving unknown JSON
fields, the layout, ID, date, and startup selection. Names are trimmed, Unicode-capable,
limited to 80 characters, and cannot contain control characters. Invalid input prompts again.

Edit snapshots exposes Rename and Delete for every saved snapshot, not just the newest 40.
Delete confirmation explains startup effects and recovery. Deletion moves only the selected
file into `snapshots/deleted`, with no overwrite of an existing archive, and clears its startup
selection. A settings-write failure rolls the archive move back. Application windows are not
closed and no layout command is queued by rename or delete.

Dialogs run on a worker thread with a one-dialog-at-a-time guard, without holding controller
locks. Linux uses kdialog with zenity fallback; Windows uses built-in PowerShell/WinForms dialogs
without creating a console. User names are process arguments/environment data, not script source.

Verification includes temporary-directory tests for legacy date labels, Unicode and invalid
names, unchanged geometry/unknown fields, archive behavior, startup clearing/preservation,
invalid IDs, and listing more than 40 snapshots. A Linux controller integration test uses scripted
dialog replies and private settings to exercise rename, both cancellations, confirmed deletion,
pending-load cleanup, and attempts to load a deleted snapshot. It never touches the real desktop
or user snapshots. The Windows dialog is compile-checked, not claimed as interactively tested.

The KWin script is unchanged from v0.2.5. No live input, window dragging, or monitor-power tests
are needed for this metadata/tray change.
