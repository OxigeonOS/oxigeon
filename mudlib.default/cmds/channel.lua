local M = {}
M.name = 'channel'
M.aliases = {'chan'}
M.category = 'communication'
M.summary = 'Channel communication commands.'
M.permission = nil

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not args or #args == 0 then
        player:send_lines(
            "Usage:",
            "  channel join <name>",
            "  channel leave <name>",
            "  channel list",
            "  channel <name> <message>"
        )
        return
    end

    local cmd = args[1]:lower()

    if cmd == "join" then
        if not args[2] then
            player:send("Join which channel?")
            return
        end
        local ch_name = args[2]:lower()
        local ok, reason = DAEMON.channel.join(ch_name, player.char_id)
        if ok then
            -- Persist to player's saved channel list
            player.channels = player.channels or {}
            local already = false
            for _, c in ipairs(player.channels) do
                if c == ch_name then already = true; break end
            end
            if not already then
                table.insert(player.channels, ch_name)
            end
            player:send("Joined channel: " .. ch_name)
        else
            player:send(reason or ("Could not join channel '" .. args[2] .. "'."))
        end
    elseif cmd == "leave" then
        if not args[2] then
            player:send("Leave which channel?")
            return
        end
        local ch_name = args[2]:lower()
        local ok, reason = DAEMON.channel.leave(ch_name, player.char_id)
        if ok then
            -- Remove from player's saved channel list
            if player.channels then
                for i, c in ipairs(player.channels) do
                    if c == ch_name then
                        table.remove(player.channels, i)
                        break
                    end
                end
            end
            player:send("Left channel: " .. ch_name)
        else
            player:send(reason or ("Could not leave channel '" .. args[2] .. "'."))
        end
    elseif cmd == "list" then
        local list = DAEMON.channel.list()
        player:send("Channels:")
        for _, info in ipairs(list) do
            local joined = DAEMON.channel.is_subscribed(info.name, player.char_id)
            player:send(string.format("  %-15s [%d subscribers]%s",
                info.name, info.subscriber_count, joined and " (joined)" or ""))
        end
    else
        -- Treat first arg as channel name, rest as message
        local ch = args[1]:lower()
        local msg = args_str:match("^%S+%s+(.*)$")
        if not msg or msg == "" then
            player:send("Send what to channel " .. ch .. "?")
            return
        end
        DAEMON.channel.send(ch, player.char_id, msg)
    end
end

return M
