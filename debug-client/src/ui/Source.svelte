<script>
  // Source with a breakpoint gutter, Lua syntax highlighting, and the vi
  // motions — because that is what your hands already do.
  //
  // **Only the visible lines are built.** This pane used to render the whole
  // file: `game/init.lua` is 1780 lines, each one a gutter button plus a span
  // per syntax run, so opening it built something like sixteen thousand nodes
  // and tokenized every line — synchronously, on the main thread, on every
  // redraw. The tab did not fail, it stopped: no rendering, no keyboard, no
  // way to tell it apart from a hang.
  //
  // The TUI never had this problem because a terminal only ever drew the rows
  // it had. A browser will happily be asked for all of them, so the window is
  // the thing that has to be written down here.

  import { untrack } from 'svelte'
  import { tokenize, withMatches } from '../lib/lua.js'

  let { app } = $props()

  /// Row height in pixels, and the CSS below is held to it. Every line is one
  /// row — `white-space: pre`, no wrapping — so the mapping from scroll offset
  /// to line number is exact rather than estimated.
  const ROW = 18
  /// Rows kept above and below the viewport, so a scroll does not flash.
  const OVERSCAN = 12

  let body = $state(null)
  let promptField = $state(null)
  let logpointField = $state(null)
  let scrollTop = $state(0)
  let viewHeight = $state(600)

  const total = $derived(app.dbg.source.length)

  const window_ = $derived.by(() => {
    const first = Math.max(0, Math.floor(scrollTop / ROW) - OVERSCAN)
    const count = Math.ceil(viewHeight / ROW) + OVERSCAN * 2
    return { first, last: Math.min(total, first + count) }
  })

  /// A fresh Map every time, deliberately.
  ///
  /// `linesFor` answers the stored one, and a `$derived` compares with `===` —
  /// so returning it would recompute and then decline to propagate, and the
  /// gutter would not repaint when a breakpoint was toggled on the line under
  /// the cursor.
  const marks = $derived.by(() => new Map(app.dbg.open ? app.dbg.linesFor(app.dbg.open) : []))

  /// Tokenize the window, not the file.
  const rendered = $derived.by(() => {
    const { first, last } = window_
    const needle = app.dbg.highlight ? app.dbg.search : ''
    const out = []
    for (let i = first; i < last; i++) {
      out.push({
        n: i + 1,
        runs: withMatches(tokenize(app.dbg.source[i] ?? '', app.dbg.blocks[i] ?? null), needle),
      })
    }
    return out
  })

  /// The line the current stop is on, in the file being shown. Only the frame
  /// under the cursor in the stack pane counts — the others are context.
  const stopLine = $derived.by(() => {
    const frame = app.dbg.frames[app.dbg.frameSel]
    if (!frame?.path || !app.dbg.stopped) return null
    return app.dbg.knownPath(frame.path) === app.dbg.open ? frame.line : null
  })

  function onScroll() {
    if (body) scrollTop = body.scrollTop
  }

  /// Bring a line into view, centring it only when it is actually outside.
  /// Scrolling on every cursor move would make `j` feel like the file was
  /// sliding rather than the cursor.
  ///
  /// Where it is *now* is read off the element, not off the `scrollTop` signal.
  /// Reading the signal made this a dependency of the effect below, so every
  /// scroll re-ran it and it hauled the view back to the cursor — the pane
  /// simply would not scroll.
  function ensureVisible(index) {
    if (!body) return
    const top = index * ROW
    const current = body.scrollTop
    if (top < current || top + ROW > current + body.clientHeight) {
      body.scrollTop = Math.max(0, top - body.clientHeight / 2)
      scrollTop = body.scrollTop
    }
  }

  function onKeydown(event) {
    if (app.dbg.focus !== 'source') return
    if (event.target instanceof HTMLInputElement) return
    const { key } = event
    const last = Math.max(0, total - 1)

    if (key === 'ArrowUp' || key === 'k') app.dbg.cursor = Math.max(0, app.dbg.cursor - 1)
    else if (key === 'ArrowDown' || key === 'j') app.dbg.cursor = Math.min(last, app.dbg.cursor + 1)
    else if (key === 'PageUp') app.dbg.cursor = Math.max(0, app.dbg.cursor - 20)
    else if (key === 'PageDown') app.dbg.cursor = Math.min(last, app.dbg.cursor + 20)
    else if (key === 'Home' || key === 'g') app.dbg.cursor = 0
    else if (key === 'End' || key === 'G') app.dbg.cursor = last
    // The two vi prompts. Both edit in the footer row, where the logpoint
    // editor already lives.
    else if (key === ':') app.dbg.sourcePrompt = { kind: 'goto', text: '' }
    else if (key === '/') app.dbg.sourcePrompt = { kind: 'search', text: '' }
    else if (key === 'n') app.dbg.find(true)
    else if (key === 'N') app.dbg.find(false)
    else return

    event.preventDefault()
    app.dbg.changed()
  }

  function promptKey(event) {
    if (event.key === 'Escape') {
      event.preventDefault()
      app.dbg.sourcePrompt = null
      app.dbg.changed()
    } else if (event.key === 'Enter') {
      event.preventDefault()
      app.dbg.commitSourcePrompt()
    }
    event.stopPropagation()
  }

  function logpointKey(event) {
    if (event.key === 'Escape') {
      event.preventDefault()
      app.dbg.cancelLogpoint()
    } else if (event.key === 'Enter') {
      event.preventDefault()
      app.dbg.commitLogpoint()
    }
    event.stopPropagation()
  }

  // Both editors take the keyboard the moment they open.
  $effect(() => {
    if (app.dbg.sourcePrompt) promptField?.focus()
  })
  $effect(() => {
    if (app.dbg.logpointEdit) logpointField?.focus()
  })

  // Follow the cursor. A stop opens whatever file it landed in and puts the
  // cursor on the line, and a pane that did not move would be showing the top
  // of a file the stop is nowhere near.
  //
  // The dependencies are named and the call is untracked, so this reacts to the
  // cursor moving and to nothing else — least of all to its own scrolling.
  $effect(() => {
    app.dbg.open
    const line = app.dbg.cursor
    untrack(() => ensureVisible(line))
  })

  $effect(() => {
    if (!body) return
    const measure = () => {
      viewHeight = body.clientHeight
    }
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(body)
    return () => observer.disconnect()
  })
