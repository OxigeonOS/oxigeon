-- mudlib/cmds/examine.lua — Look closely at one thing.
--
-- `look` renders a room and its scenery; this renders an *object* — an item you
-- are carrying, one on the floor, a creature standing here, or another player.
-- Weapons, armour, containers and requirements each contribute their own lines
-- through their own module, so a new component describes itself by existing
-- rather than by editing this file.

local Carry     = require('lib.carry')
local Weapon    = require('lib.weapon')
local Armor     = require('lib.armor')
local Container = require('lib.container')
local Requires  = require('lib.requires')
local Object    = require('lib.object')

local M = {}
M.name = 'examine'
M.aliases = { 'exa', 'x', 'inspect' }
M.category = 'items'
M.summary = 'Look closely at something.'
M.usage = { "examine <item|creature|player>" }
M.permission = nil

local function append(lines, extra)
    for _, line in ipairs(extra or {}) do lines[#lines + 1] = "  " .. line end
end

--- Everything an item has to say about itself.
local function describe_item(player, entry, item, where)
    local lines = {
        "{cyan}" .. (Object.resolve(item.short, item) or "Something") .. "{/}",
        "  " .. (Object.resolve(item.description, item) or "You see nothing special."),
    }

    if (item.weight or 0) > 0 then lines[#lines + 1] = "  Weight: " .. item.weight end
    if (item.value or 0) > 0 then lines[#lines + 1] = "  Value: " .. item.value .. " coins" end
    if item.slot then lines[#lines + 1] = "  Worn on: " .. item.slot end

    append(lines, Weapon.describe(item))
    append(lines, Armor.describe(item))
    append(lines, Container.describe(item, type(entry) == "table" and entry.id))

    -- Traits in the `condition` category — durability, charges. Shown here
    -- rather than in `score`, because `score` names `stat` and an item is not
    -- a character. This is what the third command in the category table is for.
    if DAEMON and DAEMON.trait then
        local ok, conditions = pcall(DAEMON.trait.all, item, "condition")
        if ok and #conditions > 0 then
            for _, t in ipairs(conditions) do
                if not t.hidden then
                    lines[#lines + 1] = "  " .. t.label .. ": " .. tostring(t.value)
                        .. (t.max and (" / " .. tostring(t.max)) or "")
                end
            end
        end
    end

    local requirement = Requires.describe(item)
    if requirement then
        local met = Requires.met(item, player)
        lines[#lines + 1] = (met and "  {green}" or "  {red}") .. requirement .. "{/}"
    end

    if where == "room" then
        lines[#lines + 1] = "  {yellow}(lying here){/}"
    elseif where == "equipment" then
        lines[#lines + 1] = "  {yellow}(you are wearing this){/}"
    end

    player:send_lines(lines)
end

local function describe_mob(player, mob)
    if type(mob.examine) == "function" then
        local ok, text = pcall(mob.examine, mob)
        if ok and type(text) == "string" then
            player:send(text)
            return
        end
    end
    player:send(Object.resolve(mob.short, mob) or "A creature.")
end

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not args_str or args_str == "" then
        player:send("{cyan}Examine what?{/}")
        return
    end

    -- Carried first, then the floor, then what you are wearing. The same order
    -- `Carry.find` uses everywhere, so `examine sword` and `drop sword` never
    -- disagree about which sword.
    local entry, item, where = Carry.find(player, args_str,
        { inventory = true, room = true, equipped = true })
    if entry then
        describe_item(player, entry, item, where)
        return
    end

    local room_id = DAEMON and DAEMON.world and DAEMON.world.get_character_room(player.char_id)

    if room_id and DAEMON.mobs and DAEMON.mobs.find_in_room then
        local ok, mob = pcall(DAEMON.mobs.find_in_room, room_id, args_str)
        if ok and mob then
            describe_mob(player, mob)
            return
        end
    end

    -- Another player standing here.
    if room_id and DAEMON.world then
        local room = DAEMON.world.get_room(room_id)
        local want = args_str:lower()
        for _, char_id in ipairs(room and room.get_characters and room:get_characters() or {}) do
            local other = DAEMON.character and DAEMON.character.get(char_id)
            if other and other.name and other.name:lower():find(want, 1, true) == 1 then
                player:send(other.examine and other:examine() or other.name)
                return
            end
        end
    end

    -- Scenery is the room's business, and `look <keyword>` already does it.
    if room_id and DAEMON.world then
        local room = DAEMON.world.get_room(room_id)
        local scenery = room and room.get_item and room:get_item(args_str)
        if scenery then
            player:send(Object.resolve(scenery, room) or scenery)
            return
        end
    end

    player:send("{red}You see no " .. args_str .. " here.{/}")
end

return M
