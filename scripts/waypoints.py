#!/usr/bin/env python3
"""Edit Beacon's authored waypoint chains against a running game.

The chains that guide a blind player through ALttP are hand-mapped: someone plays
to a spot, decides the guide should lead there, and writes down where "there" is.
Doing that by hand means reading coordinates out of a debugger and hand-editing a
Lua table. This is the tool for it — a line shell that holds the file open on one
side and a live Beacon session on the other, so `move 14` means "put waypoint 14
where I am standing right now".

Start Beacon with a control socket, play to the spot, and edit:

    beacon /path/to/alttp.sfc --control &
    scripts/waypoints.py
    > use COURTYARD
    > here
    room 0x71, floor 1, tile 88,495 (attr 0x00)
    > move 13
    moved 13: room 0x71 floor 1 at 88,495
    > reload
    saved plugins/alttp/waypoints.lua; plugin reloaded

Without a session the file commands all still work — only the ones that read the
game (here, add, move, test) need one. Requires nothing but the standard library.

Usage:
    scripts/waypoints.py [--file PATH] [--socket PATH] [command ...]

With a trailing command, runs just that one and exits, which is how you script it.
`--selftest` parses and rewrites the file and checks nothing changed.
"""
import json
import os
import socket
import sys

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEFAULT_FILE = os.path.join(HERE, "plugins", "alttp", "waypoints.lua")


def short(path):
    """A path as the repo names it, or in full if it lies outside the repo."""
    full = os.path.abspath(path)
    return os.path.relpath(full, HERE) if full.startswith(HERE + os.sep) else full

# Where the plugin keeps the live values this tool reads. The same addresses the
# manifest's watches name; duplicated here because the tool talks to the game, not
# to the plugin's own state.
ADDR = {
    "module": 0x7E0010,
    "indoors": 0x7E001B,
    "link_x": 0x7E0022,
    "link_y": 0x7E0020,
    "room": 0x7E00A0,
    "floor": 0x7E00EE,
}

# The order fields are written in: position, then identity, then speech, then
# behaviour, then conditions. `note` is written last, on its own line.
FIELD_ORDER = [
    "tx", "ty", "room", "level", "say", "arrival", "cue", "via", "after_lift",
    "push", "track", "track_dx", "track_dy", "gate", "done",
]
# Fields whose numbers read as hex, because the game's own documentation does.
HEX_FIELDS = {"room", "track"}
TEXT_FIELDS = {"say", "arrival", "note"}
FLAG_FIELDS = {"cue", "via", "after_lift"}


# ── The Lua data subset ─────────────────────────────────────────────────────
# waypoints.lua is data, so it needs only enough Lua to read and write literals:
# tables, strings, numbers, booleans. Numbers keep the text they were written as,
# so re-saving a file never churns 0x71 into 113.

class Num:
    """A number that remembers how it was written."""

    __slots__ = ("value", "text")

    def __init__(self, value, text=None):
        self.value = value
        self.text = text

    def __repr__(self):
        return self.text or repr(self.value)

    def __eq__(self, other):
        return self.value == (other.value if isinstance(other, Num) else other)


class Table:
    """A Lua table: positional items and named fields, both in source order."""

    __slots__ = ("items", "fields")

    def __init__(self, items=None, fields=None):
        self.items = items if items is not None else []
        self.fields = fields if fields is not None else {}

    def get(self, key, default=None):
        return self.fields.get(key, default)


class LuaError(Exception):
    pass


