-- mudlib/lib/carry.lua — Moving an item between the floor, a character and a
-- container, in one place.
--
-- `get`, `drop`, `put`, `give` and `use` are five commands asking four
-- questions between them: *what did they mean*, *may it move*, *move it*, and
-- *who should be told*. Written five times, those four answers drift — one verb
-- fires `item.dropped` and another does not, one honours a container's capacity
-- and another does not, one calls `on_pickup` and another forgets. So they are
-- written once, here, and the commands are argument parsing and messages.
--
-- ─── Where an item is ────────────────────────────────────────────────────────
--
-- Two homes, and the split is about persistence rather than about taste:
--
--   CARRIED    an entry in `player.inventory`. CHARACTER_D saves that array,
--              and has since before instances existed.
--   IN WORLD   registered in `item_d` with a `location` — `"room:<id>"` for the
--              floor, `"item:<instance id>"` for inside a container. Memory
--              tier: if the server restarts, the sword someone left in the
--              square is gone and the area reset puts the world back.
--
-- A container is the case that spans both. Its *contents* are always indexed
-- under `"item:<its id>"` whichever home the container itself is in, so `put`
-- and `get from` are the same code path as `drop` and `get`. What makes that
-- survive a save is `pack`/`unpack` below.
--
-- ─── Hooks and events ────────────────────────────────────────────────────────
--
-- `Item.on_pickup` / `on_drop` / `on_use` and the events `item.picked_up` /
-- `item.dropped` / `item.used` were all declared and **none of them was ever
-- called**, because nothing could move an item. They fire from here, so every
-- verb fires them the same way.

local Container = require('lib.container')
local Requires  = require('lib.requires')

local M = {}

local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then pcall(DAEMON.journal.error, message) end
end

--- Run one of an item's lfun hooks. Never lets a bad hook take the verb down
--- with it: dropping a sword must work even if the sword's `on_drop` is broken.
--- @param item table       the resolved item
--- @param name string      "on_pickup", "on_drop", "on_use", ...
--- @param ... any          arguments after the item
--- @return boolean ran, any result
function M.fire_hook(item, name, ...)
    local hook = type(item) == "table" and item[name]
    if type(hook) ~= "function" then return false, nil end
    local ok, result = pcall(hook, item, ...)
    if not ok then
        log_error("CARRY: " .. name .. " on '" .. tostring(item.id) .. "' raised: "
            .. tostring(result))
        return false, nil
    end
    return true, result
end

local function emit(event, data)
    if DAEMON and DAEMON.event then
        local ok, err = pcall(DAEMON.event.emit, event, data)
        if not ok then log_error("CARRY: emitting '" .. event .. "' failed: " .. tostring(err)) end
    end
end

-- ─── Finding what they meant ─────────────────────────────────────────────────

--- The item a player named, wherever they can reach it.
---
--- Order is deliberate and matches what a player expects: what you are holding
--- beats what is on the floor, because `drop lantern` should never pick the
--- one at your feet. Equipment is searched last, so `remove` finds a worn item
--- but `drop` prefers the spare in your pack.
---
--- @param player table
--- @param name string
--- @param opts table|nil  { inventory = true, room = true, equipped = false }
--- @return table|nil entry, table|nil item, string|nil where
function M.find(player, name, opts)
    opts = opts or {}
    local want_inv    = opts.inventory ~= false
    local want_room   = opts.room ~= false
    local want_equip  = opts.equipped == true

    if type(name) ~= "string" or #name == 0 then return nil, nil, nil end
    if not (DAEMON and DAEMON.items) then return nil, nil, nil end

    if want_inv and player.inventory then
        local _, item, index = DAEMON.items.find_by_name(name, player.inventory)
        if item then
            return player.inventory[index], item, "inventory"
        end
    end

    if want_room and DAEMON.world then
        local room_id = DAEMON.world.get_character_room(player.char_id)
        if room_id then
            local entry, item = DAEMON.items.find_in_room(room_id, name)
            if entry then return entry, item, "room" end
        end
    end

    if want_equip and player.equipment then
        for _, entry in pairs(player.equipment) do
            if type(entry) == "table" then
                local item = DAEMON.items.resolve(entry)
                local short = item and item.short
                if item and ((type(short) == "string" and short:lower():find(name:lower(), 1, true))
                    or entry.template:lower():gsub("_", " "):find(name:lower(), 1, true)) then
                    return entry, item, "equipment"
                end
            end
        end
    end

    return nil, nil, nil
end

--- Remove an entry from a player's inventory array by identity.
--- @return boolean
local function take_from_inventory(player, entry)
    if type(player.inventory) ~= "table" then return false end
    for i, e in ipairs(player.inventory) do
        if e == entry then
            table.remove(player.inventory, i)
            return true
        end
    end
    return false
