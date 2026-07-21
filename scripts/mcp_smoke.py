#!/usr/bin/env python3
"""Smoke test for Beacon's MCP server: an agent driving the emulator.

Spawns `beacon <rom> --mcp` and exercises the control surface end to end —
handshake, stepping, a savestate round-trip (save, mutate, load, compare),
driving the game, running plugin commands, settings, and binding — asserting on
each. Uses an isolated config directory so it never touches your real settings.

Usage:
    scripts/mcp_smoke.py /path/to/game.sfc [path/to/beacon]

The ROM is yours to supply; none ships with Beacon. The binary defaults to
target/release/beacon, then target/debug/beacon. Requires only the standard
library, and a plugin that matches the ROM (so commands have something to say).

Exit status is non-zero if any check fails.
"""
import subprocess, json, os, sys, tempfile, base64, struct, zlib


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
    # Newest wins, so a stale release build does not shadow a fresh debug one.
    return max(found, key=os.path.getmtime)


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    rom = sys.argv[1]
    binary = find_binary(sys.argv[2] if len(sys.argv) > 2 else None)
    if not os.path.exists(rom):
        sys.exit(f"ROM not found: {rom}")

    env = dict(os.environ)
    env["XDG_CONFIG_HOME"] = tempfile.mkdtemp(prefix="beacon-smoke-")

    proc = subprocess.Popen(
        [binary, rom, "--mcp", "--quiet"],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
        text=True, bufsize=1, env=env,
    )

    state = {"id": 0}

    def rpc(method, params=None):
        state["id"] += 1
        msg = {"jsonrpc": "2.0", "id": state["id"], "method": method}
        if params is not None:
            msg["params"] = params
        proc.stdin.write(json.dumps(msg) + "\n")
        proc.stdin.flush()
        return json.loads(proc.stdout.readline())

    def tool(name, args=None):
        res = rpc("tools/call", {"name": name, "arguments": args or {}})["result"]
        block = res["content"][0]
        if "text" in block:
            try:
                val = json.loads(block["text"])
            except Exception:
                val = block["text"]
        else:
            val = block  # e.g. an image content block
        return (not res.get("isError"), val)

    passed = failed = 0

    def check(label, cond, detail=""):
        nonlocal passed, failed
        ok = bool(cond)
        passed += ok
        failed += (not ok)
        print(f"  [{'PASS' if ok else 'FAIL'}] {label}" + (f"  {detail}" if detail else ""))

    try:
        init = rpc("initialize")
        check("initialize", init["result"]["protocolVersion"] == "2024-11-05")
        rpc("notifications/initialized")

        tools = rpc("tools/list")["result"]["tools"]
        check("tools/list", len(tools) >= 20, f"{len(tools)} tools")

        ok, st = tool("get_state")
        check("get_state", ok and st.get("plugin"), st.get("plugin"))

        tool("pause")

        # Savestate round-trip: save, mutate by stepping, load, compare.
        ok, sv = tool("save_state")
        check("save_state", ok and "slot" in sv["spoken"][0])
        ok, a = tool("read_memory", {"address": "0x7E0000", "length": 128})
        tool("step", {"count": 180})
        ok, b = tool("read_memory", {"address": "0x7E0000", "length": 128})
        check("stepping mutates WRAM", a["bytes"] != b["bytes"],
              f"{sum(x != y for x, y in zip(a['bytes'], b['bytes']))}/128 differ")
        tool("load_state")
        ok, c = tool("read_memory", {"address": "0x7E0000", "length": 128})
        check("load reverts WRAM exactly", c["bytes"] == a["bytes"])

        # Drive the game, then let a command answer.
        tool("set_buttons", {"buttons": ["Start"]})
        tool("step", {"count": 30})
        tool("set_buttons", {"buttons": []})
        ok, w = tool("run_command", {"id": "where"})
        check("run_command speaks", ok and w["spoken"], w.get("spoken"))

        # Settings round-trip.
        tool("set_setting", {"key": "speech.rate", "value": "80"})
        ok, g = tool("get_setting", {"key": "speech.rate"})
        check("set/get setting", ok and g["value"] == "80")

        # Binding, and the game-key refusal.
        tool("bind", {"input": "KeyJ", "action": "command:where"})
        ok, gb = tool("get_bindings")
        check("bind reflected", ok and any(x["input"] == "KeyJ" for x in gb["bindings"]))
        ok, _ = tool("bind", {"input": "KeyX", "action": "save_state"})
        check("game-key binding refused", not ok)

        # Unmapped read errors rather than lying.
        ok, _ = tool("read_memory", {"address": "0x008000", "length": 4})
        check("unmapped read errors", not ok)

        # Map mode: a valid PNG whose pixel data decompresses to the right size.
        ok, block = tool("get_map")
        if ok and isinstance(block, dict) and block.get("type") == "image":
            raw = base64.b64decode(block["data"])
            sig = raw[:8] == bytes([137, 80, 78, 71, 13, 10, 26, 10])
            w, h = struct.unpack(">II", raw[16:24])
            idat, off = b"", 8
            while off < len(raw):
                ln = struct.unpack(">I", raw[off:off + 4])[0]
                if raw[off + 4:off + 8] == b"IDAT":
                    idat += raw[off + 8:off + 8 + ln]
                off += 12 + ln
            good = sig and len(zlib.decompress(idat)) == h * (1 + w * 3)
            check("get_map returns a valid PNG", good, f"{w}x{h}")
        else:
            check("get_map returns a valid PNG", False, "no image (plugin has no map?)")

    finally:
        print(f"\n{passed} passed, {failed} failed")
        proc.stdin.close()
        try:
            proc.wait(timeout=5)
        except Exception:
            proc.kill()

    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
