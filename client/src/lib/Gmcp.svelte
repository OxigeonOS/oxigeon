<script>
  // A GMCP inspector rather than a styled HUD.
  //
  // The point of this panel is to show that the channel *works* and what a game
  // is actually sending — a fixed set of vitals bars would look better and prove
  // less, and would need editing every time the mudlib adds a package.

  let { packages } = $props()

  let collapsed = $state({})

  const entries = $derived(Object.entries(packages).sort(([a], [b]) => a.localeCompare(b)))

  function pretty(v) {
    return typeof v === 'string' ? v : JSON.stringify(v, null, 2)
  }
</script>

<aside>
  <h2>GMCP</h2>
  {#if entries.length === 0}
    <p class="empty">Nothing received yet.</p>
  {:else}
    {#each entries as [name, entry] (name)}
      <section>
        <button class="head" onclick={() => (collapsed[name] = !collapsed[name])}>
          <span class="caret">{collapsed[name] ? '▸' : '▾'}</span>
          <span class="name">{name}</span>
          <span class="count">×{entry.count}</span>
        </button>
        {#if !collapsed[name]}
          <pre>{pretty(entry.data)}</pre>
        {/if}
      </section>
    {/each}
  {/if}
</aside>

<style>
  aside {
    width: 300px;
    flex: none;
    border-left: 1px solid var(--ox-line);
    background: var(--ox-panel);
    overflow-y: auto;
    padding: 10px 12px;
  }
  h2 {
    margin: 0 0 8px;
    font-size: 11px;
    letter-spacing: 0.09em;
    text-transform: uppercase;
    color: var(--ox-dim);
  }
  .empty { color: var(--ox-dim); font-size: 12px; }
  section { margin-bottom: 6px; }
  .head {
    display: flex;
    gap: 6px;
    align-items: baseline;
    width: 100%;
    padding: 3px 4px;
    background: none;
    border: none;
    text-align: left;
  }
  .head:hover { background: var(--ox-line); }
  .caret { color: var(--ox-dim); }
  .name { color: var(--ox-accent); font-size: 12px; }
  .count { margin-left: auto; color: var(--ox-dim); font-size: 11px; }
  pre {
    margin: 2px 0 0 18px;
    font-size: 11.5px;
    color: var(--ox-fg);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
</style>