</script>

<svelte:window on:keydown={onKeydown} />

<section class="pane" class:focused={app.dbg.focus === 'source'} style="grid-area: source">
  <header>
    <span class="path">{app.dbg.open ?? 'no file open'}</span>
    <span class="spacer"></span>
    {#if app.dbg.search && app.dbg.highlight}<span class="dim">/{app.dbg.search}</span>{/if}
    <span class="dim">{total ? `${app.dbg.cursor + 1}/${total}` : ''}</span>
  </header>

  <div class="body" bind:this={body} onscroll={onScroll}>
    {#if total === 0}
      <div class="empty faint">
        {app.dbg.pendingFile ? 'reading…' : 'pick a file, or break somewhere'}
      </div>
    {:else}
      <!-- The canvas is the full height of the file, so the scrollbar tells the
           truth about how big it is; only the window inside it exists. -->
      <div class="canvas" style="height: {total * ROW}px">
        <div class="window" style="transform: translateY({window_.first * ROW}px)">
          {#each rendered as line (line.n)}
            {@const mark = marks.get(line.n)}
            {@const hasMark = marks.has(line.n)}
            <div
              class="line"
              class:cursor={line.n === app.dbg.cursor + 1}
              class:stop={line.n === stopLine}
            >
              <!-- The gutter marks a logpoint ◆ in cyan rather than ● in red,
                   because it will never stop and a gutter that promised
                   otherwise would be lying. -->
              <button
                class="gutter"
                class:bp={hasMark && mark === null}
                class:lp={hasMark && mark !== null}
                onclick={() => {
                  app.dbg.cursor = line.n - 1
                  app.dbg.toggleBreakpoint(line.n)
                }}
                oncontextmenu={(e) => {
                  e.preventDefault()
                  app.dbg.cursor = line.n - 1
                  app.dbg.beginLogpoint(line.n)
                }}
                title={mark ? `logpoint: ${mark}` : 'click to break, right-click for a logpoint'}
              >
                {hasMark ? (mark === null ? '●' : '◆') : line.n}
              </button>
              <span
                class="text"
                onclick={() => {
                  app.dbg.cursor = line.n - 1
                  app.dbg.focus = 'source'
                  app.dbg.changed()
                }}
                role="presentation"
              >
                {#each line.runs as run}<span
                    class={run.kind}
                    class:match={run.match}>{run.text}</span
                  >{/each}
              </span>
            </div>
          {/each}
        </div>
      </div>
    {/if}
  </div>

  <footer>
    {#if app.dbg.logpointEdit}
      <!-- Whatever you type becomes a breakpoint that reports instead of
           stopping. The message is a **template**, not an expression: plain
           text is printed as written, only `{...}` is evaluated. An empty
           message removes it. -->
      <div class="editor">
        <span class="lp">logpoint {app.dbg.logpointEdit.line} ›</span>
        <input
          bind:this={logpointField}
          value={app.dbg.logpointEdit.text}
          oninput={(e) => (app.dbg.logpointEdit.text = e.currentTarget.value)}
          onkeydown={logpointKey}
          placeholder="{'{attacker.name}'} hits {'{target.name}'} for {'{raw}'} — empty removes it"
          spellcheck="false"
        />
      </div>
    {:else if app.dbg.sourcePrompt}
      <div class="editor">
        <span class="sigil">{app.dbg.sourcePrompt.kind === 'goto' ? ':' : '/'}</span>
        <input
          bind:this={promptField}
          value={app.dbg.sourcePrompt.text}
          oninput={(e) => (app.dbg.sourcePrompt.text = e.currentTarget.value)}
          onkeydown={promptKey}
          spellcheck="false"
        />
        <span class="faint">
          {app.dbg.sourcePrompt.kind === 'goto' ? 'line number, or noh' : 'Enter finds, Esc abandons'}
        </span>
      </div>
    {:else}
      <kbd>:</kbd> line <kbd>/</kbd> search <kbd>n</kbd><kbd>N</kbd> next/prev
      <kbd>F9</kbd> break <kbd>^L</kbd> logpoint
    {/if}
  </footer>
</section>

<style>
  .path {
    overflow: hidden;
    text-overflow: ellipsis;
    direction: rtl;
    text-align: left;
  }

  .canvas {
    position: relative;
  }

  .window {
    position: absolute;
    inset: 0 0 auto 0;
    will-change: transform;
  }

  /* Held to ROW in the script above. A wrapped line would break the mapping
     from scroll offset to line number, which is why this never wraps. */
  .line {
    display: flex;
    align-items: center;
    white-space: pre;
    height: 18px;
    line-height: 18px;
  }

  .line.cursor {
    background: #ffffff0e;
  }

  .line.stop {
    background: #d2992244;
  }

  .gutter {
    flex: 0 0 auto;
    width: 5ch;
    height: 100%;
    text-align: right;
    padding: 0 8px 0 0;
    margin: 0;
    border: 0;
    border-radius: 0;
    color: var(--fg-faint);
    background: var(--bg-sunken);
    font-size: 11px;
    line-height: 18px;
  }

  .gutter:hover {
    color: var(--red);
    background: #ffffff10;
  }

  .gutter.bp {
    color: var(--red);
  }

  .gutter.lp {
    color: var(--cyan);
  }

  .text {
    padding: 0 8px;
    flex: 1 1 auto;
  }

  .empty {
    padding: 8px;
  }

  .keyword {
    color: #ff7b72;
  }
  .literal {
    color: #79c0ff;
  }
  .string {
    color: #a5d6ff;
  }
  .comment {
    color: #6a7580;
    font-style: italic;
  }
  .ident {
    color: #d2a8ff;
  }
  .plain {
    color: var(--fg);
  }

  /* Painted on top of the syntax colour, including inside a string or a
     comment — which is usually where you were looking. */
  .match {
    background: #d29922;
    color: #0e1116;
    border-radius: 2px;
  }

  .editor {
    display: flex;
    align-items: baseline;
    gap: 6px;
  }

  .sigil {
    color: var(--accent);
  }

  .lp {
    color: var(--cyan);
  }
</style>
