# Tilekeep portable releases

The tray, startup entry and updater use a per-user installation. Run the downloaded binary
with `--install`, then launch the installed copy. On Linux it is under
`$XDG_DATA_HOME/tilekeep/bin/tilekeep` (normally `~/.local/share/tilekeep/bin/tilekeep`);
Windows uses `%LOCALAPPDATA%\Programs\Tilekeep\tilekeep.exe`. No administrator access is required.
Enable login startup in the tray. Disabling it removes only Tilekeep's startup entry.

## Release trust setup (required once)

The intended public repository is `userfirstdigital/tilekeep`. Nothing is published by a local build.
The public default branch is `master`; CI runs on pushes and pull requests targeting it.
`release.pub` contains the public verification key, and release builds check that the repository
variable matches it. Local development archives and desktop logs are not part of the public tree.
Generate a dedicated Minisign signing key offline (`minisign -G`), retain a secure backup, and configure:

- Repository variable `TILEKEEP_UPDATE_PUBKEY`: the public key's base64 line.
- Repository secret `TILEKEEP_UPDATE_SECRET_KEY`: the complete private key file.
- Repository secret `TILEKEEP_UPDATE_KEY_PASSWORD`: its password, if encrypted.

Never commit the private key. Build version must equal the stable `vX.Y.Z` tag. The tag workflow
builds/tests Linux x64 and Windows x64, signs both binaries and `latest.json`, and publishes assets.
The signature authenticates the version/platform metadata as well as each executable.
Keep an offline backup of the encrypted private key and its password in separate secure storage.
Losing the key prevents signing updates trusted by existing installations.

## Silent-update policy

Check 60 seconds after launch and every six hours, or from the tray. Download over HTTPS with
timeouts and size bounds; require the compiled-in trusted key; reject modified metadata/binaries,
older versions and prereleases. A verified update is staged locally and installed on the next
launch, not while the user's layout is active. Verification is repeated immediately before
replacement. The previous executable is retained as `tilekeep.previous` for manual recovery.
On Windows a helper waits for the old Tilekeep process to exit before replacement and relaunch.
This closes no application windows. Settings and snapshots are retained.

Local builds without a signing key show “release signing key not configured”, not “up to date”.
An end-to-end public update is not verified until a signed newer release is actually published.
Verify one update from a signed older installed build on each OS before announcing a release.
Windows binaries are not Authenticode-signed; initial download warnings may still appear.

## Snapshots

Snapshots store application identities, executable paths, titles, monitor layout, stacks,
window rectangles, floating state and gap under the local settings directory. They never
contain a shell command or arbitrary command-line arguments. Loading matches existing windows
first and attempts to open missing applications. Extra windows stay open and outside the
restored tiling layout. Missing or differently sized monitors require fitting the layout to
the available work area; exact pixel placement requires the same screen configuration.
Browser tabs, document contents, minimized/maximized state and unsaved application state are
not restored. Apps that only open one window per process may require opening additional saved
windows manually; these can then claim their saved slots. Select a snapshot separately under
“Snapshot at startup”; enabling login startup does not automatically select a snapshot.
