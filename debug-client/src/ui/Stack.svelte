<script>
  let { app } = $props()
  const dbg = $derived((app.dbgVersion, app.dbg))

  function select(i) {
    dbg.frameSel = i
    dbg.focus = 'stack'
    if (dbg.stopped) dbg.followFrame()
    dbg.changed()
  }

  function onKeydown(event) {
    if (dbg.focus !== 'stack') return
    if (event.target instanceof HTMLInputElement) return
    if (event.ctrlKey) return // the step aliases
    if (event.key === 'ArrowUp') select(Math.max(0, dbg.frameSel - 1))
    else if (event.key === 'ArrowDown') select(Math.min(dbg.frames.length - 1, dbg.frameSel + 1))
    else return
    event.preventDefault()
  }

  const short = (path) => (path ? path.split('/').slice(-2).join('/') : '')
</script>

<svelte:window on:keydown={onKeydown} />

<section class="pane" class:focused={dbg.focus === 'stack'}>
  <header>
    stack
    <span class="spacer"></span>
    {#if dbg.stopped}<span class="dim">{dbg.stopReason}</span>{/if}
  </header>
  <div class="body">
    {#each dbg.frames as frame, i (frame.id)}
      <div
        class="row"
        class:selected={i === dbg.frameSel}
        onclick={() => select(i)}
        onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && select(i)}
        role="button"
        tabindex="-1"
      >
        <span class="faint n">{i}</span>
        <span class="name">{frame.name}</span>
        <span class="spacer"></span>
        <span class="faint where">{short(frame.path)}:{frame.line}</span>
      </div>
    {:else}
      <!-- Every control that needs a paused frame is disabled and says so.
           `stackTrace` is rejected outright while the VM is running. -->
      <div class="row faint">{dbg.attached ? 'running' : 'not attached'}</div>
    {/each}
  </div>
</section>

<style>
  .n {
    width: 2ch;
    text-align: right;
  }

  .name {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .where {
    font-size: 11px;
  }
</style>
