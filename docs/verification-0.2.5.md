# v0.2.5: connected shared-edge resizing

## Behavior

An edge drag follows its continuous geometric connection, not the layout tree's ancestry.
Windows aligned immediately above/below a vertical edge, or beside a horizontal edge, stay
aligned. Windows on the opposite side adjust too, on both growth and shrink. Both components
participate in a corner resize. Each connected window contributes its minimum-size constraint.

Vacant stretches break the connection. Matching coordinates elsewhere, sharing a tree ancestor,
or diagonally touching corners alone are not enough. Unshared edges can still expose or occupy
empty space. Existing six-logical-pixel snaps and panel boundaries remain in effect.

Connected neighbors follow during the native drag, while the saved tree remains unchanged
until release. Escape, returning to the starting edge, or pausing restores provisional neighbor
geometry. A monitor transition discards provisional state and retains the saved layout.

## Additional release-time bug found by live testing

Hiding the snap guide synchronously emits KWin `windowRemoved`. Previously that callback could
queue the old layout between ending the interactive guard and committing the resize. Its late
Wayland configure could then undo the completed drag. Untracked overlay removal no longer
causes a retile, and the interactive guard now covers guide dismissal and tree commitment.
Native Escape also exposed asynchronous cancellation: the restored client frame can arrive
after `interactiveMoveResizeFinished`. Resize release now waits 150 ms before reading that
final frame and committing the tree. Live provisional geometry stays visible during this
settling period. The [KWin API](https://develop.kde.org/docs/plasma/kwin/api/) does not provide
a cancellation argument on that signal; unusually delayed clients remain a timing limitation.

## Compositor crash and teardown hardening

During the development test sequence, the real desktop's KWin crashed with SIGSEGV while
Qt was invalidating a QML context and deleting a script component. T3 processes failed
immediately afterward. The available stack does not identify the exact JavaScript callback;
the core itself was inaccessible. This was a real desktop-session disruption, not a passing
test or an isolated application failure.

Destruction-time JavaScript performed geometry restoration, hid surfaces, and disconnected
bare JavaScript signal closures. That is unsafe work to do while the context is being
invalidated. The backend now owns per-window callbacks through child `Connections` objects,
has no `Component.onDestruction` JavaScript, and explicitly quiesces event sources and timers
while the context is alive. The host requests that orderly shutdown before unloading.

Further lifecycle verification uses a separate headless KWin, private D-Bus session, private
runtime/configuration directories, and Mesa software rendering. It does not share the user's
Wayland socket or inject system-wide input. Native-desktop resize/display harnesses now require
an additional `--allow-live-compositor-risk` acknowledgement; use them only on a disposable
session. The mitigation is tested, but the original crash has not been conclusively reproduced
and attributed to one callback.

## Verification

- 101 Rust library tests, 2 CLI tests, and 84 compositor-function tests passed.
- New/updated regressions cover all four shared-edge directions in both movement directions,
  corner connections, separate aligned groups, diagonal-only contact, minimum sizes on either
  side, live provisional geometry, Escape/pause rollback, and synchronous overlay-removal
  reentrancy. All monitor-disconnect and prior drop/placement regressions remain enabled.
- Linux and Windows-target Clippy and formatting checks passed.
- Before the crash, on Plasma 6.7.4 Wayland at 125% scale, the native resize harness managed three owned test
  windows: the resized window, its aligned neighbor below, and a window across their edge.
  Live following, growth, shrink, snap capture/escape, release stability, and native Escape
  rollback passed. The subsequent display-recovery test sequence was interrupted by the
  compositor crash; it must not be counted as successful final-revision live verification.
- With the teardown changes, 40 isolated load/resize/overlay/unload cycles passed with no
  QML errors and surviving compositor/client processes. Alternating cycles exercised orderly
  quiescence and direct unload while resize completion was pending. These use real QML/KWin
  clients but scripted resize callbacks, not physical input or a hardware-display disconnect.

Preferred compositor verification, without touching the active desktop:

```sh
node tests/plasma-isolated.cjs
```

The isolated harness retains diagnostic files in its printed temporary directory. Desktop
snapshots and application titles remain local. This change applies to Plasma Wayland; Windows/X11 retain
their existing shared-divider behavior. The monitor-recovery limitations documented in
[v0.2.4](verification-0.2.4.md) still apply.
