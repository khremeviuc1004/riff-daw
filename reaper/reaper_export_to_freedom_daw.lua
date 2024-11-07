---
--- Generated by Luanalysis
--- Created by kevin.
--- DateTime: 13/10/22 6:20 pm

---

luna = require 'lunajson'
uuid = require 'uuid'

local reaper = reaper

local function new_freedomdaw_project()
    return {
        ["song"] = {
            ["name"] = "unkown",
            ["sample_rate"] = 44100.0,
            ["block_size"] = 1024.0,
            ["tempo"] = 140.0,
            ["time_signature_numerator"] = 6,
            ["time_signature_denominator"] = 8,
            ["tracks"] = {},
            ["length_in_beats"] = 592,
            ["loops"] = {0},
            ["riff_sets"] = {0},
            ["riff_sequences"] = {0},
            ["riff_arrangements"] = {0},
            ["samples"] = {}
        }
    }
end

local function new_instrument_track(name)
    return {
        ["InstrumentTrack"] = {
            ["uuid"] = uuid(),
            ["name"] = name,
            ["mute"] = false,
            ["solo"] = false,
            ["red"] = 1.0,
            ["green"] = 0.0,
            ["blue"] = 0.0,
            ["alpha"] = 1.0,
            ["instrument"] = {
                ["uuid"] = uuid(),
                ["name"] = "",
                ["descriptive_name"] = "Unknown",
                ["format"] = "Unknown",
                ["category"] = "Unknown",
                ["manufacturer"] = "Unknown",
                ["version"] = "Unknown",
                ["file"] = "",
                ["uid"] = "Unknown",
                ["is_instrument"] = true,
                ["file_time"] = "Unknown",
                ["info_update_time"] = "Unknown",
                ["num_inputs"] = 0,
                ["num_outputs"] = 0,
                ["plugin_type"] = "Unknown",
                ["sub_plugin_id"] = nil,
                ["preset_data"] = "",
                ["parameters"] = {0},
            },
            ["effects"] = {0},

            ["riffs"] = {0},
            ["riff_refs"] = {0},
            ["automation"] = {
                ["events"] = {0}
            },
            ["volume"] = 0.0,
            ["pan"] = 0.0,
            ["midi_routings"] = {0},
            ["audio_routings"] = {0}
          }
    }
end

local function new_vst_plugin()
    return {
        ["uuid"] = uuid(),
        ["name"] = "",
        ["descriptive_name"] = "Unknown",
        ["format"] = "Unknown",
        ["category"] = "Unknown",
        ["manufacturer"] = "Unknown",
        ["version"] = "Unknown",
        ["file"] = "",
        ["uid"] = "Unknown",
        ["is_instrument"] = false,
        ["file_time"] = "Unknown",
        ["info_update_time"] = "Unknown",
        ["num_inputs"] = 0,
        ["num_outputs"] = 0,
        ["plugin_type"] = "Unknown",
        ["sub_plugin_id"] = nil,
        ["preset_data"] = "",
    }
end

local function new_riff(length)
    return {
        ["uuid"] = uuid(),
        ["name"] = "",
        ["position"] = 0.0,
        ["length"] = length,
        ["colour"] = nil,
        ["events"] = {0}
    }
end

local function new_note(position, note, velocity, duration)
    return {
        ["Note"] = {
            ["position"] = position,
            ["note"] = note,
            ["velocity"] = velocity,
            ["length"] = duration
        }
    }
end

local function new_riff_ref(position, linked_to)
    return {
        ["uuid"] = uuid(),
        ["position"] = position,
        ["linked_to"] = linked_to
    }
end

