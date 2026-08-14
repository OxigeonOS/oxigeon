<script>
  import { spanStyle } from './palette.js'

  let { lines, prompt } = $props()

  const DEFAULTS = { fg: null, bg: null }

  let el = $state(null)
  /** Whether the view is pinned to the bottom. */
  let stuck = $state(true)

  function onScroll() {
    if (!el) return
    // A few pixels of slack: a trackpad rarely lands exactly on the bottom, and
    // unsticking a view the player thinks is stuck is worse than the reverse.
    stuck = el.scrollHeight - el.scrollTop - el.clientHeight < 24
  }

  // Follow new output only when the player has not scrolled up to read
  // something. Yanking someone back to the bottom mid-sentence is the single
  // most irritating thing a web MUD client does.
  $effect(() => {
    lines.length
    prompt
    if (stuck && el) el.scrollTop = el.scrollHeight
  })
</script>

<div class="out" bind:this={el} onscroll={onScroll} role="log" aria-live="polite">
  {#each lines as line (line.id)}
    <div class="line" class:system={line.system}>
      {#if line.spans}
        {#each line.spans as s, i (i)}<span style={spanStyle(s, DEFAULTS)}>{s.text}</span>{/each}
      {:else}{line.text}{/if}
    </div>
  {/each}

  {#if prompt}
    <div class="line prompt">
      {#if prompt.spans}
        {#each prompt.spans as s, i (i)}<span style={spanStyle(s, DEFAULTS)}>{s.text}</span>{/each}
      {:else}{prompt.text}{/if}
    </div>
  {/if}
</div>

{#if !stuck}
  <button class="jump" onclick={() => { stuck = true; if (el) el.scrollTop = el.scrollHeight }}>
    ↓ jump to latest
  </button>
{/if}

<style>
  .out {
    flex: 1;
    overflow-y: auto;
    padding: 12px 14px;
    /* The server guarantees no carriage returns and wraps to the width we
       announced, so `pre-wrap` is enough — long unbroken tokens still need
       `anywhere` or a URL blows the layout out. */
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .line { min-height: 1.45em; }
  .line.system { color: var(--ox-dim); font-style: italic; }
  .line.prompt { opacity: 0.9; }

  .jump {
    position: absolute;
    right: 24px;
    bottom: 76px;
    font-size: 12px;
    opacity: 0.9;
  }
</style>
