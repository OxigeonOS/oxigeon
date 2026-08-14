<script>
  // Scopes appear as collapsed headers. `Globals` is flagged expensive by the
  // adapter and is left alone until you expand it, because expanding it reaches
  // the entire daemon graph.

  let { app } = $props()
  const dbg = $derived((app.dbgVersion, app.dbg))

  function toggle(i) {
    const node = dbg.vars[i]
    dbg.varSel = i
    dbg.focus = 'vars'
    if (!node || node.varRef <= 0 || !dbg.stopped) return dbg.changed()
    if (node.expanded) dbg.collapse(i)
    else dbg.request('variables', { variablesReference: node.varRef })
    dbg.changed()
  }

  function onKeydown(event) {
    if (dbg.focus !== 'vars') return
    if (event.target instanceof HTMLInputElement) return
    if (event.ctrlKey) return
    if (event.key === 'ArrowUp') dbg.varSel = Math.max(0, dbg.varSel - 1)
    else if (event.key === 'ArrowDown') dbg.varSel = Math.min(dbg.vars.length - 1, dbg.varSel + 1)
    else if (event.key === 'Enter') return toggle(dbg.varSel)
    else return
    event.preventDefault()
    dbg.changed()
  }
</script>

<svelte:window on:keydown={onKeydown} />

<section class="pane" class:focused={dbg.focus === 'vars'}>
  <header>
    variables
    <span class="spacer"></span>
    {#if dbg.focus !== 'vars'}<span class="faint"><kbd>Tab</kbd> for the wide view</span>{/if}
  </header>
  <div class="body">
    {#each dbg.vars as node, i (i)}
      <div
        class="row"
        class:selected={i === dbg.varSel}
        class:scope={node.depth === 0}
        style="padding-left: {8 + node.depth * 12}px"
        onclick={() => toggle(i)}
        onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && toggle(i)}
        role="button"
        tabindex="-1"
      >
        <span class="twist faint">{node.varRef > 0 ? (node.expanded ? '▾' : '▸') : ' '}</span>
        <span class="name">{node.name}</span>
        {#if node.value}<span class="value">{node.value}</span>{/if}
        {#if node.ty}<span class="faint ty">{node.ty}</span>{/if}
      </div>
    {:else}
      <div class="row faint">{dbg.stopped ? 'no scopes' : 'needs a paused frame'}</div>
    {/each}
  </div>
</section>

<style>
  .row {
    white-space: pre;
  }

  .row.scope .name {
    color: var(--accent);
    font-weight: 700;
  }

  .twist {
    width: 1ch;
  }

  .name {
    color: var(--fg);
  }

  .value {
    color: #a5d6ff;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .ty {
    font-size: 11px;
  }
</style>
