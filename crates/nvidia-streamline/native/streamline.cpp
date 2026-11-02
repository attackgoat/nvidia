#define WIN32_LEAN_AND_MEAN
#include <windows.h>

#include <cfloat>
#include <cstddef>
#include <cstdint>
#include <cstring>
#include <iterator>
#include <memory>
#include <new>
#include <utility>

#include <sl.h>
#include <sl_dlss_d.h>
#include <sl_security.h>

namespace
{
constexpr int32_t kInvalidArgument = -1000;
constexpr int32_t kInvalidSignature = -1001;
constexpr int32_t kLoadFailed = -1002;
constexpr int32_t kMissingFunction = -1003;
constexpr int32_t kOutOfMemory = -1004;
constexpr int32_t kUnexpectedException = -1005;
constexpr uint32_t kVkFormatR32Sfloat = 100;
constexpr uint32_t kDlssRrQualityBalanced = 0;
constexpr uint32_t kDlssRrQualityQuality = 1;
constexpr uint32_t kDlssRrQualityDlaa = 2;
constexpr uint32_t kDlssRrQualityPerformance = 3;
constexpr uint32_t kDlssRrQualityUltraPerformance = 4;

using NvidiaSlLogCallback = void (*)(int32_t type, const char* message);
NvidiaSlLogCallback g_logCallback = nullptr;

void streamlineLog(sl::LogType type, const char* message) noexcept
{
    if (g_logCallback != nullptr)
        g_logCallback(static_cast<int32_t>(type), message);
}

template <typename T>
bool loadFunction(HMODULE module, const char* name, T*& function) noexcept
{
    function = reinterpret_cast<T*>(GetProcAddress(module, name));
    return function != nullptr;
}

template <typename Function>
int32_t ffiBoundary(Function&& function) noexcept
{
    try
    {
        return std::forward<Function>(function)();
    }
    catch (const std::bad_alloc&)
    {
        return kOutOfMemory;
    }
    catch (...)
    {
        return kUnexpectedException;
    }
}

class Module
{
public:
    explicit Module(HMODULE handle) noexcept : handle_(handle) {}
    Module(const Module&) = delete;
    Module& operator=(const Module&) = delete;
    ~Module()
    {
        if (handle_ != nullptr)
            FreeLibrary(handle_);
    }

    HMODULE get() const noexcept { return handle_; }
    HMODULE release() noexcept
    {
        HMODULE handle = handle_;
        handle_ = nullptr;
        return handle;
    }

private:
    HMODULE handle_;
};

class LogCallbackGuard
{
public:
    LogCallbackGuard() = default;
    LogCallbackGuard(const LogCallbackGuard&) = delete;
    LogCallbackGuard& operator=(const LogCallbackGuard&) = delete;
    ~LogCallbackGuard()
    {
        if (!keep_)
            g_logCallback = nullptr;
    }

    void keep() noexcept { keep_ = true; }

private:
    bool keep_ = false;
};
}

struct NvidiaSlExtent
{
    uint32_t width;
    uint32_t height;
};

struct NvidiaSlVkImage
{
    uint64_t image;
    uint64_t view;
    uint32_t state;
    uint32_t width;
    uint32_t height;
    uint32_t format;
    uint32_t mipLevels;
    uint32_t arrayLayers;
    uint32_t flags;
    uint32_t usage;
};

enum class NvidiaSlReflectionGuideKind : uint32_t
{
    eSpecularMotionVectors = 0,
    eSpecularHitDistance = 1,
};

struct NvidiaSlReflectionGuide
{
    NvidiaSlVkImage image;
    NvidiaSlReflectionGuideKind kind;
    uint32_t reserved;
};

struct NvidiaSlDlssRrResources
{
    NvidiaSlVkImage inputColor;
    NvidiaSlVkImage outputColor;
    NvidiaSlVkImage depth;
    NvidiaSlVkImage motion;
    NvidiaSlVkImage diffuseAlbedo;
    NvidiaSlVkImage specularAlbedo;
    NvidiaSlVkImage normalRoughness;
    NvidiaSlReflectionGuide reflectionGuide;
};

struct NvidiaSlFrameConstants
{
    float cameraViewToClip[16];
    float clipToCameraView[16];
    float clipToPrevClip[16];
    float prevClipToClip[16];
    float worldToCameraView[16];
    float cameraViewToWorld[16];
    float jitterOffset[2];
    float mvecScale[2];
    float cameraPos[3];
    float cameraNear;
    float cameraUp[3];
    float cameraFar;
    float cameraRight[3];
    float cameraFov;
    float cameraForward[3];
    float cameraAspectRatio;
    uint32_t frameIndex;
    uint32_t reset;
};