local function handle_track_media_items(freedomdaw_project, reaper_track, freedomdaw_track)
    local media_item_count = reaper.CountTrackMediaItems(reaper_track)

    freedomdaw_track["InstrumentTrack"]["riffs"] = {}

    -- Add an empty riff
    local empty_riff = new_riff(4)

    empty_riff["name"] = "empty"
    table.insert(freedomdaw_track["InstrumentTrack"]["riffs"], empty_riff)

    if media_item_count > 0 then
        freedomdaw_track["InstrumentTrack"]["riff_refs"] = {}
    end

    local media_item_index = 0
    while media_item_index < media_item_count do
        local media_item = reaper.GetTrackMediaItem(reaper_track, media_item_index)
        -- need to convert from seconds to beats
        -- FIXME number reaper.TimeMap2_timeToQN(ReaProject proj, number tpos)

        local media_item_position_in_secs = reaper.GetMediaItemInfo_Value(media_item, "D_POSITION")
        local media_item_length_in_secs = reaper.GetMediaItemInfo_Value(media_item, "D_LENGTH")
        local media_item_take = reaper.GetMediaItemTake(media_item, 0)
        local retval, note_count, cc_event_count, text_syx_event_count = reaper.MIDI_CountEvts(media_item_take)

        -- print("Media item position in seconds=" .. reaper.GetMediaItemInfo_Value(media_item, "D_POSITION"))
        -- print("Media item length in seconds=" .. reaper.GetMediaItemInfo_Value(media_item, "D_LENGTH"))

        local _, _, _, media_item_position_in_beats, _ = reaper.TimeMap2_timeToBeats(0, media_item_position_in_secs)
        local _, _, _, media_item_length_in_beats, _ = reaper.TimeMap2_timeToBeats(0, media_item_length_in_secs)

        -- print("Media item position in beats=" .. media_item_position_in_beats)
        -- print("Media item length in beats=" .. media_item_length_in_beats)

        if retval and note_count > 0 then
            local riff = new_riff(media_item_length_in_beats)
            local note_event_index = 0

            riff["name"] = freedomdaw_track["InstrumentTrack"]["name"] .. " - " .. (media_item_index + 1)
            riff["position"] = 0.0
            riff["events"] = {}

            local highest_note_end_position = 0.0
            while note_event_index < note_count do
                local retval, selected, muted, startppqpos, endppqpos, chan, pitch, vel = reaper.MIDI_GetNote(media_item_take, note_event_index)
                if retval then
                    -- print("start=" .. startppqpos .. ", end=" .. endppqpos .. ", chan=" .. chan .. ", pitch=" .. pitch .. ", vel=" .. vel)
                    local start_beat = reaper.MIDI_GetProjQNFromPPQPos(media_item_take, startppqpos)
                    local duration_in_beats = reaper.MIDI_GetProjQNFromPPQPos(media_item_take, endppqpos) - start_beat
                    local adjusted_start_beat = start_beat - media_item_position_in_beats

                    if adjusted_start_beat >= 0 then
                        local note = new_note(adjusted_start_beat, pitch, vel, duration_in_beats)
                        table.insert(riff["events"], note)
                    end

                    highest_note_end_position = start_beat - media_item_position_in_beats + duration_in_beats
                end

                note_event_index = note_event_index + 1
            end

            local beats_per_measure = freedomdaw_project["song"]["time_signature_numerator"]
            local riff_length = 0.0

            while riff_length < highest_note_end_position do
                riff_length = riff_length + beats_per_measure
            end

            if riff_length == 0 then
                riff_length = 4
            end

            riff["length"] = riff_length

            if #(riff["events"]) > 0 then
                table.insert(freedomdaw_track["InstrumentTrack"]["riffs"], riff)

                local beat_count = 0
                while beat_count < media_item_length_in_beats do
                    -- print("riff_length=" .. riff_length .. ", beat_count=" .. beat_count .. ", media_item_length_in_beats=" .. media_item_length_in_beats)
                    local riff_ref = new_riff_ref(media_item_position_in_beats + beat_count, riff["uuid"])
                    table.insert(freedomdaw_track["InstrumentTrack"]["riff_refs"], riff_ref)
                    beat_count = beat_count + riff_length
                end
            end
        end

        media_item_index = media_item_index + 1
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

