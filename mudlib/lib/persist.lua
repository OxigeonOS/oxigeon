-- mudlib/lib/persist.lua — State that outlives a hot reload.
--
-- A table declared at the top of a daemon is a fresh table every time that file
-- is reloaded. For most daemons that is fine and even desirable. For the ones
-- holding unwritten game state it is data loss: reloading `cache_d` would drop
-- every dirty scope on the floor.
--
-- `set_persistent` writes into `_persistent_store`, a VM global. Hot reload
-- replaces `package.loaded`, never globals, so anything parked there survives
-- — and, because it never goes near JSON, it can hold functions, cycles and
-- object references that the document store could not.
--
-- It does NOT survive a restart. That is the difference between this and
-- durability, and it is the whole reason `cache_d` exists.
--
-- Usage:
--   local S = persist.root("trait_d", 1, function() return { defs = {} } end)
--
-- Bump the version when the shape changes: a reload that finds an older
-- version starts fresh rather than tripping over a layout its code no longer
-- understands.

local M = {}

--- Fetch the daemon's persistent root, creating it on first use.
--- @param key string       unique per daemon; the store is not namespaced
--- @param version number   the shape you expect
--- @param factory function returns a fresh root when there isn't one
--- @return table
function M.root(key, version, factory)
    local ok, existing = pcall(get_persistent, key)
    if ok and type(existing) == "table" and existing.version == version then
        return existing
    end

    local fresh = factory()
    fresh.version = version

    -- Not fatal. `set_persistent` could be permission-gated, and a daemon that
    -- refuses to load is worse than one that forgets on reload — so say so
    -- loudly and carry on.
    local stored, err = pcall(set_persistent, key, fresh)
    if not stored then
        local msg = "PERSIST: set_persistent unavailable for '" .. tostring(key)
            .. "' (" .. tostring(err) .. ") — this daemon's state will NOT survive a hot reload"
        log("error", msg)
        if DAEMON and DAEMON.journal then pcall(DAEMON.journal.error, msg) end
    end
    return fresh
end

return M
