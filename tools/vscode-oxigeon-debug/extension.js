// Oxigeon Lua Debug — a debug *type* registration and nothing more.
//
// The adapter itself lives in the Rust server (src/core/scripting/debugger/dap),
// so all this does is point VS Code at that TCP port. VS Code will not let you
// set breakpoints against a debug type no extension has contributed, which is
// why this file has to exist at all.

const vscode = require('vscode');

function activate(context) {
  context.subscriptions.push(
    vscode.debug.registerDebugAdapterDescriptorFactory('oxigeon-lua', {
      createDebugAdapterDescriptor(session) {
        const cfg = session.configuration;
        return new vscode.DebugAdapterServer(cfg.port || 4711, cfg.host || '127.0.0.1');
      },
    })
  );
}

function deactivate() {}

module.exports = { activate, deactivate };