class Parser:
    def __init__(self, text, pos=0):
        self.text, self.pos = text, pos

    def error(self, msg):
        line = self.text.count("\n", 0, self.pos) + 1
        raise LuaError(f"line {line}: {msg}")

    def skip(self):
        t, n = self.text, len(self.text)
        while self.pos < n:
            c = t[self.pos]
            if c in " \t\r\n":
                self.pos += 1
            elif t.startswith("--", self.pos):
                nl = t.find("\n", self.pos)
                self.pos = n if nl < 0 else nl + 1
            else:
                return

    def expect(self, ch):
        self.skip()
        if self.pos >= len(self.text) or self.text[self.pos] != ch:
            self.error(f"expected {ch!r}")
        self.pos += 1

    def peek(self):
        self.skip()
        return self.text[self.pos] if self.pos < len(self.text) else ""

    def value(self):
        self.skip()
        t = self.text
        if self.pos >= len(t):
            self.error("unexpected end of file")
        c = t[self.pos]
        if c == "{":
            return self.table()
        if c in "\"'":
            return self.string()
        if t.startswith("true", self.pos):
            self.pos += 4
            return True
        if t.startswith("false", self.pos):
            self.pos += 5
            return False
        if t.startswith("nil", self.pos):
            self.pos += 3
            return None
        if c == "-" or c.isdigit():
            return self.number()
        self.error(f"unexpected {c!r}")

    def number(self):
        start = self.pos
        t = self.text
        if t[self.pos] == "-":
            self.pos += 1
        if t.startswith("0x", self.pos) or t.startswith("0X", self.pos):
            self.pos += 2
            while self.pos < len(t) and t[self.pos] in "0123456789abcdefABCDEF":
                self.pos += 1
            text = t[start:self.pos]
            return Num(int(text, 16), text)
        while self.pos < len(t) and (t[self.pos].isdigit() or t[self.pos] == "."):
            self.pos += 1
        text = t[start:self.pos]
        return Num(float(text) if "." in text else int(text), text)

    def string(self):
        quote = self.text[self.pos]
        self.pos += 1
        out = []
        t = self.text
        while True:
            if self.pos >= len(t):
                self.error("unterminated string")
            c = t[self.pos]
            if c == "\\":
                nxt = t[self.pos + 1]
                out.append({"n": "\n", "t": "\t"}.get(nxt, nxt))
                self.pos += 2
            elif c == quote:
                self.pos += 1
                return "".join(out)
            else:
                out.append(c)
                self.pos += 1

    def name(self):
        start = self.pos
        t = self.text
        while self.pos < len(t) and (t[self.pos].isalnum() or t[self.pos] == "_"):
            self.pos += 1
        if self.pos == start:
            self.error("expected a name")
        return t[start:self.pos]

    def table(self):
        self.expect("{")
        tbl = Table()
        while True:
            if self.peek() == "}":
                self.pos += 1
                return tbl
            start = self.pos
            key = None
            if self.peek().isalpha() or self.peek() == "_":
                key = self.name()
                self.skip()
                if self.pos < len(self.text) and self.text[self.pos] == "=":
                    self.pos += 1
                else:
                    self.pos = start  # a bare word after all; let value() complain
                    key = None
            if key is None:
                tbl.items.append(self.value())
            else:
                tbl.fields[key] = self.value()
            self.skip()
            if self.peek() == ",":
                self.pos += 1
            elif self.peek() != "}":
                self.error("expected ',' or '}'")


def fmt(value, hex_hint=False):
    """One value, as Lua source."""
    if isinstance(value, Num):
        if value.text:
            return value.text
        return f"0x{value.value:02X}" if hex_hint else str(value.value)
    if isinstance(value, bool):
        return "true" if value else "false"
    if value is None:
        return "nil"
    if isinstance(value, str):
        body = value.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n")
        return f'"{body}"'
    if isinstance(value, Table):
        parts = [fmt(v, hex_hint) for v in value.items]
        parts += [f"{k} = {fmt(v, hex_hint)}" for k, v in value.fields.items()]
        return "{" + ", ".join(parts) + "}"
    raise LuaError(f"cannot write {value!r}")


def fmt_waypoint(wp, indent="  "):
    """A waypoint as one line of fields, with its note (if any) on the next."""
    parts = []
    for key in FIELD_ORDER:
        if key in wp.fields:
            parts.append(f"{key} = {fmt(wp.fields[key], key in HEX_FIELDS)}")
    for key, val in wp.fields.items():  # anything the tool does not know about
        if key not in FIELD_ORDER and key != "note":
            parts.append(f"{key} = {fmt(val)}")
    line = indent + "{ " + ", ".join(parts) + " }"
    note = wp.fields.get("note")
    if note:
        line = indent + "{ " + ", ".join(parts) + ",\n"
        line += indent + "  note = " + fmt(note) + " }"
    return line + ","


