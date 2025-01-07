// Adapted from the JUCE vst3 headers file

#define RELEASE 1

#include <base/source/fstring.h>
#include <pluginterfaces/base/conststringtable.h>
#include <pluginterfaces/base/funknown.h>
#include <pluginterfaces/base/ipluginbase.h>
#include <pluginterfaces/base/iplugincompatibility.h>
#include <pluginterfaces/base/ustring.h>
#include <pluginterfaces/gui/iplugview.h>
#include <pluginterfaces/gui/iplugviewcontentscalesupport.h>
#include <pluginterfaces/vst/ivstattributes.h>
#include <pluginterfaces/vst/ivstaudioprocessor.h>
#include <pluginterfaces/vst/ivstcomponent.h>
#include <pluginterfaces/vst/ivstcontextmenu.h>
#include <pluginterfaces/vst/ivsteditcontroller.h>
#include <pluginterfaces/vst/ivstevents.h>
#include <pluginterfaces/vst/ivsthostapplication.h>
#include <pluginterfaces/vst/ivstmessage.h>
#include <pluginterfaces/vst/ivstmidicontrollers.h>
#include <pluginterfaces/vst/ivstparameterchanges.h>
#include <pluginterfaces/vst/ivstplugview.h>
#include <pluginterfaces/vst/ivstprocesscontext.h>
#include <pluginterfaces/vst/vsttypes.h>
#include <pluginterfaces/vst/ivstunits.h>
#include <pluginterfaces/vst/ivstmidicontrollers.h>
#include <pluginterfaces/vst/ivstchannelcontextinfo.h>
#include <public.sdk/source/common/memorystream.h>
#include <public.sdk/source/common/threadchecker.h>
#include <public.sdk/source/vst/utility/uid.h>
#include <public.sdk/source/vst/utility/vst2persistence.h>
#include <public.sdk/source/vst/vsteditcontroller.h>
#include <public.sdk/source/vst/vstpresetfile.h>
#include <public.sdk/source/vst/hosting/module.h>
#include <public.sdk/source/vst/hosting/plugprovider.h>
#include <public.sdk/source/vst/utility/optional.h>
#include <public.sdk/source/vst/hosting/connectionproxy.h>
#include <public.sdk/source/vst/hosting/hostclasses.h>
#include <public.sdk/source/vst/hosting/pluginterfacesupport.h>

#include <base/source/baseiids.cpp>
#include <base/source/fbuffer.cpp>
#include <base/source/fdebug.cpp>
#include <base/source/fobject.cpp>
#include <base/source/fstreamer.cpp>
#include <base/source/fstring.cpp>

// The following shouldn't leak from fstring.cpp
#undef stricmp
#undef strnicmp
#undef snprintf
#undef vsnprintf
#undef snwprintf
#undef vsnwprintf

#if VST_VERSION >= 0x030608
#include <base/thread/source/flock.cpp>
#include <pluginterfaces/base/coreiids.cpp>
#else
#include <base/source/flock.cpp>
#endif

#pragma push_macro ("True")
#undef True
#pragma push_macro ("False")
#undef False

#include <base/source/updatehandler.cpp>
#include <pluginterfaces/base/conststringtable.cpp>
#include <pluginterfaces/base/funknown.cpp>
#include <pluginterfaces/base/ipluginbase.h>
#include <pluginterfaces/base/ustring.cpp>
#include <pluginterfaces/gui/iplugview.h>
#include <pluginterfaces/gui/iplugviewcontentscalesupport.h>
#include <pluginterfaces/vst/ivstchannelcontextinfo.h>
#include <pluginterfaces/vst/ivstmidicontrollers.h>
#include <public.sdk/source/common/commonstringconvert.cpp>
#include <public.sdk/source/common/memorystream.cpp>
#include <public.sdk/source/common/pluginview.cpp>
#include <public.sdk/source/common/threadchecker_linux.cpp>
#include <public.sdk/source/vst/hosting/hostclasses.cpp>
#include <public.sdk/source/vst/moduleinfo/moduleinfoparser.cpp>
#include <public.sdk/source/vst/utility/stringconvert.cpp>
#include <public.sdk/source/vst/utility/vst2persistence.cpp>
#include <public.sdk/source/vst/utility/uid.h>
#include <public.sdk/source/vst/hosting/connectionproxy.cpp>
#include <public.sdk/source/vst/vstbus.cpp>
#include <public.sdk/source/vst/vstcomponent.cpp>
#include <public.sdk/source/vst/vstcomponentbase.cpp>
#include <public.sdk/source/vst/vsteditcontroller.cpp>
#include <public.sdk/source/vst/vstinitiids.cpp>
#include <public.sdk/source/vst/vstparameters.cpp>
#include <public.sdk/source/vst/vstpresetfile.cpp>
#include <public.sdk/source/vst/hosting/pluginterfacesupport.cpp>
#include <public.sdk/source/vst/hosting/eventlist.h>
#include <public.sdk/source/vst/hosting/eventlist.cpp>
#include <public.sdk/source/vst/hosting/processdata.h>
#include <public.sdk/source/vst/hosting/processdata.cpp>