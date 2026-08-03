-- mudlib/daemons/shop_d.lua — Buying and selling.
--
-- `Item.value` and `Player:award_gold`/`spend_gold` existed and had no shop to
-- meet. This is that shop, and it is in the mudlib rather than the game layer
-- because the *mechanism* — a stock list, a price, a restock, a ledger — is the
-- same for every game. Which shops exist, what they sell and what they say is
-- content, and lives in an area file.
--
-- ─── Where the state goes ────────────────────────────────────────────────────
--
-- Three different answers, chosen by the rule in state-cache.md:
--
--   the shop's definition   in memory, from the area file — reloadable, and
--                           regenerated on every boot anyway
--   current stock levels    in memory, restocked on a task — a shop that
--                           forgets it sold three daggers over a restart is
--                           behaving correctly, because the restock would have
--                           refilled them
--   the purchase ledger     the document store, write-through — "who bought
--                           what for how much" is the one thing here nobody
--                           wants to lose, and it is the first real consumer of
--                           `db_insert` / `db_find` / `db_incr` outside a test
--
-- ─── Prices ─────────────────────────────────────────────────────────────────
--
-- One number on the item (`value`) and two rates on the shop. A shop sells at
-- `value * buy_rate` and buys at `value * sell_rate`, and `sell_rate` is well
-- under 1 — the gap is the gold sink, and making it a per-shop number rather
-- than a constant is what lets one shop be a bad place to sell.
--
-- Exposes:
--   DAEMON.shop.register(spec) / register_all(list) / get(id) / all()
--   DAEMON.shop.in_room(room_id)  -> spec | nil
--   DAEMON.shop.stock(shop_id)    -> array of { item_id, price, quantity }
--   DAEMON.shop.buy(player, shop_id, name, count)  -> ok, why
--   DAEMON.shop.sell(player, shop_id, name)        -> ok, why
--   DAEMON.shop.restock(shop_id) / restock_all()
--   DAEMON.shop.ledger(filter, opts)

local M = {}

local LEDGER = "shop_ledger"

--- shop_id -> spec
M._shops = {}
--- shop_id -> { item_id -> quantity }. Memory: see the header.
M._stock = {}
--- room_id -> shop_id, so a command can find the shop without being told.
M._by_room = {}

local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then pcall(DAEMON.journal.error, message) end
end

local function log_warn(message)
    log("warn", message)
    if DAEMON and DAEMON.journal then pcall(DAEMON.journal.warn, message) end
end

-- ─── Registration ────────────────────────────────────────────────────────────

--- Declare one shop.
--- @param spec table  { id, name, room, keeper, buy_rate, sell_rate,
---                      buys = { tag | "*" }, stock = { { item, price, count,
---                      restock } } }
--- @return boolean
function M.register(spec)
    if type(spec) ~= "table" or type(spec.id) ~= "string" or #spec.id == 0 then
        log_warn("SHOP_D.register: a shop needs a string id")
        return false
    end
    if type(spec.room) ~= "string" then
        log_warn("SHOP_D.register('" .. spec.id .. "'): a shop needs a room")
        return false
    end

    local shop = {
        id        = spec.id,
        name      = spec.name or spec.id,
        room      = spec.room,
        keeper    = spec.keeper,          -- mob template id, for flavour text
        -- Sells at value * buy_rate; buys at value * sell_rate. The gap is the
        -- gold sink, and it is per shop so one can be a bad place to sell.
        buy_rate  = tonumber(spec.buy_rate) or 1.0,
        sell_rate = tonumber(spec.sell_rate) or 0.35,
        -- Which tags this shop will take. `"*"` takes anything with a value.
        buys      = type(spec.buys) == "table" and spec.buys or { "*" },
        stock     = {},
        greeting  = spec.greeting,
        farewell  = spec.farewell,
    }

    for _, line in ipairs(spec.stock or {}) do
        local item_id = line.item or line.item_id
        if type(item_id) ~= "string" then
            log_warn("SHOP_D.register('" .. spec.id .. "'): a stock line needs an item")
        else
            shop.stock[#shop.stock + 1] = {
                item    = item_id,
                -- nil means "work it out from the item's value", so a price
                -- only appears in an area file when it disagrees with the item.
                price   = tonumber(line.price),
                count   = tonumber(line.count) or 1,
                -- 0 means it never comes back — a unique for sale.
                restock = tonumber(line.restock) or tonumber(line.count) or 1,
            }
        end
    end

    M._shops[shop.id] = shop
    M._by_room[shop.room] = shop.id
    M.restock(shop.id, true)
    return true
