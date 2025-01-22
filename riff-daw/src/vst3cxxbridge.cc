#pragma once

#include "riff-daw/include/vst3cxxbridge.h"
#include <iostream>
#include <map>
#include <chrono>
#include <thread>
#include <vector>
#include <algorithm>
#include <sys/select.h>
#include <sys/time.h>
#include <unordered_map>
#include "riff-daw/include/vst3headers.h"


namespace org {
namespace hremeviuc {

void dumpTUID(const Steinberg::TUID tuid)
{
    for(auto i = 0; i < 16; i++)
    {
        std::cout << static_cast<int>(tuid[i]) << " ";
    }
}

class PresetStream : public Steinberg::IBStream
{
public:
    PresetStream(rust::Slice<uint8_t> inputData) : data(inputData) { FUNKNOWN_CTOR }
    ~PresetStream() { FUNKNOWN_DTOR }

    DECLARE_FUNKNOWN_METHODS

    Steinberg::tresult PLUGIN_API read (void* buffer, Steinberg::int32 numBytes, Steinberg::int32* numBytesRead = nullptr) override
    {
        std::cout << "PresetStream::read called: numBytes = " << numBytes << std::endl;
        Steinberg::int32 readableNumberOfBytes = data.length() - streamPosition;
        Steinberg::int32 bytesToRead = readableNumberOfBytes >= numBytes ? numBytes : data.length() - streamPosition;
        std::cout << "PresetStream::read: readableNumberOfBytes=" << readableNumberOfBytes << ", bytesToRead=" << bytesToRead << ", streamPosition=" << streamPosition << std::endl;
        if (bytesToRead > 0)
        {
            uint8_t* read_buffer = static_cast<uint8_t*>(buffer);
            for (auto index = 0; index < bytesToRead; index++)
            {
                read_buffer[index] = data[streamPosition + index];
            }
        }
        if (numBytesRead != nullptr)
        {
            *numBytesRead = bytesToRead;
        }
        streamPosition += bytesToRead;
        return Steinberg::kResultOk;
    }
    Steinberg::tresult PLUGIN_API write (void* buffer, Steinberg::int32 numBytes, Steinberg::int32* numBytesWritten = nullptr) override
    {
        std::cout << "PresetStream::write called: numBytes=" << numBytes << std::endl;
        uint8_t* read_buffer = static_cast<uint8_t*>(buffer);
        for (auto index = 0; index < numBytes; index++)
        {
            data[index] = read_buffer[index];
        }
        bytesWritten += numBytes;
        std::cout << std::endl;

        std::cout << "PresetStream::write called: data.size()=" << data.size() << ", data.length()=" << data.length() << std::endl;

        if (numBytesWritten != nullptr)
        {
            *numBytesWritten = numBytes;
        }
        return Steinberg::kResultOk;
    }
    Steinberg::tresult PLUGIN_API seek (Steinberg::int64 pos, Steinberg::int32 mode, Steinberg::int64* result = nullptr) override
    {
        std::cout << "PresetStream::seek called." << std::endl;
        return Steinberg::kResultOk;
    }
    Steinberg::tresult PLUGIN_API tell (Steinberg::int64* pos) override
    {
        std::cout << "PresetStream::tell called." << std::endl;
        return Steinberg::kResultOk;
    }

    int getBytesWritten() {return bytesWritten;}

private:
    rust::Slice<uint8_t> data;
    int bytesWritten = 0;
    Steinberg::int32 streamPosition = 0;
};

IMPLEMENT_FUNKNOWN_METHODS (org::hremeviuc::PresetStream, Steinberg::IBStream, Steinberg::IBStream::iid)

class RunLoop : public Steinberg::Linux::IRunLoop
{
public:
    bool keepAlive = true;
    std::mutex timerMutex;
    std::vector<Steinberg::Linux::ITimerHandler*> timerHandlers;
    std::mutex eventHandlerMutex;
    std::unordered_multimap<Steinberg::Linux::IEventHandler*, int> eventHandlers;

    RunLoop() {}
    ~RunLoop()
    {
        stop();
    }

	Steinberg::tresult PLUGIN_API registerEventHandler (Steinberg::Linux::IEventHandler *handler, Steinberg::Linux::FileDescriptor fd) override
    {
        std::cout << "RunLoop registerEventHandler called: fd=" << int(fd) << std::endl;
        std::lock_guard<std::mutex> guard(eventHandlerMutex);
        eventHandlers.emplace(handler, int(fd));
        return Steinberg::kResultOk;
    }

	Steinberg::tresult PLUGIN_API unregisterEventHandler (Steinberg::Linux::IEventHandler *handler) override
    {
        std::cout << "RunLoop unregisterEventHandler called." << std::endl;
        std::lock_guard<std::mutex> guard(eventHandlerMutex);
        eventHandlers.erase(handler);
        return Steinberg::kResultOk;
    }

	Steinberg::tresult PLUGIN_API registerTimer (Steinberg::Linux::ITimerHandler *handler, Steinberg::Linux::TimerInterval msecs) override
    {
        std::cout << "RunLoop registerTimer called." << std::endl;
        std::lock_guard<std::mutex> guard(timerMutex);
        timerHandlers.push_back(handler);
        return Steinberg::kResultOk;
    }

	Steinberg::tresult PLUGIN_API unregisterTimer (Steinberg::Linux::ITimerHandler *handler) override
    {
        std::cout << "RunLoop unregisterTimer called." << std::endl;
        std::lock_guard<std::mutex> guard(timerMutex);
        std::vector<Steinberg::Linux::ITimerHandler*>::iterator findIterator = std::find(timerHandlers.begin(), timerHandlers.end(), handler);
        if (findIterator != timerHandlers.end())
        {
            timerHandlers.erase(findIterator);
            return Steinberg::kResultOk;
        }
        return Steinberg::kResultFalse;
    }

	Steinberg::tresult PLUGIN_API queryInterface (const Steinberg::TUID _iid, void **obj) override
	{
        std::cout << "RunLoop queryInterface called: _iid=";
        dumpTUID(_iid);
        std::cout << ", Steinberg::Vst::IHostApplication::iid=";
        dumpTUID(Steinberg::Vst::IHostApplication::iid);
        std::cout << ", Steinberg::FUnknown::iid=";
        dumpTUID(Steinberg::FUnknown::iid);
        std::cout << ", Steinberg::Linux::IRunLoop::iid=";
        dumpTUID(Steinberg::Linux::IRunLoop::iid);
        std::cout << ", Funknown=" << Steinberg::FUnknownPrivate::iidEqual(_iid, Steinberg::FUnknown::iid) << ", IRunLoop=" << Steinberg::FUnknownPrivate::iidEqual(_iid, Steinberg::Linux::IRunLoop::iid) << std::endl;
		if (Steinberg::FUnknownPrivate::iidEqual(_iid, Steinberg::FUnknown::iid) || Steinberg::FUnknownPrivate::iidEqual(_iid, Steinberg::Linux::IRunLoop::iid)) {
            std::cout << "RunLoop queryInterface - FUnknown or IRunLoop requested." << std::endl;
			addRef();
			*obj = this;
			return Steinberg::kResultOk;
		}

		*obj = nullptr;
		return Steinberg::kNoInterface;
	}

