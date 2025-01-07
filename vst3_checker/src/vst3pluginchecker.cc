#pragma once

#include "../include/vst3pluginchecker.h"
#include <iostream>
#include <chrono>
#include"../include/vst3headers.h"

void checkPlugin(char* vst3_plugin_path)
{
    std::string path(vst3_plugin_path);
    std::cout << "Path: " << path << std::endl;
    std::string error;
    Steinberg::IPtr<Steinberg::Vst::HostApplication> hostApplication = Steinberg::owned(new Steinberg::Vst::HostApplication());
    std::string instrumentString("Instrument");

    VST3::Hosting::Module::Ptr module = VST3::Hosting::Module::create(path, error);
    if (! module)
        return;

    auto factoryInfo = module->getFactory().info();
    std::cout << factoryInfo.vendor() << std::endl;
    std::cout << factoryInfo.url() << std::endl;
    std::cout << factoryInfo.email() << std::endl;

    Steinberg::IPtr<Steinberg::Vst::PlugProvider> plugProvider;
    for (auto& classInfo : module->
        getFactory().classInfos())
    {
        if (classInfo.category() == kVstAudioEffectClass)
        {
            std::cout << classInfo.category() << std::endl;
            std::cout << classInfo.name() << std::endl;
            std::cout << classInfo.cardinality() << std::endl;

            bool instrument = false;

            auto subCategories = classInfo.subCategories();
            for (auto & element : subCategories) {
                std::cout << "Sub-category: " << element << std::endl;

                if (element.contains(instrumentString))
                {
                    instrument = true;
                }
            }

            std::cout << "##########" << classInfo.name() << ":" << path << ":" << classInfo.ID().toString() << ":" << (instrument ? 2 : 1) << ":VST3" << std::endl;

            plugProvider = Steinberg::owned(new Steinberg::Vst::PlugProvider(module->getFactory(), classInfo, true));
            std::cout << "Created PlugProvider." << std::endl;

            Steinberg::Vst::PluginContextFactory::instance().setPluginContext(hostApplication.get());

            if (plugProvider.get()->initialize() )
            {
                std::cout << "Initailised PlugProvider." << std::endl;
                Steinberg::IPtr<Steinberg::Vst::IComponent> component = plugProvider.get()->getComponentPtr();
                Steinberg::IPtr<Steinberg::Vst::IEditController> controller = plugProvider.get()->getControllerPtr();

                auto inputBusCount = component.get()->getBusCount(Steinberg::Vst::MediaTypes::kAudio, Steinberg::Vst::BusDirections::kInput);
                std::cout << "input bus count=" << inputBusCount << std::endl;
                for (auto index = 0; index < inputBusCount; index++)
                {
                    Steinberg::Vst::BusInfo inputBusInfo;
                    component.get()->getBusInfo(Steinberg::Vst::MediaTypes::kAudio, Steinberg::Vst::BusDirections::kInput, index, inputBusInfo);
                    std::cout << "Input bus " << index << " channel count: " << inputBusInfo.channelCount << std::endl;
                }

                auto outputBusCount = component.get()->getBusCount(Steinberg::Vst::MediaTypes::kAudio, Steinberg::Vst::BusDirections::kOutput);
                std::cout << "output bus count=" << outputBusCount << std::endl;
                for (auto index = 0; index < outputBusCount; index++)
                {
                    Steinberg::Vst::BusInfo outputBusInfo;
                    component.get()->getBusInfo(Steinberg::Vst::MediaTypes::kAudio, Steinberg::Vst::BusDirections::kOutput, index, outputBusInfo);
                    std::cout << "Output bus " << index << " channel count: " << outputBusInfo.channelCount << std::endl;
                }

                auto inputEventBusCount = component.get()->getBusCount(Steinberg::Vst::MediaTypes::kEvent, Steinberg::Vst::BusDirections::kInput);
                std::cout << "input event bus count=" << inputEventBusCount << std::endl;
                for (auto index = 0; index < inputEventBusCount; index++)
                {
                    Steinberg::Vst::BusInfo inputEventBusInfo;
                    component.get()->getBusInfo(Steinberg::Vst::MediaTypes::kEvent, Steinberg::Vst::BusDirections::kInput, index, inputEventBusInfo);
                    std::cout << "Input event bus " << index << " channel count: " << inputEventBusInfo.channelCount << std::endl;
                }

                auto outputEventBusCount = component.get()->getBusCount(Steinberg::Vst::MediaTypes::kEvent, Steinberg::Vst::BusDirections::kOutput);
                std::cout << "output event bus count=" << outputEventBusCount << std::endl;
                for (auto index = 0; index < outputEventBusCount; index++)
                {
                    Steinberg::Vst::BusInfo outputEventBusInfo;
                    component.get()->getBusInfo(Steinberg::Vst::MediaTypes::kEvent, Steinberg::Vst::BusDirections::kOutput, index, outputEventBusInfo);
                    std::cout << "Output event bus " << index << " channel count: " << outputEventBusInfo.channelCount << std::endl;
                }

                Steinberg::IPlugView* plugview = controller.get()->createView(Steinberg::Vst::ViewType::kEditor);
                Steinberg::ViewRect* viewRect = new Steinberg::ViewRect(1, 1, 1, 1);
                plugview->getSize(viewRect);
                if (plugview->attached(0, Steinberg::kPlatformTypeX11EmbedWindowID) != Steinberg::kResultOk)
                {
                    std::cout << "Failed to open window." << std::endl;
                }

                std::cout << "left=" << viewRect->left << ", right=" << viewRect->right << ", top=" << viewRect->top << ", bottom=" << viewRect->bottom << ", width=" << viewRect->getWidth() << ", height" << viewRect->getHeight() << std::endl;
                std::cout << "Param count=" << controller.get()->getParameterCount() << std::endl;

                Steinberg::FUnknownPtr<Steinberg::Vst::IAudioProcessor> processor = component.get();

                std::cout << "Latency samples=" << processor->getLatencySamples() << std::endl;
                std::cout << "Tail samples=" << processor->getTailSamples() << std::endl;

//                continue;

                Steinberg::Vst::ProcessSetup processSetUp {
                    Steinberg::Vst::ProcessModes::kRealtime,
                    Steinberg::Vst::SymbolicSampleSizes::kSample32,
                    1024,
                    44100.0
                };
                if (processor->setupProcessing(processSetUp) == Steinberg::kResultOk)
                {
                }
                if (inputBusCount > 0)
                {
                    component.get()->activateBus(Steinberg::Vst::kAudio, Steinberg::Vst::kInput, 0, true);
                }
                if (outputBusCount > 0)
                {
                    component.get()->activateBus(Steinberg::Vst::kAudio, Steinberg::Vst::kOutput, 0, true);
                }
                Steinberg::TBool okToProcess = true;
                processor->setProcessing(okToProcess);

                std::cout << "Processing..." << std::endl;

                Steinberg::Vst::ProcessContext processContext;
                processContext.state = 0;
                processContext.state = Steinberg::Vst::ProcessContext::StatesAndFlags::kPlaying;
                processContext.state |= Steinberg::Vst::ProcessContext::kSystemTimeValid;
                processContext.state |= Steinberg::Vst::ProcessContext::kTempoValid;
                processContext.state |= Steinberg::Vst::ProcessContext::kTimeSigValid;
                processContext.state |= Steinberg::Vst::ProcessContext::kContTimeValid;
                processContext.state |= Steinberg::Vst::ProcessContext::kSystemTimeValid;
                processContext.sampleRate = 44100.0;
                processContext.projectTimeSamples = 0;
                processContext.systemTime = std::chrono::duration_cast<std::chrono::nanoseconds>(std::chrono::system_clock::now().time_since_epoch()).count();
                processContext.continousTimeSamples = 0;
                processContext.projectTimeMusic = 0.0;
                processContext.barPositionMusic = 0.0;
                processContext.cycleStartMusic = 0.0;
                processContext.cycleEndMusic = 0.0;
                processContext.tempo = 140.0;
                processContext.timeSigNumerator = 4;
                processContext.timeSigDenominator = 4;
                processContext.chord = Steinberg::Vst::Chord {
                    0,
                    0,
                    Steinberg::Vst::Chord::kChordMask
                };
                processContext.smpteOffsetSubframes = 0;
                processContext.frameRate = Steinberg::Vst::FrameRate {
                    44100,
                    Steinberg::Vst::FrameRate::kPullDownRate
                };
                processContext.samplesToNextClock = 0;

                Steinberg::Vst::HostProcessData processData;
                processData.prepare(*component, 1024, processSetUp.symbolicSampleSize);
                processData.processMode = Steinberg::Vst::ProcessModes::kRealtime;
                processData.numSamples = 1024;
                processData.inputEvents = new Steinberg::Vst::EventList[inputEventBusCount];
                processData.outputEvents = new Steinberg::Vst::EventList[outputEventBusCount];
                processData.processContext = &processContext;

                Steinberg::Vst::Event eventNoteOn = {};
                eventNoteOn.busIndex = 0;
                eventNoteOn.sampleOffset = 0;
                eventNoteOn.ppqPosition = 0.0;
                eventNoteOn.flags = Steinberg::Vst::Event::EventFlags::kIsLive;
                eventNoteOn.noteOn.noteId = -1;
                eventNoteOn.type = Steinberg::Vst::Event::kNoteOnEvent;
                eventNoteOn.noteOn.channel = 0;
                eventNoteOn.noteOn.pitch = 60;
                eventNoteOn.noteOn.velocity = 1.0;

                if (inputEventBusCount > 0)
                {
                    processData.inputEvents[0].addEvent(eventNoteOn);
                }

                Steinberg::Vst::Event eventNoteOff = {};
                eventNoteOff.busIndex = 0;
                eventNoteOff.sampleOffset = 0;
                eventNoteOff.ppqPosition = 0.0;
                eventNoteOff.flags = Steinberg::Vst::Event::EventFlags::kIsLive;
                eventNoteOff.noteOff.noteId = -1;
                eventNoteOff.type = Steinberg::Vst::Event::kNoteOffEvent;
                eventNoteOff.noteOff.channel = 0;
                eventNoteOff.noteOff.pitch = 60;
                eventNoteOff.noteOff.velocity = 0.0;

                if (component.get()->setActive(true) != Steinberg::kResultTrue)
                {
                    std::cout << "Failed to set the component to active." << std::endl;
                }
                else
                {
                    bool clearInputEvents = true;
                    while(processContext.projectTimeSamples / 1024 < 100)
                    {
                        std::cout << "Processing block: " << processContext.projectTimeSamples / 1024 + 1 << std::endl;
                        processor->process(processData);

                        // detect if non zero samples have been found
                        // this causes some crashing for some reason
//                        if (processData.numOutputs > 0)
//                        {
//                            for (auto index = 0; index < 1024; index++) {
//                                if (processData.outputs[0].channelBuffers32[0][index] > 0.0 || processData.outputs[0].channelBuffers32[0][index] < 0.0) {
//                                    std::cout << "Non zero samples found!" << std::endl;
//                                    break;
//                                }
//                            }
//                        }

                        processContext.projectTimeSamples += 1024;
                        processContext.systemTime = std::chrono::duration_cast<std::chrono::nanoseconds>(std::chrono::system_clock::now().time_since_epoch()).count();
                        processContext.continousTimeSamples += 1024;

                        if (clearInputEvents) {
                            static_cast<Steinberg::Vst::EventList&>(processData.inputEvents[0]).clear();
                            clearInputEvents = false;
                        }

                        if (processContext.projectTimeSamples / 1024 == 44) {
                            processData.inputEvents[0].addEvent(eventNoteOff);
                            clearInputEvents = true;
                        }
                    }

                    std::cout << "Finished processing." << std::endl;

                    component.get()->setActive(false);
                    processor->setProcessing(false);
                }
            }
            else {
                std::cout << "Failed to initailise the PlugProvider." << std::endl;
            }
        }
    }
}