def dump(waypoints, prologue):
    """The whole file: the prologue verbatim, everything else regenerated."""
    out = [prologue.rstrip("\n"), "", "WAYPOINTS = {", ""]
    for name, chain in waypoints.fields.items():
        out.append(f"{name} = {{")
        if chain.fields.get("note"):
            out.append("  note = " + fmt(chain.fields["note"]) + ",")
        for wp in chain.items:
            out.append(fmt_waypoint(wp))
        out.append("},")
        out.append("")
    out.append("}")
    return "\n".join(out) + "\n"


def load(path):
    """Splits the file at `WAYPOINTS =` and parses everything after it."""
    text = open(path).read()
    marker = "\nWAYPOINTS ="
    at = text.find(marker)
    if at < 0:
        raise LuaError(f"{path}: no `WAYPOINTS =` assignment found")
    prologue = text[:at]
    p = Parser(text, at + len(marker))
    p.expect("{")
    p.pos -= 1
    return p.table(), prologue


# ── The live session ────────────────────────────────────────────────────────

class Session:
    """A connection to a `beacon --control` session, or nothing at all.

    Absent a socket the tool still edits the file; only the commands that read the
    game go quiet, and they say why rather than failing obscurely.
    """

    def __init__(self, path):
        self.path = path
        self.io = None
        self._id = 0
        self.why = ""
        try:
            s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            # A timeout rather than a blocking wait: a socket that accepts but
            # never answers (an older Beacon serving one client at a time, with
            # something else already attached) would otherwise hang here forever
            # with nothing to explain it.
            s.settimeout(5.0)
            s.connect(path)
            self.io = s.makefile("rw", encoding="utf-8", newline="\n")
            self.rpc("initialize")
        except socket.timeout:
            self.io = None
            self.why = (f"{path} accepted but did not answer — another client may"
                        " be attached to this session")
        except OSError as e:
            self.io = None
            self.why = f"no session at {path} ({e.strerror})"

    def rpc(self, method, params=None):
        self._id += 1
        msg = {"jsonrpc": "2.0", "id": self._id, "method": method}
        if params is not None:
            msg["params"] = params
        self.io.write(json.dumps(msg) + "\n")
        self.io.flush()
        return json.loads(self.io.readline())

    def tool(self, name, args=None):
        """(ok, value) for a tool call, or (False, why) with no session."""
        if not self.io:
            return False, self.why
        try:
            res = self.rpc("tools/call", {"name": name, "arguments": args or {}})["result"]
        except (OSError, ValueError) as e:
            self.io = None
            self.why = f"session closed ({e})"
            return False, self.why
        block = res["content"][0]
        text = block.get("text", "")
        try:
            value = json.loads(text)
        except ValueError:
            value = text
        return not res.get("isError"), value

    def eval(self, code):
        ok, val = self.tool("eval_lua", {"code": code})
        if not ok:
            return None, val if isinstance(val, str) else json.dumps(val)
        return val.get("result"), None

    def position(self):
        """Link's live tile position, or (None, why)."""
        code = (
            "return string.format('%d %d %d %d %d %d',"
            f" mem.u16(0x{ADDR['link_x']:X}) >> 3, mem.u16(0x{ADDR['link_y']:X}) >> 3,"
            f" mem.u16(0x{ADDR['room']:X}), mem.u8(0x{ADDR['floor']:X}),"
            f" mem.u8(0x{ADDR['module']:X}), mem.u8(0x{ADDR['indoors']:X}))"
        )
        result, err = self.eval(code)
        if result is None:
            return None, err
        tx, ty, room, floor, module, indoors = (int(x) for x in result.split())
        return {
            "tx": tx, "ty": ty, "room": room, "level": floor,
            "indoors": bool(indoors), "module": module,
        }, None


