<script>
  // The files pane is a collapsed tree rather than a list of paths. Every
  // `.lua` file under `mudlib/` and `game/` is several hundred rows, all of
  // them beginning `mudlib/` — a list you read rather than navigate. Only the
  // two roots start open.

  import { label } from '../lib/tree.js'

  let { app } = $props()
  const dbg = $derived((app.dbgVersion, app.dbg))

  let body = $state(null)

  function pick(i, row) {
    dbg.fileSel = i
    dbg.focus = 'files'
    if (row.isDir) dbg.toggleDir(row.path)
    else dbg.openFile(row.path, { focus: true })
  }

  function onKeydown(event) {
    if (dbg.focus !== 'files') return
    if (event.target instanceof HTMLInputElement) return
    const row = dbg.rows[dbg.fileSel]
    if (!row) return
    const { key } = event
    const last = Math.max(0, dbg.rows.length - 1)

    if (key === 'ArrowUp' || key === 'k') dbg.fileSel = Math.max(0, dbg.fileSel - 1)
    else if (key === 'ArrowDown' || key === 'j') dbg.fileSel = Math.min(last, dbg.fileSel + 1)
    // Open a directory, or a file. Enter on an open directory closes it, which
    // is what makes it a toggle rather than a one-way trip.
    else if (key === 'Enter' || key === 'ArrowRight' || key === 'l') {
      if (row.isDir) {
        // Already open: step into it rather than closing it.
        if ((key === 'ArrowRight' || key === 'l') && row.expanded) {
          dbg.fileSel = Math.min(last, dbg.fileSel + 1)
        } else dbg.toggleDir(row.path)
      } else dbg.openFile(row.path, { focus: true })
    }
    // Close this directory, or jump to the parent of whatever this is.
    else if (key === 'ArrowLeft' || key === 'h') {
      if (row.isDir && row.expanded) dbg.toggleDir(row.path)
      else {
        const parent = row.path.slice(0, row.path.lastIndexOf('/'))
        const at = dbg.rows.findIndex((r) => r.path === parent)
        if (at !== -1) dbg.fileSel = at
      }
    } else return

    event.preventDefault()
    dbg.changed()
  }

  // Keep the selection on screen — a stop opens whatever file it landed in, and
  // reveals it, which can move the selection a long way.
  $effect(() => {
    dbg.fileSel
    body?.querySelector('.selected')?.scrollIntoView({ block: 'nearest' })
  })
</script>

<svelte:window on:keydown={onKeydown} />

<section class="pane" class:focused={dbg.focus === 'files'} style="grid-area: files">
  <header>
    files
    <span class="spacer"></span>
    <span class="dim">{dbg.files.length}</span>
  </header>
  <div class="body" bind:this={body}>
    {#each dbg.rows as row, i (row.path)}
      <div
        class="row"
        class:selected={i === dbg.fileSel}
        class:open={!row.isDir && dbg.open === row.path}
        style="padding-left: {8 + row.depth * 12}px"
        onclick={() => pick(i, row)}
        onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && pick(i, row)}
        role="treeitem"
        tabindex="-1"
        aria-selected={i === dbg.fileSel}
      >
        <span class="twist faint">{row.isDir ? (row.expanded ? '▾' : '▸') : ' '}</span>
        <span class="name" class:dir={row.isDir}>{label(row.path)}</span>
        <span class="spacer"></span>
        <!-- A red dot beside a folder means something inside it has a
             breakpoint, so collapsing the tree never hides one. -->
        {#if dbg.markedUnder(row.path, row.isDir)}
          <span class="mark">●</span>
        {/if}
      </div>
    {/each}
  </div>
  <footer><kbd>j</kbd><kbd>k</kbd> move <kbd>l</kbd> open <kbd>h</kbd> close</footer>
</section>

<style>
  .name.dir {
    color: var(--fg-dim);
  }

  .row.open .name {
    color: var(--accent);
  }

  .twist {
    width: 1ch;
  }

  .mark {
    color: var(--red);
    font-size: 9px;
  }
</style>
