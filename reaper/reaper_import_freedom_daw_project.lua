---
--- Generated by Luanalysis
--- Created by kevin.
--- DateTime: 14/10/22 9:33 am
---

local luna = require 'lunajson'
local reaper = reaper
local bpm = 140.0
local timesig_num = 4
local timesig_denom = 4

-- -- got this from stack overflow
-- local function split(pString, pPattern)
--     local Table = {}  -- NOTE: use {n = 0} in Lua-5.0
--     local fpat = "(.-)" .. pPattern
--     local last_end = 1
--     local s, e, cap = pString:find(fpat, 1)
--     while s do
--        if s ~= 1 or cap ~= "" then
--       table.insert(Table,cap)
--        end
--        last_end = e+1
--        s, e, cap = pString:find(fpat, last_end)
--     end
--     if last_end <= #pString then
--        cap = pString:sub(last_end)
--        table.insert(Table, cap)
--     end
--     return Table
--  end


--     local reaper_track = reaper.GetTrack(0, 0)
--     local _, track_state_chunk = reaper.GetTrackStateChunk(reaper_track, "", false)

--     print(track_state_chunk)

--     local lines = split(track_state_chunk, "\n")
--     local start_of_vst = nil
--     local end_of_vst = nil
--     local instrument_vst_chunk = ""
--     local found_start_line_less_128_chars = false
--     local found_end_line_less_128_chars = false
--     for line_number, line in ipairs(lines) do

--     --  print(line .. "\n")
--       end_of_vst = string.match(line, "^(>)$")

--       if found_start_line_less_128_chars and start_of_vst ~= nil and line:len() < 128 then
--         break
--       end

--       if end_of_vst ~= nil then
--         break
--       end

--       if found_start_line_less_128_chars == false and  start_of_vst ~= nil and line:len() < 128 then
--         found_start_line_less_128_chars = true
--       elseif found_start_line_less_128_chars and start_of_vst ~= nil then
--         instrument_vst_chunk = instrument_vst_chunk .. line

--         --reaper.ShowConsoleMsg(line:len() .. "\n")
--       end

--       if start_of_vst == nil then
--         start_of_vst = string.match(line, "^(<VST.*)$")
--       end

--     end

--     local count = 0
--     local line = ""
--     for c in instrument_vst_chunk:gmatch(".") do
--       line = line .. c

--       if line:len() > 127 then
--         print(line)
--         line = ""
--       end
--     end

--     --print(instrument_vst_chunk)



local function read_file()
    local file_contents = ""
    -- for line in io.lines("/tmp/import_into_reaper.fdaw") do
    for line in io.lines("/tmp/lua.fdaw") do
        file_contents = file_contents .. line .. "\n"
    end

    return luna.decode(file_contents)
end


local function handle_track_media_items(reaper_track, freedomdaw_track)
    for _, riff_ref in ipairs(freedomdaw_track["riff_refs"]) do
        -- look up the riff
        local linked_to_riff = nil
        for _, riff in ipairs(freedomdaw_track["riffs"]) do
            if riff_ref["linked_to"] == riff["uuid"] then
                print("Found a linked to riff")
                linked_to_riff = riff
                break
            end
        end

        if linked_to_riff ~= nil then
            local riff_ref_position = riff_ref["position"]
            local riff_ref_position_in_secs = reaper.TimeMap2_beatsToTime(0, riff_ref_position)
            local riff_length_in_secs = reaper.TimeMap2_beatsToTime(0, linked_to_riff["length"])
            local media_item = reaper.CreateNewMIDIItemInProj(reaper_track, riff_ref_position_in_secs, riff_ref_position_in_secs + riff_length_in_secs)
            local media_item_take = reaper.GetMediaItemTake(media_item, 0)

            -- reaper.SetMediaItemInfo_Value(media_item, "B_LOOPSRC", number newvalue)

            for _, event in ipairs(linked_to_riff["events"]) do
                if event["Note"] ~= nil then
                    local note_position = event["Note"]["position"]
                    local note = event["Note"]["note"]
                    local velocity = event["Note"]["velocity"]
                    local duration = event["Note"]["duration"]
                    print("Note: position=" .. note_position .. ", duration=" .. duration)

                    local riff_ref_position_in_secs = reaper.TimeMap2_QNToTime(0, riff_ref_position)
                    local position_in_secs = reaper.TimeMap2_QNToTime(0, note_position)
                    local duration_in_secs = reaper.TimeMap2_QNToTime(0, duration)
                    print("Note: position in secs=" .. position_in_secs .. ", duration in secs=" .. duration_in_secs)

                    local note_start_ppq = reaper.MIDI_GetPPQPosFromProjTime(media_item_take, riff_ref_position_in_secs + position_in_secs)
                    local note_end_ppq = reaper.MIDI_GetPPQPosFromProjTime(media_item_take, riff_ref_position_in_secs + position_in_secs + duration_in_secs)
                    print("Note: ppq start=" .. note_start_ppq .. ", ppq end=" .. note_end_ppq)


                    if not reaper.MIDI_InsertNote(media_item_take, false, false, note_start_ppq, note_end_ppq, 0, note, velocity) then
                        print("Failed to insert note!")
                    end
                end
            end

            reaper.MIDI_Sort(media_item_take)
        end
    end
