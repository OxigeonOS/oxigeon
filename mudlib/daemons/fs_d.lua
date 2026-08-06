-- mudlib/daemons/fs_d.lua — Where each session is standing in the file tree.
--
-- The file efuns are jailed to two roots, and a builder needs to move around
-- them: `ls`, `cd`, `pwd`, `cat`. This holds the one piece of state that needs
-- holding — a current directory per session — and does the path arithmetic.
--
-- ─── The virtual root ────────────────────────────────────────────────────────
--
-- The shell presents **one namespace with two mount points**:
--
--     /                      the two roots
--     /game/areas/crypt/     the content layer — where OLC writes
--     /mudlib/lib/           the system layer
--
-- Not a merged view, which is what `list_dir` gives an unprefixed path. The
-- merge is right for discovery — the layer that would be `require`d is the layer
-- that is reported — and wrong for a person deciding where a file goes:
-- `game/cmds/verify.lua` shadowing `mudlib/cmds/admin/verify.lua` shows as one
-- entry, so you edit the copy that is not loaded and nothing happens.
--
-- The virtual path is also the *permission* path. `config/permissions.toml`
-- keys on `/game/areas`, and that is what `ls` prints and what `cd` accepts.
-- One namespace for the config, the shell and the error messages, rather than
-- three that have to be kept in step.
--
-- ─── Why not on olc_d ────────────────────────────────────────────────────────
--
-- `cd` is useful without a build session — an admin reading `/mudlib/logs` — and
-- hanging it off OLC state would mean `olc done` silently threw your working
-- directory away. Separate daemons for separate lifetimes.
--
-- See docs/src/lua-api/file-access.md.

local M = {}

--- The mount points, in display order. `list_dir` and the efun prefixes know
--- these names; so does `permissions.toml`.
M.ROOTS = { "game", "mudlib" }

-- session_id → { cwd = "/game/areas/crypt", previous = "/" }
M._sessions = {}

local function state_of(session_id)
    local s = M._sessions[session_id]
    if not s then
        s = { cwd = "/", previous = "/" }
        M._sessions[session_id] = s
    end
    return s
end

--- Is this the name of a mount point?
--- @param name string
--- @return boolean
function M.is_root(name)
    for _, r in ipairs(M.ROOTS) do
        if r == name then return true end
    end
    return false
end

