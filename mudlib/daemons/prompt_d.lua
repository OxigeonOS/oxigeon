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

local function resolve_var(var, player, char_id)
    if var == "h" then return tostring(player.stats and player.stats.hp or 0) end
    if var == "H" then return tostring(player.stats and player.stats.max_hp or 0) end
    if var == "m" then return tostring(player.stats and player.stats.mp or 0) end
    if var == "M" then return tostring(player.stats and player.stats.max_mp or 0) end
    if var == "g" then return tostring(player.gold or 0) end
    if var == "x" then return tostring(player.xp or 0) end
    if var == "l" then return tostring(player.stats and player.stats.level or 1) end
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

    local template = M.get_template(char_id)

    -- Resolve variables
    local resolved = template:gsub("%%(.)", function(var)
        return resolve_var(var, player, char_id)
    end)

    send_prompt(session_id, resolved)
end

log("debug", "PROMPT_D: daemon loaded")

return M
