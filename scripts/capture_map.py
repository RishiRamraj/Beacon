#!/usr/bin/env python3
"""Drive ALttP from boot to in-game entirely over MCP, and save the map.

A demonstration that an agent can operate Beacon end to end with nothing but the
MCP control surface: it boots the ROM, skips the intro, creates a save file
(navigating the name-entry screen), starts the game, advances the opening until
Link is controllable, then fetches the plugin's map — a picture of what the
plugin believes the state to be — as a PNG.

Because the emulator is deterministic, the same input sequence always reaches
the same state, so this is reproducible rather than flaky.

Usage:
    scripts/capture_map.py /path/to/alttp.sfc [output.png] [path/to/beacon]

Requires only the standard library. The ROM is yours to supply.
"""
import subprocess, json, os, sys, tempfile, base64


def find_binary(explicit):
    if explicit:
        return explicit
    here = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    found = [
        os.path.join(here, rel)
        for rel in ("target/release/beacon", "target/debug/beacon")
        if os.path.exists(os.path.join(here, rel))
    ]
    if not found:
        sys.exit("no beacon binary found; build it or pass a path")
    return max(found, key=os.path.getmtime)


class Beacon:
    def __init__(self, binary, rom):
        env = dict(os.environ)
        env["XDG_CONFIG_HOME"] = tempfile.mkdtemp(prefix="beacon-demo-")
        self.p = subprocess.Popen(
            [binary, rom, "--mcp", "--quiet"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            text=True, bufsize=1, env=env)
        self._id = 0
        self._rpc("initialize")
        self.tool("pause")

    def _rpc(self, method, params=None):
        self._id += 1
        msg = {"jsonrpc": "2.0", "id": self._id, "method": method}
        if params is not None:
            msg["params"] = params
        self.p.stdin.write(json.dumps(msg) + "\n")
        self.p.stdin.flush()
        return json.loads(self.p.stdout.readline())

    def tool(self, name, args=None):
        res = self._rpc("tools/call", {"name": name, "arguments": args or {}})["result"]
        block = res["content"][0]
        if "text" in block:
            try:
                return json.loads(block["text"])
            except Exception:
                return block["text"]
        return block

    def step(self, n):
        self.tool("step", {"count": n})

    def u8(self, addr):
        return self.tool("read_memory", {"address": addr, "length": 1})["bytes"][0]

    def u16(self, addr):
        b = self.tool("read_memory", {"address": addr, "length": 2})["bytes"]
        return b[0] | (b[1] << 8)

    def press(self, buttons, hold=4, gap=8):
        self.tool("set_buttons", {"buttons": buttons})
        self.step(hold)
        self.tool("set_buttons", {"buttons": []})
        self.step(gap)

    def module(self):
        return self.u8("0x7E0010")

    def submodule(self):
        return self.u8("0x7E0011")

    def close(self):
        self.p.stdin.close()
        try:
            self.p.wait(timeout=5)
        except Exception:
            self.p.kill()


def drive_to_ingame(b):
    """Boot -> intro -> file select -> name entry -> game -> free control."""
    b.step(400)  # boot to the intro

    # Skip the intro cinematic to the file-select screen (module 0x01).
    for _ in range(150):
        b.press(["Start"], hold=12, gap=12)
        if b.module() == 0x01:
            break
    else:
        return False, "never reached file select"

    # Select the first (empty) file, which opens name entry (module 0x04).
    b.press(["A"], hold=6, gap=20)
    b.press(["A"], hold=6, gap=20)
    b.press(["Start"], hold=6, gap=20)
    if b.module() != 0x04:
        return False, "never reached name entry"

    # Enter a few letters and confirm, returning to file select with a file.
    for _ in range(8):
        b.press(["A"], hold=4, gap=8)
    b.press(["Start"], hold=6, gap=20)
    if b.module() != 0x01:
        return False, "name entry did not complete"

    # Launch the file and advance the opening until Link is loaded in his house.
    for _ in range(80):
        b.press(["A"], hold=4, gap=4)
        b.press(["Start"], hold=4, gap=4)
        b.step(40)
        if b.module() in (0x07, 0x09) and b.u16("0x7E0022") != 0:
            break
    else:
        return False, "game did not load"

    # Advance the opening cutscene until Link has free control (submodule 0).
    for _ in range(120):
        b.press(["A"], hold=3, gap=3)
        b.step(30)
        if b.module() in (0x07, 0x09) and b.submodule() == 0:
            return True, "free control"
    return False, "never reached free control"


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    rom = sys.argv[1]
    out = sys.argv[2] if len(sys.argv) > 2 else os.path.join(
        os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
        "docs", "images", "alttp-map.png",
    )
    binary = find_binary(sys.argv[3] if len(sys.argv) > 3 else None)
    if not os.path.exists(rom):
        sys.exit(f"ROM not found: {rom}")

    b = Beacon(binary, rom)
    try:
        ok, why = drive_to_ingame(b)
        x, y = b.u16("0x7E0022"), b.u16("0x7E0020")
        print(f"{'reached in-game' if ok else 'stopped'}: {why} "
              f"(module {b.module():#04x}, Link at {x},{y})")
        if not ok:
            sys.exit(1)
        block = b.tool("get_map")
        if not (isinstance(block, dict) and block.get("type") == "image"):
            sys.exit("get_map did not return an image")
        os.makedirs(os.path.dirname(out), exist_ok=True)
        with open(out, "wb") as f:
            f.write(base64.b64decode(block["data"]))
        print(f"saved map to {out}")
    finally:
        b.close()


if __name__ == "__main__":
    main()