	Steinberg::uint32 PLUGIN_API addRef  () override { return 1001; }
	Steinberg::uint32 PLUGIN_API release () override { return 1001; }

	void stop()
	{
        std::cout << "RunLoop stop called." << std::endl;
        std::cout << "RunLoop stop - set keepAlive to false." << std::endl;
	    keepAlive = false;
        std::cout << "RunLoop stop - waiting for run loop thread to finish..." << std::endl;
	    timer.join();
        std::cout << "RunLoop stop - waiting for run loop thread should have finished." << std::endl;
        std::lock_guard<std::mutex> timerGuard(timerMutex);
        std::lock_guard<std::mutex> eventHandlerGuard(eventHandlerMutex);
        std::cout << "RunLoop stop - clearing timerHandlers." << std::endl;
	    timerHandlers.clear();
        std::cout << "RunLoop stop - clearing event handlers." << std::endl;
	    eventHandlers.clear();
        std::cout << "RunLoop stop - Done." << std::endl;
	}

private:
    void run()
    {
        while(keepAlive)
        {
//            std::this_thread::sleep_for(std::chrono::milliseconds(300));
//            std::cout << "RunLoop: still alive." << std::endl;
            {
                int numberOfFileDescriptors = 0;

                fd_set readFileDescriptors;
                fd_set writeFileDescriptors;
                fd_set exceptFileDescriptors;

                FD_ZERO(&readFileDescriptors);
                FD_ZERO(&writeFileDescriptors);
                FD_ZERO(&exceptFileDescriptors);

                // add all the event handler file descriptors
                {
                    std::lock_guard<std::mutex> guard(eventHandlerMutex);
                    for(auto const& [key, value] : eventHandlers)
                    {
                        int fd = value;
                        FD_SET(fd, &readFileDescriptors);
                        FD_SET(fd, &writeFileDescriptors);
                        FD_SET(fd, &exceptFileDescriptors);

                        numberOfFileDescriptors = fd > numberOfFileDescriptors ? fd : numberOfFileDescriptors;
//                        std::cout << "RunLoop: FD_SET event handler file descriptors=" << fd << ", number of file descriptors=" << numberOfFileDescriptors << std::endl;
                    }
                }

                timeval timeOut;
                timeOut.tv_sec = 0;
                timeOut.tv_usec = 300000;

                const int result = select(numberOfFileDescriptors, &readFileDescriptors, &writeFileDescriptors, nullptr /*&exceptFileDescriptors*/, &timeOut);

                if (result == EBADF)
                {
                    std::cout << "RunLoop: select reports one of the event handler file descriptors as bad." << std::endl;
                }

                if (result > 0)
                {
                    for(auto const& [key, value] : eventHandlers)
                    {
                        int fd = value;
                        if (FD_ISSET(fd, &readFileDescriptors) || FD_ISSET(fd, &writeFileDescriptors) || FD_ISSET(fd, &exceptFileDescriptors))
                        {
//                            std::cout << "RunLoop: file descriptor event fired." << std::endl;
                                Steinberg::Linux::IEventHandler* eventHandler = key;
                                eventHandler->onFDIsSet(fd);
                        }
                    }
                }
            }
            {
                std::lock_guard<std::mutex> guard(timerMutex);
                for(auto & element : timerHandlers)
                {
                    element->onTimer();
                }
            }
        }

        std::cout << "RunLoop thread loop exited." << std::endl;
    }

    std::thread timer = std::thread{&RunLoop::run, this};
};

//static Steinberg::IPtr<Steinberg::Linux::IRunLoop> runLoop = Steinberg::owned(new RunLoop());

class SimplePlugFrame : public Steinberg::IPlugFrame
{
public:
    SimplePlugFrame(
        rust::Box<Vst3Host> vst3Sender,
        rust::Fn<rust::Box<Vst3Host>(rust::Box<Vst3Host> context, int32_t new_window_width, int32_t new_window_height)> sendPluginWindowResizeFunc)
        : vst3Host(std::move(vst3Sender)), sendPluginWindowResize(sendPluginWindowResizeFunc)
    {}
    ~SimplePlugFrame() {}

    Steinberg::tresult PLUGIN_API resizeView (Steinberg::IPlugView* view, Steinberg::ViewRect* newSize)
    {
        std::cout << "SimplePlugFrame: resize called." << std::endl;
        if (view && newSize)
        {
            vst3Host = sendPluginWindowResize(std::move(vst3Host), newSize->getWidth(), newSize->getHeight());
            view->onSize(newSize);
			return Steinberg::kResultOk;
        }
        else
        {
			return Steinberg::kInvalidArgument;
        }
    }

	Steinberg::tresult PLUGIN_API queryInterface (const Steinberg::TUID _iid, void **obj) override
	{
        std::cout << "SimplePlugFrame queryInterface called." << std::endl;
		if (Steinberg::FUnknownPrivate::iidEqual(_iid, Steinberg::FUnknown::iid) || Steinberg::FUnknownPrivate::iidEqual(_iid, Steinberg::IPlugFrame::iid)) {
			addRef();
			*obj = this;
            std::cout << "SimplePlugFrame queryInterface returning IPlugFrame." << std::endl;
			return Steinberg::kResultOk;
		}

        std::cout << "SimplePlugFrame queryInterface returning IRunLoop." << std::endl;
		return runLoop.get()->queryInterface(_iid, obj);
	}

	Steinberg::uint32 PLUGIN_API addRef  () override { return 1002; }
	Steinberg::uint32 PLUGIN_API release () override { return 1002; }

	void shutdownRunLoop()
	{
        std::cout << "SimplePlugFrame shutdownRunLoop called." << std::endl;
	    static_cast<RunLoop*>(runLoop.get())->stop();
	}

private:
    rust::Box<Vst3Host> vst3Host;
    rust::Fn<rust::Box<Vst3Host>(rust::Box<Vst3Host> context, int32_t new_window_width, int32_t new_window_height)> sendPluginWindowResize;
    Steinberg::IPtr<Steinberg::Linux::IRunLoop> runLoop = Steinberg::owned(new RunLoop());
};

class Vst3HostApplication : public Steinberg::Vst::IHostApplication
{
public:
	Vst3HostApplication();
	virtual ~Vst3HostApplication() noexcept {FUNKNOWN_DTOR}

