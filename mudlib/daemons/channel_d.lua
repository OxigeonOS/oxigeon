local M = {}

local _channels = {}

local function log_error(msg)
    log("error", msg)
    if DAEMON and DAEMON.journal then
        local ok = pcall(function() DAEMON.journal.error(msg) end)
    end
end

function M.create(name, config)
    if _channels[name] then
        return false, "Channel already exists"
    end
    _channels[name] = {
        subscribers = {},
        config = config or {}
    }
    return true
end

function M.destroy(name)
    if not _channels[name] then
        return false
    end
    _channels[name] = nil
    return true
end

function M.join(name, char_id)
    local chan = _channels[name]
    if not chan then
        return false, "Channel does not exist"
    end
    chan.subscribers[char_id] = true
    return true
end

function M.leave(name, char_id)
    local chan = _channels[name]
    if not chan then
        return false
    end
    chan.subscribers[char_id] = nil
    return true
end

function M.send(name, sender_char_id, message)
    local chan = _channels[name]
    if not chan then return false end
    
    local sender_name = "Someone"
    local ok, sender = pcall(function() return get_character(sender_char_id) end)
    if ok and sender and sender.name then
        sender_name = sender.name
    end

    local color = chan.config.color or ""
    local reset = color ~= "" and "{/}" or ""
    local tag = "[" .. color .. name:upper() .. reset .. "]"
    local formatted_msg = tag .. " " .. sender_name .. ": " .. message

    local sids_ok, sids = pcall(all_sessions)
    if not sids_ok then return false end
    
    for _, sid in ipairs(sids) do
        local sess_ok, sess = pcall(function() return get_session(sid) end)
        if sess_ok and sess and sess.character_id and chan.subscribers[sess.character_id] then
            local player = get_player(sid)
            if player then
                local send_ok, send_err = pcall(player.send, player, formatted_msg)
                if not send_ok then
                    log_error("channel_d send error: " .. tostring(send_err))
                end
            end
        end
    end
end

function M.list()
    local result = {}
    for name, chan in pairs(_channels) do
        local count = 0
        for _, _ in pairs(chan.subscribers) do
            count = count + 1
        end
        table.insert(result, {name = name, subscriber_count = count, config = chan.config})
    end
    return result
end

function M.get_subscribers(name)
    local chan = _channels[name]
    if not chan then return {} end
    local subs = {}
    for cid, _ in pairs(chan.subscribers) do
        table.insert(subs, cid)
    end
    return subs
end

function M.is_subscribed(name, char_id)
    local chan = _channels[name]
    if not chan then return false end
    return chan.subscribers[char_id] == true
end

-- Initialize default channel
M.create("ooc", {color = "{cyan}", prefix = "[OOC]"})

log("info", "channel_d loaded")
return M
