-- mudlib/daemons/prompt_d.lua — Prompt rendering daemon
-- Handles per-character prompt templates and dynamic variable resolution.
-- Variables: %h (hp), %H (max hp), %m (mp), %M (max mp), %g (gold),
--           %x (xp), %l (level), %r (room), %n (name), %% (literal %)

local M = {}

local DEFAULT_TEMPLATE = "> "

-- ─── Template storage ────────────────────────────────────────────────────────
-- Templates are persisted via player.custom.prompt_template

--- Get the current prompt template for a character.
-- @param char_id number
-- @return string
function M.get_template(char_id)
    if not DAEMON or not DAEMON.character then return DEFAULT_TEMPLATE end
    local ok, player = pcall(DAEMON.character.get, char_id)
    if ok and player and player.custom and player.custom.prompt_template then
        return player.custom.prompt_template
    end
    return DEFAULT_TEMPLATE
end

--- Set a custom prompt template for a character.
-- Pass nil to reset to default.
-- @param char_id number
-- @param template_str string|nil
function M.set_template(char_id, template_str)
    if not DAEMON or not DAEMON.character then return end
    local ok, player = pcall(DAEMON.character.get, char_id)
    if ok and player then
        player.custom = player.custom or {}
        player.custom.prompt_template = template_str
    end
end

-- ─── Variable resolution ─────────────────────────────────────────────────────

--- A stat as the player experiences it, buffs included.
---
--- Reading `player.stats.max_hp` directly would show the stored number, which
--- for a derived trait is not stored at all and for a buffed one is the wrong
--- answer. This is the prompt, so it renders on every command — TRAIT_D's memo
--- is what keeps that cheap.
local function stat(player, id, fallback)
    if player.trait then
        local ok, v = pcall(player.trait, player, id)
        if ok and type(v) == "number" then return tostring(v) end
    end
    local raw = player.stats and player.stats[id]
    return tostring(type(raw) == "number" and raw or fallback)
end

local function resolve_var(var, player, char_id)
    if var == "h" then return stat(player, "hp", 0) end
    if var == "H" then return stat(player, "max_hp", 0) end
    if var == "m" then return stat(player, "mp", 0) end
    if var == "M" then return stat(player, "max_mp", 0) end
    if var == "g" then return tostring(player.gold or 0) end
    if var == "x" then return tostring(player.xp or 0) end
    if var == "l" then return stat(player, "level", 1) end
    if var == "n" then return tostring(player.name or "Someone") end
    if var == "r" then
        if DAEMON.world then
            local ok, room = pcall(DAEMON.world.get_character_room_obj, char_id)
            if ok and room and room.short then
                local Object = require('lib.object')
                return Object.resolve(room.short, room) or "Nowhere"
            end
        end
        return "Nowhere"
    end
    if var == "%" then return "%" end
    -- Unknown variable — return as-is
    return "%" .. var
end

-- ─── Render ──────────────────────────────────────────────────────────────────

--- Render and send the prompt to a session.
-- Resolves variables from the Player object and world state.
-- If OLC mode is active, renders the OLC prompt instead.
-- @param session_id string
function M.render(session_id)
    -- Get session info
    local ok, session = pcall(get_session, session_id)
    if not ok or not session or not session.character_id then
        send_prompt(session_id, DEFAULT_TEMPLATE)
        return
    end

    local char_id = session.character_id

    -- OLC mode override
    if DAEMON.olc and DAEMON.olc.is_active(session_id) then
        local state = DAEMON.olc.get_state(session_id)
        local area_name = state and state.area_name or "unknown"
        send_prompt(session_id, "[OLC " .. area_name .. "] > ")
        return
    end

    -- Get player for variable resolution
    local p_ok, player = pcall(DAEMON.character.get, char_id)
    if not p_ok or not player then
        send_prompt(session_id, DEFAULT_TEMPLATE)
        return
    end

    -- Bring regenerating gauges up to date before showing them. This is the
    -- only place in the game that runs on every single command, which makes it
    -- both the right place to do this and the place where it has to be cheap:
    -- a settle that earned nothing returns without writing anything.
    if DAEMON.trait then pcall(DAEMON.trait.touch, player) end

    local template = M.get_template(char_id)

    -- Resolve variables
    local resolved = template:gsub("%%(.)", function(var)
        return resolve_var(var, player, char_id)
    end)

    send_prompt(session_id, resolved)
end

log("debug", "PROMPT_D: daemon loaded")

return M