-- techniques in this function have been gleaned from https://github.com/EUGEN27771/ReaScripts/blob/master/FX/gen_Save%20Preset%20for%20last%20touched%20FX.lua
local function handle_preset(freedomdaw_audio_plugin, reaper_track, fx_index, is_instrument)
    -- Get the preset data
    local _, track_state_chunk = reaper.GetTrackStateChunk(reaper_track, "", false)
    local start_index, end_index = track_state_chunk:find("<FXCHAIN")
    
    track_state_chunk = track_state_chunk:gsub("(%d+)<(%x+)>", "%1{%2}")
    
    -- fast forward to the fx that we are interested in
    for index = 1, fx_index + 1 do
        start_index, end_index = track_state_chunk:find("<%u+%s.->", end_index)
    end
    
    local retval, reaper_fx_type = reaper.TrackFX_GetNamedConfigParm(reaper_track, fx_index, "fx_type")

    print("reaper_fx_type")
    print(reaper_fx_type)
    print(reaper_fx_type:len())

    if reaper_fx_type == "VSTi" or reaper_fx_type == "VST" then
        print("Found VST2 plugin")
        -- there are 3 blocks
        -- the VST preset is in block 2
        -- overwrite block 2s

        local vst24_xml = track_state_chunk:sub(start_index, end_index)
        print("vst24_xml")
        print(vst24_xml)
        local block_count = 1
        local block_1 = ""
        local block_1_max_line_length = 280
        local block_2 = ""

        if is_instrument then
            block_1_max_line_length = 128
        end

        print("block#"..block_count)
        for line in vst24_xml:gmatch("(.-)\n") do
            if line:match("<VST%s") == nil then
                print(line:len().." "..line)
                if block_count == 1 then
                    block_1 = block_1..line
                elseif block_count == 2 then
                    block_2 = block_2..line
                end
                
                if block_count == 1 and line:len() < block_1_max_line_length then
                    block_count = block_count + 1
                    if block_count < 4 then
                    print("block#"..block_count)
                    end
                elseif block_count == 2 and line:len() < 280 then
                    block_count = block_count + 1
                    if block_count < 4 then
                    print("block#"..block_count)
                    end
                end
            end
        end
        
        print("Extracted block#1")
        print(block_1)
        print("Extracted block#2")
        print(block_2)

        freedomdaw_audio_plugin["preset_data"] = block_2
        freedomdaw_audio_plugin["plugin_type"] = reaper_fx_type:gsub("i", "").."24"
    else
        print("Did not match VST2")
    end
end

local function handle_track_effects(freedomdaw_project, reaper_track, freedomdaw_track)
    -- local track_instrument_plugin_index = reaper.TrackFX_GetInstrument(reaper_track)
    local track_instrument_plugin_index = 0
    local track_fx_count = reaper.TrackFX_GetCount(reaper_track)

    if track_fx_count > 1 then
        freedomdaw_track["InstrumentTrack"]["effects"] = {}
    end

    local track_fx_index = 0
    while track_fx_index < track_fx_count do
        if track_fx_index ~= track_instrument_plugin_index then
            local effect = new_vst_plugin()
            -- local retval, track_effect_plugin_name = reaper.TrackFX_GetFXName(reaper_track, track_fx_index)
            local retval, track_effect_plugin_name = reaper.TrackFX_GetNamedConfigParm(reaper_track, track_fx_index, "fx_name")
            if retval then
                effect["name"] = track_effect_plugin_name:gsub("VST: ", ""):gsub(" %(.*%)", "")
            end

            local retval, track_effect_plugin_type = reaper.TrackFX_GetNamedConfigParm(reaper_track, track_fx_index, "fx_type")
            if retval then
                effect["category"] = track_effect_plugin_type
            end

            local retval, track_effect_plugin_ident = reaper.TrackFX_GetNamedConfigParm(reaper_track, track_fx_index, "fx_ident")
            if retval then
                print("track_effect_plugin_ident="..track_effect_plugin_ident)
                effect["sub_plugin_id"] = track_effect_plugin_ident:gsub("^.*<", "")
                effect["file"] = track_effect_plugin_ident:gsub("<.*", "")
            else
                local retval, track_effect_plugin_module_name = reaper.BR_TrackFX_GetFXModuleName(reaper_track, track_fx_index)
                if retval then
                    print("track_effect_plugin_module_name="..track_effect_plugin_module_name)
                    effect["file"] = track_effect_plugin_module_name
                end
            end

            handle_preset(effect, reaper_track, track_fx_index, false)

            table.insert(freedomdaw_track["InstrumentTrack"]["effects"], effect)
        end
        track_fx_index = track_fx_index + 1
    end
end

