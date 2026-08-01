local M = {}

local ansi_codes = {
    black = "30", red = "31", green = "32", yellow = "33", blue = "34", magenta = "35", cyan = "36", white = "37",
    bg_black = "40", bg_red = "41", bg_green = "42", bg_yellow = "43", bg_blue = "44", bg_magenta = "45", bg_cyan = "46", bg_white = "47",
    bright_black = "90", bright_red = "91", bright_green = "92", bright_yellow = "93", bright_blue = "94", bright_magenta = "95", bright_cyan = "96", bright_white = "97",
    bold = "1", dim = "2", italic = "3", underline = "4",
    ["/"] = "0"
}

function M.colorize(text)
    if not text then return "" end
    return string.gsub(text, "{(.-)}", function(tag)
        local code = ansi_codes[tag]
        if code then
            return string.char(27) .. "[" .. code .. "m"
        else
            return "{" .. tag .. "}"
        end
    end)
end

function M.strip(text)
    if not text then return "" end
    return string.gsub(text, "{.-}", "")
end

return M
