"""Opt-in protocol diagnosis; prints only static classifications, no config/PII."""
import json
import os
from pathlib import Path
import select
import signal
import subprocess
import tempfile
import time
import re
import uuid
from collections import deque

root = Path(__file__).resolve().parents[2]
schema = json.loads((root / "artifacts/production-readiness/ai/proposal-schema.json").read_text())
system = json.loads(re.search(r'pub\(crate\) const SYSTEM: &str = ("[^\n]+");', (root / "crates/editor-agent/src/planner.rs").read_text())[1])
uid = lambda: str(uuid.uuid4())
clip = {"id":uid(),"name":"Synthetic title","content":{"type":"text","text":"Synthetic title","font_size":64,"color":[255]*4},"start":2000000,"source_in":0,"duration":3000000,"playback_rate":1,
    "transform":{"position":[0,0],"scale":[1,1],"rotation":0,"opacity":1,"flip_x":False,"flip_y":False,"crop":[0]*4},"linked_id":None,"group_id":None,"gain":1}
project = {"id":uid(),"name":"Synthetic AI acceptance","settings":{"width":1920,"height":1080,"frame_rate":{"numerator":30,"denominator":1},"background":[14,15,18,255],"audio_sample_rate":48000},"assets":{},"folders":{},"tracks":[{"id":uid(),"name":"Text 1","kind":"text","locked":False,"hidden":False,"muted":False,"clips":[clip]}],"markers":[]}
project["tracks"][:0] = [{"id":uid(),"name":f"{kind.title()} 1","kind":kind,"locked":False,"hidden":False,"muted":False,"clips":[]} for kind in ["video","audio"]]
prompt = json.dumps({"request":"Rename this project to 'Native AI verified', move the selected title exactly one second later without changing its duration, and set a 1080 by 1920 portrait canvas. Preserve frame rate, background and audio settings. Propose these edits now.","selected_clip_ids":[clip["id"]],"playhead_microseconds":2000000,"project":project})
features = ["shell_tool", "unified_exec", "code_mode_host", "apps", "multi_agent", "view_image", "image_generation", "plugins", "hooks", "tool_suggest", "sleep_tool", "goals", "browser_use", "computer_use", "skill_search", "in_app_browser", "memories"]
with tempfile.TemporaryDirectory(prefix="neptune-video-ai-probe-") as directory:
    args = ["codex", "app-server"]
    for feature in features:
        args.extend(["-c", f"features.{feature}=false"])
    args.extend(["-c", 'web_search="disabled"'])
    process = subprocess.Popen(args, cwd=directory, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                               stderr=subprocess.DEVNULL, start_new_session=True, text=True)
    def send(method, params=None, ident=None):
        value = {"method":method}
        if params is not None:
            value["params"] = params
        if ident is not None:
            value["id"] = ident
        process.stdin.write(json.dumps(value) + "\n")
        process.stdin.flush()
    try:
        send("initialize", {"clientInfo":{"name":"neptune-video", "title":"Neptune Video", "version":"0.1.0"}}, "init")
        deadline = time.monotonic() + 60
        frames = deque()
        buffer = b""
        while time.monotonic() < deadline:
            if not frames:
                ready, _, _ = select.select([process.stdout], [], [], 1)
                if not ready:
                    continue
                chunk = os.read(process.stdout.fileno(), 65536)
                if not chunk:
                    raise RuntimeError("EOF")
                buffer += chunk
                if len(buffer) > 2097152:
                    raise RuntimeError("budget")
                lines = buffer.split(b"\n")
                buffer = lines.pop()
                frames.extend(line for line in lines if line.strip())
                if not frames:
                    continue
            line = frames.popleft()
            value = json.loads(line)
            ident = value.get("id")
            if ident == "init":
                send("initialized")
                send("account/read", {}, "account")
            elif ident == "account":
                send("config/read", {"cwd":directory, "includeLayers":False}, "config")
            elif ident == "config":
                config = value["result"]["config"]
                servers = {name:{"enabled":False} for name in config.get("mcp_servers", {})}
                send("thread/start", {"cwd":directory,"approvalPolicy":"never", "approvalsReviewer":"user",
                    "sandbox":"read-only","ephemeral":True,
                    "developerInstructions":system,
                    "config":{"mcp_servers":servers,"features":{name:False for name in features},"web_search":"disabled","model_reasoning_effort":"high"}}, "thread")
            elif ident == "thread":
                send("turn/start", {"threadId":value["result"]["thread"]["id"], "input":[{"type":"text","text":prompt}],
                    "approvalPolicy":"never","approvalsReviewer":"user","sandboxPolicy":{"type":"readOnly","networkAccess":False},"outputSchema":schema,"effort":"high"}, "turn")
            if value.get("method") == "error" or value.get("error"):
                error = value.get("error") or value.get("params", {}).get("error", {})
                message = str(error.get("message", "")).lower()
                markers = ["schema", "oneof", "anyof", "additionalproperties", "prefixitems", "items", "not permitted", "unsupported", "authentication", "billing", "minitems", "maxitems", "required"]
                print(json.dumps({"error_classifications":{word:word in message for word in markers},
                    "error_info":error.get("codexErrorInfo") if isinstance(error.get("codexErrorInfo"), str) else None}), flush=True)
                break
            if value.get("method") == "turn/completed":
                print(json.dumps({"turn_status":value["params"]["turn"]["status"]}), flush=True)
                break
        else:
            raise RuntimeError("deadline")
    finally:
        os.killpg(process.pid, signal.SIGKILL)
        process.wait()
