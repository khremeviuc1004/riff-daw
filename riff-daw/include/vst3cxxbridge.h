#pragma once

#include "riff-daw/src/vst3_cxx_bridge.rs.h"
#include <cstdint>
#include "rust/cxx.h"
#include <memory>
//#include "riff-daw/include/vst3headers.h"

namespace org {
namespace hremeviuc {

bool createPlugin(
    rust::String vst3_plugin_path,
    rust::String riff_daw_plugin_uuid,
    rust::String vst3_plugin_uid,
    double sampleRate,
    int32_t blockSize,
    rust::Box<Vst3Host> vst3Host,
    rust::Fn<rust::Box<Vst3Host>(rust::Box<Vst3Host> context, int32_t param_id, float param_value)> sendParameterChange
);

bool showPluginEditor(
    rust::String riff_daw_plugin_uuid,
    uint32_t xid,
    rust::Box<Vst3Host> vst3Host,
    rust::Fn<rust::Box<Vst3Host>(rust::Box<Vst3Host> context, int32_t new_window_width, int32_t new_window_height)> sendPluginWindowResize
);
uint32_t vst3_plugin_get_window_height(rust::String riff_daw_plugin_uuid);
uint32_t vst3_plugin_get_window_width(rust::String riff_daw_plugin_uuid);
void vst3_plugin_get_window_refresh(rust::String riff_daw_plugin_uuid);

bool vst3_plugin_process(
    rust::String riff_daw_plugin_uuid,
    rust::Slice<const float> channel1InputBuffer,
    rust::Slice<const float> channel2InputBuffer,
    rust::Slice<float> channel1OutputBuffer,
    rust::Slice<float> channel2OutputBuffer
);

bool addEvent(rust::String riff_daw_plugin_uuid, EventType eventType, int32_t blockPosition, uint32_t data1, uint32_t data2, int32_t data3, double data4);
rust::String getVstPluginName(rust::String riff_daw_plugin_uuid);
bool setProcessing(rust::String riff_daw_plugin_uuid, bool processing);
bool setActive(rust::String riff_daw_plugin_uuid, bool active);
int32_t vst3_plugin_get_preset(rust::String riff_daw_plugin_uuid, rust::Slice<uint8_t> preset_buffer, uint32_t maxSize);
void vst3_plugin_set_preset(rust::String riff_daw_plugin_uuid, rust::Slice<uint8_t> preset_buffer);

int32_t vst3_plugin_get_parameter_count(rust::String riff_daw_plugin_uuid);
void vst3_plugin_get_parameter_info(
    rust::String riff_daw_plugin_uuid,
    int32_t index,
    uint32_t& id,
    rust::Slice<uint16_t> title,
    rust::Slice<uint16_t> short_title,
    rust::Slice<uint16_t> units,
    int32_t& step_count,
    double& default_normalised_value,
    int32_t& unit_id,
    int32_t& flags
);

void vst3_plugin_remove(rust::String riff_daw_plugin_uuid);

} // namespace hremeviuc
} // namespace org