end

-- ─── Weight ──────────────────────────────────────────────────────────────────

--- What a character is carrying, including the contents of their containers.
--- @param player table
--- @return number
function M.carried_weight(player)
    if type(player.inventory) ~= "table" or not (DAEMON and DAEMON.items) then return 0 end
    local total = 0
    for _, entry in ipairs(player.inventory) do
        local item = DAEMON.items.resolve(entry)
        if item then
            total = total + Container.total_weight(item, type(entry) == "table" and entry.id, nil)
        end
    end
    return total
end

--- What a character *can* carry.
---
--- A trait, not a constant, so a strength buff or a bag of holding changes it
--- through the ordinary effect path and nothing here has to know. Falls back to
--- a generous constant when the game has defined no such trait, because a game
--- that has not opted into encumbrance should not silently get it.
--- @param player table
--- @return number|nil  nil when the game defines no carry_capacity trait
function M.carry_capacity(player)
    if not (DAEMON and DAEMON.trait and DAEMON.trait.get_def) then return nil end
    if not DAEMON.trait.get_def("carry_capacity") then return nil end

    -- `has`, not `value`. An entity that does not *hold* the trait has no
    -- limit; one that holds it and reads zero has a limit of zero. Those are
    -- different states and the presence rule is what distinguishes them —
    -- reading `value` alone would give a mob with no strength a capacity of
    -- nothing and stop it picking up its own loot.
    if not DAEMON.trait.has(player, "carry_capacity") then return nil end

    -- Through the daemon rather than `player:trait`, because this is reached
    -- from `take` and `give` and the thing on the other end is not always a
    -- full Object. `DAEMON.trait.value` works on any table; `:trait` needs the
    -- metatable.
    return DAEMON.trait.value(player, "carry_capacity")
end

--- Would taking this put them over their limit?
--- @return boolean ok, string|nil why
function M.can_carry(player, item, entry)
    local capacity = M.carry_capacity(player)
    if not capacity then return true end

    local adding = Container.total_weight(item, type(entry) == "table" and entry.id, nil)
    if M.carried_weight(player) + adding > capacity then
        return false, "You cannot carry any more."
    end
    return true
end

-- ─── The four moves ──────────────────────────────────────────────────────────

