# Private native chooser environment

Date: 2026-10-12. Scope: backend environment preparation for native
chooser acceptance. No editor window was launched, no chooser request was sent
and no UI input was driven by this task. No system packages, user configuration
or user desktop session were modified. Cargo remains withheld for the separate
Windows backend check.

## Locally available prerequisites

The host has `/usr/bin/Xvfb`, `dbus-daemon`, `dbus-run-session`, `gdbus`,
`zenity`, `xwininfo` and `xdpyinfo`. Installed packages are xdg-desktop-portal
1.21.1, xdg-desktop-portal-gtk 1.15.3 and GTK 3.24.52. Backend executables are
`/usr/libexec/xdg-desktop-portal` and
`/usr/libexec/xdg-desktop-portal-gtk`; public descriptors/service files are in
`/usr/share/xdg-desktop-portal/portals/gtk.portal` and
`/usr/share/dbus-1/services/`.

Current `rfd` 0.17.2 defaults to `xdg-portal`/Wayland; Neptune Video does not enable the
optional `gtk3` build feature. The installed rfd source explicitly falls back
to Zenity for Open/Save/folder dialogs when portal calls fail. Therefore both a
real GTK portal and a real Zenity fallback are available without changing Rust
dependencies. A fallback observation must identify its actual route; source
configuration does not establish successful chooser behavior.

## Prepared harness and observed readiness

[run-private.py](../../artifacts/production-readiness/portal/run-private.py)
creates an owned Xvfb display, private DBus with only a task-local service
directory, and explicit GTK/frontend/permission/document portal children. It
keeps HOME intact while isolating XDG config/data/cache/state/runtime and GTK
settings. Its private `portals.conf` selects GTK FileChooser/Settings and leaves
other portal backends disabled. Service environments copy only public session
variables, and the harness never launches Neptune Video.

The [two-second readiness probe](../../artifacts/production-readiness/portal/probe/report.json)
passed: the private frontend exported `org.freedesktop.portal.FileChooser`
version 4 and actual OpenFile/SaveFile/SaveFiles methods. The
[introspection](../../artifacts/production-readiness/portal/probe/introspection.txt)
and [startup log](../../artifacts/production-readiness/portal/probe-run.log)
record that proof. All tracked helper processes exited and the owned document
process group was confirmed empty. The probe's temporary environment was
removed; its session descriptor is historical and must not be reused.

The private backend logged missing access implementations for unused
web-extension/location/camera interfaces, and no AT-SPI bus service. Those did
not prevent FileChooser export; this probe provides no accessibility acceptance.
No file selection, cancellation, parent-window behavior or Neptune Video save/open
behavior is established by service introspection.

## Handoff to the frontend owner

Start a fresh owned session before driving the UI:

```sh
python3 artifacts/production-readiness/portal/run-private.py --output artifacts/native/chooser-session --ttl 1800
```

Keep that command running, read its `session.json` only after `status: ready`,
and use the recorded `environment` plus `unset` names when launching Neptune Video and
the OS-input driver. Readiness is announced on stdout. Both must use this same
DISPLAY and DBUS session; inheriting the user's WAYLAND_DISPLAY would select
the wrong desktop. Use fresh task-owned Neptune Video settings, agent descriptors,
projects and fixture paths. The harness refuses output directories from prior
sessions, so choose a new output path for each attempt.

The frontend owner can merge only those environment values into its own launch
environment:

```python
session = json.loads(Path("artifacts/native/chooser-session/session.json").read_text())
assert session["status"] == "ready"
env = os.environ.copy()
for name in session["unset"]:
    env.pop(name, None)
env.update(session["environment"])
# Launch the native editor with this environment for real OS input/capture.
```

After the editor and chooser have closed, create `stop.request` in that output
directory or send SIGTERM to the exact harness process. It stops only its own
tracked process groups and deletes its private runtime. `report.json` then
records helper exit codes. No process-name cleanup or user-session portal
replacement is needed.

Required actual acceptance: Open and Cancel; Save As and Cancel; Duplicate to
a new filename and own/occupied-path refusal; import files/folders and Cancel;
Unicode/space/percent paths; source/original bytes preserved; dirty/history/
context behavior after each accepted or cancelled route. Capture real dialogs
and the resulting project/file state. These remain pending until the native
flows are exercised; backend service readiness is not substituted for that
evidence.

Primary references: [portal backend configuration](https://flatpak.github.io/xdg-desktop-portal/docs/portals.conf.html)
defines the private XDG config and GTK preference, and the
[FileChooser API](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.FileChooser.html)
defines OpenFile/SaveFile requests and asynchronous responses. Installed rfd's
`src/backend/xdg_desktop_portal.rs:92,249` owns its portal/Zenity routing.