--- Split a virtual path into its segments, dropping empties.
--- @param path string
--- @return table  array of segment strings
local function segments(path)
    local out = {}
    for seg in tostring(path):gmatch("[^/]+") do
        out[#out + 1] = seg
    end
    return out
end

--- Pull leading `-x` flags off an argument string.
---
--- Only *leading* flags, and each has to be its own token. A path is the whole
--- remainder rather than the next word, because a directory may contain a space
--- and splitting on one would make it unreachable rather than merely awkward —
--- and a filename beginning with `-` past the first token stays a filename.
---
---     M.flags("-n /mudlib/lib/strings.lua")  ->  { n = true }, "/mudlib/lib/strings.lua"
---     M.flags("areas -l")                    ->  {},           "areas -l"
---
--- @param args_str string|nil
--- @return table flags  { [letter] = true }
--- @return string rest  the remainder, trimmed
function M.flags(args_str)
    local rest = tostring(args_str or ""):gsub("^%s+", "")
    local out = {}
    while true do
        local flag, tail = rest:match("^%-(%a+)%s+(.*)$")
        if not flag then
            -- A trailing flag with nothing after it: `ls -l`.
            flag = rest:match("^%-(%a+)%s*$")
            if not flag then break end
            tail = ""
        end
        for letter in flag:gmatch("%a") do out[letter] = true end
        rest = tail:gsub("^%s+", "")
    end
    return out, (rest:gsub("%s+$", ""))
end

--- Where this session is standing. Defaults to the virtual root.
--- @param session_id string
--- @return string
function M.cwd(session_id)
    return state_of(session_id).cwd
end

--- Resolve a path the way a shell would, without touching the filesystem.
---
--- Lexical on purpose: this is the user interface, and the efun jail is the
--- boundary. Defence in depth — a bug here is a wrong listing, not an escape.
---
---   /absolute   from the virtual root
---   relative    from the cwd
---   .           the cwd
---   ..          the parent; `..` at `/` stays at `/`
---   ~           your OLC area's directory while building, else `/`
---   -           the previous directory
---
--- `..` above a mount point lands at `/`, which lists the mount points. It does
--- not fall out of the tree, because there is nothing above the tree.
--- @param session_id string
--- @param path string|nil
--- @return string  a normalized virtual path, always starting with `/`
function M.resolve(session_id, path)
    local s = state_of(session_id)
    path = tostring(path or "")

    if path == "" or path == "." then return s.cwd end
    if path == "-" then return s.previous end

    local base
    if path == "~" or path:sub(1, 2) == "~/" then
        base = segments(M.home(session_id))
        path = path:sub(2)
    elseif path:sub(1, 1) == "/" then
        base = {}
    else
        base = segments(s.cwd)
    end

    for _, seg in ipairs(segments(path)) do
        if seg == "." then
            -- nothing
        elseif seg == ".." then
            table.remove(base)
        else
            base[#base + 1] = seg
        end
    end

    if #base == 0 then return "/" end
    return "/" .. table.concat(base, "/")
end

--- The directory `~` means: the area you are building, or the virtual root.
--- @param session_id string
--- @return string
function M.home(session_id)
    if DAEMON and DAEMON.olc and DAEMON.olc.is_active(session_id) then
        local state = DAEMON.olc.get_state(session_id)
        if state and state.area_name then
            return "/game/areas/" .. state.area_name
        end
    end
    return "/"
end

--- Move. Refuses a path that is not a directory, so a mistyped `cd` fails where
--- it was typed rather than making the next `ls` mysteriously empty.
--- @param session_id string
--- @param path string
--- @return string|nil resolved, string|nil why
function M.chdir(session_id, path)
    local target = M.resolve(session_id, path)

    if target ~= "/" then
        local segs = segments(target)
        if not M.is_root(segs[1]) then
            return nil, "'" .. segs[1] .. "' is not a root. The roots are: "
                .. table.concat(M.ROOTS, ", ")
        end
        -- A mount point itself always exists; anything deeper has to be listed.
        if #segs > 1 and not M.is_dir(target) then
            return nil, "no such directory: " .. target
        end
    end

    local s = state_of(session_id)
    s.previous = s.cwd
    s.cwd = target
    return target
end

--- Turn a virtual path into the rooted form the file efuns take.
---
---   /game/areas/crypt/rooms.lua  ->  game:areas/crypt/rooms.lua
---
--- Always rooted, never bare: an unprefixed efun path means "the mudlib" for a
--- write and "game first" for a read, and a shell that leaned on either default
--- would show one file and edit another.
--- @param virtual string
--- @return string|nil efun_path, string|nil why
function M.to_efun_path(virtual)
    local segs = segments(virtual)
    if #segs == 0 then return nil, "the virtual root is not a file" end
    if not M.is_root(segs[1]) then
        return nil, "'" .. tostring(segs[1]) .. "' is not a root"
    end
    local root = table.remove(segs, 1)
    return root .. ":" .. table.concat(segs, "/")
end

--- List a virtual directory, or nil if it is not one.
---
--- `/` is answered from `M.ROOTS` rather than from the filesystem — there is no
--- directory holding the two roots, and inventing one would mean the shell
--- could show you a place the efuns cannot reach.
--- @param virtual string
--- @return table|nil  array of { name, is_dir, size, root }
function M.list(virtual)
    if virtual == "/" then
        local out = {}
        for _, r in ipairs(M.ROOTS) do
            out[#out + 1] = { name = r, is_dir = true, size = 0, root = r }
        end
        return out
    end

    local efun_path, why = M.to_efun_path(virtual)
    if not efun_path then return nil, why end
    if type(list_dir) ~= "function" then return nil, "list_dir is unavailable" end

    local ok, entries = pcall(list_dir, efun_path)
    if not ok or type(entries) ~= "table" then return nil, "no such directory" end
    return entries
end

--- Is this virtual path a directory we can list?
--- @param virtual string
--- @return boolean
function M.is_dir(virtual)
    if virtual == "/" then return true end
    local segs = segments(virtual)
    if #segs == 1 and M.is_root(segs[1]) then return true end
    return M.list(virtual) ~= nil
end

--- The permission this session is missing to read this path, or nil.
---
--- Two questions in one: what does the rule demand, and do you hold it. Split
--- across two call sites they drift; the answer callers want is "may I".
--- @param session_id string
--- @param virtual string
--- @param op string  "read" | "write"
--- @return string|nil  the permission that is missing
function M.missing_permission(session_id, virtual, op)
    if type(dir_permission) ~= "function" then return nil end
    local needed = dir_permission(virtual, op)
    if not needed then return nil end
    if type(has_permission) ~= "function" then return nil end
    if has_permission(session_id, needed) then return nil end
    return needed
end

--- Forget a session. Called from on_disconnect.
--- @param session_id string
function M.cleanup(session_id)
    M._sessions[session_id] = nil
end

log("debug", "FS_D: daemon loaded")

return M
