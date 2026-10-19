#!/usr/bin/env python3
"""Exercise a task-owned Linux X11 window through OS input and the live API.

Requires an existing isolated Xvfb display, a generated .nvp project, FFmpeg
and a built native binary. It never touches the person's desktop or projects.
"""
from __future__ import annotations

import argparse
import ctypes
import ctypes.util
import hashlib
import json
import os
import re
from pathlib import Path
import socket
import subprocess
import time
import uuid


def wait_for(check, message, timeout=8.0):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = check()
        if value:
            return value
        time.sleep(0.04)
    raise AssertionError(message)


class Input:
    def __init__(self, display):
        self.x = ctypes.CDLL(ctypes.util.find_library("X11"))
        self.t = ctypes.CDLL(ctypes.util.find_library("Xtst"))
        self.x.XOpenDisplay.argtypes = [ctypes.c_char_p]
        self.x.XOpenDisplay.restype = ctypes.c_void_p
        self.x.XStringToKeysym.argtypes = [ctypes.c_char_p]
        self.x.XStringToKeysym.restype = ctypes.c_ulong
        self.x.XKeysymToKeycode.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
        self.x.XKeysymToKeycode.restype = ctypes.c_uint
        self.x.XFlush.argtypes = [ctypes.c_void_p]
        self.x.XCloseDisplay.argtypes = [ctypes.c_void_p]
        self.x.XSetInputFocus.argtypes=[ctypes.c_void_p,ctypes.c_ulong,ctypes.c_int,ctypes.c_ulong]
        self.t.XTestFakeKeyEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]
        self.t.XTestFakeButtonEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]
        self.t.XTestFakeMotionEvent.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_ulong]
        self.display = self.x.XOpenDisplay(display.encode())
        if not self.display:
            raise RuntimeError("Isolated X11 display is unavailable")
        # Xvfb has no window manager to focus a newly mapped window. Locate the
        # observed task-owned Neptune Video surface, then route real OS key events to it.
        tree=subprocess.check_output(["xwininfo","-root","-tree"],env=dict(os.environ,DISPLAY=display),text=True)
        candidates=re.findall(r'(0x[0-9a-f]+) "[^"\n]*Neptune Video[^"\n]*"',tree)
        if not candidates:
            raise RuntimeError(f"Task window was not found in isolated display: {tree}")
        self.x.XSetInputFocus(self.display,int(candidates[0],16),2,0)
        self.x.XFlush(self.display)
        time.sleep(0.15)

    def key(self, name, down):
        sym = self.x.XStringToKeysym(name.encode())
        code = self.x.XKeysymToKeycode(self.display, sym)
        assert code, f"No X11 keycode for {name}"
        self.t.XTestFakeKeyEvent(self.display, code, down, 0)
        self.x.XFlush(self.display)

    def chord(self, key, control=False, shift=False):
        mods = (["Control_L"] if control else []) + (["Shift_L"] if shift else [])
        for mod in mods:
            self.key(mod, True)
        self.key(key, True)
        self.key(key, False)
        for mod in reversed(mods):
            self.key(mod, False)
        time.sleep(0.12)

    def click(self, x, y):
        self.t.XTestFakeMotionEvent(self.display, -1, x, y, 0)
        self.t.XTestFakeButtonEvent(self.display, 1, 1, 0)
        self.t.XTestFakeButtonEvent(self.display, 1, 0, 0)
        self.x.XFlush(self.display)
        time.sleep(0.15)

    def drag(self, start_x, start_y, end_x, end_y):
        self.t.XTestFakeMotionEvent(self.display, -1, start_x, start_y, 0)
        self.t.XTestFakeButtonEvent(self.display, 1, 1, 0)
        self.x.XFlush(self.display)
        time.sleep(0.12)
        for step in range(1, 9):
            x = round(start_x + (end_x - start_x) * step / 8)
            y = round(start_y + (end_y - start_y) * step / 8)
            self.t.XTestFakeMotionEvent(self.display, -1, x, y, 0)
            self.x.XFlush(self.display)
            time.sleep(0.04)
        self.t.XTestFakeButtonEvent(self.display, 1, 0, 0)
        self.x.XFlush(self.display)
        time.sleep(0.15)

    def type(self, value):
        for char in value:
            self.chord("space" if char == " " else char)

    def close(self):
        self.x.XCloseDisplay(self.display)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--project", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--ffmpeg", type=Path, required=True)
    parser.add_argument("--display", default=":92")
    args = parser.parse_args()
    assert args.display != os.environ.get("DISPLAY"), "Use an isolated display, not the person's desktop"
    args.output.mkdir(parents=True, exist_ok=True)
    source = json.loads(args.project.read_text())["project"]
    hashes = {str(asset["path"]): hashlib.sha256(Path(asset["path"]).read_bytes()).hexdigest() for asset in source["assets"].values()}
    project = args.output / "smoke.nvp"
    project.write_bytes(args.project.read_bytes())
    session = args.output / "session.json"
    env = dict(os.environ, DISPLAY=args.display, WAYLAND_DISPLAY="", WINIT_UNIX_BACKEND="x11")
    log = (args.output / "native.log").open("w")
    process = subprocess.Popen([str(args.binary.resolve()), str(project.resolve()), "--settings", str((args.output / "settings.json").resolve()), "--agent-session", str(session.resolve()), "--size", "1440x900"], env=env, stdout=log, stderr=log)
    driver = None
    report = {"scope": "Linux X11 native input + live protocol; excludes audible output and display latency", "checks": []}
    try:
        wait_for(session.exists, "Native window did not publish an agent session")
        descriptor = json.loads(session.read_text())

        def call(operation, **fields):
            request = dict(request_id=str(uuid.uuid4()), operation=operation, **fields)
            with socket.create_connection(tuple(descriptor["address"].rsplit(":", 1)[:1]) + (int(descriptor["address"].rsplit(":", 1)[1]),), timeout=8) as connection:
                connection.sendall(json.dumps(dict(token=descriptor["token"], request=request)).encode() + b"\n")
                with connection.makefile("rb") as reader:
                    result = json.loads(reader.readline())
            return result

        def inspect():
            return call("inspect")

        def edit(command):
            state = inspect()
            result = call("apply", expected_revision=state["revision"], expected_project_id=state["result"]["project"]["id"], expected_context_id=state["result"]["context_id"], command=command)
            assert result["result"]["type"] == "applied", result
            return result

        def clips():
            return sum(len(track["clips"]) for track in inspect()["result"]["project"]["tracks"])

        def screenshot(name):
            path = args.output / f"{name}.png"
            subprocess.run([str(args.ffmpeg.resolve()), "-hide_banner", "-loglevel", "error", "-f", "x11grab", "-video_size", "1440x900", "-i", f"{args.display}.0+0,0", "-frames:v", "1", "-y", str(path)], check=True, env=env, timeout=10)
            return str(path.resolve())

        wait_for(lambda: inspect()["result"]["project"]["id"] == source["id"], "Project did not load")
        assert clips() == 2
        report["checks"].append("Native project open + live inspection")
        video = next(clip for track in source["tracks"] for clip in track["clips"] if clip["content"]["type"] == "media")
        edit(dict(type="set_gain", clip_id=video["id"], gain=0.0))
        driver = Input(args.display)
        driver.click(460, 670)
        report["before_duplicate_screenshot"] = screenshot("before-duplicate")
        driver.chord("d", control=True)
        wait_for(lambda: clips() == 3, "Native duplicate shortcut failed")
        driver.chord("z", control=True)
        wait_for(lambda: clips() == 2, "Native undo shortcut failed")
        driver.chord("z", control=True, shift=True)
        wait_for(lambda: clips() == 3, "Native redo shortcut failed")
        report["checks"].append("OS pointer selection + duplicate/undo/redo shortcuts")
        driver.chord("z", control=True)
        wait_for(lambda: clips() == 2, "Return to original clips failed")
        driver.click(460, 670)
        driver.chord("c", control=True)
        driver.chord("v", control=True)
        wait_for(lambda: clips() == 3, "Native clipboard copy/paste failed")
        report["checks"].append("OS clipboard copy/paste")
        driver.chord("z", control=True)
        wait_for(lambda: clips() == 2, "Undo paste failed")

        def current_video():
            return next(clip for track in inspect()["result"]["project"]["tracks"]
                        for clip in track["clips"] if clip["id"] == video["id"])

        # These coordinates are observed on the fixed 1440x900, six-second
        # fixture. Each held gesture must commit once and reverse completely.
        original_video = current_video()
        revision = inspect()["revision"]
        driver.drag(460, 670, 550, 670)
        wait_for(lambda: current_video()["start"] > original_video["start"], "Native clip move gesture failed")
        assert inspect()["revision"] == revision + 1, "A held move created multiple edits"
        driver.chord("z", control=True)
        wait_for(lambda: current_video() == original_video, "Undo native clip move failed")
        report["checks"].append("Native clip drag commits once and undoes exactly")

        revision = inspect()["revision"]
        driver.drag(862, 670, 795, 670)
        wait_for(lambda: current_video()["duration"] < original_video["duration"], "Native right-edge trim gesture failed")
        assert inspect()["revision"] == revision + 1, "A held trim created multiple edits"
        driver.chord("z", control=True)
        wait_for(lambda: current_video() == original_video, "Undo native trim failed")
        report["checks"].append("Native trim gesture commits once and preserves source on undo")

        driver.click(460, 670)
        revision = inspect()["revision"]
        driver.drag(600, 330, 640, 350)
        wait_for(lambda: current_video()["transform"]["position"] != original_video["transform"]["position"], "Native canvas transform gesture failed")
        assert inspect()["revision"] == revision + 1, "A held canvas drag created multiple edits"
        driver.chord("z", control=True)
        wait_for(lambda: current_video() == original_video, "Undo native transform failed")
        report["checks"].append("Native canvas position gesture commits once and undoes exactly")

        def video_locked():
            return next(track["locked"] for track in inspect()["result"]["project"]["tracks"]
                        if any(clip["id"] == video["id"] for clip in track["clips"]))

        driver.click(141, 670)
        wait_for(video_locked, "Native track lock control failed")
        revision = inspect()["revision"]
        driver.drag(460, 670, 550, 670)
        assert inspect()["revision"] == revision and current_video() == original_video
        driver.click(141, 670)
        wait_for(lambda: not video_locked(), "Native track unlock control failed")
        report["checks"].append("Native track lock blocks held editing gestures")

        driver.click(460, 670)
        driver.chord("Home")
        driver.chord("Right")
        driver.chord("m")
        state = inspect()["result"]["project"]
        assert any(marker["time"] == 33_333 for marker in state["markers"]), state["markers"]
        driver.chord("s")
        wait_for(lambda: clips() == 3, "Native split shortcut failed")
        report["checks"].append("Native rational frame step + marker + split")
        driver.chord("k", control=True)
        driver.type("rename project smoke")
        driver.chord("Return")
        wait_for(lambda: inspect()["result"]["project"]["name"] == "smoke", "Native command panel input failed")
        report["checks"].append("Native command panel dispatch")
        report["agent_screenshot"] = screenshot("agent")
        driver.click(1208, 65)
        driver.click(420, 667)
        report["selected_screenshot"] = screenshot("selected")
        driver.chord("s", control=True)
        wait_for(lambda: not inspect()["result"]["dirty"], "Native save acknowledgement failed")
        assert json.loads(project.read_text())["project"]["name"] == "smoke"
        report["checks"].append("Native save + durable project content")
        before = inspect()
        stale = call("apply", expected_revision=before["revision"] - 1, expected_project_id=source["id"], expected_context_id=before["result"]["context_id"], command=dict(type="rename_project", name="stale"))
        assert stale["result"]["type"] == "error"
        assert inspect()["result"]["project"]["name"] == "smoke"
        report["checks"].append("Live stale AI plan refused")
        driver.chord("comma", control=True)
        report["settings_screenshot"] = screenshot("settings")
        driver.chord("Escape")
        driver.chord("Home")
        driver.chord("space")
        time.sleep(0.7)
        report["playing_screenshot"] = screenshot("playing")
        driver.chord("space")
        report["checks"].append("Native playback transport + settings sheet")
        for path, digest in hashes.items():
            assert hashlib.sha256(Path(path).read_bytes()).hexdigest() == digest
        report["checks"].append("Source media hashes unchanged")
        report["status"] = "passed"
    except BaseException as error:
        report["status"] = "failed"
        report["error"] = str(error)
        raise
    finally:
        (args.output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
        if driver:
            driver.close()
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)
        log.close()
    print(json.dumps(report))


if __name__ == "__main__":
    main()
