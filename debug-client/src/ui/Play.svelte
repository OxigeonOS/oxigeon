<script>
  import { untrack } from 'svelte'
  import { css } from '../lib/spans.js'
  import FreezeBanner from './FreezeBanner.svelte'

  let { app } = $props()

  let body = $state(null)
  let field = $state(null)
  let pinned = $state(true)


  // Only follow the tail if the reader has not scrolled away from it.
  //
  // `pinned` is read through `untrack` on purpose. Assigning `scrollTop` fires
  // a `scroll` event, `onScroll` writes `pinned`, and if this effect depended
  // on it that write would re-run the effect, which scrolls again. That loop
  // does not hang — Svelte throws `effect_update_depth_exceeded` — but a throw
  // inside an effect takes the rest of the render down with it, which looks
  // exactly like the pane having died.
  $effect(() => {
    app.scrollback.length
    app.prompt
    if (untrack(() => pinned) && body && body.scrollTop !== body.scrollHeight) {
      body.scrollTop = body.scrollHeight
    }
  })

  function onScroll() {
    if (!body) return
    const atBottom = body.scrollHeight - body.scrollTop - body.clientHeight < 24
    // Only write when it actually changes, so a scroll that keeps us pinned
    // does not invalidate anything downstream.
    if (atBottom !== pinned) pinned = atBottom
  }

  function onKeydown(event) {
    if (event.key === 'Enter') {
      app.submit()
      pinned = true
    } else if (event.key === 'ArrowUp') {
      event.preventDefault()
      app.recall(-1)
    } else if (event.key === 'ArrowDown') {
      event.preventDefault()
      app.recall(1)
    } else if (event.key === 'Escape') {
      app.input = ''
      pinned = true
    }
  }

  // The pager wraps to what we tell it, so tell it the truth about the pane.
  $effect(() => {
    if (!body) return
    const report = () => {
      const probe = document.createElement('span')
      probe.textContent = '0'.repeat(100)
      probe.style.cssText = 'position:absolute;visibility:hidden;white-space:pre'
      body.appendChild(probe)
      const box = probe.getBoundingClientRect()
      const cell = box.width / 100
      const row = box.height
      probe.remove()
      if (cell > 0 && row > 0) {
        app.size(
          Math.max(20, Math.floor(body.clientWidth / cell) - 1),
          Math.max(5, Math.floor(body.clientHeight / row))
        )
      }
    }
    report()
    const observer = new ResizeObserver(report)
    observer.observe(body)
    return () => observer.disconnect()
  })

  const bar = (now, max) => (max ? Math.max(0, Math.min(100, (now / max) * 100)) : 0)
</script>