static_assert(sizeof(NvidiaSlVkImage) == 48);
static_assert(offsetof(NvidiaSlVkImage, state) == 16);
static_assert(offsetof(NvidiaSlVkImage, usage) == 44);
static_assert(sizeof(NvidiaSlReflectionGuideKind) == 4);
static_assert(sizeof(NvidiaSlReflectionGuide) == 56);
static_assert(offsetof(NvidiaSlReflectionGuide, kind) == 48);
static_assert(offsetof(NvidiaSlReflectionGuide, reserved) == 52);
static_assert(sizeof(NvidiaSlDlssRrResources) == 392);
static_assert(offsetof(NvidiaSlDlssRrResources, reflectionGuide) == 336);
static_assert(sizeof(NvidiaSlFrameConstants) == 472);
static_assert(offsetof(NvidiaSlFrameConstants, jitterOffset) == 384);
static_assert(offsetof(NvidiaSlFrameConstants, frameIndex) == 464);

struct NvidiaSlContext
{
    HMODULE interposer = nullptr;
    PFun_slInit* init = nullptr;
    PFun_slShutdown* shutdown = nullptr;
    PFun_slIsFeatureSupported* isFeatureSupported = nullptr;
    PFun_slEvaluateFeature* evaluateFeature = nullptr;
    PFun_slFreeResources* freeResources = nullptr;
    PFun_slGetFeatureFunction* getFeatureFunction = nullptr;
    PFun_slGetNewFrameToken* getNewFrameToken = nullptr;
    PFun_slSetConstants* setConstants = nullptr;
    PFun_slSetTagForFrame* setTagForFrame = nullptr;
    PFun_slDLSSDGetOptimalSettings* dlssdGetOptimalSettings = nullptr;
    PFun_slDLSSDSetOptions* dlssdSetOptions = nullptr;
};

