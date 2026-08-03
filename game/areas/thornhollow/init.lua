-- game/areas/thornhollow/init.lua — One area, three room files.
--
-- `ROOM_D.merge` joins the arrays and keeps the first `_meta` it finds, so the
-- area has one identity, one entry in `areas`, and one reset — while three
-- builders can work on it without touching each other's file.
--
-- This file is *not* the area data. It returns the merged array, and
-- `game/init.lua` does the registering, so the split between "what the area is"
-- and "when it is loaded" stays where it is everywhere else.

local ROOM_D = DAEMON and DAEMON.room

local meta = {
    _meta = {
        name   = "thornhollow",
        title  = "Thornhollow",
        author = "Oxigeon",
        level  = "1-10",
        status = "live",
    },
}

if not ROOM_D then
    log("error", "thornhollow: room_d is not loaded — the town cannot be built")
    return meta
end

return ROOM_D.merge(
    meta,
    require('areas.thornhollow.square'),
    require('areas.thornhollow.market'),
    require('areas.thornhollow.undercroft')
)
