-- Need a new session in Ardour with the "Midi region copies are independent" set to false in the session properties
ardour {
    ["type"]    = "EditorAction",
    name        = "Import RiffDAW Piece",
    license     = "MIT",
    author      = "Kevin Hremeviuc",
    description = [[Imports a RiffDAW piece into an Ardour session]]
}

function factory()
    return function()
        -- https://raw.githubusercontent.com/rxi/json.lua/refs/heads/master/json.lua
        --
        -- json.lua
        --
        -- Copyright (c) 2020 rxi
        --
        -- Permission is hereby granted, free of charge, to any person obtaining a copy of
        -- this software and associated documentation files (the "Software"), to deal in
        -- the Software without restriction, including without limitation the rights to
        -- use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies
        -- of the Software, and to permit persons to whom the Software is furnished to do
        -- so, subject to the following conditions:
        --
        -- The above copyright notice and this permission notice shall be included in all
        -- copies or substantial portions of the Software.
        --
        -- THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
        -- IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
        -- FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
        -- AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
        -- LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
        -- OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
        -- SOFTWARE.
        --

        local json = { _version = "0.1.2" }

        -------------------------------------------------------------------------------
        -- Encode
        -------------------------------------------------------------------------------

        local encode

        local escape_char_map = {
            [ "\\" ] = "\\",
            [ "\"" ] = "\"",
            [ "\b" ] = "b",
            [ "\f" ] = "f",
            [ "\n" ] = "n",
            [ "\r" ] = "r",
            [ "\t" ] = "t",
        }

        local escape_char_map_inv = { [ "/" ] = "/" }
        for k, v in pairs(escape_char_map) do
            escape_char_map_inv[v] = k
        end


        local function escape_char(c)
            return "\\" .. (escape_char_map[c] or string.format("u%04x", c:byte()))
        end


        local function encode_nil(val)
            return "null"
        end


        local function encode_table(val, stack)
            local res = {}
            stack = stack or {}

            -- Circular reference?
            if stack[val] then error("circular reference") end

            stack[val] = true

            if rawget(val, 1) ~= nil or next(val) == nil then
                -- Treat as array -- check keys are valid and it is not sparse
                local n = 0
                for k in pairs(val) do
                    if type(k) ~= "number" then
                        error("invalid table: mixed or invalid key types")
                    end
                    n = n + 1
                end
                if n ~= #val then
                    error("invalid table: sparse array")
                end
                -- Encode
                for i, v in ipairs(val) do
                    table.insert(res, encode(v, stack))
                end
                stack[val] = nil
                return "[" .. table.concat(res, ",") .. "]"

            else
                -- Treat as an object
                for k, v in pairs(val) do
                    if type(k) ~= "string" then
                        error("invalid table: mixed or invalid key types")
                    end
                    table.insert(res, encode(k, stack) .. ":" .. encode(v, stack))
                end
                stack[val] = nil
                return "{" .. table.concat(res, ",") .. "}"
            end
        end


        local function encode_string(val)
            return '"' .. val:gsub('[%z\1-\31\\"]', escape_char) .. '"'
        end


        local function encode_number(val)
            -- Check for NaN, -inf and inf
            if val ~= val or val <= -math.huge or val >= math.huge then
                error("unexpected number value '" .. tostring(val) .. "'")
            end
            return string.format("%.14g", val)
        end


        local type_func_map = {
            [ "nil"     ] = encode_nil,
            [ "table"   ] = encode_table,
            [ "string"  ] = encode_string,
            [ "number"  ] = encode_number,
            [ "boolean" ] = tostring,
        }


        encode = function(val, stack)
            local t = type(val)
            local f = type_func_map[t]
            if f then
                return f(val, stack)
            end
            error("unexpected type '" .. t .. "'")
        end


        function json.encode(val)
            return ( encode(val) )
        end


        -------------------------------------------------------------------------------
        -- Decode
        -------------------------------------------------------------------------------

        local parse

        local function create_set(...)
            local res = {}
            for i = 1, select("#", ...) do
                res[ select(i, ...) ] = true
            end
            return res
        end

        local space_chars   = create_set(" ", "\t", "\r", "\n")
        local delim_chars   = create_set(" ", "\t", "\r", "\n", "]", "}", ",")
        local escape_chars  = create_set("\\", "/", '"', "b", "f", "n", "r", "t", "u")
        local literals      = create_set("true", "false", "null")

        local literal_map = {
            [ "true"  ] = true,
            [ "false" ] = false,
            [ "null"  ] = nil,
        }


        local function next_char(str, idx, set, negate)
            for i = idx, #str do
                if set[str:sub(i, i)] ~= negate then
                    return i
                end
            end
            return #str + 1
        end


        local function decode_error(str, idx, msg)
            local line_count = 1
            local col_count = 1
            for i = 1, idx - 1 do
                col_count = col_count + 1
                if str:sub(i, i) == "\n" then
                    line_count = line_count + 1
                    col_count = 1
                end
            end
            error( string.format("%s at line %d col %d", msg, line_count, col_count) )
        end


        local function codepoint_to_utf8(n)
            -- http://scripts.sil.org/cms/scripts/page.php?site_id=nrsi&id=iws-appendixa
            local f = math.floor
            if n <= 0x7f then
                return string.char(n)
            elseif n <= 0x7ff then
                return string.char(f(n / 64) + 192, n % 64 + 128)
            elseif n <= 0xffff then
                return string.char(f(n / 4096) + 224, f(n % 4096 / 64) + 128, n % 64 + 128)
            elseif n <= 0x10ffff then
                return string.char(f(n / 262144) + 240, f(n % 262144 / 4096) + 128,
                        f(n % 4096 / 64) + 128, n % 64 + 128)
            end
            error( string.format("invalid unicode codepoint '%x'", n) )
        end


        local function parse_unicode_escape(s)
            local n1 = tonumber( s:sub(1, 4),  16 )
            local n2 = tonumber( s:sub(7, 10), 16 )
            -- Surrogate pair?
            if n2 then
                return codepoint_to_utf8((n1 - 0xd800) * 0x400 + (n2 - 0xdc00) + 0x10000)
            else
                return codepoint_to_utf8(n1)
            end
        end


        local function parse_string(str, i)
            local res = ""
            local j = i + 1
            local k = j

            while j <= #str do
                local x = str:byte(j)

                if x < 32 then
                    decode_error(str, j, "control character in string")

                elseif x == 92 then -- `\`: Escape
                    res = res .. str:sub(k, j - 1)
                    j = j + 1
                    local c = str:sub(j, j)
                    if c == "u" then
                        local hex = str:match("^[dD][89aAbB]%x%x\\u%x%x%x%x", j + 1)
                                or str:match("^%x%x%x%x", j + 1)
                                or decode_error(str, j - 1, "invalid unicode escape in string")
                        res = res .. parse_unicode_escape(hex)
                        j = j + #hex
                    else
                        if not escape_chars[c] then
                            decode_error(str, j - 1, "invalid escape char '" .. c .. "' in string")
                        end
                        res = res .. escape_char_map_inv[c]
                    end
                    k = j + 1

                elseif x == 34 then -- `"`: End of string
                    res = res .. str:sub(k, j - 1)
                    return res, j + 1
                end

                j = j + 1
            end

            decode_error(str, i, "expected closing quote for string")
        end


        local function parse_number(str, i)
            local x = next_char(str, i, delim_chars)
            local s = str:sub(i, x - 1)
            local n = tonumber(s)
            if not n then
                decode_error(str, i, "invalid number '" .. s .. "'")
            end
            return n, x
        end


        local function parse_literal(str, i)
            local x = next_char(str, i, delim_chars)
            local word = str:sub(i, x - 1)
            if not literals[word] then
                decode_error(str, i, "invalid literal '" .. word .. "'")
            end
            return literal_map[word], x
        end


        local function parse_array(str, i)
            local res = {}
            local n = 1
            i = i + 1
            while 1 do
                local x
                i = next_char(str, i, space_chars, true)
                -- Empty / end of array?
                if str:sub(i, i) == "]" then
                    i = i + 1
                    break
                end
                -- Read token
                x, i = parse(str, i)
                res[n] = x
                n = n + 1
                -- Next token
                i = next_char(str, i, space_chars, true)
                local chr = str:sub(i, i)
                i = i + 1
                if chr == "]" then break end
                if chr ~= "," then decode_error(str, i, "expected ']' or ','") end
            end
            return res, i
        end


        local function parse_object(str, i)
            local res = {}
            i = i + 1
            while 1 do
                local key, val
                i = next_char(str, i, space_chars, true)
                -- Empty / end of object?
                if str:sub(i, i) == "}" then
                    i = i + 1
                    break
                end
                -- Read key
                if str:sub(i, i) ~= '"' then
                    decode_error(str, i, "expected string for key")
                end
                key, i = parse(str, i)
                -- Read ':' delimiter
                i = next_char(str, i, space_chars, true)
                if str:sub(i, i) ~= ":" then
                    decode_error(str, i, "expected ':' after key")
                end
                i = next_char(str, i + 1, space_chars, true)
                -- Read value
                val, i = parse(str, i)
                -- Set
                res[key] = val
                -- Next token
                i = next_char(str, i, space_chars, true)
                local chr = str:sub(i, i)
                i = i + 1
                if chr == "}" then break end
                if chr ~= "," then decode_error(str, i, "expected '}' or ','") end
            end
            return res, i
        end


        local char_func_map = {
            [ '"' ] = parse_string,
            [ "0" ] = parse_number,
            [ "1" ] = parse_number,
            [ "2" ] = parse_number,
            [ "3" ] = parse_number,
            [ "4" ] = parse_number,
            [ "5" ] = parse_number,
            [ "6" ] = parse_number,
            [ "7" ] = parse_number,
            [ "8" ] = parse_number,
            [ "9" ] = parse_number,
            [ "-" ] = parse_number,
            [ "t" ] = parse_literal,
            [ "f" ] = parse_literal,
            [ "n" ] = parse_literal,
            [ "[" ] = parse_array,
            [ "{" ] = parse_object,
        }


        parse = function(str, idx)
            local chr = str:sub(idx, idx)
            local f = char_func_map[chr]
            if f then
                return f(str, idx)
            end
            decode_error(str, idx, "unexpected character '" .. chr .. "'")
        end


        function json.decode(str)
            if type(str) ~= "string" then
                error("expected argument of type string, got " .. type(str))
            end
            local res, idx = parse(str, next_char(str, 1, space_chars, true))
            idx = next_char(str, idx, space_chars, true)
            if idx <= #str then
                decode_error(str, idx, "trailing garbage")
            end
            return res
        end

        -- end of json.lua

        --local base64 = require 'base64'
        local bpm = 140.0
        local timesig_num = 4
        local timesig_denom = 4


        function read_file()
            print("Reading the RiffDAW JSON file...")
            local file_contents = ""
            for line in io.lines("/tmp/lua.fdaw") do
                file_contents = file_contents .. line .. "\n"
            end

            return json.decode(file_contents)
        end


        function handle_track_riff_references(ardour_track, freedomdaw_track)
            print("Handling riff references")
            local created_regions = {}

            for _, riff_ref in ipairs(freedomdaw_track["riff_refs"]) do
                print("Processing a riff reference")
                -- check if there is an existing region
                local already_created_region = created_regions[riff_ref["linked_to"]]

                print("already_created_region: ", already_created_region)

                -- look up the riff
                local linked_to_riff = nil
                if already_created_region == nil then
                    print("Searching for the linked riff...")
                    for _, riff in ipairs(freedomdaw_track["riffs"]) do
                        if riff_ref["linked_to"] == riff["uuid"] then
                            print("Found a linked to riff")
                            linked_to_riff = riff
                            break
                        end
                    end
                end

                if linked_to_riff ~= nil or already_created_region ~= nil then
                    local riff_ref_position_in_beats = riff_ref["position"]
                    local region_position_in_samples = math.floor(Session:nominal_sample_rate() * riff_ref_position_in_beats / bpm * 60.0)
                    local playlist = ardour_track:playlist ()
                    local proc     = ARDOUR.LuaAPI.nil_proc ()
                    local inter_thread_info = ARDOUR.InterThreadInfo ()

                    print("Session:nominal_sample_rate(): ", Session:nominal_sample_rate(), " riff_ref_position_in_beats: ", riff_ref_position_in_beats, ", region_position_in_samples: ", region_position_in_samples)

                    if linked_to_riff ~= nil then
                        local riff_length_in_samples = math.floor(Session:nominal_sample_rate() * linked_to_riff["length"] / bpm * 60.0)

                        print("linked_to_riff[length]: ", linked_to_riff["length"], ", riff_length_in_samples: ", riff_length_in_samples)

                        local region = ardour_track:bounce_range (0, riff_length_in_samples, inter_thread_info, proc, false, linked_to_riff["name"], false)
                        playlist:add_region (region, Temporal.timepos_t (region_position_in_samples), 1, false)

                        local midi_region = region:to_midiregion ()
                        local midi_region_model = midi_region:midi_source(0):model ()
                        local midi_command = midi_region_model:new_note_diff_command ("Add MIDI Events")

                        print("Looping through the events")
                        for _, event in ipairs(linked_to_riff["events"]) do
                            if event["Note"] ~= nil then
                                local note_position = event["Note"]["position"]
                                local channel = event["Note"]["channel"]
                                local note = event["Note"]["note"]
                                local velocity = event["Note"]["velocity"]
                                local duration = event["Note"]["length"]
                                local new_note = ARDOUR.LuaAPI.new_noteptr (channel, Temporal.Beats.from_double(note_position), Temporal.Beats.from_double(duration), note, velocity)
                                midi_command:add (new_note)
                                print("Note: position=" .. note_position .. ", note=" .. note .. ", duration=" .. duration .. ", velocity=" .. velocity )
                            end
                        end

                        midi_region_model:apply_command (Session, midi_command)
                        created_regions[linked_to_riff["uuid"]] = region
                    else
                        local region = ARDOUR.RegionFactory.clone_region (already_created_region, true, false)
                        playlist:add_region (region, Temporal.timepos_t (region_position_in_samples), 1, false)
                    end
                end
            end
        end

        function handle_track_effects(ardour_track, freedomdaw_effects)
            print("Handling track effects")
            for effect_number, effect in ipairs(freedomdaw_effects) do
                local effect_name = effect["name"]
                local plugin_type = effect["plugin_type"] --  should be using this

                -- find the installed plugin with a similar name
                for installed_plugin in ARDOUR.LuaAPI.list_plugins():iter() do
                    -- might need to check the type
                    if effect_name:find("^" .. installed_plugin.name) ~= nil then
                        local proc = ARDOUR.LuaAPI.new_plugin(Session, installed_plugin.unique_id, installed_plugin.type, "");
                        ardour_track:add_processor_by_index(proc, effect_number + 1, nil, true)
                        break
                    end
                end
            end
        end


        function handle_instrument(ardour_track, freedomdaw_instrument)
            print("Handling instrument")
            local instrument_name = freedomdaw_instrument["name"]
            local plugin_type = freedomdaw_instrument["plugin_type"] --  should be using this

            for installed_plugin in ARDOUR.LuaAPI.list_plugins():iter() do
                -- might need to check the type
                if instrument_name:find("^" .. installed_plugin.name) ~= nil then
                    local proc = ARDOUR.LuaAPI.new_plugin(Session, installed_plugin.unique_id, installed_plugin.type, "");
                    ardour_track:add_processor_by_index(proc, 0, nil, true)
                    break
                end
            end
        end

        function handle_instrument_track(instrument_track)
            local track_name = instrument_track["name"]
            local track_list = Session:new_midi_track(
                    ARDOUR.ChanCount(ARDOUR.DataType ("midi"), 1),
                    ARDOUR.ChanCount(ARDOUR.DataType ("midi"), 1),
                    false,
                    ARDOUR.PluginInfo(),
                    nil,
                    nil,
                    1,
                    track_name,
                    ARDOUR.PresentationInfo.max_order,
                    ARDOUR.TrackMode.Normal,
                    true
            )
            local ardour_track = track_list:back()

            handle_instrument(ardour_track, instrument_track["instrument"])
            handle_track_effects(ardour_track, instrument_track["effects"])
            handle_track_riff_references(ardour_track, instrument_track)
        end


        function main()
            local freedomdaw_project = read_file()

            timesig_num = freedomdaw_project["song"]["time_signature_numerator"]
            timesig_denom = freedomdaw_project["song"]["time_signature_denominator"]
            bpm = freedomdaw_project["song"]["tempo"]

            local temporal_meter = Temporal.Meter(timesig_num, timesig_denom)
            local tempo_map = Temporal.TempoMap.write_copy ()
            local start_time = Temporal.timepos_t (0)
            tempo_map:set_tempo (Temporal.Tempo (bpm, bpm, timesig_denom), start_time)
            tempo_map:set_meter(temporal_meter, start_time)
            Temporal.TempoMap.update (tempo_map)

            for track_number, freedomdaw_track_type in ipairs(freedomdaw_project["song"]["tracks"]) do
                print("Processing track: ", track_number)
                if freedomdaw_track_type["InstrumentTrack"] ~= nil then
                    handle_instrument_track(freedomdaw_track_type["InstrumentTrack"])
                elseif freedomdaw_track_type["AudioTrack"] ~= nil then
                    print("Audio track handling is not implemented.")
                elseif freedomdaw_track_type["MidiTrack"] ~= nil then
                    print("Midi track handling is not implemented.")
                end
            end
        end

        main()

    end
end