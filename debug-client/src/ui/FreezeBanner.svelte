<script>
  // The point of the whole window.
  //
  // Hitting a breakpoint stops the entire Lua VM and *every player on the
  // server freezes*. From an editor that is invisible: you see a stopped stack,
  // you do not see the game standing still. So the game pane greys out under a
  // banner counting the adapter's own `auto_continue_secs` down.
  //
  // `worldFrozen` is kept apart from `stopped` because a Lua 5.5 build can
  // suspend just the one dispatch — and drawing a freeze banner over a game
  // that is demonstrably still being played is its own kind of lie.

  let { app } = $props()

  const dbg = $derived((app.dbgVersion, app.dbg))
  const left = $derived((app.now, dbg.autoContinueIn()))

  const where = $derived.by(() => {
    const frame = dbg.frames[0]
    if (!frame) return dbg.stopReason
    const file = frame.path?.split('/').slice(-1)[0] ?? frame.name
    return `${dbg.stopReason} at ${file}:${frame.line}`
  })

  const mmss = (s) => `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`
</script>

<div class="banner" class:frozen={dbg.worldFrozen}>
  <div class="title">
    {dbg.worldFrozen ? '⏸  VM PAUSED' : '⏸  DISPATCH SUSPENDED'}
  </div>
  <div class="where">{where}</div>
  <div class="cost">
    {#if dbg.worldFrozen}
      every player on this server is frozen
    {:else}
      one dispatch is held — everyone else is still playing
    {/if}
  </div>
  {#if left !== null}
    <div class="count" class:soon={left < 30}>auto-continue in {mmss(left)}</div>
  {/if}
  <div class="keys">
    <button onclick={() => dbg.request('continue', { threadId: dbg.stopId })}>
      <kbd>^G</kbd> continue
    </button>
    <button onclick={() => (app.tab = 'Debug')}><kbd>F2</kbd> debug</button>
  </div>
</div>

<style>
  .banner {
    position: absolute;
    inset: 20% 15% auto 15%;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 14px;
    border: 1px solid var(--yellow);
    border-radius: 5px;
    background: #1a1508f2;
    box-shadow: 0 10px 40px #000a;
    text-align: center;
  }

  .banner.frozen {
    border-color: var(--red);
    background: #1a0c0af2;
  }

  .title {
    font-size: 16px;
    font-weight: 700;
    letter-spacing: 0.08em;
    color: var(--yellow);
  }

  .banner.frozen .title {
    color: var(--red);
  }

  .where {
    color: var(--fg);
  }

  .cost {
    color: var(--fg-dim);
    font-size: 11px;
  }

  .count {
    color: var(--fg-dim);
  }

  .count.soon {
    color: var(--yellow);
  }

  .keys {
    display: flex;
    gap: 6px;
    margin-top: 4px;
  }
</style>