# ── The shell ───────────────────────────────────────────────────────────────

class Editor:
    def __init__(self, path, session):
        self.path = path
        self.session = session
        self.waypoints, self.prologue = load(path)
        self.chain_name = next(iter(self.waypoints.fields), None)
        self.dirty = False

    # -- helpers

    @property
    def chain(self):
        return self.waypoints.fields[self.chain_name]

    def wp(self, arg):
        """The waypoint numbered `arg` (1-based, as the file and the guide count)."""
        try:
            n = int(arg)
        except (TypeError, ValueError):
            raise LuaError(f"'{arg}' is not a waypoint number")
        items = self.chain.items
        if not 1 <= n <= len(items):
            raise LuaError(f"{self.chain_name} has waypoints 1 to {len(items)}")
        return n, items[n - 1]

    def summary(self, n, wp):
        f = wp.fields
        where = f"{fmt(f.get('tx'))},{fmt(f.get('ty'))}"
        if "room" in f:
            where = f"room {fmt(f['room'], True)} floor {fmt(f.get('level', Num(0)))} at {where}"
        else:
            where = f"overworld {where}"
        bits = [f"{n}: {where}"]
        for key in ("say", "arrival"):
            if key in f:
                bits.append(f'{key} "{f[key]}"')
        for key in FLAG_FIELDS:
            if f.get(key) is True:
                bits.append(key)
        if "push" in f:
            bits.append(f"push {fmt(f['push'])}")
        if "track" in f:
            bits.append(f"track {fmt(f['track'], True)}")
        for key in ("gate", "done"):
            if key in f:
                bits.append(f"{key} {fmt(f[key])}")
        return "  ".join(bits)

    def live(self):
        pos, err = self.session.position()
        if pos is None:
            raise LuaError(err or "no live session")
        return pos

    def place(self, wp, pos):
        """Points a waypoint at a live position, dropping room/level outdoors."""
        wp.fields["tx"] = Num(pos["tx"])
        wp.fields["ty"] = Num(pos["ty"])
        if pos["module"] == 0x07:
            wp.fields["room"] = Num(pos["room"], f"0x{pos['room']:02X}")
            wp.fields["level"] = Num(pos["level"])
        else:
            wp.fields.pop("room", None)
            wp.fields.pop("level", None)
        self.dirty = True

    def clause(self, text):
        """A gate/done clause, written exactly as it appears in the file."""
        text = text.strip()
        if not text:
            return None
        p = Parser(text)
        value = p.value()
        p.skip()
        if p.pos != len(text):
            raise LuaError("trailing text after the clause")
        if not isinstance(value, Table) or not value.items:
            raise LuaError('a clause looks like {"keys"} or {"tile_outside", 0xF0, 0xFF}')
        return value

    # -- commands

    def cmd_help(self, arg):
        return HELP.strip()

    def cmd_chains(self, arg):
        out = []
        for name, chain in self.waypoints.fields.items():
            mark = "*" if name == self.chain_name else " "
            out.append(f"{mark} {name}: {len(chain.items)} waypoints")
        return "\n".join(out)

    def cmd_use(self, arg):
        name = arg.strip().upper()
        if name not in self.waypoints.fields:
            raise LuaError(f"no chain named {name}; try 'chains'")
        self.chain_name = name
        return f"{name}: {len(self.chain.items)} waypoints"

    def cmd_list(self, arg):
        parts = arg.split()
        first = int(parts[0]) if parts else 1
        count = int(parts[1]) if len(parts) > 1 else len(self.chain.items)
        rows = []
        for n in range(first, min(first + count, len(self.chain.items) + 1)):
            rows.append(self.summary(n, self.chain.items[n - 1]))
        return "\n".join(rows) or "(empty)"

    def cmd_show(self, arg):
        n, wp = self.wp(arg.strip())
        out = [self.summary(n, wp)]
        if wp.fields.get("note"):
            out.append(f"   {wp.fields['note']}")
        return "\n".join(out)

    def cmd_here(self, arg):
        pos = self.live()
        if pos["module"] == 0x07:
            attr, err = self.session.eval(
                f"return string.format('0x%02X', mem.u8(0x7F{2 if pos['level'] == 0 else 3}000"
                f" + ({pos['ty']} & 63) * 64 + ({pos['tx']} & 63)))"
            )
            where = (f"room 0x{pos['room']:02X}, floor {pos['level']},"
                     f" tile {pos['tx']},{pos['ty']}")
            return where + (f" (attr {attr})" if attr else "")
        return f"overworld, tile {pos['tx']},{pos['ty']}"

    def cmd_add(self, arg):
        pos = self.live()
        wp = Table()
        self.place(wp, pos)
        at = len(self.chain.items) + 1
        if arg.strip():
            at, _ = self.wp(arg.strip())
        self.chain.items.insert(at - 1, wp)
        return f"added {self.summary(at, wp)}"

    def cmd_move(self, arg):
        n, wp = self.wp(arg.strip())
        self.place(wp, self.live())
        return f"moved {self.summary(n, wp)}"

    def cmd_del(self, arg):
        n, wp = self.wp(arg.strip())
        del self.chain.items[n - 1]
        self.dirty = True
        return f"deleted {self.summary(n, wp)}"

    def cmd_set(self, arg):
        num, _, rest = arg.strip().partition(" ")
        key, _, value = rest.strip().partition(" ")
        n, wp = self.wp(num)
        if not key:
            raise LuaError("set <n> <field> [value]")
        if not value.strip():
            wp.fields.pop(key, None)
            self.dirty = True
            return f"{n}: cleared {key}"
        if key in TEXT_FIELDS:
            wp.fields[key] = value.strip()
        elif key in ("gate", "done"):
            wp.fields[key] = self.clause(value)
        elif key in FLAG_FIELDS:
            wp.fields[key] = value.strip() not in ("false", "0", "no")
        else:
            wp.fields[key] = Parser(value.strip()).value()
        self.dirty = True
        return f"{n}: {key} = {fmt(wp.fields[key], key in HEX_FIELDS)}"

    def cmd_test(self, arg):
        """Runs a waypoint's compiled gate and done against the live frame."""
        n, wp = self.wp(arg.strip())
        if not self.session.io:
            raise LuaError(self.session.why)
        out = []
        for key in ("gate", "done"):
            if key not in wp.fields:
                out.append(f"{key}: (none)")
                continue
            expr = (f"local wp = WAYPOINTS.{self.chain_name}[{n}]\n"
                    f"if type(wp.{key}) ~= 'function' then return 'not compiled' end\n"
                    f"return tostring(wp.{key}({{ module = mem.u8(0x7E0010) }}, wp))")
            result, err = self.session.eval(expr)
            out.append(f"{key}: {result if result is not None else err}")
        if self.dirty:
            out.append("(unsaved edits — 'reload' first to test what you just changed)")
        return "\n".join(out)

    def cmd_save(self, arg):
        text = dump(self.waypoints, self.prologue)
        load_check = Parser(text, text.index("\nWAYPOINTS =") + len("\nWAYPOINTS ="))
        load_check.expect("{")
        load_check.pos -= 1
        load_check.table()  # refuse to write anything we cannot read back
        open(self.path, "w").write(text)
        self.dirty = False
        return f"saved {short(self.path)}"

    def cmd_reload(self, arg):
        saved = self.cmd_save(arg)
        ok, val = self.session.tool("reload_plugin")
        if not ok:
            return f"{saved}; reload failed: {val}"
        return f"{saved}; plugin reloaded"

    def cmd_quit(self, arg):
        if self.dirty:
            self.dirty = False  # a second quit leaves anyway
            return "unsaved edits — 'save' to keep them, or 'quit' again to discard"
        raise SystemExit(0)

    COMMANDS = {
        "help": cmd_help, "?": cmd_help,
        "chains": cmd_chains, "use": cmd_use,
        "list": cmd_list, "ls": cmd_list, "show": cmd_show,
        "here": cmd_here, "add": cmd_add, "move": cmd_move, "del": cmd_del,
        "set": cmd_set, "test": cmd_test,
        "save": cmd_save, "reload": cmd_reload,
        "quit": cmd_quit, "exit": cmd_quit,
    }

    def run(self, line):
        verb, _, arg = line.strip().partition(" ")
        if not verb:
            return None
        # `note 14 some prose` reads better than `set 14 note some prose`, so the
        # field names double as commands.
        if verb in TEXT_FIELDS or verb in ("gate", "done"):
            num, _, value = arg.strip().partition(" ")
            return self.cmd_set(f"{num} {verb} {value}")
        fn = self.COMMANDS.get(verb)
        if fn is None:
            raise LuaError(f"unknown command '{verb}'; try 'help'")
        return fn(self, arg)


