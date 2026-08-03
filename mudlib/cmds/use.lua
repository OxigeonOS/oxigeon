-- mudlib/cmds/use.lua — Use an item for whatever it does.
--
-- The generic verb behind `Item.on_use`, which was declared and never called
-- because nothing could reach an item to call it with. What "use" *means* is
-- the item's business: a lantern lights, a lockpick picks, a scroll reads. The
-- command's job is to find the thing, check it can be used, call the hook and
-- fire the event.
--
-- `drink` remains its own verb. A potion is drunk, not used, and the
-- `drinkable` component already knows how.

local Carry     = require('lib.carry')
local Container = require('lib.container')
local Object    = require('lib.object')

local M = {}
M.name = 'use'
M.aliases = { 'u' }
M.category = 'items'
M.summary = 'Use an item.'
M.usage = {
    "use <item>            whatever it does",
    "use <item> on <target>",
}
M.permission = nil

local function split_on(args_str)
    local what, target = args_str:match("^(.-)%s+on%s+(.+)$")
    if what then return what, target end
    return args_str, nil
end

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not args_str or args_str == "" then
        player:send("{cyan}Use what?{/}")
        return
    end

    local what, target = split_on(args_str)

    local entry, item = Carry.find(player, what,
        { inventory = true, room = true, equipped = true })
    if not entry then
        player:send("{red}You have no " .. what .. ".{/}")
        return
    end

    -- A container's "use" is opening it, which is what a player means when they
    -- type `use chest`. Handled here rather than as a separate verb, so a chest
    -- and a lantern answer the same word.
    if Container.is(item) and type(item.on_use) ~= "function" then
        local closed = Container.is_closed(item, entry.id)
        local ok, why = Container.set_closed(item, entry.id, not closed)
        if not ok then
            player:send("{red}" .. why .. "{/}")
            return
        end
        local name = item.short or entry.template
        player:send((closed and "You open " or "You close ") .. name .. ".")
        player:message_room(player.name .. (closed and " opens " or " closes ") .. name .. ".")
        return
    end

    if type(item.on_use) ~= "function" then
        player:send("{red}You cannot think of anything to do with "
            .. (Object.resolve(item.short, item) or "that") .. ".{/}")
        return
    end

    local ran, result = Carry.fire_hook(item, "on_use", player.char_id, target)
    if not ran then
        player:send("{red}Nothing happens.{/}")
        return
    end
    -- A hook that returns a string has said what happened; one that returns
    -- nothing has already sent whatever it wanted to.
    if type(result) == "string" then player:send(result) end

    if DAEMON and DAEMON.event then
        pcall(DAEMON.event.emit, "item.used", {
            char_id     = player.char_id,
            instance_id = entry.id,
            template_id = entry.template,
            target      = target,
        })
    end
end

return M