	Steinberg::tresult PLUGIN_API getName (Steinberg::Vst::String128 name) override;
	Steinberg::tresult PLUGIN_API createInstance (Steinberg::TUID cid, Steinberg::TUID _iid, void** obj) override;

	DECLARE_FUNKNOWN_METHODS

	Steinberg::Vst::PlugInterfaceSupport* getPlugInterfaceSupport () const { return plugInterfaceSupport; }

private:
	Steinberg::IPtr<Steinberg::Vst::PlugInterfaceSupport> plugInterfaceSupport;
    Steinberg::IPtr<Steinberg::Linux::IRunLoop> runLoop = Steinberg::owned(new RunLoop());
};

Vst3HostApplication::Vst3HostApplication()
{
	FUNKNOWN_CTOR

	plugInterfaceSupport = owned(new Steinberg::Vst::PlugInterfaceSupport);
}

Steinberg::tresult PLUGIN_API Vst3HostApplication::getName(Steinberg::Vst::String128 name)
{
//	return VSTGUI::StringConvert::convert("Riff DAW VST3 HostApplication", name) ? Steinberg::kResultTrue : Steinberg::kInternalError;
	return Steinberg::kInternalError;
}

Steinberg::tresult PLUGIN_API Vst3HostApplication::createInstance (Steinberg::TUID cid, Steinberg::TUID _iid, void** obj)
{
	if (Steinberg::FUnknownPrivate::iidEqual(cid, Steinberg::Vst::IMessage::iid) && Steinberg::FUnknownPrivate::iidEqual(_iid, Steinberg::Vst::IMessage::iid))
	{
		*obj = new Steinberg::Vst::HostMessage;
		return Steinberg::kResultTrue;
	}
	if (Steinberg::FUnknownPrivate::iidEqual(cid, Steinberg::Vst::IAttributeList::iid) && Steinberg::FUnknownPrivate::iidEqual(_iid, Steinberg::Vst::IAttributeList::iid))
	{
		if (auto al = Steinberg::Vst::HostAttributeList::make())
		{
			*obj = al.take ();
			return Steinberg::kResultTrue;
		}
		return Steinberg::kOutOfMemory;
	}
	*obj = nullptr;
	return Steinberg::kResultFalse;
}

Steinberg::tresult PLUGIN_API Vst3HostApplication::queryInterface (const char* _iid, void** obj)
{
    std::cout << "Vst3HostApplication queryInterface called." << std::endl;
    std::cout << "Vst3HostApplication queryInterface checking for Funknown." << std::endl;
	QUERY_INTERFACE (_iid, obj, Steinberg::FUnknown::iid, Steinberg::Vst::IHostApplication)
    std::cout << "Vst3HostApplication queryInterface checking for IHostApplication." << std::endl;
	QUERY_INTERFACE (_iid, obj, Steinberg::Vst::IHostApplication::iid, Steinberg::Vst::IHostApplication)

    std::cout << "Vst3HostApplication queryInterface checking IRunLoop." << std::endl;
    if (runLoop.get()->queryInterface(_iid, obj) == Steinberg::kResultTrue)
    {
        std::cout << "Vst3HostApplication queryInterface returning IRunLoop." << std::endl;
        return runLoop.get()->queryInterface(_iid, obj);
    }

    std::cout << "Vst3HostApplication queryInterface checking PlugInterfaceSupport." << std::endl;
	if (plugInterfaceSupport && plugInterfaceSupport->queryInterface(_iid, obj) == Steinberg::kResultTrue)
	{
       return Steinberg::kResultOk;
	}

    std::cout << "Vst3HostApplication queryInterface no matches." << std::endl;
    *obj = nullptr;
    return Steinberg::kNoInterface;
}

Steinberg::uint32 PLUGIN_API Vst3HostApplication::addRef ()
{
	return 1;
}

Steinberg::uint32 PLUGIN_API Vst3HostApplication::release ()
{
	return 1;
}


class ComponentHandler : public Steinberg::Vst::IComponentHandler
{
public:
    ComponentHandler(
        rust::Box<Vst3Host> vst3Sender,
        rust::Fn<rust::Box<Vst3Host>(rust::Box<Vst3Host> context, int32_t param_id, float param_value)> sendParameterChangeFunc)
        : vst3Host(std::move(vst3Sender)), sendParameterChange(sendParameterChangeFunc)
    {  }
    ~ComponentHandler() {  }

	Steinberg::tresult PLUGIN_API beginEdit (Steinberg::Vst::ParamID id) override
	{
		std::cout << "beginEdit: id=" << id << std::endl;
		currentParamId = id;
		return Steinberg::kNotImplemented;
	}
	Steinberg::tresult PLUGIN_API performEdit (Steinberg::Vst::ParamID id, Steinberg::Vst::ParamValue valueNormalized) override
	{
		std::cout << "performEdit: id=" << id << ", valueNormalized=" << valueNormalized << std::endl;
		if (currentParamId == id)
		{
		    vst3Host = sendParameterChange(std::move(vst3Host), static_cast<int32_t>(id), static_cast<float>(valueNormalized));
		}
		return Steinberg::kNotImplemented;
	}
	Steinberg::tresult PLUGIN_API endEdit (Steinberg::Vst::ParamID id) override
	{
		std::cout << "endEdit: id=" << id << std::endl;
		currentParamId = fakeParamId;
		return Steinberg::kNotImplemented;
	}
	Steinberg::tresult PLUGIN_API restartComponent (Steinberg::int32 flags) override
	{
		std::cout << "restartComponent: flags=" << flags << std::endl;
		return Steinberg::kNotImplemented;
	}

	Steinberg::tresult PLUGIN_API queryInterface (const Steinberg::TUID _iid, void** obj) override
	{
        return Steinberg::kNoInterface;
	}

	Steinberg::uint32 PLUGIN_API addRef () override { return 1000; }
	Steinberg::uint32 PLUGIN_API release () override { return 1000; }

private:
    rust::Box<Vst3Host> vst3Host;
    rust::Fn<rust::Box<Vst3Host>(rust::Box<Vst3Host> context, int32_t param_id, float param_value)> sendParameterChange;
    const uint32_t fakeParamId = 999999999;
    Steinberg::Vst::ParamID currentParamId = fakeParamId; // allow one at a time for now
};

class Vst3PluginHandler
{
public:
    Vst3PluginHandler();
    ~Vst3PluginHandler();

