<script>
  // The pane the variables tree cannot be.
  //
  // Traits are derived values over a dependency graph, and effects modify them
  // without ever being stored — so `entity.stats[id]` is the *stored* number
  // and, for anything derived or buffed, the wrong answer. A debugger can only
  // show you the raw table.
  //
  // This reads through `DAEMON.trait.all` and `DAEMON.effect.active` instead,
  // and shows the effective value with the stored base beside it where they
  // differ. `max_hp` on a fresh character has **no** stored value at all; this
  // pane shows what the formula produces.

  let { app } = $props()
  const dbg = $derived((app.dbgVersion, app.dbg))

  let field = $state(null)

  const groups = $derived.by(() => {
    const by = new Map()
    for (const t of dbg.inspect.traits) {
      const key = t.group || 'ungrouped'
      if (!by.has(key)) by.set(key, [])
      by.get(key).push(t)
    }
    return [...by.entries()]
  })

  function submit(event) {
    event.stopPropagation()
    if (event.key === 'Enter') dbg.requestInspect()
  }

  const differs = (t) => t.base !== '' && t.base !== t.value && t.base !== 'nil'
</script>

<div class="inspect">
  <section class="pane">
    <header>
      target
      <span class="spacer"></span>
      <span class="faint">any Lua expression in the paused frame</span>
    </header>
    <div class="query">
      <span class="caret">=</span>
      <input
        bind:this={field}
        bind:value={dbg.inspect.target}
        onkeydown={submit}
        spellcheck="false"
        autocomplete="off"
        placeholder="player"
      />
      <button disabled={!dbg.stopped} onclick={() => dbg.requestInspect()}>
        {dbg.inspect.pending ? 'reading…' : 'read'}
      </button>
    </div>
    {#if dbg.inspect.error}
      <div class="error">{dbg.inspect.error}</div>
    {/if}
  </section>

  <div class="tables">
    <section class="pane">
      <header>
        traits
        <span class="spacer"></span>
        <span class="dim">{dbg.inspect.traits.length}</span>
      </header>
      <div class="body">
        <div class="row head faint">
          <span class="id">id</span>
          <span class="val">value</span>
          <span class="base">base</span>
          <span class="max">max</span>
          <span class="kind">kind</span>
        </div>
        {#each groups as [group, traits] (group)}
          <div class="group faint">{group}</div>
          {#each traits as t (t.id)}
            <div class="row" class:failed={t.failed}>
              <span class="id" title={t.label}>{t.id}</span>
              <span class="val">{t.value}</span>
              <!-- The stored base, shown only where it differs from the
                   effective value: that difference is the entire point. -->
              <span class="base" class:differs={differs(t)}>{differs(t) ? t.base : ''}</span>
              <span class="max faint">{t.max}</span>
              <span class="kind faint">{t.kind}</span>
            </div>
          {/each}
        {:else}
          <div class="row faint">
            {dbg.stopped ? 'nothing read yet' : 'attach and break somewhere'}
          </div>
        {/each}
      </div>
    </section>

    <section class="pane">
      <header>
        effects
        <span class="spacer"></span>
        <span class="dim">{dbg.inspect.effects.length}</span>
      </header>
      <div class="body">
        {#each dbg.inspect.effects as e (e.id)}
          <div class="row">
            <span class="id">{e.label || e.id}</span>
            {#if Number(e.stacks) > 1}<span class="faint">×{e.stacks}</span>{/if}
            <span class="spacer"></span>
            <span class="faint">{e.expires || '∞'}</span>
          </div>
        {:else}
          <div class="row faint">none</div>
        {/each}
      </div>
    </section>
  </div>
</div>

<style>
  .inspect {
    flex: 1 1 auto;
    min-height: 0;
    display: grid;
    grid-template-rows: auto minmax(0, 1fr);
    gap: 4px;
  }

  .tables {
    display: grid;
    grid-template-columns: minmax(0, 2fr) minmax(0, 1fr);
    gap: 4px;
    min-height: 0;
  }

  .query {
    display: flex;
    align-items: baseline;
    gap: 6px;
    padding: 4px 8px;
  }

  .caret {
    color: var(--accent);
  }

  .error {
    padding: 2px 8px 6px;
    color: var(--yellow);
  }

  .row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 6ch 6ch 6ch 7ch;
    gap: 8px;
  }

  .row.head,
  .group {
    font-size: 11px;
  }

  .group {
    padding: 4px 8px 0;
    text-transform: lowercase;
    letter-spacing: 0.05em;
  }

  .id {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .val {
    color: #79c0ff;
    text-align: right;
  }

  .base,
  .max {
    text-align: right;
  }

  .base.differs {
    color: var(--fg-dim);
  }

  .failed .val {
    color: var(--red);
  }

  /* The effects pane is a plain list, not the trait grid. */
  section:last-child .row {
    display: flex;
    gap: 6px;
  }
</style>