<div class="play">
  <section class="pane game" class:frozen={app.dbg.worldFrozen}>
    <header>
      game
      {#if !pinned}
        <button onclick={() => { pinned = true; body.scrollTop = body.scrollHeight }}>
          jump to bottom
        </button>
      {/if}
      <span class="spacer"></span>
      <span class="dim">{app.scrollback.length} lines</span>
    </header>

    <div class="body" bind:this={body} onscroll={onScroll}>
      {#each app.scrollback as line, i (i)}
        <div class="line">
          {#each line as span}<span style={css(span)}>{span.text}</span>{/each}
        </div>
      {/each}
      {#if app.prompt}
        <div class="line prompt">
          {#each app.prompt as span}<span style={css(span)}>{span.text}</span>{/each}
        </div>
      {/if}
    </div>

    <footer class="entry">
      <span class="caret">›</span>
      <!-- The driver sends `IAC WILL ECHO` around a password prompt; a masked
           line never enters the recallable history. -->
      {#if app.masked}
        <input
          bind:this={field}
          type="password"
          bind:value={app.input}
          onkeydown={onKeydown}
          placeholder="(hidden)"
          autocomplete="off"
        />
        <span class="warn">masked</span>
      {:else}
        <input
          bind:this={field}
          bind:value={app.input}
          onkeydown={onKeydown}
          placeholder="type a command"
          autocomplete="off"
          spellcheck="false"
        />
      {/if}
    </footer>

    {#if app.dbg.stopped}
      <FreezeBanner {app} />
    {/if}
  </section>

  <aside>
    <section class="pane">
      <header>room</header>
      <div class="body pad">
        <div class="name">{app.room.name || '—'}</div>
        <!-- The dotted room id, because that is what `goto` takes and what the
             room's file is named after. -->
        <div class="dim id">{app.room.id}</div>
        {#if app.room.exits.length}
          <div class="exits">
            <span class="faint">exits</span>
            {#each app.room.exits as exit}
              <button onclick={() => app.command(exit)}>{exit}</button>
            {/each}
          </div>
        {/if}
      </div>
    </section>

    <section class="pane">
      <header>vitals</header>
      <div class="body pad">
        <div class="gauge">
          <span class="faint">hp</span>
          <div class="track"><div class="fill hp" style="width:{bar(app.vitals.hp, app.vitals.maxhp)}%"></div></div>
          <span>{app.vitals.hp ?? '—'}<span class="faint">/{app.vitals.maxhp ?? '—'}</span></span>
        </div>
        <div class="gauge">
          <span class="faint">mp</span>
          <div class="track"><div class="fill mp" style="width:{bar(app.vitals.mp, app.vitals.maxmp)}%"></div></div>
          <span>{app.vitals.mp ?? '—'}<span class="faint">/{app.vitals.maxmp ?? '—'}</span></span>
        </div>
        <div class="stats">
          <span class="faint">level</span><span>{app.vitals.level ?? '—'}</span>
          <span class="faint">xp</span><span>{app.vitals.xp ?? '—'}</span>
          <span class="faint">gold</span><span>{app.vitals.gold ?? '—'}</span>
        </div>
      </div>
    </section>

    <section class="pane grow">
      <header>effects</header>
      <div class="body">
        {#each app.effects as effect}
          <div class="row">
            <span>{effect.label}</span>
            {#if effect.stacks > 1}<span class="faint">×{effect.stacks}</span>{/if}
            <span class="spacer"></span>
            <!-- `remaining == -1` means no expiry. -->
            <span class="faint">{effect.remaining < 0 ? '∞' : `${effect.remaining}s`}</span>
          </div>
        {:else}
          <div class="row faint">none</div>
        {/each}
      </div>
    </section>
  </aside>
</div>

<style>
  .play {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 22em;
    gap: 4px;
    flex: 1 1 auto;
    min-height: 0;
  }

  .game {
    position: relative;
  }

  .game.frozen .body {
    opacity: 0.35;
  }

  .line {
    padding: 0 8px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .prompt {
    color: var(--fg-dim);
  }

  .entry {
    display: flex;
    align-items: baseline;
    gap: 6px;
    border-top: 1px solid var(--border);
    padding: 3px 8px;
  }

  .caret {
    color: var(--accent);
  }

  aside {
    display: grid;
    grid-template-rows: auto auto minmax(0, 1fr);
    gap: 4px;
    min-height: 0;
  }

  .grow {
    min-height: 0;
  }

  .pad {
    padding: 4px 8px;
  }

  .name {
    font-weight: 700;
  }

  .id {
    font-size: 11px;
  }

  .exits {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    align-items: baseline;
    margin-top: 4px;
  }

  .gauge {
    display: grid;
    grid-template-columns: 2.5em 1fr auto;
    gap: 6px;
    align-items: center;
    margin-bottom: 3px;
  }

  .track {
    height: 8px;
    background: var(--bg-sunken);
    border-radius: 4px;
    overflow: hidden;
  }

  .fill {
    height: 100%;
    transition: width 0.2s;
  }

  .fill.hp {
    background: var(--red);
  }

  .fill.mp {
    background: var(--cyan);
  }

  .stats {
    display: grid;
    grid-template-columns: auto 1fr auto 1fr auto 1fr;
    gap: 4px;
    margin-top: 4px;
    font-size: 11px;
  }
</style>
