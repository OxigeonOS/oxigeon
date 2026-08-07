-- mudlib/login.lua — Login and account creation flow
-- Handles the full login sequence including password masking via ECHO.
--
-- `authenticate` and `create_account` are asynchronous: they hand the password
-- to a worker pool and return immediately, because Argon2 takes a few hundred
-- milliseconds and the whole game runs on the Lua thread. The answer arrives
-- later at M.on_result, wired to the driver's on_auth_result hook in init.lua.
-- While a session is waiting, its input is ignored — see M.handle_input.

local M = {}

-- Per-session login state (cleared on disconnect or successful login)
local login_state = {}

local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then
        DAEMON.journal.error(message)
    end
end

--- Send the player back to the username prompt.
local function restart(session_id, state, message)
    if message then send(session_id, message) end
    send(session_id, "\r\nUsername: ")
    state.step = "username"
    state.username = nil
end

--- Display the welcome banner and prompt for a username
function M.greet(session_id)
    local name = config("game.name")
    send(session_id, "\r\n")
    send(session_id, "╔═══════════════════════════════════════════╗\r\n")
    send(session_id, "║                                           ║\r\n")
    send(session_id, "║        Welcome to " .. string.format("%-24s", name) .. "║\r\n")
    send(session_id, "║                                           ║\r\n")
    send(session_id, "╠═══════════════════════════════════════════╣\r\n")
    send(session_id, "║  Type 'new' to create an account          ║\r\n")
    send(session_id, "║  Or enter your username to log in         ║\r\n")
    send(session_id, "╚═══════════════════════════════════════════╝\r\n")
    send(session_id, "\r\nUsername: ")

    login_state[session_id] = { step = "username" }
end

--- Handle input during the login/registration sequence
function M.handle_input(session_id, text)
    local state = login_state[session_id]
    if not state then
        M.greet(session_id)
        return
    end

    -- A hash is in flight for this session. Dropping the line is deliberate:
    -- queueing it would let one connection stack up Argon2 work, which is the
    -- denial of service this whole path exists to prevent.
    if state.step == "waiting" then
        return
    end

    -- Trim input
    text = text:gsub("^%s+", ""):gsub("%s+$", "")

    if state.step == "username" then
        if text:lower() == "new" then
            state.step = "new_username"
            send(session_id, "\r\nChoose a username (letters and numbers only): ")
        elseif text == "" then
            send(session_id, "\r\nUsername: ")
        else
            state.username = text
            state.step = "password"
            start_echo(session_id)
            send(session_id, "\r\nPassword: ")
        end

    elseif state.step == "password" then
        stop_echo(session_id)
        send(session_id, "\r\n")

        if text == "" then
            restart(session_id, state, "Password cannot be empty.\r\n")
            return
        end

        state.step = "waiting"
        send(session_id, "Checking...\r\n")
        authenticate(session_id, state.username, text)

    elseif state.step == "new_username" then
        if text == "" or text:len() < 3 then
            send(session_id, "Username must be at least 3 characters.\r\nChoose a username: ")
            return
        end
        -- Basic validation: alphanumeric only
        if text:match("[^%a%d]") then
            send(session_id, "Username may only contain letters and numbers.\r\nChoose a username: ")
            return
        end
        state.username = text
        state.step = "new_password"
        start_echo(session_id)
        send(session_id, "\r\nChoose a password (min 8 characters): ")

    elseif state.step == "new_password" then
        stop_echo(session_id)
        send(session_id, "\r\n")

        state.step = "waiting"
        send(session_id, "Creating your account...\r\n")
        create_account(session_id, state.username, text)
    end
end

--- Called by the driver when an off-thread hash finishes.
-- @param session_id string
-- @param kind string  "authenticate" or "create_account"
-- @param account table|nil  the account on success
-- @param err string|nil  a player-facing message on failure
function M.on_result(session_id, kind, account, err)
    local state = login_state[session_id]
    if not state then
        -- The player disconnected while the hash was running, or logged in by
        -- another route. Nothing to do, and nowhere to send a message.
        log("debug", "Auth result for " .. tostring(session_id) .. " with no login state")
        return
    end

    if not account then
        restart(session_id, state, (err or "Login failed.") .. "\r\n")
        return
    end

    if kind == "create_account" then
        send(session_id, "Account created! Welcome, " .. tostring(state.username) .. "!\r\n")
    end

    -- enter_game touches the world, character and channel daemons; a failure
    -- in any of them would otherwise leave the session stuck in "waiting"
    -- with no prompt and no way out.
    local ok, enter_err = pcall(M.enter_game, session_id, account)
    if not ok then
        log_error("LOGIN: enter_game failed for session " .. tostring(session_id)
            .. ": " .. tostring(enter_err))
        send(session_id, "\r\nSomething went wrong entering the game. Please try again.\r\n")
        if login_state[session_id] then
            restart(session_id, login_state[session_id])
        end
    end
