-- game/areas/thornhollow/custom.lua — the half of this area that is code.
--
-- OLC regenerates an area's `rooms.lua`, `items.lua` and `mobs.lua` wholesale.
-- This file is the other side of that bargain: it is hand-written, OLC never
-- reads or writes it, and it holds everything that cannot be expressed as data.
--
-- Three shapes go here, and thornhollow currently uses only the third:
--
--   rooms = { ["thornhollow.square"] = { actions = { ... }, description = fn } }
--   items = { ["some_item"] = { on_use = function(item, user_id) ... end } }
--   mobs  = { ["some_mob"]  = { on_death = function(mob) ... end } }
--   on_load = function(area_name) ... end
--
-- Patches are merged over the generated data **before** it is constructed, so a
-- patched `damage` reaches `weapon.from_data` rather than an already-built
-- component. See `mudlib/lib/patch.lua`.
--
-- Thornhollow is hand-authored and not OLC-managed — its rooms carry inline
-- action functions — so nothing here is *required* yet. `on_load` is, though:
-- it is the only place a content hook can live now that `game/init.lua`
-- discovers areas instead of naming them.

return {
    --- Anything this area needs doing once its data has loaded.
    ---
    --- Called last, and called again on every `areas reset`, so it has to be
    --- idempotent. That is not a style note: the reset path exists precisely to
    --- re-run this, and a version that spawned on every call would fill the
    --- undercroft with chests one reset at a time.
    on_load = function(area_name)
        -- The town strongbox is an *instance* in a room rather than a template
        -- in a registry: a particular chest with particular contents, not the
        -- idea of a chest. It lived in `game/init.lua` until areas were
        -- discovered rather than listed, and this is where it belongs — beside
        -- the area it furnishes.
        if not (DAEMON and DAEMON.items) then return end

        local room_id = "thornhollow.undercroft"
        for _, entry in ipairs(DAEMON.items.in_room(room_id)) do
            if entry.template == "vault_chest" then return end
        end

        DAEMON.items.spawn("vault_chest", DAEMON.items.location("room", room_id))
    end,
}