HELP = """
chains                 the chains in the file; * marks the one being edited
use NAME               edit that chain
list [FROM [COUNT]]    the chain's waypoints, numbered as the guide counts them
show N                 one waypoint in full, including its note
here                   where Link is standing right now
add [N]                a waypoint at Link's position, appended or inserted at N
move N                 point waypoint N at Link's position
del N                  delete waypoint N
say N TEXT             what the guide says setting off toward N (empty clears)
arrival N TEXT         what it says on reaching N
note N TEXT            why this waypoint is here, for whoever edits it next
gate N CLAUSE          when N becomes a target, e.g. gate 14 {"keys"}
done N CLAUSE          when N's errand is already carried out
set N FIELD [VALUE]    any other field (level, push, track, via, cue, ...)
test N                 run N's gate and done against the live frame
save                   write the file
reload                 save, then reload the plugin in the running session
quit                   leave
"""


def main():
    args = sys.argv[1:]
    path, sock_path, selftest, rest = DEFAULT_FILE, None, False, []
    while args:
        a = args.pop(0)
        if a == "--file":
            path = args.pop(0)
        elif a == "--socket":
            sock_path = args.pop(0)
        elif a == "--selftest":
            selftest = True
        elif a in ("-h", "--help"):
            sys.exit(__doc__)
        else:
            rest.append(a)

    if selftest:
        return round_trip(path)

    if sock_path is None:
        runtime = os.environ.get("XDG_RUNTIME_DIR", "/tmp")
        sock_path = os.path.join(runtime, "beacon-control.sock")

    session = Session(sock_path)
    editor = Editor(path, session)

    if rest:
        return echo(editor, " ".join(rest))

    print(f"{short(path)}: "
          f"{len(editor.waypoints.fields)} chains, editing {editor.chain_name}")
    print(session.why if not session.io else "connected to the running session")
    print("'help' for commands.")
    while True:
        try:
            line = input("> ")
        except (EOFError, KeyboardInterrupt):
            print()
            return 0
        echo(editor, line)


def echo(editor, line):
    try:
        out = editor.run(line)
    except LuaError as e:
        print(f"error: {e}")
        return 1
    except SystemExit:
        raise
    except Exception as e:  # a bad number, a short command line
        print(f"error: {e}")
        return 1
    if out:
        print(out)
    return 0


def round_trip(path):
    """Parses the file and writes it back, checking nothing moved.

    The editor rewrites this file wholesale every save, so the one thing it must
    never do is quietly reformat, reorder, or drop anything. Run after touching
    the parser or the writer.
    """
    original = open(path).read()
    waypoints, prologue = load(path)
    rewritten = dump(waypoints, prologue)
    if rewritten == original:
        chains = len(waypoints.fields)
        total = sum(len(c.items) for c in waypoints.fields.values())
        print(f"round-trip clean: {chains} chains, {total} waypoints, byte-identical")
        return 0
    import difflib
    diff = difflib.unified_diff(
        original.splitlines(True), rewritten.splitlines(True), "on disk", "rewritten")
    sys.stdout.writelines(diff)
    print("round-trip CHANGED the file")
    return 1


if __name__ == "__main__":
    sys.exit(main())