end

--- Transition to the in-game state
function M.enter_game(session_id, account)
    local chars = get_characters(account.id)
    local char

    if #chars == 0 then
        -- Auto-create a character matching the account name
        char = create_character(account.id, account.username)
        if char then
            send(session_id, "\r\nCharacter '" .. char.name .. "' created.\r\n")
        else
            send(session_id, "\r\nError creating character. Please contact an admin.\r\n")
            return
        end
    else
        char = chars[1]
    end

    if char then
        -- Step 1: mark session as authenticated (enforces multisession policy)
        authenticate_session(session_id, account.id)
        -- Step 2: mark session as playing with the chosen character
        enter_game_session(session_id, account.id, char.id)
        send(session_id, "\r\nWelcome back, " .. char.name .. "!\r\n")

        -- Step 3: place character in the world and show the room
        if DAEMON and DAEMON.world then
            -- No fallback room id. Naming one game's room in a mudlib file is
            -- how a second game inherits it silently; `game.start_room` is
            -- required, and a game that has not set it should find out here
            -- rather than by putting everyone in a room that does not exist.
            local start = config("game.start_room")
            if start then
                DAEMON.world.place_character(char.id, start)
            else
                log("error", "LOGIN: game.start_room is not set in server.toml")
                if DAEMON.journal then
                    pcall(DAEMON.journal.error,
                        "LOGIN: game.start_room is not set — nobody can be placed")
                end
            end

            -- Step 4: load character into a Player object
            if DAEMON.character then
                local load_ok, player = pcall(DAEMON.character.load, char.id)
                if load_ok and player then
                    -- Link the Player to this session
                    player.session_id = session_id

                    -- Restore saved channel subscriptions
                    if DAEMON.channel and player.channels then
                        local ch_ok, ch_err = pcall(DAEMON.channel.restore_channels,
                            char.id, player.channels)
                        if not ch_ok then
                            log("error", "Failed to restore channels for char "
                                .. tostring(char.id) .. ": " .. tostring(ch_err))
                        end
                    end
                else
                    log("error", "Failed to load Player for char "
                        .. tostring(char.id) .. ": " .. tostring(player))
                    if DAEMON.journal then
                        DAEMON.journal.error("Player load failed for char "
                            .. tostring(char.id) .. ": " .. tostring(player))
                    end
                end
            end

            local room = DAEMON.world.get_room(start)
            if room then
                send(session_id, "\r\n" .. room:get_appearance(session_id) .. "\r\n")
            else
                log("error", "Start room '" .. start
                    .. "' not found — character placed but room missing")
                send(session_id, "\r\nYou are floating in the void. The start room could not be found.\r\n")
            end
        else
            log("warn", "World daemon not loaded — character not placed in world")
            send(session_id, "Type 'help' for a list of commands.\r\n")
        end

        -- Announce the arrival. `player.login` is a documented event name that
        -- nothing emitted, which is the same shape `room.entered` had: the
        -- convention existed and the event did not, so a game daemon wanting to
        -- do something once per login had nothing to listen to.
        --
        -- Last, and after the room has been shown: a listener that writes to
        -- the player should appear below the room description rather than above
        -- it, and one that raises must not have prevented them arriving.
        if DAEMON and DAEMON.event then
            local ok, err = pcall(DAEMON.event.emit, "player.login", {
                char_id    = char.id,
                session_id = session_id,
                account_id = account.id,
                name       = char.name,
            })
            if not ok then
                log("error", "LOGIN: a player.login listener raised: " .. tostring(err))
                if DAEMON.journal then
                    pcall(DAEMON.journal.error, "player.login listener raised: " .. tostring(err))
                end
            end
        end

        if DAEMON and DAEMON.prompt then
            DAEMON.prompt.render(session_id)
        else
            send_prompt(session_id, "> ")
        end
    end

    -- Clean up login state
    login_state[session_id] = nil
end

--- Clean up when a session disconnects
function M.cleanup(session_id)
    login_state[session_id] = nil
end

return M
