-- mudlib/cmds/put.lua — Into a container.
--
--   put coin in backpack
--   put all in chest
--
-- `in` and `into` both work, because both are what people type.

local Carry     = require('lib.carry')
local Container = require('components.container')

local M = {}
M.name = 'put'
M.aliases = {}
M.category = 'items'
M.summary = 'Put something into a container.'
M.usage = {
    "put <item> in <container>",
    "put all in <container>",
}
M.permission = nil

local function split_in(args_str)
    local what, where = args_str:match("^(.-)%s+into%s+(.+)$")
    if not what then what, where = args_str:match("^(.-)%s+in%s+(.+)$") end
    return what, where
end

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not args_str or args_str == "" then
        player:send("{cyan}Usage: put <item> in <container>{/}")
        return
    end
    if not (DAEMON and DAEMON.items) then
        player:send("{red}You cannot put things anywhere here.{/}")
        return
    end

    local what, container_name = split_in(args_str)
    if not what or what == "" or not container_name then
        player:send("{cyan}Usage: put <item> in <container>{/}")
        return
    end

    local centry, citem = Carry.find(player, container_name, { inventory = true, room = true })
    if not centry then
        player:send("{red}You see no " .. container_name .. " here.{/}")
        return
    end
    if not Container.is(citem) then
        player:send("{red}That is not a container.{/}")
        return
    end
    local cname = citem.short or centry.template

    local function put_one(entry, item)
        if entry == centry then
            player:send("{red}It will not hold itself.{/}")
            return false
        end
        local name = item.display_name and item:display_name() or item.short or entry.template
        local ok, why = Carry.put_in(player, entry, item, centry, citem)
        if not ok then
            player:send("{red}" .. (why or "It will not fit.") .. "{/}")
            return false
        end
        player:send("You put " .. name .. " in " .. cname .. ".")
        player:message_room(player.name .. " puts " .. name .. " in " .. cname .. ".")
        return true
    end

    if what:lower() == "all" then
        if not player.inventory or #player.inventory == 0 then
            player:send("You are not carrying anything.")
            return
        end
        local carried = {}
        for i, entry in ipairs(player.inventory) do carried[i] = entry end

        local moved = 0
        for _, entry in ipairs(carried) do
            local item = DAEMON.items.resolve(entry)
            -- Stops at the first refusal rather than reporting a full container
            -- once per remaining item, which is a screen of the same line.
            if item then
                if not put_one(entry, item) then break end
                moved = moved + 1
            end
        end
        if moved == 0 then player:send("Nothing went in.") end
        return
    end

    local entry, item = Carry.find(player, what, { inventory = true, room = true })
    if not entry then
        player:send("{red}You have no " .. what .. ".{/}")
        return
    end
    put_one(entry, item)
end

return M
