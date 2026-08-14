<script>
  let { app } = $props()

  // Read `dbgVersion` so this redraws when the debugger moves, and `now` so the
  // countdown ticks while the VM is frozen and nothing else is arriving.
  const dbg = $derived((app.dbgVersion, app.dbg))

  const dapLabel = $derived.by(() => {
    if (dbg.worldFrozen) return { text: 'frozen', kind: 'stop' }
    // One dispatch is held and the game is still running. Saying "stopped" here
    // is what made a live server look dead.
    if (dbg.stopped) return { text: 'suspended', kind: 'stop' }
    if (dbg.attached) return { text: 'attached', kind: 'up' }
    return { text: app.dap.state === 'down' ? `down: ${app.dap.why}` : app.dap.state, kind: app.dap.state }
  })

  const telnetLabel = $derived(
    app.telnet.state === 'down' ? `down: ${app.telnet.why}` : app.telnet.state
  )
</script>

<footer>
  {#if app.link.state !== 'up'}
    <span class="label">bridge</span>
    <span class="down">{app.link.why || app.link.state}</span>
  {/if}

  <span class="label">telnet</span>
  <span class={app.telnet.state}>{telnetLabel}</span>

  <span class="label">dap</span>
  <span class={dapLabel.kind}>{dapLabel.text}</span>

  {#if dbg.attached}
    <!-- Worth saying out loud: an attached client forces LuaJIT onto the
         interpreter, so "everything is slow" is expected, not a bug. -->
    <span class="note">JIT off while attached</span>
  {/if}

  <span class="spacer"></span>

  {#if app.info}
    <span class="note">{app.info.telnet} · {app.info.dap}</span>
  {/if}
  <span class="note"><kbd>F1</kbd>–<kbd>F4</kbd> tabs · <kbd>^J</kbd> journal</span>
</footer>

<style>
  footer {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 1px 6px;
    font-size: 11px;
    white-space: nowrap;
    overflow: hidden;
  }

  .label {
    color: var(--fg-faint);
  }

  .note {
    color: var(--fg-faint);
  }

  .up {
    color: var(--green);
  }

  .connecting {
    color: var(--yellow);
  }

  .down {
    color: var(--red);
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .stop {
    color: #0e1116;
    background: var(--yellow);
    padding: 0 5px;
    border-radius: 3px;
    font-weight: 700;
  }
</style>