--- Floor or container -> a character's hands.
---
--- The instance stays registered in `item_d` while carried, with no location:
--- a container someone is carrying must still be findable by its id, because
--- its contents are indexed under it.
--- @param player table
--- @param entry table   the instance
--- @param item table    the resolved item
--- @return boolean ok, string|nil why
function M.take(player, entry, item)
    local ok, why = M.can_carry(player, item, entry)
    if not ok then return false, why end

    if not DAEMON.items.move(entry, nil) then
        return false, "You cannot pick that up."
    end
    player.inventory[#player.inventory + 1] = entry

    M.fire_hook(item, "on_pickup", player.char_id)
    emit("item.picked_up", {
        char_id     = player.char_id,
        instance_id = entry.id,
        template_id = entry.template,
    })
    return true
end

--- A character's hands -> the floor of the room they are in.
--- @return boolean ok, string|nil why
function M.drop(player, entry, item)
    local room_id = DAEMON.world and DAEMON.world.get_character_room(player.char_id)
    if not room_id then return false, "There is nowhere to put it." end

    if not take_from_inventory(player, entry) then
        return false, "You are not carrying that."
    end
    -- Registered on the way out, not on the way in: an entry loaded from a save
    -- has never been in the index, so this is the first time the world learns
    -- it exists.
    M.ensure_registered(entry)
    DAEMON.items.move(entry, DAEMON.items.location("room", room_id))

    M.fire_hook(item, "on_drop", player.char_id)
    emit("item.dropped", {
        char_id     = player.char_id,
        instance_id = entry.id,
        template_id = entry.template,
        room_id     = room_id,
    })
    return true
end

--- Anywhere -> inside a container.
--- @param container_entry table  the container's instance
--- @param container table        the resolved container
--- @return boolean ok, string|nil why
function M.put_in(player, entry, item, container_entry, container)
    if entry == container_entry then
        return false, "It will not hold itself."
    end
    local ok, why = Container.can_accept(container, container_entry.id, item)
    if not ok then return false, why end

    take_from_inventory(player, entry)
    M.ensure_registered(entry)
    if not DAEMON.items.move(entry, DAEMON.items.location("item", container_entry.id)) then
        -- The move refused — almost certainly a containment cycle. Put it back
        -- rather than deleting the player's item, which is the failure mode
        -- that matters here.
        player.inventory[#player.inventory + 1] = entry
        return false, "That would not work."
    end

    emit("item.stored", {
        char_id     = player.char_id,
        instance_id = entry.id,
        template_id = entry.template,
        container   = container_entry.id,
    })
    return true
end

--- One character's hands -> another's.
--- @return boolean ok, string|nil why
function M.give(player, entry, item, recipient)
    if type(recipient) ~= "table" or type(recipient.inventory) ~= "table" then
        return false, "You cannot give it to them."
    end
    local ok, why = M.can_carry(recipient, item, entry)
    if not ok then return false, "They cannot carry any more." end

    if not take_from_inventory(player, entry) then
        return false, "You are not carrying that."
    end
    recipient.inventory[#recipient.inventory + 1] = entry

    emit("item.given", {
        from_char_id = player.char_id,
        to_char_id   = recipient.char_id,
        instance_id  = entry.id,
        template_id  = entry.template,
    })
    return true
end

--- An inventory entry from an old save has no instance id and is not in the
--- index. Give it both, so it can be dropped, put into a container, or opened.
--- @param entry table
--- @return table entry
function M.ensure_registered(entry)
    if type(entry) ~= "table" or not (DAEMON and DAEMON.items) then return entry end
    if type(entry.id) == "string" and DAEMON.items.get_instance(entry.id) then
        return entry
    end

    entry.id = entry.id
        or ("item:" .. (type(uuid) == "function" and uuid() or tostring(entry):sub(8)))
    DAEMON.items._instances[entry.id] = entry
    if entry.location then
        DAEMON.items.move(entry, entry.location)
    end
    return entry
end

-- ─── Persistence ─────────────────────────────────────────────────────────────

--- Fold a container's contents onto the entry, so `to_save` writes them.
---
--- Contents live in `item_d`'s location index, which is memory only — correct
--- for a sword on a floor and wrong for a backpack in somebody's pack. This is
--- called on the way out; `unpack` puts them back in the index on the way in.
--- @param entries table  an inventory array
--- @return table  a copy safe to serialise
function M.pack(entries)
    local out = {}
    if type(entries) ~= "table" or not (DAEMON and DAEMON.items) then return out end

    for i, entry in ipairs(entries) do
        if type(entry) ~= "table" then
            out[i] = entry
        else
            local copy = {}
            for k, v in pairs(entry) do
                -- `location` is where it was in a world that no longer exists
                -- by the time this is read back. It is carried, and that is
                -- what the inventory array already says.
                if k ~= "location" and k ~= "contents" then copy[k] = v end
            end
            if type(entry.id) == "string" then
                local contents = DAEMON.items.contents(entry.id)
                if #contents > 0 then
                    copy.contents = M.pack(contents)
                end
            end
            out[i] = copy
        end
    end
    return out
end

--- Put a loaded inventory's containers back into the location index.
--- @param entries table  the player's inventory array
function M.unpack(entries)
    if type(entries) ~= "table" or not (DAEMON and DAEMON.items) then return end

    for _, entry in ipairs(entries) do
        if type(entry) == "table" then
            M.ensure_registered(entry)
            local contents = entry.contents
            entry.contents = nil
            if type(contents) == "table" then
                for _, child in ipairs(contents) do
                    if type(child) == "table" then
                        M.ensure_registered(child)
                        DAEMON.items.move(child, DAEMON.items.location("item", entry.id))
                        -- Recurse, so a bag inside a chest inside a pack comes
                        -- back whole rather than one level deep.
                        if type(child.contents) == "table" then
                            M.unpack({ child })
                        end
                    end
                end
            end
        end
    end
end

--- Take a character's items out of the world index on logout.
---
--- Without this every item anyone ever carried stays in `_instances` forever,
--- which is L1's shape applied to items: the store is keyed by an id that is
--- never reused, and nothing else prunes it.
--- @param player table
function M.release(player)
    if type(player) ~= "table" or type(player.inventory) ~= "table" then return end
    if not (DAEMON and DAEMON.items) then return end

    local function release_entry(entry)
        if type(entry) ~= "table" or type(entry.id) ~= "string" then return end
        for _, child in ipairs(DAEMON.items.contents(entry.id)) do
            release_entry(child)
        end
        DAEMON.items.destroy(entry)
    end

    for _, entry in ipairs(player.inventory) do release_entry(entry) end
    for _, entry in pairs(player.equipment or {}) do release_entry(entry) end
end

-- ─── Requirements ────────────────────────────────────────────────────────────

--- Re-exported so the equipment verbs have one import rather than two, and so
--- the refusal message is the same one `examine` shows.
M.requirements_met = Requires.met

return M