namespace
{
template <typename T>
sl::Result loadFeatureFunction(NvidiaSlContext* context, const char* name, T*& function)
{
    void* address = nullptr;
    const sl::Result result = context->getFeatureFunction(sl::kFeatureDLSS_RR, name, address);
    if (result == sl::Result::eOk)
        function = reinterpret_cast<T*>(address);
    return result;
}

sl::Result ensureDlssRrFunctions(NvidiaSlContext* context)
{
    if (context->dlssdGetOptimalSettings != nullptr && context->dlssdSetOptions != nullptr)
        return sl::Result::eOk;

    sl::Result result = loadFeatureFunction(
        context,
        "slDLSSDGetOptimalSettings",
        context->dlssdGetOptimalSettings);
    if (result != sl::Result::eOk)
        return result;
    return loadFeatureFunction(context, "slDLSSDSetOptions", context->dlssdSetOptions);
}

void copyMatrix(sl::float4x4& output, const float (&input)[16]) noexcept
{
    std::memcpy(&output, input, sizeof(input));
}

void setIdentity(sl::float4x4& output) noexcept
{
    static constexpr float kIdentity[16] = {
        1.0f, 0.0f, 0.0f, 0.0f,
        0.0f, 1.0f, 0.0f, 0.0f,
        0.0f, 0.0f, 1.0f, 0.0f,
        0.0f, 0.0f, 0.0f, 1.0f,
    };
    copyMatrix(output, kIdentity);
}

bool validDlssRrQuality(uint32_t quality) noexcept
{
    return quality <= kDlssRrQualityUltraPerformance;
}

sl::DLSSMode dlssRrMode(uint32_t quality) noexcept
{
    switch (quality)
    {
    case kDlssRrQualityQuality:
        return sl::DLSSMode::eMaxQuality;
    case kDlssRrQualityDlaa:
        return sl::DLSSMode::eDLAA;
    case kDlssRrQualityPerformance:
        return sl::DLSSMode::eMaxPerformance;
    case kDlssRrQualityUltraPerformance:
        return sl::DLSSMode::eUltraPerformance;
    default:
        return sl::DLSSMode::eBalanced;
    }
}

sl::DLSSDOptions dlssRrOptions(uint32_t width, uint32_t height, uint32_t quality)
{
    sl::DLSSDOptions options{};
    options.mode = dlssRrMode(quality);
    options.outputWidth = width;
    options.outputHeight = height;
    options.colorBuffersHDR = sl::Boolean::eTrue;
    options.normalRoughnessMode = sl::DLSSDNormalRoughnessMode::ePacked;
    options.alphaUpscalingEnabled = sl::Boolean::eFalse;
    options.dlaaPreset = sl::DLSSDPreset::ePresetE;
    options.qualityPreset = sl::DLSSDPreset::ePresetE;
    options.balancedPreset = sl::DLSSDPreset::ePresetE;
    options.performancePreset = sl::DLSSDPreset::ePresetE;
    options.ultraPerformancePreset = sl::DLSSDPreset::ePresetE;
    options.ultraQualityPreset = sl::DLSSDPreset::ePresetE;
    return options;
}

bool validImage(const NvidiaSlVkImage& image) noexcept
{
    return image.image != 0 && image.view != 0 && image.width != 0 && image.height != 0
        && image.format != 0 && image.mipLevels != 0 && image.arrayLayers != 0;
}

bool validReflectionGuide(const NvidiaSlReflectionGuide& guide) noexcept
{
    if (!validImage(guide.image) || guide.reserved != 0)
        return false;
    switch (guide.kind)
    {
    case NvidiaSlReflectionGuideKind::eSpecularMotionVectors:
        return true;
    case NvidiaSlReflectionGuideKind::eSpecularHitDistance:
        return guide.image.format == kVkFormatR32Sfloat;
    default:
        return false;
    }
}

sl::BufferType reflectionGuideBufferType(NvidiaSlReflectionGuideKind kind) noexcept
{
    return kind == NvidiaSlReflectionGuideKind::eSpecularHitDistance
        ? sl::kBufferTypeSpecularHitDistance
        : sl::kBufferTypeSpecularMotionVectors;
}

sl::Resource streamlineResource(const NvidiaSlVkImage& image)
{
    auto* native = reinterpret_cast<void*>(static_cast<uintptr_t>(image.image));
    auto* view = reinterpret_cast<void*>(static_cast<uintptr_t>(image.view));
    sl::Resource resource(sl::ResourceType::eTex2d, native, nullptr, view, image.state);
    resource.width = image.width;
    resource.height = image.height;
    resource.nativeFormat = image.format;
    resource.mipLevels = image.mipLevels;
    resource.arrayLayers = image.arrayLayers;
    resource.flags = image.flags;
    resource.usage = image.usage;
    return resource;
}
}

extern "C" int32_t nvidia_sl_init(
    const wchar_t* interposerPath,
    const wchar_t* pluginPath,
    const char* projectId,
    const char* engineVersion,
    NvidiaSlLogCallback logCallback,
    NvidiaSlContext** output) noexcept
{
    return ffiBoundary([&]() -> int32_t {
        if (interposerPath == nullptr || pluginPath == nullptr || projectId == nullptr
            || projectId[0] == '\0' || engineVersion == nullptr || engineVersion[0] == '\0'
            || output == nullptr)
        {
            return kInvalidArgument;
        }
        *output = nullptr;

        if (!sl::security::verifyEmbeddedSignature(interposerPath))
            return kInvalidSignature;

        Module interposer(LoadLibraryExW(
            interposerPath,
            nullptr,
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS));
        if (interposer.get() == nullptr)
            return kLoadFailed;

        auto context = std::unique_ptr<NvidiaSlContext>(new (std::nothrow) NvidiaSlContext{});
        if (context == nullptr)
            return kOutOfMemory;
        if (!loadFunction(interposer.get(), "slInit", context->init)
            || !loadFunction(interposer.get(), "slShutdown", context->shutdown)
            || !loadFunction(interposer.get(), "slIsFeatureSupported", context->isFeatureSupported)
            || !loadFunction(interposer.get(), "slEvaluateFeature", context->evaluateFeature)
            || !loadFunction(interposer.get(), "slFreeResources", context->freeResources)
            || !loadFunction(interposer.get(), "slGetFeatureFunction", context->getFeatureFunction)
            || !loadFunction(interposer.get(), "slGetNewFrameToken", context->getNewFrameToken)
            || !loadFunction(interposer.get(), "slSetConstants", context->setConstants)
            || !loadFunction(interposer.get(), "slSetTagForFrame", context->setTagForFrame))
        {
            return kMissingFunction;
        }

        g_logCallback = logCallback;
        LogCallbackGuard callbackGuard;
        const sl::Feature features[] = { sl::kFeatureDLSS_RR };
        const wchar_t* pluginPaths[] = { pluginPath };
        sl::Preferences preferences{};
        preferences.engine = sl::EngineType::eCustom;
        preferences.engineVersion = engineVersion;
        preferences.featuresToLoad = features;
        preferences.numFeaturesToLoad = static_cast<uint32_t>(std::size(features));
        preferences.flags = sl::PreferenceFlags::eDisableCLStateTracking
            | sl::PreferenceFlags::eUseManualHooking
            | sl::PreferenceFlags::eUseFrameBasedResourceTagging;
        preferences.logLevel = sl::LogLevel::eDefault;
        preferences.logMessageCallback = streamlineLog;
        preferences.pathsToPlugins = pluginPaths;
        preferences.projectId = projectId;
        preferences.numPathsToPlugins = static_cast<uint32_t>(std::size(pluginPaths));
        preferences.renderAPI = sl::RenderAPI::eVulkan;
        preferences.showConsole = false;

        const sl::Result result = context->init(preferences, sl::kSDKVersion);
        if (result != sl::Result::eOk)
            return static_cast<int32_t>(result);

        context->interposer = interposer.release();
        *output = context.release();
        callbackGuard.keep();
        return static_cast<int32_t>(sl::Result::eOk);
    });
}

