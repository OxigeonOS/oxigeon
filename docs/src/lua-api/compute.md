# Compute — Running Long Lua Off the Game Thread

The whole game runs on one Lua thread. Anything expensive on it — a pathfind
across a large map, generating an area, a simulation pass — freezes every
connected player for its duration.

`compute()` hands the work to a pool of worker threads, each with its own
LuaJIT VM, and delivers the answer later through a mudlib hook. The submitting
command returns immediately.

```toml
# config/server.toml — off by default
[compute]
enabled = true
```

## The shape of it

```lua
-- game/compute/pathfind.lua — a compute module. Pure Lua, no efuns.
local M = {}

function M.route(args)
    -- args.graph was copied in; nothing here can see the live world.
    local path = dijkstra(args.graph, args.from, args.to)
    return { path = path, cost = #path }
end

return M
```

```lua
-- game/cmds/travel.lua — on the game thread
local id, err = compute("compute.pathfind", "route",
    { graph = DAEMON.world.exit_graph(), from = here, to = dest },
    { tag = session_id, deadline_ms = 3000 })

if not id then
    send(session_id, "Cannot plan a route right now: " .. err)
    return
end
send(session_id, "Plotting a course...")
```

```lua
-- mudlib/init.lua
function on_compute_result(id, ok, value, err, meta)
    local sid = meta.tag
    if not ok then
        send(sid, "Route planning failed: " .. tostring(err))
        return
    end
    -- The world moved while this ran. ALWAYS revalidate before acting.
    if not DAEMON.world.still_connected(value.path) then
        send(sid, "The way has changed since you set out.")
        return
    end
    follow(sid, value.path)
end
```

> [!IMPORTANT]
> That revalidation is the most important line on this page. A compute result
> is **a proposal about a world that has since changed**, never an
> authoritative fact. Nothing stopped the game while the job ran — that is the
> entire point — so anything it computed may be stale by the time you have it.

## Reference

### `compute(module, fn_name, args, opts) → id | nil, err`

Queues `module.fn_name(args)` on a worker. Returns a job id as a **string**, or
`nil` plus a message.

`opts` is optional: `tag` is any value, echoed back untouched in `meta.tag`;
`deadline_ms` overrides `[compute] default_deadline_ms`.

The module must live under a configured root (`compute/` by default), relative
to `game/` or `mudlib/`. It is loaded with `require`, so it is cached after the
first call and lives in a normal, reviewable, version-controlled file.

### `compute_cancel(id) → boolean`

Asks a job to stop; returns whether it was still live. A queued job is dropped
immediately. A *running* job only stops if `[compute] instruction_limit` is set
— otherwise there is no hook to notice, and the job can only stop itself by
polling `compute_cancelled()`.

### `on_compute_result(id, ok, value, err, meta)` *(mudlib hook)*

Called for every job. `meta` carries:

| Field | Meaning |
|---|---|
| `kind` | `"ok"`, `"error"`, `"load_error"`, `"timeout"`, `"cancelled"`, `"budget"`, `"refused"` |
| `tag` | whatever you passed in `opts.tag` |
| `module`, `fn` | what was run |
| `queued_ms` | time spent waiting for a worker |
| `run_ms` | time spent running |

> [!NOTE]
> **If `compute` returns an id, exactly one `on_compute_result` fires for it.
> If it returns `nil`, none does.**
>
> `nil` is only for mistakes correct code never makes — compute disabled, a
> module outside the roots, arguments that cannot be copied. Everything
> operational, including a full queue, arrives through the hook, because the
> cleanup a mudlib does is identical for all of them.

### Inside a job

Three intrinsics, and nothing else beyond the standard library:

| Function | Purpose |
|---|---|
| `compute_log(level, message)` | Buffered and written to the journal when the job finishes |
| `compute_deadline_ms()` | Milliseconds left, so a job can return a partial answer |
| `compute_cancelled()` | Whether `compute_cancel` was called |

`compute_log` matters more than it looks: **a debug adapter cannot attach to a
worker VM**, so without it there is no way to see inside a job at all.

## A worker VM has no efuns

Not one. No `send`, no `get_player`, no `db_get`, no `set_object_state`, no
file access. Arguments in, a value out.

This is not caution for its own sake — each candidate fails concretely:

- **`get_current_session` is a thread-local.** On a worker it is permanently
  `nil`, so session-scoped efuns would not error, they would quietly return
  nothing. Silent wrongness is the worst failure available.
- **`set_object_state` and `get_persistent` are Lua globals.** A second VM gets
  its own empty copies: writes vanish, reads lie, and it looks like it works
  right up until it matters.
- **The session table and the database genuinely are thread-safe**, so `send()`
  *would* work. That is the trap, not the reassurance — it would interleave a
  worker's output with the game thread's and let a job watch the world change
  underneath it.

Arguments being the only channel in is a feature: it forces a job to state what
it depends on, which makes it reproducible and testable as plain Lua with no
driver at all.

## What crosses the boundary

Values are copied, not shared — mlua's `Lua` is `!Send`, so nothing else is
possible. Numbers, strings, booleans, `nil`, and tables all survive exactly,
including tables that are both a list and a map, and integer keys.

Functions, coroutines and userdata are **refused** at the call site rather than
silently becoming `nil`. So are cycles and anything past `max_arg_depth` /
`max_arg_nodes`.

> [!TIP]
> Copying happens on the game thread, so it is the one part of a compute job a
> player waits for. Prefer small arguments and small results: ship a seed
> rather than a snapshot, return a plan rather than a world, and `require`
> static data inside the compute module rather than marshalling it.

## When a job never comes back

Rust cannot kill a thread. With the compiler on — the default — a job that
ignores its deadline runs until the process exits, burning one worker.

What you get is that it costs **one worker, not the game**:

- the deadline still tells the caller, so nothing waits forever;
- the pool degrades to refusing new jobs rather than blocking;
- `server_info().compute.wedged` counts the loss, and the driver logs it at
  error level.

Setting `[compute] instruction_limit` makes workers recoverable, and makes
deadlines and `compute_cancel` enforceable rather than advisory — at the cost
of the compiler in that VM, which is worth about 2.1× on the arithmetic-heavy
work that belongs here. See
[Performance & the JIT Trade-off](./performance.md).

## Reloads

`reload` recycles every worker: each rebuilds its VM before its next job. There
is no partial reload, because a worker holds no state anyone may depend on —
which is exactly the property the game VM lacks.

A compute module that fails to load fails **loudly on every job** rather than
silently keeping the old version. That is a deliberate divergence from
`hot_reload`: keeping a stale copy would mean fixing a syntax error, seeing the
same wrong answer, and having no idea why.

## Configuration

See the `[compute]` block in `config/server.toml`, which documents every key
inline. The ones worth thinking about are `workers` (keep it well under your
core count — compute competes with the game thread), `queue_depth` (shallow on
purpose: a deep queue only produces answers that arrive after they stopped
mattering), and `instruction_limit`.

## When not to use it

Below about 10 ms of work the round trip and the copying cost more than they
save, and you have turned a synchronous answer into an asynchronous one, which
costs you code. This facility protects **game-thread latency**, not total CPU.
