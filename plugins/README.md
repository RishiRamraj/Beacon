# Plugins

Each subdirectory here is one Beacon plugin: a TOML manifest plus a Lua script
that instruments a single game. Beacon selects one automatically by hashing the
ROM — the user never chooses.

- **[alttp/](alttp/)** — The Legend of Zelda: A Link to the Past. The reference
  plugin, also compiled into the binary.

To write your own, read **[../docs/plugins.md](../docs/plugins.md)** — the manifest
format and the complete Lua host API — and use `alttp/` as the worked example. Drop
your plugin directory in here (or in a `plugins/` directory beside the installed
executable); a plugin matching the same ROM as a built-in overrides it.