extern "C" int32_t nvidia_sl_is_dlss_rr_supported(
    NvidiaSlContext* context,
    void* physicalDevice) noexcept
{
    return ffiBoundary([&]() -> int32_t {
        if (context == nullptr || physicalDevice == nullptr)
            return kInvalidArgument;

        // Streamline 2.9 bypasses its Vulkan adapter callback when deviceLUID is null.
        uint8_t forceAdapterCheck[8] = {};
        sl::AdapterInfo adapter{};
        adapter.deviceLUID = forceAdapterCheck;
        adapter.deviceLUIDSizeInBytes = sizeof(forceAdapterCheck);
        adapter.vkPhysicalDevice = physicalDevice;
        return static_cast<int32_t>(
            context->isFeatureSupported(sl::kFeatureDLSS_RR, adapter));
    });
}

extern "C" int32_t nvidia_sl_dlss_rr_optimal_settings(
    NvidiaSlContext* context,
    uint32_t outputWidth,
    uint32_t outputHeight,
    uint32_t quality,
    NvidiaSlExtent* output) noexcept
{
    return ffiBoundary([&]() -> int32_t {
        if (context == nullptr || output == nullptr || outputWidth == 0 || outputHeight == 0
            || !validDlssRrQuality(quality))
            return kInvalidArgument;

        const sl::Result loadResult = ensureDlssRrFunctions(context);
        if (loadResult != sl::Result::eOk)
            return static_cast<int32_t>(loadResult);

        const sl::DLSSDOptions options = dlssRrOptions(outputWidth, outputHeight, quality);
        sl::DLSSDOptimalSettings settings{};
        const sl::Result result = context->dlssdGetOptimalSettings(options, settings);
        if (result != sl::Result::eOk)
            return static_cast<int32_t>(result);
        if (settings.optimalRenderWidth == 0 || settings.optimalRenderHeight == 0)
            return kInvalidArgument;

        output->width = settings.optimalRenderWidth;
        output->height = settings.optimalRenderHeight;
        return static_cast<int32_t>(sl::Result::eOk);
    });
}

