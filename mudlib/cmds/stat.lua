-- mudlib/cmds/stat.lua — The short admin look at a player, room, mob or item.
--
-- The old hardcoded `help` advertised this command for a long time before it
-- existed. `objdump` is the exhaustive version and prints a screen and a half;
-- this is the one you use while walking around, and it accepts anything with an
-- id rather than only a player or a room.

local M = {}
M.name = 'stat'
M.aliases = { '@stat' }
M.category = 'admin'
M.summary = 'Inspect a player, room, mob or item.'
M.usage = {
    "stat                 the room you are standing in",
    "stat <player>        an online character",
    "stat <room_id>       any room, by its dotted id",
    "stat <mob>           a creature in this room, by name prefix",
    "stat <item>          an item template id",
}
M.permission = 'admin'

local function line(label, value)
    return string.format("  {yellow}%-12s{/} %s", label, tostring(value))
end

--- An online character by exact name. Prefix matching is deliberately not used
--- for players: guessing between two names that share three letters is a bug
--- waiting to happen on a moderation command.
local function find_player(name)
    local want = name:lower()
    for _, sid in ipairs(all_sessions()) do
        local s = get_session(sid)
        if s and s.state == "playing" and s.character_id then
            local p = DAEMON.character and DAEMON.character.get(s.character_id)
            if p and p.name and p.name:lower() == want then return p end
        end
    end
    return nil
end

local function show_player(viewer, p)
    local room_id = DAEMON.world and DAEMON.world.get_character_room(p.char_id) or "?"
    local lines = {
        "{cyan}Player{/} " .. p.name .. (p.title and (" — " .. p.title) or ""),
        line("Char id", p.char_id),
        line("Account", p.account_id),
        line("Room", room_id),
        line("Level", p:trait("level")),
        line("Health", p:trait("hp") .. " / " .. p:trait("max_hp")),
        line("Mana", p:trait("mp") .. " / " .. p:trait("max_mp")),
        line("Gold", p.gold or 0),
        line("Experience", p.xp or 0),
    }

    if DAEMON.trait then
        local present = DAEMON.trait.present(p)
        local categories = DAEMON.trait.categories(p)
        lines[#lines + 1] = line("Traits",
            #present .. " present (" .. table.concat(categories, ", ") .. ") — see `traits`")
    end
    if DAEMON.effect then
        local ok, active = pcall(DAEMON.effect.active, p)
        if ok then lines[#lines + 1] = line("Effects", #active .. " active — see `effects`") end
    end

    lines[#lines + 1] = line("Inventory", #(p.inventory or {}) .. " item(s)")
    viewer:send_lines(lines)
end

local function show_room(viewer, room)
    local exits = {}
    for dir, target in pairs(room.exits or {}) do
        exits[#exits + 1] = dir .. " -> "
            .. (type(target) == "table" and tostring(target.target) or tostring(target))
    end
    table.sort(exits)

    local occupants = room.get_characters and room:get_characters() or {}

    local lines = {
        "{cyan}Room{/} " .. room.id,
        line("Short", require('lib.object').resolve(room.short, room) or "(none)"),
        line("Light", room.light_level or 0),
        line("Exits", #exits > 0 and table.concat(exits, ", ") or "(none)"),
        line("Occupants", #occupants),
    }

    local state = get_all_object_state(room.id)
    if state and next(state) then
        local keys = {}
        for k in pairs(state) do keys[#keys + 1] = tostring(k) end
        table.sort(keys)
        lines[#lines + 1] = line("State", table.concat(keys, ", "))
    else
        lines[#lines + 1] = line("State", "(none)")
    end

    viewer:send_lines(lines)
end

local function show_mob(viewer, mob)
    local lines = {
        "{cyan}Mob{/} " .. (mob.short or mob.id),
        line("Instance", mob.id),
        line("Template", mob.template or "(none)"),
        line("Health", mob:trait("hp") .. " / " .. mob:trait("max_hp")),
        line("Level", mob:trait("level")),
        line("Faction", mob.faction or "(none)"),
        line("Flags", table.concat({
            mob.aggressive and "aggressive" or nil,
            mob.stationary and "stationary" or nil,
            mob.unique and "unique" or nil,
        }, ", ")),
    }
    if mob.loot_table and #mob.loot_table > 0 then
        lines[#lines + 1] = line("Loot", #mob.loot_table .. " entr(y/ies)")
    end
    viewer:send_lines(lines)
end

local function show_item(viewer, item)
    local lines = {
        "{cyan}Item{/} " .. (item.short or item.id),
        line("Id", item.id),
        line("Weight", item.weight or 0),
        line("Value", item.value or 0),
        line("Slot", item.slot or "(not equippable)"),
    }
    if item.weapon then
        lines[#lines + 1] = line("Weapon",
            string.format("%d-%d %s, speed %s", item.weapon.min or 0, item.weapon.max or 0,
                item.weapon.damage_type or "physical", tostring(item.weapon.speed or 1)))
    end
    if item.armour then
        lines[#lines + 1] = line("Armour",
            string.format("defense %s, %s", tostring(item.armour.defense or 0),
                item.armour.armor_type or "light"))
    end
    local requires = require('lib.requires').describe(item)
    if requires then lines[#lines + 1] = line("Requires", requires) end
    viewer:send_lines(lines)
end

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    -- No argument: the room you are standing in, which is what you want nine
    -- times out of ten and saves typing an id you would have to look up.
    if not args_str or args_str == "" then
        local room = DAEMON.world and DAEMON.world.get_character_room_obj(player.char_id)
        if not room then
            player:send("{red}You are nowhere the world knows about.{/}")
            return
        end
        show_room(player, room)
        return
    end

    local target = find_player(args_str)
    if target then show_player(player, target) return end

    local room = DAEMON.world and DAEMON.world.get_room(args_str)
    if room then show_room(player, room) return end

    -- A creature standing here, by name prefix — the same match `attack` uses,
    -- so `stat lur` and `attack lur` never disagree about which one they meant.
    if DAEMON.mobs and DAEMON.mobs.find_in_room then
        local room_id = DAEMON.world and DAEMON.world.get_character_room(player.char_id)
        if room_id then
            local ok, mob = pcall(DAEMON.mobs.find_in_room, room_id, args_str)
            if ok and mob then show_mob(player, mob) return end
        end
    end

    if DAEMON.items then
        local ok, item = pcall(DAEMON.items.get, args_str)
        if ok and item then show_item(player, item) return end
    end

    player:send("{red}Nothing called '" .. args_str .. "' — no player, room, creature or item.{/}")
end

return M