local function handle_instrument(freedomdaw_project, reaper_track, freedomdaw_track)
    -- GetInstrument does not work for Repo-1 and 5 as the VST shell plugin stuff does not return as an instrument
    -- local track_instrument_plugin_index = reaper.TrackFX_GetInstrument(reaper_track)
    local track_instrument_plugin_index = 0
    local retval, track_instrument_plugin_name = reaper.TrackFX_GetFXName(reaper_track, track_instrument_plugin_index)
    if retval then
        freedomdaw_track["InstrumentTrack"]["instrument"]["name"] = track_instrument_plugin_name:gsub("CLAPi: ", ""):gsub("CLAPi: ", ""):gsub("VSTi: ", ""):gsub("VST: ", ""):gsub(" %(.*%)", "")
        freedomdaw_track["InstrumentTrack"]["instrument"]["is_instrument"] = true

        local retval, track_instrument_plugin_type = reaper.TrackFX_GetNamedConfigParm(reaper_track, track_instrument_plugin_index, "fx_type")
        if retval then
            freedomdaw_track["InstrumentTrack"]["instrument"]["category"] = track_instrument_plugin_type
        end

        local retval, track_instrument_plugin_ident = reaper.TrackFX_GetNamedConfigParm(reaper_track, track_instrument_plugin_index, "fx_ident")
        if retval then
            print("track_instrument_plugin_ident="..track_instrument_plugin_ident)
            freedomdaw_track["InstrumentTrack"]["instrument"]["sub_plugin_id"] = track_instrument_plugin_ident:gsub("^.*<", "")
            freedomdaw_track["InstrumentTrack"]["instrument"]["file"] = track_instrument_plugin_ident:gsub("<.*", "")
            print("freedomdaw_track[InstrumentTrack][instrument][file]="..freedomdaw_track["InstrumentTrack"]["instrument"]["file"])
        else
            local retval, track_instrument_plugin_index_module_name = reaper.BR_TrackFX_GetFXModuleName(reaper_track, track_instrument_plugin_index)
            if retval then
                print("track_instrument_plugin_index_module_name="..track_instrument_plugin_index_module_name)
                freedomdaw_track["InstrumentTrack"]["instrument"]["file"] = track_instrument_plugin_index_module_name
            end
        end

        handle_preset(freedomdaw_track["InstrumentTrack"]["instrument"], reaper_track, track_instrument_plugin_index, true)
    end
end

local function handle_instrument_track(freedomdaw_project, reaper_track)
    local success, reaper_track_name = reaper.GetTrackName(reaper_track)

    local freedomdaw_track = new_instrument_track()
    freedomdaw_track["InstrumentTrack"]["uuid"] = uuid()
    freedomdaw_track["InstrumentTrack"]["name"] = reaper_track_name

    -- mute, solo, volume, pan
    local mute = reaper.GetMediaTrackInfo_Value(reaper_track, "B_MUTE")
    local solo = reaper.GetMediaTrackInfo_Value(reaper_track, "I_SOLO")
    local volume = reaper.GetMediaTrackInfo_Value(reaper_track, "D_VOL")
    local pan = reaper.GetMediaTrackInfo_Value(reaper_track, "D_PAN")

    print("volume=" .. volume .. ", pan=" .. pan)

    if solo == 0 then
        solo = false
    else
        solo = true
    end

    if mute == 0 then
        mute = false
    else
        mute = true
    end

    freedomdaw_track["InstrumentTrack"]["mute"] = mute
    freedomdaw_track["InstrumentTrack"]["solo"] = solo
    freedomdaw_track["InstrumentTrack"]["volume"] = volume * 0.5
    freedomdaw_track["InstrumentTrack"]["pan"] = pan


    table.insert(freedomdaw_project["song"]["tracks"], freedomdaw_track)

    handle_instrument(freedomdaw_project, reaper_track, freedomdaw_track)
    handle_track_effects(freedomdaw_project, reaper_track, freedomdaw_track)
    handle_track_media_items(freedomdaw_project, reaper_track, freedomdaw_track)
end

local function write_freedomdaw_project_file(freedomdaw_project)
    local json = luna.encode( freedomdaw_project )
    local data, num_replaces = json:gsub("%[0%]", "[]")

    -- print( data )

    local freedomdaw_project_file = io.open("/tmp/lua.fdaw", "w")
    if freedomdaw_project_file ~= nil then
        freedomdaw_project_file:write(data)
        freedomdaw_project_file:write("\n")
        freedomdaw_project_file:close()
    end
end


local function main()
    local retval, project_name = reaper.GetSetProjectInfo_String(0, "PROJECT_NAME", "", false)
    local reaper_proj_num_tracks = reaper.GetNumTracks()

    -- print("Project name: " .. project_name)

    local freedomdaw_project = new_freedomdaw_project()
    freedomdaw_project["song"]["name"] = project_name

    local bpm, bpi = reaper.GetProjectTimeSignature2(0)
    freedomdaw_project["song"]["tempo"] = bpm

    local timesig_num, timesig_denom, _ = reaper.TimeMap_GetTimeSigAtTime(0, 0)
    freedomdaw_project["song"]["time_signature_numerator"] = timesig_num
    freedomdaw_project["song"]["time_signature_denominator"] = timesig_denom

    local reaper_track_index = 0
    while reaper_track_index < reaper_proj_num_tracks do
        local reaper_track = reaper.GetTrack(0, reaper_track_index)

        handle_instrument_track(freedomdaw_project, reaper_track)

        reaper_track_index = reaper_track_index + 1
    end

    write_freedomdaw_project_file(freedomdaw_project)
end




main()
