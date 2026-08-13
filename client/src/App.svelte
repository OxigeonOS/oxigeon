<script>
  import { Connection, defaultUrl } from './lib/connection.js'
  import Output from './lib/Output.svelte'
  import Gmcp from './lib/Gmcp.svelte'

  let url = $state(defaultUrl())
  let status = $state('disconnected') // disconnected | connecting | connected
  let conn = $state(null)

  let lines = $state([])
  let prompt = $state(null)
  let packages = $state({})

  let entry = $state('')
  /** Set by an `echo` frame. True means a password is being asked for. */
  let masked = $state(false)
  let showGmcp = $state(true)

  /** Command history, oldest first. `histAt === null` means "typing fresh". */
  let history = $state([])
  let histAt = $state(null)

  let nextId = 0
  let inputEl = $state(null)
  let outerEl = $state(null)
  let rulerEl = $state(null)

  function push(frame, system = false) {
    // Every line gets an id so the keyed `{#each}` never re-renders the whole
    // scrollback when one line is appended.
    lines.push({ id: nextId++, ...frame, system })
    // A session left open all evening would otherwise grow without bound. The
    // cap is generous enough that nobody scrolls past it in practice.
    if (lines.length > 5000) lines.splice(0, lines.length - 5000)
  }

  function system(text) {
    push({ text }, true)
  }

  /**
   * How many characters fit across the output pane.
   *
   * Measured rather than assumed: the server wraps to whatever we announce, so
   * a wrong number here shows up as ragged text rather than as an error. The
   * ruler is 100 characters wide so the division is not dominated by rounding.
   */
  function measureWidth() {
    if (!rulerEl || !outerEl) return 80
    const perChar = rulerEl.getBoundingClientRect().width / 100
    if (!perChar) return 80
    const pane = outerEl.getBoundingClientRect().width - (showGmcp ? 300 : 0) - 28
    return Math.max(20, Math.min(500, Math.floor(pane / perChar)))
  }

  function connect() {
    if (conn) conn.close()
    lines = []
    prompt = null
    packages = {}
    status = 'connecting'
    system(`Connecting to ${url} …`)

    const c = new Connection(url, {
      onOpen: () => {
        status = 'connected'
        system('Connected.')
        c.hello({ width: measureWidth(), height: 40 })
      },
      onText: (f) => push(f),
      onPrompt: (f) => (prompt = f),
      onEcho: (m) => {
        masked = m
        inputEl?.focus()
      },
      onGmcp: (pkg, data) => {
        const prev = packages[pkg]
        packages[pkg] = { data, count: (prev?.count ?? 0) + 1 }
      },
      onServerError: (m) => system(`Server refused a frame: ${m}`),
      onProtocolError: (m) => system(m),
      onBye: (reason) => system(reason ? `Server closed the session: ${reason}` : 'Server closed the session.'),
      onClose: (code, reason) => {
        status = 'disconnected'
        masked = false
        prompt = null
        system(`Disconnected (${code}${reason ? `: ${reason}` : ''}).`)
      },
      onUnknown: (f) => system(`Ignoring an unrecognised frame type: ${f.type}`),
    })
    conn = c
    c.connect()
  }

  function submit(ev) {
    ev.preventDefault()
    if (status !== 'connected') return
    const text = entry
    // A blank line is meaningful — the login flow and the pager both use it —
    // so it is sent rather than swallowed.
    conn.send(text)

    // Terminal clients get a visual break for free: the command you typed is
    // echoed on its own line, so each response starts against something. Here
    // the entry box is outside the scrollback, so without this every response
    // butts straight up against the last one. Two spacers in a row would be a
    // gap nobody asked for, so a repeat is skipped.
    if (!lines.length || !lines[lines.length - 1].spacer) {
      push({ text: '', spacer: true })
    }

    // Never put a password in the history.
    if (!masked && text.trim()) {
      if (history[history.length - 1] !== text) history.push(text)
      if (history.length > 200) history.shift()
    }
    histAt = null
    entry = ''
  }

  function onKey(ev) {
    if (ev.key !== 'ArrowUp' && ev.key !== 'ArrowDown') return
    if (masked || history.length === 0) return
    ev.preventDefault()
    if (ev.key === 'ArrowUp') {
      histAt = histAt === null ? history.length - 1 : Math.max(0, histAt - 1)
      entry = history[histAt]
    } else {
      if (histAt === null) return
      histAt += 1
      if (histAt >= history.length) {
        histAt = null
        entry = ''
      } else {
        entry = history[histAt]
      }
    }
  }

  // A window resize is this transport's NAWS. Repeating `hello` is exactly how
  // the protocol expects a client to report one.
  $effect(() => {
    const onResize = () => {
      if (status === 'connected') conn?.hello({ width: measureWidth() })
    }
    window.addEventListener('resize', onResize)
    return () => window.removeEventListener('resize', onResize)
  })

  // Toggling the side panel changes the usable width just as a resize does.
  $effect(() => {
    showGmcp
    if (status === 'connected') conn?.hello({ width: measureWidth() })
  })
</script>

<!-- 100 monospace characters, off-screen, for the width measurement. -->
<span class="ruler" bind:this={rulerEl} aria-hidden="true"
  >0123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789</span
>

<div class="app" bind:this={outerEl}>
  <header>
    <strong>Oxigeon</strong>
    <input
      bind:value={url}
      spellcheck="false"
      aria-label="Server URL"
      disabled={status === 'connected'}
    />
    <span class="status {status}">{status}</span>
    {#if status === 'connected'}
      <button onclick={() => conn.close()}>Disconnect</button>
    {:else}
      <button onclick={connect} disabled={status === 'connecting'}>Connect</button>
    {/if}
    <button onclick={() => (showGmcp = !showGmcp)}>{showGmcp ? 'Hide' : 'Show'} GMCP</button>
  </header>

  <main>
    <div class="pane">
      <Output {lines} {prompt} />
      <form onsubmit={submit}>
        <input
          bind:this={inputEl}
          bind:value={entry}
          onkeydown={onKey}
          type={masked ? 'password' : 'text'}
          autocomplete="off"
          spellcheck="false"
          placeholder={status === 'connected'
            ? masked
              ? 'password (hidden)'
              : 'type a command'
            : 'not connected'}
          disabled={status !== 'connected'}
          aria-label="Command"
        />
        <button type="submit" disabled={status !== 'connected'}>Send</button>
      </form>
    </div>
    {#if showGmcp}
      <Gmcp {packages} />
    {/if}
  </main>
</div>

<style>
  .ruler {
    position: absolute;
    visibility: hidden;
    white-space: pre;
    pointer-events: none;
    font-family: var(--ox-mono);
    font-size: 14px;
  }

  .app {
    display: flex;
    flex-direction: column;
    height: 100%;
    position: relative;
  }

  header {
    display: flex;
    gap: 10px;
    align-items: center;
    padding: 8px 14px;
    border-bottom: 1px solid var(--ox-line);
    background: var(--ox-panel);
  }
  header input { flex: 1; min-width: 0; }

  .status {
    font-size: 11px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--ox-dim);
  }
  .status.connected { color: var(--ox-good); }
  .status.disconnected { color: var(--ox-bad); }

  main {
    flex: 1;
    display: flex;
    min-height: 0;
  }
  .pane {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  form {
    display: flex;
    gap: 8px;
    padding: 10px 14px;
    border-top: 1px solid var(--ox-line);
    background: var(--ox-panel);
  }
  form input { flex: 1; min-width: 0; }
</style>
