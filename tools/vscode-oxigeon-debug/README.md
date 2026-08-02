# Oxigeon Lua Debug (VS Code)

A registration shim, and nothing more.

VS Code's debugging is entirely DAP-based, but it will not speak DAP to an
arbitrary port you name: the `"type"` in `launch.json` must resolve to a debug
type contributed by an *installed extension*. Nothing in the box registers one
for Lua-over-TCP. Without this extension you get:

- *"Configured debug type 'oxigeon-lua' is not supported"* — F5 never opens a socket
- Breakpoints in `.lua` files render as hollow grey circles and never bind,
  because `contributes.breakpoints` is what permits them

The adapter itself — breakpoints, stepping, stack traces, variables — lives in
the Rust server under `src/core/scripting/debugger/dap/`. The only functional
line here points VS Code at its TCP address:

```js
new vscode.DebugAdapterServer(cfg.port || 4711, cfg.host || '127.0.0.1');
```

## Install

No build step; it is plain CommonJS.

```powershell
# Windows (run as Administrator, or enable Developer Mode for symlinks)
New-Item -ItemType SymbolicLink `
  -Path "$env:USERPROFILE\.vscode\extensions\oxigeon-debug-0.1.0" `
  -Target "C:\Code\oxigeon\tools\vscode-oxigeon-debug"
```

```bash
# macOS / Linux
ln -s "$PWD/tools/vscode-oxigeon-debug" ~/.vscode/extensions/oxigeon-debug-0.1.0
```

Copying the directory works just as well. Reload the VS Code window afterwards.

To package it properly instead: `npx @vscode/vsce package`, then
`code --install-extension oxigeon-debug-0.1.0.vsix`.

## Use

1. Enable `[servers.debug]` in `config/driver.toml`.
2. Start the server.
3. F5 → *Attach to Oxigeon* (already in `.vscode/launch.json`).

If something looks wrong, use the *Attach to Oxigeon (protocol trace)*
configuration — it dumps the whole DAP conversation to the Debug Console.

See `docs/src/lua-api/debugging.md` for capabilities, limitations, and the
freeze-the-world semantics.