    bool setActive(bool active);
    bool setProcessing(bool startProcessing);
    bool process(rust::Slice<const float> channel1InputBuffer, rust::Slice<const float> channel2InputBuffer, rust::Slice<float> channel1OutputBuffer, rust::Slice<float> channel2OutputBuffer);
    bool initialise(
        std::string daw_plugin_uuid,
        std::string vst3_plugin_path,
        std::string vst3_plugin_uid,
        double sampleRate,
        int32_t blockSize,
        rust::Box<Vst3Host> vst3Host,
        rust::Fn<rust::Box<Vst3Host>(rust::Box<Vst3Host> context, int32_t param_id, float param_value)> sendParameterChange
        );

    bool createView(
        uint32_t xid,
        rust::Box<Vst3Host> vst3Host,
        rust::Fn<rust::Box<Vst3Host>(rust::Box<Vst3Host> context, int32_t new_window_width, int32_t new_window_height)> sendPluginWindowResize
    );
    Steinberg::ViewRect* getViewSize();

    bool addEvent(EventType eventType, int32_t blockPosition, uint32_t data1, uint32_t data2, int32_t data3, double data4);
    bool addParameterChange();

    void getPresetData();

    std::string& getName();

    Vst3HostApplication* getHostApplication() {return hostApplication.get();}
    Steinberg::Vst::IComponent* getComponentPtr() {return component.get();}
    Steinberg::Vst::IEditController* getEditControllerPtr() {return editController.get();}
    Steinberg::IPlugView* getPlugViewPtr()
    {
        if (plugView == nullptr)
        {
            return nullptr;
        }
        return plugView.get();
    }
    SimplePlugFrame* getPlugFramePtr()
    {
        if (simplePlugFrame == nullptr)
        {
            return nullptr;
        }
        return simplePlugFrame.get();
    }

private:
    VST3::Hosting::Module::Ptr module = nullptr;

    Steinberg::IPtr<Vst3HostApplication> hostApplication = nullptr;

    Steinberg::IPtr<Steinberg::Vst::PlugProvider> plugProvider = nullptr;

    Steinberg::IPtr<Steinberg::Vst::IComponent> component = nullptr;
    Steinberg::FUnknownPtr<Steinberg::Vst::IAudioProcessor> audioProcessor = nullptr;
    Steinberg::IPtr<Steinberg::Vst::IEditController> editController = nullptr;
    Steinberg::IPtr<ComponentHandler> componentHandler = nullptr;
    Steinberg::IPtr<Steinberg::IPlugView> plugView = nullptr;
    Steinberg::IPtr<SimplePlugFrame> simplePlugFrame = nullptr;

    Steinberg::Vst::ProcessSetup processSetUp = {};
    Steinberg::Vst::ProcessContext processContext = {};
    Steinberg::Vst::ProcessData processData;

    Steinberg::Vst::SampleRate sampleRate = 44100.0;
    int32_t blockSize = 1024;

    Steinberg::Vst::AudioBusBuffers* inputAudioBuffers = nullptr;
    Steinberg::Vst::AudioBusBuffers* outputAudioBuffers = nullptr;

