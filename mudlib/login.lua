-- mudlib/login.lua — Login and account creation flow
-- Handles the full login sequence including password masking via ECHO.

local M = {}

-- Per-session login state (cleared on disconnect or successful login)
local login_state = {}

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
            send(session_id, "Password cannot be empty.\r\n\r\nUsername: ")
            state.step = "username"
            state.username = nil
            return
        end

        local account = authenticate(state.username, text)
        if account then
            M.enter_game(session_id, account)
        else
            send(session_id, "Invalid username or password. Please try again.\r\n\r\nUsername: ")
            state.step = "username"
            state.username = nil
        end

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

        local account = create_account(state.username, text)
        if account then
            send(session_id, "Account created! Welcome, " .. state.username .. "!\r\n")
            M.enter_game(session_id, account)
        else
            send(session_id, "Could not create account. The name may already be taken, or the password is too short.\r\n")
            send(session_id, "\r\nUsername: ")
            state.step = "username"
            state.username = nil
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
        send(session_id, "Type 'help' for a list of commands.\r\n")
        send(session_id, "\r\n> ")
    end

    -- Clean up login state
    login_state[session_id] = nil
end

--- Clean up when a session disconnects
function M.cleanup(session_id)
    login_state[session_id] = nil
end

return M
