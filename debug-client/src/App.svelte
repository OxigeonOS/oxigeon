<script>
  import { onMount } from 'svelte'
  import { App, TABS } from './lib/app.svelte.js'
  import Play from './ui/Play.svelte'
  import Debug from './ui/Debug.svelte'
  import Inspect from './ui/Inspect.svelte'
  import Trace from './ui/Trace.svelte'
  import Journal from './ui/Journal.svelte'
  import StatusBar from './ui/StatusBar.svelte'

  const app = new App()

  onMount(() => {
    app.start()
    return () => app.stop()
  })

  // Tab switching and the journal toggle work from anywhere. F1–F4 are what the
  // TUI documents and they arrive in Chrome, so they are what the tab bar says.
  // `Alt`+1..4 does the same thing, for the browsers that do keep a function
  // key for themselves — the same bargain the step controls make with `Ctrl`.
  function onKeydown(event) {
    const { key, altKey, ctrlKey } = event
    if (altKey && /^[1-4]$/.test(key)) {
      event.preventDefault()
      app.tab = TABS[Number(key) - 1]
      return
    }
    const fn = key.match(/^F([1-4])$/)
    if (fn) {
      event.preventDefault()
      app.tab = TABS[Number(fn[1]) - 1]
      return
    }
    if (ctrlKey && (key === 'j' || key === 'J')) {
      event.preventDefault()
      app.showJournal = !app.showJournal
    }
  }
</script>

<svelte:window on:keydown={onKeydown} />

<div class="shell">
  <nav>
    {#each TABS as tab, i (tab)}
      <button
        class:active={app.tab === tab}
        onclick={() => (app.tab = tab)}
        title="F{i + 1} — or Alt+{i + 1}, if the browser keeps the function key"
      >
        <span class="fkey">F{i + 1}</span>
        {tab}
      </button>
    {/each}
    <span class="spacer"></span>
    <button class:active={app.showJournal} onclick={() => (app.showJournal = !app.showJournal)}>
      <span class="fkey">^J</span> journal
    </button>
  </nav>

  <main>
    {#if app.tab === 'Play'}
      <Play {app} />
    {:else if app.tab === 'Debug'}
      <Debug {app} />
    {:else if app.tab === 'Inspect'}
      <Inspect {app} />
    {:else}
      <Trace {app} />
    {/if}
  </main>

  {#if app.showJournal}
    <Journal {app} />
  {/if}

  <StatusBar {app} />
</div>

<style>
  .shell {
    display: grid;
    grid-template-rows: auto minmax(0, 1fr) auto auto;
    height: 100vh;
    gap: 4px;
    padding: 4px;
  }

  nav {
    display: flex;
    gap: 4px;
    align-items: center;
  }

  nav button {
    border-color: transparent;
  }

  nav button.active {
    color: #0e1116;
    background: var(--accent);
    border-color: var(--accent);
    font-weight: 700;
  }

  nav button.active .fkey {
    color: #0e1116cc;
  }

  .fkey {
    color: var(--fg-faint);
  }

  main {
    min-height: 0;
    display: flex;
  }
</style>