extern "C" int32_t nvidia_sl_evaluate_dlss_rr(
    NvidiaSlContext* context,
    void* commandBuffer,
    uint32_t quality,
    const NvidiaSlFrameConstants* frame,
    const NvidiaSlDlssRrResources* resources) noexcept
{
    return ffiBoundary([&]() -> int32_t {
        if (context == nullptr || commandBuffer == nullptr || frame == nullptr || resources == nullptr)
            return kInvalidArgument;
        if (!validDlssRrQuality(quality))
            return kInvalidArgument;
        if (!validImage(resources->inputColor)
            || !validImage(resources->outputColor)
            || !validImage(resources->depth)
            || !validImage(resources->motion)
            || !validImage(resources->diffuseAlbedo)
            || !validImage(resources->specularAlbedo)
            || !validImage(resources->normalRoughness)
            || !validReflectionGuide(resources->reflectionGuide))
        {
            return kInvalidArgument;
        }

        const sl::Result loadResult = ensureDlssRrFunctions(context);
        if (loadResult != sl::Result::eOk)
            return static_cast<int32_t>(loadResult);

        sl::FrameToken* token = nullptr;
        sl::Result result = context->getNewFrameToken(token, &frame->frameIndex);
        if (result != sl::Result::eOk || token == nullptr)
            return result == sl::Result::eOk ? kInvalidArgument : static_cast<int32_t>(result);

        const sl::ViewportHandle viewport(0u);
        sl::Constants constants{};
        copyMatrix(constants.cameraViewToClip, frame->cameraViewToClip);
        copyMatrix(constants.clipToCameraView, frame->clipToCameraView);
        setIdentity(constants.clipToLensClip);
        copyMatrix(constants.clipToPrevClip, frame->clipToPrevClip);
        copyMatrix(constants.prevClipToClip, frame->prevClipToClip);
        constants.jitterOffset = { frame->jitterOffset[0], frame->jitterOffset[1] };
        constants.mvecScale = { frame->mvecScale[0], frame->mvecScale[1] };
        constants.cameraPinholeOffset = { 0.0f, 0.0f };
        constants.cameraPos = { frame->cameraPos[0], frame->cameraPos[1], frame->cameraPos[2] };
        constants.cameraUp = { frame->cameraUp[0], frame->cameraUp[1], frame->cameraUp[2] };
        constants.cameraRight = {
            frame->cameraRight[0],
            frame->cameraRight[1],
            frame->cameraRight[2],
        };
        constants.cameraFwd = {
            frame->cameraForward[0],
            frame->cameraForward[1],
            frame->cameraForward[2],
        };
        constants.cameraNear = frame->cameraNear;
        constants.cameraFar = frame->cameraFar;
        constants.cameraFOV = frame->cameraFov;
        constants.cameraAspectRatio = frame->cameraAspectRatio;
        constants.motionVectorsInvalidValue = FLT_MIN;
        constants.depthInverted = sl::Boolean::eTrue;
        constants.cameraMotionIncluded = sl::Boolean::eTrue;
        constants.motionVectors3D = sl::Boolean::eFalse;
        constants.reset = frame->reset != 0 ? sl::Boolean::eTrue : sl::Boolean::eFalse;
        constants.orthographicProjection = sl::Boolean::eFalse;
        constants.motionVectorsDilated = sl::Boolean::eFalse;
        constants.motionVectorsJittered = sl::Boolean::eFalse;
        result = context->setConstants(constants, *token, viewport);
        if (result != sl::Result::eOk)
            return static_cast<int32_t>(result);

        sl::DLSSDOptions options = dlssRrOptions(
            resources->outputColor.width,
            resources->outputColor.height,
            quality);
        copyMatrix(options.worldToCameraView, frame->worldToCameraView);
        copyMatrix(options.cameraViewToWorld, frame->cameraViewToWorld);
        result = context->dlssdSetOptions(viewport, options);
        if (result != sl::Result::eOk)
            return static_cast<int32_t>(result);

        sl::Resource nativeResources[] = {
            streamlineResource(resources->inputColor),
            streamlineResource(resources->outputColor),
            streamlineResource(resources->depth),
            streamlineResource(resources->motion),
            streamlineResource(resources->diffuseAlbedo),
            streamlineResource(resources->specularAlbedo),
            streamlineResource(resources->normalRoughness),
            streamlineResource(resources->reflectionGuide.image),
        };
        const sl::Extent renderExtent = {
            0,
            0,
            resources->inputColor.width,
            resources->inputColor.height,
        };
        const sl::Extent outputExtent = {
            0,
            0,
            resources->outputColor.width,
            resources->outputColor.height,
        };
        const sl::BufferType reflectionGuideType =
            reflectionGuideBufferType(resources->reflectionGuide.kind);
        sl::ResourceTag tags[] = {
            { &nativeResources[0], sl::kBufferTypeScalingInputColor, sl::eValidUntilEvaluate, &renderExtent },
            { &nativeResources[1], sl::kBufferTypeScalingOutputColor, sl::eValidUntilEvaluate, &outputExtent },
            { &nativeResources[2], sl::kBufferTypeDepth, sl::eValidUntilEvaluate, &renderExtent },
            { &nativeResources[3], sl::kBufferTypeMotionVectors, sl::eValidUntilEvaluate, &renderExtent },
            { &nativeResources[4], sl::kBufferTypeAlbedo, sl::eValidUntilEvaluate, &renderExtent },
            { &nativeResources[5], sl::kBufferTypeSpecularAlbedo, sl::eValidUntilEvaluate, &renderExtent },
            { &nativeResources[6], sl::kBufferTypeNormalRoughness, sl::eValidUntilEvaluate, &renderExtent },
            { &nativeResources[7], reflectionGuideType, sl::eValidUntilEvaluate, &renderExtent },
        };
        result = context->setTagForFrame(
            *token,
            viewport,
            tags,
            static_cast<uint32_t>(std::size(tags)),
            commandBuffer);
        if (result != sl::Result::eOk)
            return static_cast<int32_t>(result);

        const sl::BaseStructure* inputs[] = { &viewport };
        return static_cast<int32_t>(context->evaluateFeature(
            sl::kFeatureDLSS_RR,
            *token,
            inputs,
            static_cast<uint32_t>(std::size(inputs)),
            commandBuffer));
    });
}