    std::string daw_plugin_uuid;
    std::string name;
};

Vst3PluginHandler::Vst3PluginHandler() {}

Vst3PluginHandler::~Vst3PluginHandler()
{
    // need to null out everything

}

bool Vst3PluginHandler::initialise(
    std::string daw_plugin_uuid,
    std::string vst3_plugin_path,
    std::string vst3_plugin_uid,
    double sampleRate,
    int32_t blockSize,
    rust::Box<Vst3Host> vst3Host,
    rust::Fn<rust::Box<Vst3Host>(rust::Box<Vst3Host> context, int32_t param_id, float param_value)> sendParameterChange)
{
    std::string path(vst3_plugin_path);
    std::cout << "Path: " << path << std::endl;
    std::string plugin_uid(vst3_plugin_uid);
    std::cout << "Plugin UID: " << plugin_uid << std::endl;
    std::string error;
    hostApplication = Steinberg::owned(new Vst3HostApplication());

    this->daw_plugin_uuid.append(daw_plugin_uuid);

    module = VST3::Hosting::Module::create(path, error);
    if (! module)
        return false;

    for (auto& classInfo : module->getFactory().classInfos())
    {
        if (classInfo.category() == kVstAudioEffectClass && classInfo.ID().toString().compare(plugin_uid) == 0)
        {
            const VST3::Hosting::PluginFactory& factory = module->getFactory();
            factory.setHostContext(static_cast<Steinberg::FUnknown*>(hostApplication.get()));

            plugProvider = Steinberg::owned(new Steinberg::Vst::PlugProvider(factory, classInfo, true));
            std::cout << "Created PlugProvider." << std::endl;

            Steinberg::Vst::PluginContextFactory::instance().setPluginContext(static_cast<Steinberg::FUnknown*>(hostApplication.get()));

            if (plugProvider.get()->initialize() )
            {
                std::cout << "Initialised PlugProvider." << std::endl;
                component = plugProvider.get()->getComponentPtr();
                audioProcessor = component.get();
                editController = plugProvider.get()->getControllerPtr();

                componentHandler = Steinberg::owned(new ComponentHandler(std::move(vst3Host), std::move(sendParameterChange)));
                editController.get()->setComponentHandler(componentHandler.get());

                processSetUp.processMode  = Steinberg::Vst::ProcessModes::kRealtime;
                processSetUp.symbolicSampleSize = Steinberg::Vst::SymbolicSampleSizes::kSample32;
                processSetUp.maxSamplesPerBlock = blockSize;
                processSetUp.sampleRate = sampleRate;

                if (audioProcessor->setupProcessing(processSetUp) != Steinberg::kResultOk)
                {
                    std::cout << "Failed to setup processing for the audio processor." << std::endl;
                    return false;
                }

                auto inputAudioBusCount = component.get()->getBusCount(Steinberg::Vst::MediaTypes::kAudio, Steinberg::Vst::BusDirections::kInput);
                for (auto index = 0; index < inputAudioBusCount; index++)
                {
                    component.get()->activateBus(Steinberg::Vst::kAudio, Steinberg::Vst::kInput, index, true);
                }

                auto outputAudioBusCount = component.get()->getBusCount(Steinberg::Vst::MediaTypes::kAudio, Steinberg::Vst::BusDirections::kOutput);
                for (auto index = 0; index < outputAudioBusCount; index++)
                {
                    component.get()->activateBus(Steinberg::Vst::kAudio, Steinberg::Vst::kOutput, index, true);
                }

                auto inputEventBusCount = component.get()->getBusCount(Steinberg::Vst::MediaTypes::kEvent, Steinberg::Vst::BusDirections::kInput);
                for (auto index = 0; index < inputEventBusCount; index++)
                {
                    component.get()->activateBus(Steinberg::Vst::kEvent, Steinberg::Vst::kInput, index, true);
                }

                auto outputEventBusCount = component.get()->getBusCount(Steinberg::Vst::MediaTypes::kEvent, Steinberg::Vst::BusDirections::kOutput);
                for (auto index = 0; index < outputEventBusCount; index++)
                {
                    component.get()->activateBus(Steinberg::Vst::kEvent, Steinberg::Vst::kOutput, index, true);
                }

                Steinberg::TBool okToProcess = true;
                audioProcessor->setProcessing(okToProcess);
                // the following commented out code bombs out for u-he plugins because they don't return kResultOK
                // - putting in a 5s delay does not help
                // - the u-he plugins work anyway if you ignore setProcessing(true) not returning kResultOK
//                if (audioProcessor->setProcessing(okToProcess) != Steinberg::kResultOk)
//                {
//                    std::cout << "Failed to set the audio processor to processing." << std::endl;
//                    return false;
//                }

                processContext.state = 0;
                processContext.state = Steinberg::Vst::ProcessContext::StatesAndFlags::kPlaying;
                processContext.state |= Steinberg::Vst::ProcessContext::kSystemTimeValid;
                processContext.state |= Steinberg::Vst::ProcessContext::kTempoValid;
                processContext.state |= Steinberg::Vst::ProcessContext::kTimeSigValid;
                processContext.state |= Steinberg::Vst::ProcessContext::kContTimeValid;
                processContext.state |= Steinberg::Vst::ProcessContext::kSystemTimeValid;
                processContext.sampleRate = sampleRate;
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
                processContext.smpteOffsetSubframes = 0;
                processContext.frameRate = Steinberg::Vst::FrameRate {
                    sampleRate,
                    Steinberg::Vst::FrameRate::kPullDownRate
                };
                processContext.samplesToNextClock = 0;

                processData.processMode = Steinberg::Vst::ProcessModes::kRealtime;
                processData.symbolicSampleSize = processSetUp.symbolicSampleSize;
                processData.numSamples = processSetUp.maxSamplesPerBlock;

                processData.numInputs = inputAudioBusCount;
                processData.inputs = new Steinberg::Vst::AudioBusBuffers[inputAudioBusCount];
                for (auto index = 0; index < inputAudioBusCount; index++)
                {
                    Steinberg::Vst::BusInfo inputAudioBusInfo;
                    component.get()->getBusInfo(Steinberg::Vst::MediaTypes::kAudio, Steinberg::Vst::BusDirections::kInput, index, inputAudioBusInfo);
                    processData.inputs[index].numChannels = inputAudioBusInfo.channelCount;
                    processData.inputs[index].channelBuffers32 = new Steinberg::Vst::Sample32*[inputAudioBusInfo.channelCount];

                    for (auto channelIndex = 0; channelIndex < inputAudioBusInfo.channelCount; channelIndex++)
                    {
                        processData.inputs[index].channelBuffers32[channelIndex] = new Steinberg::Vst::Sample32[processSetUp.maxSamplesPerBlock];
                    }
                }

                processData.numOutputs = outputAudioBusCount;
                processData.outputs = new Steinberg::Vst::AudioBusBuffers[outputAudioBusCount];
                for (auto index = 0; index < outputAudioBusCount; index++)
                {
                    Steinberg::Vst::BusInfo outputAudioBusInfo;
                    component.get()->getBusInfo(Steinberg::Vst::MediaTypes::kAudio, Steinberg::Vst::BusDirections::kOutput, index, outputAudioBusInfo);
                    processData.outputs[index].numChannels = outputAudioBusInfo.channelCount;
                    processData.outputs[index].channelBuffers32 = new Steinberg::Vst::Sample32*[outputAudioBusInfo.channelCount];

                    for (auto channelIndex = 0; channelIndex < outputAudioBusInfo.channelCount; channelIndex++)
                    {
                        processData.outputs[index].channelBuffers32[channelIndex] = new Steinberg::Vst::Sample32[processSetUp.maxSamplesPerBlock];
                    }
                }

                processData.inputEvents = new Steinberg::Vst::EventList[inputEventBusCount];
                processData.outputEvents = new Steinberg::Vst::EventList[outputEventBusCount];
                processData.inputParameterChanges = new Steinberg::Vst::ParameterChanges(20000);
                processData.outputParameterChanges = new Steinberg::Vst::ParameterChanges(20000);
                processData.processContext = &processContext;

                if (component.get()->setActive(true) != Steinberg::kResultTrue)
                {
                    std::cout << "Failed to set the component to active." << std::endl;
                    return false;
                }

                name = classInfo.name();

                std::flush(std::cout);

                return true;
            }
        }
    }

    return false;
}

bool Vst3PluginHandler::setActive(bool active)
{
    if (component.get()->setActive(active) != Steinberg::kResultTrue)
    {
        std::cout << "Failed to set the component to active with value: " << active << std::endl;
        return false;
    }

    return true;
}

bool Vst3PluginHandler::setProcessing(bool startProcessing)
{
    if (audioProcessor && audioProcessor->setProcessing(startProcessing) != Steinberg::kResultOk)
    {
        std::cout << "Failed to set the audio processor to processing with value: " << startProcessing << std::endl;
        return false;
    }

    return true;
}

bool Vst3PluginHandler::process(
    rust::Slice<const float> channel1InputBuffer,
    rust::Slice<const float> channel2InputBuffer,
    rust::Slice<float> channel1OutputBuffer,
    rust::Slice<float> channel2OutputBuffer
)
{
    try
    {
        // copy rust side input channel buffers to audio input
        if (component.get()->getBusCount(Steinberg::Vst::MediaTypes::kAudio, Steinberg::Vst::BusDirections::kInput) > 0)
        {
            for(auto index =0; index < processSetUp.maxSamplesPerBlock; index++)
            {
                processData.inputs[0].channelBuffers32[0][index] = channel1InputBuffer[index];
                processData.inputs[0].channelBuffers32[1][index] = channel2InputBuffer[index];
            }
        }

        if (audioProcessor && audioProcessor->process(processData) == Steinberg::kResultFalse)
        {
            std::cout << "Failed to get the audio processor to process." << std::endl;
            return false;
        }

        processContext.projectTimeSamples += 1024;
        processContext.systemTime = std::chrono::duration_cast<std::chrono::nanoseconds>(std::chrono::system_clock::now().time_since_epoch()).count();
        processContext.continousTimeSamples += 1024;

        auto inputEventBusCount = component.get()->getBusCount(Steinberg::Vst::MediaTypes::kEvent, Steinberg::Vst::BusDirections::kInput);
        for (auto index = 0; index < inputEventBusCount; index++)
        {
            static_cast<Steinberg::Vst::EventList&>(processData.inputEvents[index]).clear();
        }

        auto outputEventBusCount = component.get()->getBusCount(Steinberg::Vst::MediaTypes::kEvent, Steinberg::Vst::BusDirections::kOutput);
        for (auto index = 0; index < outputEventBusCount; index++)
        {
            for (auto eventIndex = 0; eventIndex < processData.outputEvents[index].getEventCount(); eventIndex++)
            {
                // TODO do something with the output events
                std::cout << "Found an output event." << std::endl;
            }
            static_cast<Steinberg::Vst::EventList&>(processData.outputEvents[index]).clear();
        }

        static_cast<Steinberg::Vst::ParameterChanges*>(processData.inputParameterChanges)->clearQueue();

        // copy audio output to rust side input channel buffers
        for(auto index =0; index < processSetUp.maxSamplesPerBlock; index++)
        {
            channel1OutputBuffer[index] = processData.outputs[0].channelBuffers32[0][index];
            channel2OutputBuffer[index] = processData.outputs[0].channelBuffers32[1][index];
        }
    }
    catch(const std::out_of_range& ex)
    {
        std::cout << "Failed to find process data: " << daw_plugin_uuid << std::endl;
        return false;
    }

    return true;
}

bool Vst3PluginHandler::createView(
    uint32_t xid,
    rust::Box<Vst3Host> vst3Host,
    rust::Fn<rust::Box<Vst3Host>(rust::Box<Vst3Host> context, int32_t new_window_width, int32_t new_window_height)> sendPluginWindowResize
)
{
    if (plugView == nullptr)
    {
        plugView = editController.get()->createView(Steinberg::Vst::ViewType::kEditor);

        if (plugView)
        {
            simplePlugFrame = Steinberg::owned(new SimplePlugFrame(std::move(vst3Host), std::move(sendPluginWindowResize)));
            plugView.get()->setFrame(simplePlugFrame.get());

            if (plugView->attached((void *)xid, Steinberg::kPlatformTypeX11EmbedWindowID) != Steinberg::kResultOk)
            {
                std::cout << "Failed to open window." << std::endl;
                return false;
            }
        }
    }

    return true;
}

Steinberg::ViewRect* Vst3PluginHandler::getViewSize()
{
    Steinberg::ViewRect* viewRect = new Steinberg::ViewRect(1, 1, 1, 1);
    if (plugView && plugView != nullptr)
    {
        plugView->getSize(viewRect);
    }

    return viewRect;
}

bool Vst3PluginHandler::addEvent(EventType eventType, int32_t blockPosition, uint32_t data1, uint32_t data2, int32_t data3, double data4)
{
    if (component && component.get()->getBusCount(Steinberg::Vst::MediaTypes::kEvent, Steinberg::Vst::BusDirections::kInput) > 0)
    {
        Steinberg::Vst::Event event = {};
        event.busIndex = 0;
        event.sampleOffset = blockPosition;
        event.ppqPosition = 0.0;
        event.flags = Steinberg::Vst::Event::EventFlags::kIsLive;

        switch(eventType) {
            case EventType::NoteOn:
            {
                std::cout << "Vst3PluginHandler::addEvent - note on: noteId=" << data3 << std::endl;
                event.type = Steinberg::Vst::Event::kNoteOnEvent;
                event.noteOn.noteId = data3;
                event.noteOn.channel = 0;
                event.noteOn.pitch = static_cast<Steinberg::int16>(data1);
                event.noteOn.velocity = static_cast<float>(data2) / 127.0;
                event.noteOn.tuning = 0.0;
                processData.inputEvents[0].addEvent(event);
                break;
            }
            case EventType::NoteOff:
            {
                std::cout << "Vst3PluginHandler::addEvent - note off: noteId=" << data3 << std::endl;
                event.type = Steinberg::Vst::Event::kNoteOffEvent;
                event.noteOff.noteId = data3;
                event.noteOff.channel = 0;
                event.noteOff.pitch = static_cast<Steinberg::int16>(data1);
                event.noteOff.velocity = static_cast<float>(data2) / 127.0;
                event.noteOff.tuning = 0.0;
                processData.inputEvents[0].addEvent(event);
                break;
            }
            case EventType::KeyPressureAfterTouch:
            {
                std::cout << "Vst3PluginHandler::addEvent - key poly pressure after touch" << std::endl;
                event.type = Steinberg::Vst::Event::kPolyPressureEvent;
                event.polyPressure.channel = 0;
                event.polyPressure.pitch = data1;
                event.polyPressure.pressure = static_cast<float>(data2) / 127.0f;
                processData.inputEvents[0].addEvent(event);
                break;
            }
            case EventType::Controller:
            {
                std::cout << "Vst3PluginHandler::addEvent - controller" << std::endl;
                // need to get the controller mapping
                Steinberg::Vst::IMidiMapping* midiMapping = nullptr;
                if (editController.get()->queryInterface(Steinberg::Vst::IMidiMapping::iid, (void**)&midiMapping) == Steinberg::kResultOk)
                {
                    Steinberg::Vst::ParamID id = 0;
                    midiMapping->getMidiControllerAssignment(0, 0, data1, id);
                    int32_t index = 0;
                    Steinberg::Vst::IParamValueQueue *parameterQueue = processData.inputParameterChanges->addParameterData(id , index);
                    std::cout << "Parameter: blockPosition=" << blockPosition << ", controller=" << data1 << ", value=" << (static_cast<double>(data2) / 127.0) << ", index=" << index << ", id=" << id << std::endl;
                    if (parameterQueue && parameterQueue->addPoint(blockPosition, (static_cast<double>(data2) / 127.0), index))
                    {
                        std::cout << "Problem adding parameter to the queue." << std::endl;
                    }
                }
                break;
            }
            case EventType::PitchBend:
            {
                std::cout << "Vst3PluginHandler::addEvent - pitch bend" << std::endl;
                // need to get the pitch bend mapping
                Steinberg::Vst::IMidiMapping* midiMapping = nullptr;
                if (editController.get()->queryInterface(Steinberg::Vst::IMidiMapping::iid, (void**)&midiMapping) == Steinberg::kResultOk)
                {
                    Steinberg::Vst::ParamID id = 0;
                    midiMapping->getMidiControllerAssignment(0, 0, Steinberg::Vst::ControllerNumbers::kPitchBend, id);
                    int32_t index = 0;
                    Steinberg::Vst::IParamValueQueue *parameterQueue = processData.inputParameterChanges->addParameterData(id , index);
                    std::cout << "Parameter: blockPosition=" << blockPosition << ", value=" << (static_cast<float>(data3 + 8192) / 16384.0) << ", index=" << index << ", id=" << id << std::endl;
                    if (parameterQueue && parameterQueue->addPoint(blockPosition, (static_cast<float>(data3 + 8192) / 16384.0), index))
                    {
                        std::cout << "Problem adding parameter to the queue." << std::endl;
                    }
                }
                break;
            }
            case EventType::Parameter:
            {
                int32_t index = 0;
                Steinberg::Vst::IParamValueQueue *parameterQueue = processData.inputParameterChanges->addParameterData(data1, index);
                std::cout << "Parameter: blockPosition=" << blockPosition << ", value=" << (static_cast<double>(data2) / 127.0) << ", index=" << index << std::endl;
                if (parameterQueue && parameterQueue->addPoint(blockPosition, (static_cast<double>(data2) / 127.0), index))
                {
                    std::cout << "Problem adding parameter to the queue." << std::endl;
                }
                break;
            }
            case EventType::NoteExpression:
            {
                std::cout << "Vst3PluginHandler::addEvent - note expression: type=" << data1 << ", noteId=" << data3 << ", value=" << data4 << std::endl;
                event.type = Steinberg::Vst::Event::kNoteExpressionValueEvent;
                event.noteExpressionValue.typeId = data1;
                event.noteExpressionValue.noteId = data3;
                event.noteExpressionValue.value = data4;
                processData.inputEvents[0].addEvent(event);
                break;
            }
        }

        return true;
    }

    return false;
}

bool Vst3PluginHandler::addParameterChange()
{
    return true;
}

std::string& Vst3PluginHandler::getName()
{
    return name;
}

void Vst3PluginHandler::getPresetData()
{
    if (component)
    {
        Steinberg::Vst::BufferStream bufferStream;
        component.get()->getState(&bufferStream);
    }
}

thread_local std::map<std::string, Vst3PluginHandler> vst3Plugins;

bool createPlugin(
    rust::String vst3_plugin_path,
    rust::String riff_daw_plugin_uuid,
    rust::String vst3_plugin_uid,
    double sampleRate,
    int32_t blockSize,
    rust::Box<Vst3Host> vst3Host,
    rust::Fn<rust::Box<Vst3Host>(rust::Box<Vst3Host> context, int32_t param_id, float param_value)> sendParameterChange
)
{
    Vst3PluginHandler vst3Plugin;
    std::string daw_plugin_uuid(riff_daw_plugin_uuid);
    std::cout << "createPlugin called for plugin uuid: " << daw_plugin_uuid << std::endl;

    if (!vst3Plugin.initialise(daw_plugin_uuid, std::string(vst3_plugin_path), std::string(vst3_plugin_uid), sampleRate, blockSize, std::move(vst3Host), std::move(sendParameterChange)))
    {
        std::cout << "Failed to create functional vst3 plugin." << std::endl;
        return false;
    }

    vst3Plugins[daw_plugin_uuid] = vst3Plugin;

    std::cout << "Dumping plugin UUIDs for thread id=" << std::this_thread::get_id() << std::endl;
    for(auto const& [key, value] : vst3Plugins)
    {
        std::cout << "key=" << key << std::endl;
    }
    std::cout << "Finished dumping plugin UUIDs for thread id=" << std::this_thread::get_id() << std::endl;

    return true;
}

bool showPluginEditor(
    rust::String riff_daw_plugin_uuid,
    uint32_t xid,
    rust::Box<Vst3Host> vst3Host,
    rust::Fn<rust::Box<Vst3Host>(rust::Box<Vst3Host> context, int32_t new_window_width, int32_t new_window_height)> sendPluginWindowResize
)
{
    try
    {
        Vst3PluginHandler& vst3PluginHandler = vst3Plugins.at(std::string(riff_daw_plugin_uuid));
        return vst3PluginHandler.createView(xid, std::move(vst3Host), std::move(sendPluginWindowResize));
    }
    catch(const std::out_of_range& ex)
    {
        std::cout << "Failed to find vst3 plugin: " << riff_daw_plugin_uuid << std::endl;
        return false;
    }

    return true;
}


uint32_t vst3_plugin_get_window_height(rust::String riff_daw_plugin_uuid)
{
    try
    {
        Vst3PluginHandler& vst3PluginHandler = vst3Plugins.at(std::string(riff_daw_plugin_uuid));

        return vst3PluginHandler.getViewSize()->getHeight();
    }
    catch(const std::out_of_range& ex)
    {
        std::cout << "vst3_plugin_get_window_height: Can't find plug in." << std::endl;
    }

    return 800;
}

uint32_t vst3_plugin_get_window_width(rust::String riff_daw_plugin_uuid)
{
    try
    {
        Vst3PluginHandler& vst3PluginHandler = vst3Plugins.at(std::string(riff_daw_plugin_uuid));

        return vst3PluginHandler.getViewSize()->getWidth();
    }
    catch(const std::out_of_range& ex)
    {
        std::cout << "vst3_plugin_get_window_width: Can't find plugin." << std::endl;
    }

    return 600;
}

void vst3_plugin_get_window_refresh(rust::String riff_daw_plugin_uuid)
{
    try
    {
        Vst3PluginHandler& vst3PluginHandler = vst3Plugins.at(std::string(riff_daw_plugin_uuid));
        Steinberg::Vst::IMessage* refreshMessage = allocateMessage(vst3PluginHandler.getHostApplication());
    }
    catch(const std::out_of_range& ex)
    {
        std::cout << "vst3_plugin_get_window_refresh: Can't find plugin." << std::endl;
    }
}

bool vst3_plugin_process(
    rust::String riff_daw_plugin_uuid,
    rust::Slice<const float> channel1InputBuffer,
    rust::Slice<const float> channel2InputBuffer,
    rust::Slice<float> channel1OutputBuffer,
    rust::Slice<float> channel2OutputBuffer
)
{
    try
    {
        Vst3PluginHandler& vst3PluginHandler = vst3Plugins.at(std::string(riff_daw_plugin_uuid));

        return vst3PluginHandler.process(channel1InputBuffer, channel2InputBuffer, channel1OutputBuffer, channel2OutputBuffer);
    }
    catch(const std::out_of_range& ex)
    {
        std::cout << "vst3_plugin_process: Can't find plugin." << std::endl;
    }

    return false;
}

bool addEvent(rust::String riff_daw_plugin_uuid, EventType eventType, int32_t blockPosition, uint32_t data1, uint32_t data2, int32_t data3, double data4)
{
    try
    {
        Vst3PluginHandler& vst3PluginHandler = vst3Plugins.at(std::string(riff_daw_plugin_uuid));

        return vst3PluginHandler.addEvent(eventType, blockPosition, data1, data2, data3, data4);
    }
    catch(const std::out_of_range& ex)
    {
        std::cout << "addEvent: Can't find plugin." << std::endl;
    }

    return false;
}

rust::String getVstPluginName(rust::String riff_daw_plugin_uuid)
{
    try
    {
        Vst3PluginHandler& vst3PluginHandler = vst3Plugins.at(std::string(riff_daw_plugin_uuid));

        return std::string(vst3PluginHandler.getName());
    }
    catch(const std::out_of_range& ex)
    {
        std::cout << "getVstPluginName: Can't find plugin." << std::endl;
    }

    return std::string("Failed to get vst3 plugin name.");
}

bool setProcessing(rust::String riff_daw_plugin_uuid, bool processing)
{
    try
    {
        Vst3PluginHandler& vst3PluginHandler = vst3Plugins.at(std::string(riff_daw_plugin_uuid));

        return vst3PluginHandler.setProcessing(processing);
    }
    catch(const std::out_of_range& ex)
    {
        std::cout << "setProcessing: Can't find plugin." << std::endl;
    }

    return false;
}

bool setActive(rust::String riff_daw_plugin_uuid, bool active)
{
    try
    {
        Vst3PluginHandler& vst3PluginHandler = vst3Plugins.at(std::string(riff_daw_plugin_uuid));

        return vst3PluginHandler.setActive(active);
    }
    catch(const std::out_of_range& ex)
    {
        std::cout << "setActive: Can't find plugin." << std::endl;
    }

    return false;
}

int32_t vst3_plugin_get_preset(rust::String riff_daw_plugin_uuid, rust::Slice<uint8_t> preset_buffer, uint32_t maxSize)
{
    try
    {
        Vst3PluginHandler& vst3PluginHandler = vst3Plugins.at(std::string(riff_daw_plugin_uuid));
        PresetStream presetStream(preset_buffer);
        Steinberg::Vst::IComponent* component = vst3PluginHandler.getComponentPtr();
        component->getState(&presetStream);

        return presetStream.getBytesWritten();
    }
    catch(const std::out_of_range& ex)
    {
        std::cout << "vst3_plugin_get_preset: Can't find plugin." << std::endl;
    }

    return 0;
}

void vst3_plugin_set_preset(rust::String riff_daw_plugin_uuid, rust::Slice<uint8_t> preset_buffer)
{
    try
    {
        Vst3PluginHandler& vst3PluginHandler = vst3Plugins.at(std::string(riff_daw_plugin_uuid));
        PresetStream presetStream(preset_buffer);
        Steinberg::Vst::IComponent* component = vst3PluginHandler.getComponentPtr();
        component->setState(&presetStream);
    }
    catch(const std::out_of_range& ex)
    {
        std::cout << "vst3_plugin_set_preset: Can't find plugin." << std::endl;
    }
}

int32_t vst3_plugin_get_parameter_count(rust::String riff_daw_plugin_uuid)
{
    try
    {
        Vst3PluginHandler& vst3PluginHandler = vst3Plugins.at(std::string(riff_daw_plugin_uuid));
        Steinberg::Vst::IEditController* editController = vst3PluginHandler.getEditControllerPtr();

        int parameterCount = editController->getParameterCount();
//        std::cout << "Vst3 plugin parameter count=" << parameterCount << std::endl;

        return parameterCount;
    }
    catch(const std::out_of_range& ex)
    {
        std::cout << "vst3_plugin_get_parameter_count: Can't find plugin." << std::endl;
    }

    return 0;
}

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
)
{
    try
    {
        Vst3PluginHandler& vst3PluginHandler = vst3Plugins.at(std::string(riff_daw_plugin_uuid));
        Steinberg::Vst::IEditController* editController = vst3PluginHandler.getEditControllerPtr();

        int parameterCount = editController->getParameterCount();
//        std::cout << "Vst3 plugin parameter count=" << parameterCount << std::endl;

        if (index < parameterCount)
        {
            Steinberg::Vst::ParameterInfo parameterInfo = {};
            if (editController->getParameterInfo(index, parameterInfo) == Steinberg::kResultOk)
            {
//                 std::cout << "Parameter: index=" << index << ", id=" << parameterInfo.id << ", step_count=" << parameterInfo.stepCount
//                  << ", default normalised value=" << parameterInfo.defaultNormalizedValue << ", unit id=" << parameterInfo.unitId << ", flags=" << parameterInfo.flags << std::endl;
                for(auto charIndex = 0; charIndex < 128; charIndex++)
                {
                    title[charIndex] = parameterInfo.title[charIndex];
                    short_title[charIndex] = parameterInfo.shortTitle[charIndex];
                    units[charIndex] = parameterInfo.units[charIndex];
                }

                id = parameterInfo.id;
                step_count = parameterInfo.stepCount;
                default_normalised_value = parameterInfo.defaultNormalizedValue;
                unit_id = parameterInfo.unitId;
                flags = parameterInfo.flags;
            }
        }
    }
    catch(const std::out_of_range& ex)
    {
        std::cout << "vst3_plugin_get_parameter_info: Can't find plugin." << std::endl;
    }
}

void vst3_plugin_remove(rust::String riff_daw_plugin_uuid)
{
    try
    {
        std::string daw_plugin_uuid(riff_daw_plugin_uuid);
        std::cout << "vst3_plugin_remove called for plugin uuid: " << daw_plugin_uuid << std::endl;
        Vst3PluginHandler& vst3PluginHandler = vst3Plugins[daw_plugin_uuid];
        std::cout << "vst3_plugin_remove found vst3 plugin." << std::endl;

        Steinberg::IPlugView* plugView = vst3PluginHandler.getPlugViewPtr();
        if (plugView)
        {
            std::cout << "vst3_plugin_remove retrieved IPlugView." << std::endl;
            Steinberg::IPlugFrame* plugFrame = vst3PluginHandler.getPlugFramePtr();
            if (plugFrame != nullptr)
            {
                std::cout << "vst3_plugin_remove retrieved IPlugFrame." << std::endl;
                static_cast<SimplePlugFrame*>(plugFrame)->shutdownRunLoop();
    //            plugView->setFrame(nullptr);
            }
    //        std::cout << "vst3_plugin_remove calling IPlugView->removed()." << std::endl;
    //        plugView->removed();
        }

//        std::cout << "vst3_plugin_remove calling vst3Plugins.erase(daw_plugin_uuid)." << std::endl;
//        vst3Plugins.erase(daw_plugin_uuid);
    }
    catch(const std::exception& e)
    {
        std::cout << "Exception: " << e.what() << std::endl;
    }
}

} // namespace hremeviuc
} // namespace org
