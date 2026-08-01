-- mudlib/lib/color.lua — ANSI color tag processing
-- Translates {tag} color markup into ANSI escape sequences.
--
-- Supported tag formats:
--   {red}, {green}, {bold}, ...   — Named 16-color and style tags
--   {fg:N}                        — 256-color foreground (N = 0–255)
--   {bg:N}                        — 256-color background (N = 0–255)
--   {/}                           — Reset all attributes
--
-- Named 256-color aliases are also supported for common xterm values:
--   {orange}, {pink}, {salmon}, {azure}, {teal}, {olive}, etc.

local M = {}

local ESC = string.char(27)

-- ─── Standard 16-color + style codes ─────────────────────────────────────────

local ansi_codes = {
    -- Foreground (standard)
    black = "30", red = "31", green = "32", yellow = "33",
    blue = "34", magenta = "35", cyan = "36", white = "37",
    -- Background (standard)
    bg_black = "40", bg_red = "41", bg_green = "42", bg_yellow = "43",
    bg_blue = "44", bg_magenta = "45", bg_cyan = "46", bg_white = "47",
    -- Bright foreground
    bright_black = "90", bright_red = "91", bright_green = "92",
    bright_yellow = "93", bright_blue = "94", bright_magenta = "95",
    bright_cyan = "96", bright_white = "97",
    -- Styles
    bold = "1", dim = "2", italic = "3", underline = "4",
    blink = "5", inverse = "7", strikethrough = "9",
    -- Reset
    ["/"] = "0",
}

-- ─── Named 256-color aliases ─────────────────────────────────────────────────
-- Maps friendly names → xterm-256 color numbers.
-- Use as {orange}, {pink}, etc.

local xterm_names = {
    -- Warm tones
    orange       = 208,  dark_orange  = 166,  light_orange = 214,
    pink         = 211,  hot_pink     = 198,  deep_pink    = 162,
    salmon       = 209,  coral        = 203,  tomato       = 196,
    gold         = 220,  amber        = 214,
    -- Cool tones
    azure        = 39,   sky_blue     = 117,  steel_blue   = 67,
    teal         = 30,   aqua         = 51,   turquoise    = 80,
    slate        = 66,   navy         = 17,   indigo       = 54,
    violet       = 135,  purple       = 129,  lavender     = 183,
    -- Earth tones
    olive        = 142,  lime         = 118,  chartreuse   = 82,
    forest       = 22,   emerald      = 35,   sea_green    = 85,
    brown        = 130,  tan          = 180,  khaki        = 143,
    sienna       = 94,   maroon       = 52,   crimson      = 160,
    -- Grays
    silver       = 7,    gray         = 245,  dark_gray    = 240,
    light_gray   = 250,  charcoal     = 236,
    -- Misc
    cornflower   = 69,   periwinkle   = 104,  rose         = 174,
    peach        = 217,  plum         = 96,   mint         = 121,
    ivory        = 230,  snow         = 255,  midnight     = 17,
}

--- Translate color tags in text to ANSI escape sequences.
-- @param text string  Text containing {tag} markup
-- @return string      Text with tags replaced by ANSI codes
function M.colorize(text)
    if not text then return "" end
    return string.gsub(text, "{(.-)}", function(tag)
        -- 1. Check standard named codes
        local code = ansi_codes[tag]
        if code then
            return ESC .. "[" .. code .. "m"
        end

        -- 2. Check 256-color foreground: {fg:N}
        local n = tag:match("^fg:(%d+)$")
        if n then
            return ESC .. "[38;5;" .. n .. "m"
        end

        -- 3. Check 256-color background: {bg:N}
        n = tag:match("^bg:(%d+)$")
        if n then
            return ESC .. "[48;5;" .. n .. "m"
        end

        -- 4. Check named xterm-256 aliases
        local xterm_n = xterm_names[tag]
        if xterm_n then
            return ESC .. "[38;5;" .. xterm_n .. "m"
        end

        -- 5. Check named xterm-256 background aliases: {bg_orange}, etc.
        local bg_name = tag:match("^bg_(.+)$")
        if bg_name then
            local bg_n = xterm_names[bg_name]
            if bg_n then
                return ESC .. "[48;5;" .. bg_n .. "m"
            end
        end

        -- Unknown tag — leave as-is
        return "{" .. tag .. "}"
    end)
end

--- Strip all color tags from text (for plain-text output or width calculation).
-- @param text string  Text containing {tag} markup
-- @return string      Text with all {tag} sequences removed
function M.strip(text)
    if not text then return "" end
    return string.gsub(text, "{.-}", "")
end

return M