end

-- got this from stack overflow
local function split(pString, pPattern)
    local Table = {}  -- NOTE: use {n = 0} in Lua-5.0
    local fpat = "(.-)" .. pPattern
    local last_end = 1
    local s, e, cap = pString:find(fpat, 1)
    while s do
       if s ~= 1 or cap ~= "" then
      table.insert(Table,cap)
       end
       last_end = e+1
       s, e, cap = pString:find(fpat, last_end)
    end
    if last_end <= #pString then
       cap = pString:sub(last_end)
       table.insert(Table, cap)
    end
    return Table
 end

local function handle_preset(freedomdaw_audio_plugin, reaper_track, fx_index)

    if freedomdaw_audio_plugin["preset_data"] ~= nil then
        local _, track_state_chunk = reaper.GetTrackStateChunk(reaper_track, "", false)
        local lines = split(track_state_chunk, "\n")

    else
        for index, parameter in ipairs(freedomdaw_audio_plugin["parameters"]) do
            print(parameter["index"] .. ", " .. parameter["name"] .. ", " .. parameter["value"])
            reaper.TrackFX_SetParamNormalized(reaper_track, fx_index, parameter["index"], parameter["value"])
        end
    end
end


local function handle_track_effects(reaper_track, freedomdaw_track)
    for effect_number, effect in ipairs(freedomdaw_track["effects"]) do
        reaper.TrackFX_AddByName(reaper_track, effect["name"], false, 1)
        handle_preset(effect, reaper_track, effect_number + 1)
    end
end


local function handle_instrument(reaper_track, freedomdaw_track)
    local track_instrument_plugin_index = reaper.TrackFX_AddByName(reaper_track, freedomdaw_track["instrument"]["name"], false, 1)

    handle_preset(freedomdaw_track["instrument"], reaper_track, track_instrument_plugin_index)
end

local function handle_instrument_track(reaper_track, freedomdaw_track_type)

    reaper.GetSetMediaTrackInfo_String(reaper_track, "P_NAME", freedomdaw_track_type["InstrumentTrack"]["name"], true)

    local mute = freedomdaw_track_type["InstrumentTrack"]["mute"]
    local solo = freedomdaw_track_type["InstrumentTrack"]["solo"]
    local volume = freedomdaw_track_type["InstrumentTrack"]["volume"]
    local pan = freedomdaw_track_type["InstrumentTrack"]["pan"]

    if solo == false then
        solo = 0
    else
        solo = 1
    end

    if mute == false then
        mute = 0
    else
        mute = 1
    end

    reaper.SetMediaTrackInfo_Value(reaper_track, "B_MUTE", mute)
    reaper.SetMediaTrackInfo_Value(reaper_track, "I_SOLO", solo)
    reaper.SetMediaTrackInfo_Value(reaper_track, "D_VOL", volume * 2.0)
    reaper.SetMediaTrackInfo_Value(reaper_track, "D_PAN", pan)

    handle_instrument(reaper_track, freedomdaw_track_type["InstrumentTrack"])
    handle_track_effects(reaper_track, freedomdaw_track_type["InstrumentTrack"])
    handle_track_media_items(reaper_track, freedomdaw_track_type["InstrumentTrack"])
end


local function main()
    local freedomdaw_project = read_file()

    timesig_num = freedomdaw_project["song"]["time_signature_numerator"]
    timesig_denom = freedomdaw_project["song"]["time_signature_denominator"]


    reaper.GetSetProjectInfo_String(0, "PROJECT_NAME", freedomdaw_project["song"]["name"], true)
    bpm = freedomdaw_project["song"]["tempo"]
    reaper.SetCurrentBPM(0, bpm, false)

    reaper.SetTempoTimeSigMarker(0, -1, 0, 0, 0, bpm, timesig_num, timesig_denom, true)

    for track_number, freedomdaw_track_type in ipairs(freedomdaw_project["song"]["tracks"]) do
        reaper.InsertTrackAtIndex(track_number - 1, true)

        local reaper_track = reaper.GetTrack(0, track_number - 1)

        if freedomdaw_track_type["InstrumentTrack"] ~= nil then
            handle_instrument_track(reaper_track, freedomdaw_track_type)
        elseif freedomdaw_track_type["AudioTrack"] ~= nil then
        elseif freedomdaw_track_type["MidiTrack"] ~= nil then
        end
    end

    reaper.UpdateArrange()
end



main()