end

function M.register_all(list)
    if type(list) ~= "table" then
        log_warn("SHOP_D.register_all: expected an array of specs")
        return 0
    end
    local n = 0
    for _, spec in ipairs(list) do
        if M.register(spec) then n = n + 1 end
    end
    log("info", "SHOP_D: registered " .. n .. " shop(s)")
    return n
end

function M.get(id)         return M._shops[id] end
function M.in_room(room_id) return M._shops[M._by_room[room_id or ""] or ""] end

function M.all()
    local out = {}
    for id in pairs(M._shops) do out[#out + 1] = id end
    table.sort(out)
    return out
end

-- ─── Prices ──────────────────────────────────────────────────────────────────

--- What the shop charges for one of these.
---
--- Always at least 1: a free item is a way to farm gold by selling it back, and
--- "it costs nothing" is never the answer anyone wanted.
--- @return number|nil  nil when the item does not exist
function M.price_of(shop, item_id, line)
    if type(shop) ~= "table" then return nil end
    if line and line.price then return math.max(1, math.floor(line.price)) end

    local item = DAEMON and DAEMON.items and DAEMON.items.get(item_id)
    if not item then return nil end
    return math.max(1, math.floor((item.value or 0) * shop.buy_rate))
end

--- What the shop pays for one of these. Zero means it will not take it.
--- @return number
function M.offer_for(shop, item)
    if type(shop) ~= "table" or type(item) ~= "table" then return 0 end
    if (item.value or 0) <= 0 then return 0 end

    local wanted = false
    for _, tag in ipairs(shop.buys) do
        if tag == "*" then wanted = true break end
        if item.has_tag and item:has_tag(tag) then wanted = true break end
    end
    if not wanted then return 0 end

    return math.max(1, math.floor(item.value * shop.sell_rate))
end

-- ─── Stock ───────────────────────────────────────────────────────────────────

--- Refill a shop.
---
--- Two different numbers, and the difference is the point. `count` is what the
--- shop opens with; `restock` is what comes back. A unique for sale declares
--- `count = 1, restock = 0` — one exists, and when it is gone it is gone.
--- Using one number for both would make "stocks one" and "always has one" the
--- same statement.
---
--- Idempotent and non-destructive in the useful direction: it raises a depleted
--- line and never lowers one a builder has topped up by hand.
--- @param shop_id string
--- @param initial boolean|nil  fill to `count` rather than to `restock`
--- @return number  lines refilled
function M.restock(shop_id, initial)
    local shop = M._shops[shop_id]
    if not shop then return 0 end

    local stock = M._stock[shop_id] or {}
    M._stock[shop_id] = stock

    local n = 0
    for _, line in ipairs(shop.stock) do
        local target = initial and line.count or line.restock
        local have = stock[line.item] or 0
        if have < target then
            stock[line.item] = target
            n = n + 1
        end
    end
    return n
end

function M.restock_all()
    local n = 0
    for id in pairs(M._shops) do n = n + M.restock(id) end
    if n > 0 then log("debug", "SHOP_D: restocked " .. n .. " line(s)") end
    return n
end

--- What is on the shelves right now.
--- @return table  array of { item, item_id, price, quantity }
function M.stock(shop_id)
    local shop = M._shops[shop_id]
    if not shop then return {} end
    local held = M._stock[shop_id] or {}

    local out = {}
    for _, line in ipairs(shop.stock) do
        local item = DAEMON and DAEMON.items and DAEMON.items.get(line.item)
        if item then
            out[#out + 1] = {
                item_id  = line.item,
                item     = item,
                price    = M.price_of(shop, line.item, line),
                quantity = held[line.item] or 0,
            }
        end
    end
    return out
end

--- Find a stock line by name, the same way every other "what did they mean"
--- lookup works — so `buy dag` and `examine dag` cannot disagree.
--- @return table|nil
function M.find_in_stock(shop_id, name)
    if type(name) ~= "string" or #name == 0 then return nil end
    name = name:lower():gsub("_", " ")

    for _, line in ipairs(M.stock(shop_id)) do
        local short = line.item.short
        if type(short) == "string" and short:lower():gsub("_", " "):find(name, 1, true) then
            return line
        end
        if line.item_id:lower():gsub("_", " "):find(name, 1, true) then
            return line
        end
    end
    return nil
end

-- ─── The ledger ──────────────────────────────────────────────────────────────

--- Record one transaction.
---
--- Write-through, in the document store: stock levels can be lost on a restart
--- because a restock would have refilled them anyway, and "who bought what for
--- how much" cannot. It is also what makes an economy question answerable after
--- the fact rather than only while it is happening.
local function record(shop, player, kind, item_id, count, gold)
    if type(db_insert) ~= "function" then return end
    local ok, err = pcall(db_insert, LEDGER, {
        shop    = shop.id,
        kind    = kind,                     -- "buy" | "sell"
        char_id = player.char_id,
        who     = player.name,
        item    = item_id,
        count   = count,
        gold    = gold,
        at      = os_time(),
    })
    if not ok then
        log_error("SHOP_D: could not write the ledger: " .. tostring(err))
    end

    -- A running total per shop, atomically. `db_incr` creates the document if
    -- it is missing, so a new shop needs no bootstrap.
    if type(db_incr) == "function" then
        pcall(db_incr, "shop_totals", shop.id, kind .. "_gold", gold)
        pcall(db_incr, "shop_totals", shop.id, kind .. "_count", count)
    end
end

--- What one shop has taken and paid out, all told.
---
--- Kept by `db_incr` rather than by summing the ledger: two sales in the same
--- tick must not lose one to a read-modify-write, and summing a growing table
--- to print two numbers gets slower every day the shop is open.
--- @param shop_id string
--- @return table  { buy_gold, buy_count, sell_gold, sell_count }
function M.totals(shop_id)
    local empty = { buy_gold = 0, buy_count = 0, sell_gold = 0, sell_count = 0 }
    if type(db_get) ~= "function" then return empty end
    local ok, rec = pcall(db_get, "shop_totals", shop_id)
    if not ok or type(rec) ~= "table" or type(rec.data) ~= "table" then return empty end

    for key in pairs(empty) do
        empty[key] = tonumber(rec.data[key]) or 0
    end
    return empty
end

--- Query the ledger. Thin on purpose — the filter language is the interface,
--- and wrapping it would only hide the half of it nobody had thought to expose.
--- @param filter table|nil
--- @param opts table|nil  { limit, offset, sort, order }
--- @return table  array of records
function M.ledger(filter, opts)
    if type(db_find) ~= "function" then return {} end
    -- `sort` names a *document* field, so it is dotted through `data` — the
    -- record wrapper is `{ id, collection, created_at, data }` and the sort key
    -- has to say which half it means.
    local ok, rows = pcall(db_find, LEDGER, filter or {},
        opts or { sort = "at", order = "desc" })
    if not ok then
        log_error("SHOP_D: could not read the ledger: " .. tostring(rows))
        return {}
    end

    -- Unwrapped, because every caller wants the transaction rather than the
    -- envelope. `id` is folded in, since that is the one part of the envelope
    -- anyone needs.
    local out = {}
    for i, rec in ipairs(rows) do
        local doc = rec.data or {}
        doc.id = rec.id
        out[i] = doc
    end
    return out
end

-- ─── Buying and selling ──────────────────────────────────────────────────────

--- @param player table
--- @param shop_id string
--- @param name string
--- @param count number|nil
--- @return boolean ok, string|nil why, table|nil { item, price, count }
function M.buy(player, shop_id, name, count)
    local shop = M._shops[shop_id]
    if not shop then return false, "There is no shop here." end
    if type(player) ~= "table" then return false, "Nobody is buying." end

    count = math.max(1, math.floor(tonumber(count) or 1))

    local line = M.find_in_stock(shop_id, name)
    if not line then return false, "They do not sell that." end
    if line.quantity <= 0 then return false, "They are out of stock." end
    if line.quantity < count then
        return false, "They only have " .. line.quantity .. " of those."
    end

    local total = line.price * count
    -- `spend_gold` returns false when they cannot afford it, which is the whole
    -- reason it returns anything. Checking first and then spending would be two
    -- reads with a gap between them.
    if not player.spend_gold or not player:spend_gold(total) then
        return false, "You cannot afford that — it costs " .. total .. " gold."
    end

    -- Stock comes off only after the gold has gone, so a failed payment cannot
    -- empty a shelf.
    local held = M._stock[shop_id]
    held[line.item_id] = (held[line.item_id] or 0) - count

    for _ = 1, count do
        player:add_item(line.item_id)
    end

    record(shop, player, "buy", line.item_id, count, total)
    if DAEMON and DAEMON.event then
        pcall(DAEMON.event.emit, "shop.bought", {
            shop = shop.id, char_id = player.char_id,
            item = line.item_id, count = count, gold = total,
        })
    end

    return true, nil, { item = line.item, price = line.price, count = count, total = total }
end

--- @return boolean ok, string|nil why, table|nil { item, gold }
function M.sell(player, shop_id, name)
    local shop = M._shops[shop_id]
    if not shop then return false, "There is no shop here." end
    if not (DAEMON and DAEMON.items) then return false, "Nothing can change hands here." end

    local Carry = require('lib.carry')
    local entry, item = Carry.find(player, name, { inventory = true, room = false })
    if not entry then return false, "You are not carrying that." end

    local offer = M.offer_for(shop, item)
    if offer <= 0 then
        return false, "They have no use for that."
    end

    -- Out of the inventory first: a `award_gold` that succeeded against an item
    -- that then failed to leave would print money.
    local removed = false
    for i, e in ipairs(player.inventory) do
        if e == entry then
            table.remove(player.inventory, i)
            removed = true
            break
        end
    end
    if not removed then return false, "You are not carrying that." end

    -- The instance is gone for good, and so is its object state. A shop is not
    -- a container; what it bought does not come back on the shelf as the same
    -- object, because the shelf holds *counts of a template*.
    pcall(DAEMON.items.destroy, entry)

    player:award_gold(offer)
    record(shop, player, "sell", entry.template, 1, offer)
    if DAEMON and DAEMON.event then
        pcall(DAEMON.event.emit, "shop.sold", {
            shop = shop.id, char_id = player.char_id,
            item = entry.template, gold = offer,
        })
    end

    return true, nil, { item = item, gold = offer }
end

-- ─── The restock task ────────────────────────────────────────────────────────
--
-- Through `task_d` rather than a raw ticker, which is what `task_d` is for and
-- what nothing was using it for. `tasks` lists it, `tasks run shop.restock`
-- fires it now, and `tasks pause` stops it — none of which a bare ticker
-- offers.

if DAEMON and DAEMON.task then
    local seconds = 600
    if type(config) == "function" then
        local ok, configured = pcall(config, "game.shop_restock_seconds")
        if ok and type(configured) == "number" and configured > 0 then
            seconds = configured
        end
    end

    local ok, err = pcall(DAEMON.task.schedule, {
        id       = "shop.restock",
        interval = seconds,
        label    = "Restock every shop",
        func     = function() return M.restock_all() end,
    })
    if not ok then
        log_error("SHOP_D: could not schedule the restock task: " .. tostring(err))
    end
end

log("info", "shop_d daemon loaded")

return M
