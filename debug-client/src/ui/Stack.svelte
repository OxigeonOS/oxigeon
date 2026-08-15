<script>
  let { app } = $props()

  function select(i) {
    app.dbg.frameSel = i
    app.dbg.focus = 'stack'
    if (app.dbg.stopped) app.dbg.followFrame()
    app.dbg.changed()
  }

  function onKeydown(event) {
    if (app.dbg.focus !== 'stack') return
    if (event.target instanceof HTMLInputElement) return
    if (event.ctrlKey) return // the step aliases
    if (event.key === 'ArrowUp') select(Math.max(0, app.dbg.frameSel - 1))
    else if (event.key === 'ArrowDown') select(Math.min(app.dbg.frames.length - 1, app.dbg.frameSel + 1))
    else return
    event.preventDefault()
  }

  const short = (path) => (path ? path.split('/').slice(-2).join('/') : '')
</script>

<svelte:window on:keydown={onKeydown} />

<section class="pane" class:focused={app.dbg.focus === 'stack'}>
  <header>
    stack
    <span class="spacer"></span>
    {#if app.dbg.stopped}<span class="dim">{app.dbg.stopReason}</span>{/if}
  </header>
  <div class="body">
    {#each app.dbg.frames as frame, i (frame.id)}
      <div
        class="row"
        class:selected={i === app.dbg.frameSel}
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
      <div class="row faint">{app.dbg.attached ? 'running' : 'not attached'}</div>
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
