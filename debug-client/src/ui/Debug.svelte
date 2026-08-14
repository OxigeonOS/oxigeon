<script>
  import { stepCommand } from '../lib/debugview.js'
  import FileTree from './FileTree.svelte'
  import Source from './Source.svelte'
  import Stack from './Stack.svelte'
  import Variables from './Variables.svelte'
  import Repl from './Repl.svelte'

  let { app } = $props()
  const dbg = $derived((app.dbgVersion, app.dbg))

  const PANES = ['files', 'source', 'stack', 'vars', 'repl']

  /// Tab onto the variables pane and it takes the middle column, swapping
  /// places with the source. A narrow strip is enough to see *that* a local
  /// exists and not much else, and reading values is most of what a debugger is
  /// for. Tab again and the source comes back — one keystroke each way, and no
  /// mode to get stuck in.
  const varsWide = $derived(dbg.focus === 'vars')

  function step(command) {
    if (!dbg.stopped) return
    dbg.request(command, { threadId: dbg.stopId })
  }

  function onKeydown(event) {
    // The line editors own the keyboard while they are open, or typing a
    // message — or a search — would trip every binding below. They are real
    // inputs, so this is also what keeps a keystroke from being handled twice.
    if (event.target instanceof HTMLInputElement) return

    const { key, ctrlKey: ctrl, shiftKey: shift } = event

    // Execution control works from any pane, and only while stopped — the
    // adapter rejects all of these outright when the VM is running.
    if (dbg.stopped) {
      const command = stepCommand(event)
      if (command) {
        event.preventDefault()
        return step(command)
      }
    }

    if (ctrl && (key === 'p' || key === 'P') && !dbg.stopped) {
      // Consumed by the next *line* event, so it lands on the next command a
      // player types rather than immediately.
      event.preventDefault()
      return dbg.request('pause', { threadId: dbg.stopId })
    }
    if ((key === 'F9' && shift) || (ctrl && (key === 'l' || key === 'L'))) {
      event.preventDefault()
      return dbg.beginLogpoint()
    }
    if (key === 'F9') {
      event.preventDefault()
      return dbg.toggleBreakpoint()
    }
    if (key === 'Tab') {
      event.preventDefault()
      const at = PANES.indexOf(dbg.focus)
      dbg.focus = PANES[(at + (shift ? PANES.length - 1 : 1)) % PANES.length]
      dbg.changed()
      return
    }
  }
</script>

<svelte:window on:keydown={onKeydown} />

<div class="debug" class:vars-wide={varsWide}>
  <div class="controls">
    <button disabled={!dbg.stopped} onclick={() => step('continue')} title="F5 / Ctrl+G">
      ▶ continue
    </button>
    <button disabled={!dbg.stopped} onclick={() => step('next')} title="F10 / Ctrl+→">
      ⤼ over
    </button>
    <button disabled={!dbg.stopped} onclick={() => step('stepIn')} title="F11 / Ctrl+↓">
      ↓ into
    </button>
    <button disabled={!dbg.stopped} onclick={() => step('stepOut')} title="Shift+F11 / Ctrl+↑">
      ↑ out
    </button>
    <button
      disabled={dbg.stopped || !dbg.attached}
      onclick={() => dbg.request('pause', { threadId: dbg.stopId })}
      title="Ctrl+P — lands on the next line event, i.e. the next command a player types"
    >
      ⏸ pause
    </button>
    <span class="spacer"></span>
    <!-- F11 is full-screen and F12 is developer tools; neither can be
         intercepted, so the Ctrl aliases are the ones that always arrive. -->
    <span class="faint">
      <kbd>^G</kbd> continue <kbd>^→</kbd> over <kbd>^↓</kbd> into <kbd>^↑</kbd> out
      <kbd>F9</kbd> breakpoint <kbd>^L</kbd> logpoint <kbd>Tab</kbd> pane
    </span>
  </div>

  <FileTree {app} />
  <Source {app} />
  <div class="right">
    <Stack {app} />
    <Variables {app} />
  </div>
  <Repl {app} />
</div>

<style>
  .debug {
    flex: 1 1 auto;
    min-height: 0;
    display: grid;
    grid-template-columns: 22em minmax(0, 1fr) 26em;
    grid-template-rows: auto minmax(0, 1fr) 11em;
    grid-template-areas:
      'controls controls controls'
      'files    source   right'
      'repl     repl     repl';
    gap: 4px;
  }

  /* Tab onto variables and it takes the middle column. */
  .debug.vars-wide {
    grid-template-columns: 22em 26em minmax(0, 1fr);
    grid-template-areas:
      'controls controls controls'
      'files    source   right'
      'repl     repl     repl';
  }

  .controls {
    grid-area: controls;
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .right {
    grid-area: right;
    display: grid;
    grid-template-rows: 40% minmax(0, 1fr);
    gap: 4px;
    min-height: 0;
    min-width: 0;
  }

  .controls .faint {
    font-size: 11px;
  }
</style>
