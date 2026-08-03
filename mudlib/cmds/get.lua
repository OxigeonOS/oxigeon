-- mudlib/cmds/get.lua — Pick something up.
--
--   get lantern              from the floor
--   get all                  everything on the floor
--   get coin from backpack   out of a container, carried or on the floor

local Carry     = require('lib.carry')
local Container = require('lib.container')

local M = {}
M.name = 'get'
M.aliases = { 'take', 'g' }
M.category = 'items'
M.summary = 'Pick something up.'
M.usage = {
    "get <item>              pick it up off the floor",
    "get all                 everything you can carry",
    "get <item> from <container>",
}
M.permission = nil

--- Split "coin from backpack" into its two halves. Returns nil for the
--- container when there is no `from`, which is the common case.
local function split_from(args_str)
    local what, where = args_str:match("^(.-)%s+from%s+(.+)$")
    if what then return what, where end
    return args_str, nil
end

--- The container they named, whether it is carried or on the floor.
--- @return table|nil entry, table|nil item, string|nil why
local function find_container(player, name)
    local entry, item = Carry.find(player, name, { inventory = true, room = true })
    if not entry then
        return nil, nil, "You see no " .. name .. " here."
    end
    if not Container.is(item) then
        return nil, nil, "That is not a container."
    end
    if Container.is_locked(item, entry.id) then
        return nil, nil, "It is locked."
    end
    if Container.is_closed(item, entry.id) then
        return nil, nil, "It is closed."
    end
    return entry, item, nil
end

--- Pick one thing up and say so, in the room as well as to the taker.
local function take_one(player, entry, item, source_description)
    local ok, why = Carry.take(player, entry, item)
    if not ok then
        player:send("{red}" .. (why or "You cannot take that.") .. "{/}")
        return false
    end
    local name = item.display_name and item:display_name() or item.short or entry.template
    player:send("You take " .. name .. (source_description or "") .. ".")
    player:message_room(player.name .. " takes " .. name .. (source_description or "") .. ".")
    return true
end

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not args_str or args_str == "" then
        player:send("{cyan}Get what?{/}")
        return
    end
    if not (DAEMON and DAEMON.items and DAEMON.world) then
        player:send("{red}You cannot pick things up here.{/}")
        return
    end

    local what, container_name = split_from(args_str)

    -- ─── Out of a container ──────────────────────────────────────────────────
    if container_name then
        local centry, citem, why = find_container(player, container_name)
        if not centry then
            player:send("{red}" .. why .. "{/}")
            return
        end

        local cname = citem.short or centry.template
        if what:lower() == "all" then
            local contents = DAEMON.items.contents(centry.id)
            if #contents == 0 then
                player:send("There is nothing in " .. cname .. ".")
                return
            end
            local taken = 0
            for _, child in ipairs(contents) do
                local citem2 = DAEMON.items.resolve(child)
                if citem2 and take_one(player, child, citem2, " from " .. cname) then
                    taken = taken + 1
                end
            end
            if taken == 0 then player:send("You could not take anything.") end
            return
        end

        local entry, item = DAEMON.items.find_in_container(centry.id, what)
        if not entry then
            player:send("{red}There is no " .. what .. " in " .. cname .. ".{/}")
            return
        end
        take_one(player, entry, item, " from " .. cname)
        return
    end

    -- ─── Off the floor ───────────────────────────────────────────────────────
    local room_id = DAEMON.world.get_character_room(player.char_id)
    if not room_id then
        player:send("{red}You are nowhere.{/}")
        return
    end

    if what:lower() == "all" then
        local ground = DAEMON.items.in_room(room_id)
        if #ground == 0 then
            player:send("There is nothing here to take.")
            return
        end
        local taken = 0
        -- Over a snapshot: `Carry.take` moves the item out of the room index,
        -- and mutating the collection you are iterating skips every second one.
        for _, entry in ipairs(ground) do
            local item = DAEMON.items.resolve(entry)
            if item and take_one(player, entry, item, nil) then taken = taken + 1 end
        end
        if taken == 0 then player:send("You could not take anything.") end
        return
    end

    local entry, item = DAEMON.items.find_in_room(room_id, what)
    if not entry then
        player:send("{red}You see no " .. what .. " here.{/}")
        return
    end
    take_one(player, entry, item, nil)
end

return M