extern "C" int32_t nvidia_sl_shutdown(NvidiaSlContext* context) noexcept
{
    if (context == nullptr)
        return kInvalidArgument;

    int32_t result = static_cast<int32_t>(sl::Result::eOk);
    try
    {
        if (context->dlssdGetOptimalSettings != nullptr)
        {
            const sl::ViewportHandle viewport(0u);
            context->freeResources(sl::kFeatureDLSS_RR, viewport);
        }
    }
    catch (const std::bad_alloc&)
    {
        result = kOutOfMemory;
    }
    catch (...)
    {
        result = kUnexpectedException;
    }

    try
    {
        const sl::Result shutdownResult = context->shutdown();
        if (result == static_cast<int32_t>(sl::Result::eOk))
            result = static_cast<int32_t>(shutdownResult);
    }
    catch (const std::bad_alloc&)
    {
        if (result == static_cast<int32_t>(sl::Result::eOk))
            result = kOutOfMemory;
    }
    catch (...)
    {
        if (result == static_cast<int32_t>(sl::Result::eOk))
            result = kUnexpectedException;
    }

    FreeLibrary(context->interposer);
    delete context;
    g_logCallback = nullptr;
    return result;
}

extern "C" const char* nvidia_sl_result_name(int32_t result) noexcept
{
    switch (result)
    {
    case kInvalidArgument: return "invalid argument";
    case kInvalidSignature: return "invalid Streamline signature";
    case kLoadFailed: return "unable to load Streamline interposer";
    case kMissingFunction: return "Streamline API function missing";
    case kOutOfMemory: return "out of host memory";
    case kUnexpectedException: return "unexpected native exception";
    default: break;
    }

    switch (static_cast<sl::Result>(result))
    {
    case sl::Result::eOk: return "ok";
    case sl::Result::eErrorIO: return "I/O error";
    case sl::Result::eErrorDriverOutOfDate: return "driver out of date";
    case sl::Result::eErrorOSOutOfDate: return "OS out of date";
    case sl::Result::eErrorOSDisabledHWS: return "hardware scheduling disabled";
    case sl::Result::eErrorDeviceNotCreated: return "device not created";
    case sl::Result::eErrorNoSupportedAdapterFound: return "no supported adapter";
    case sl::Result::eErrorAdapterNotSupported: return "adapter not supported";
    case sl::Result::eErrorNoPlugins: return "plugins missing";
    case sl::Result::eErrorVulkanAPI: return "Vulkan API error";
    case sl::Result::eErrorNGXFailed: return "NGX failure";
    case sl::Result::eErrorMissingProxy: return "proxy missing";
    case sl::Result::eErrorInvalidIntegration: return "invalid integration";
    case sl::Result::eErrorMissingInputParameter: return "input parameter missing";
    case sl::Result::eErrorNotInitialized: return "not initialized";
    case sl::Result::eErrorInvalidParameter: return "invalid parameter";
    case sl::Result::eErrorFeatureMissing: return "feature missing";
    case sl::Result::eErrorFeatureNotSupported: return "feature unsupported";
    case sl::Result::eErrorFeatureFailedToLoad: return "feature failed to load";
    case sl::Result::eErrorInvalidState: return "invalid state";
    case sl::Result::eWarnOutOfVRAM: return "out of VRAM";
    default: return "unknown Streamline result";
    }
}
