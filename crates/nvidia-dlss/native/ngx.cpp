#include <vulkan/vulkan.h>

#include <cstddef>
#include <cstdint>
#include <cstring>
#include <filesystem>
#include <memory>
#include <new>
#include <string>
#include <utility>

#include <nvsdk_ngx_helpers_vk.h>
#include <nvsdk_ngx_helpers_dlssd.h>
#include <nvsdk_ngx_helpers_dlssd_vk.h>

namespace
{
constexpr int32_t kInvalidArgument = -1000;
constexpr int32_t kOutOfMemory = -1001;
constexpr int32_t kTooManyExtensions = -1002;
constexpr int32_t kFeatureUnsupported = -1003;
constexpr int32_t kUnexpectedException = -1004;
constexpr uint32_t kMaxExtensions = 32;
constexpr uint32_t kSpecularMotion = 0;
constexpr uint32_t kSpecularHitDistance = 1;
constexpr uint32_t kDlssRrQualityBalanced = 0;
constexpr uint32_t kDlssRrQualityQuality = 1;
constexpr uint32_t kDlssRrQualityDlaa = 2;

struct NgxIdentity
{
    uint64_t applicationId;
    const char* projectId;
    const char* engineVersion;
};

struct NgxExtensions
{
    uint32_t count;
    char names[kMaxExtensions][VK_MAX_EXTENSION_NAME_SIZE];
};

struct NgxExtent
{
    uint32_t width;
    uint32_t height;
};

struct NgxDlssRrConfig
{
    uint32_t color; // 0 HDR, 1 LDR
    uint32_t motionResolution; // 0 render, 1 output
    uint32_t depth; // 0 reversed HW, 1 forward HW, 2 linear view-space
};
static_assert(sizeof(NgxDlssRrConfig) == 12);
static_assert(offsetof(NgxDlssRrConfig, motionResolution) == 4);
static_assert(offsetof(NgxDlssRrConfig, depth) == 8);

bool validConfig(const NgxDlssRrConfig& config)
{
    return config.color <= 1 && config.motionResolution <= 1 && config.depth <= 2;
}

struct NgxVkImage
{
    uint64_t image;
    uint64_t view;
    uint32_t state;
    uint32_t width;
    uint32_t height;
    uint32_t format;
    VkImageSubresourceRange subresourceRange;
    uint32_t flags;
    uint32_t usage;
};

struct NgxDlssRrResources
{
    NgxVkImage inputColor;
    NgxVkImage outputColor;
    NgxVkImage depth;
    NgxVkImage motion;
    NgxVkImage diffuseAlbedo;
    NgxVkImage specularAlbedo;
    NgxVkImage normalRoughness;
    NgxVkImage reflectionGuide;
    uint32_t reflectionGuideKind;
    uint32_t reserved;
};

struct NgxFrameConstants
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

static_assert(sizeof(NgxIdentity) == 24);
static_assert(sizeof(NgxExtensions) == 4 + 32 * VK_MAX_EXTENSION_NAME_SIZE);
static_assert(sizeof(NgxVkImage) == 64);
static_assert(alignof(NgxVkImage) == 8);
static_assert(offsetof(NgxVkImage, subresourceRange) == 32);
static_assert(sizeof(VkImageSubresourceRange) == 20);
static_assert(offsetof(VkImageSubresourceRange, aspectMask) == 0);
static_assert(offsetof(VkImageSubresourceRange, baseMipLevel) == 4);
static_assert(offsetof(VkImageSubresourceRange, levelCount) == 8);
static_assert(offsetof(VkImageSubresourceRange, baseArrayLayer) == 12);
static_assert(offsetof(VkImageSubresourceRange, layerCount) == 16);
static_assert(offsetof(NgxVkImage, usage) == 56);
static_assert(sizeof(NgxDlssRrResources) == 520);
static_assert(offsetof(NgxDlssRrResources, reflectionGuideKind) == 512);
static_assert(sizeof(NgxFrameConstants) == 472);
static_assert(offsetof(NgxFrameConstants, mvecScale) == 392);

bool validString(const char* value)
{
    return value != nullptr && value[0] != '\0';
}

bool validIdentity(const NgxIdentity* identity)
{
    return identity != nullptr
        && (identity->applicationId != 0
            || (validString(identity->projectId) && validString(identity->engineVersion)));
}

void setIdentifier(
    NVSDK_NGX_Application_Identifier& output,
    const NgxIdentity& identity)
{
    if (identity.applicationId != 0)
    {
        output.IdentifierType = NVSDK_NGX_Application_Identifier_Type_Application_Id;
        output.v.ApplicationId = identity.applicationId;
    }
    else
    {
        output.IdentifierType = NVSDK_NGX_Application_Identifier_Type_Project_Id;
        output.v.ProjectDesc.ProjectId = identity.projectId;
        output.v.ProjectDesc.EngineType = NVSDK_NGX_ENGINE_TYPE_CUSTOM;
        output.v.ProjectDesc.EngineVersion = identity.engineVersion;
    }
}

struct DiscoveryInfo
{
    std::wstring applicationDataPath;
    std::wstring runtimePath;
    const wchar_t* paths[1]{};
    NVSDK_NGX_FeatureCommonInfo common{};
    NVSDK_NGX_FeatureDiscoveryInfo discovery{};

    DiscoveryInfo(
        const NgxIdentity& identity,
        const char* applicationDataPathUtf8,
        const char* runtimePathUtf8)
        : applicationDataPath(std::filesystem::u8path(applicationDataPathUtf8).wstring())
        , runtimePath(std::filesystem::u8path(runtimePathUtf8).wstring())
    {
        paths[0] = runtimePath.c_str();
        common.PathListInfo.Path = paths;
        common.PathListInfo.Length = 1;
        discovery.SDKVersion = NVSDK_NGX_Version_API;
        discovery.FeatureID = NVSDK_NGX_Feature_RayReconstruction;
        setIdentifier(discovery.Identifier, identity);
        discovery.ApplicationDataPath = applicationDataPath.c_str();
        discovery.FeatureInfo = &common;
    }
};

struct NgxContext
{
    NgxDlssRrConfig config{};
    std::wstring applicationDataPath;
    std::wstring runtimePath;
    const wchar_t* paths[1]{};
    NVSDK_NGX_FeatureCommonInfo common{};
    VkDevice device{};
    NVSDK_NGX_Parameter* parameters{};
    NVSDK_NGX_Handle* feature{};
    uint32_t renderWidth{};
    uint32_t renderHeight{};
    uint32_t outputWidth{};
    uint32_t outputHeight{};
    uint32_t quality{};

    NgxContext(std::wstring dataPath, std::wstring featurePath, VkDevice deviceHandle)
        : applicationDataPath(std::move(dataPath))
        , runtimePath(std::move(featurePath))
        , device(deviceHandle)
    {
        paths[0] = runtimePath.c_str();
        common.PathListInfo.Path = paths;
        common.PathListInfo.Length = 1;
    }
};

template<typename Function>
int32_t guarded(Function&& function) noexcept
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

int32_t resultCode(NVSDK_NGX_Result result)
{
    return result == NVSDK_NGX_Result_Success ? 0 : static_cast<int32_t>(result);
}

int32_t copyExtensions(
    uint32_t count,
    const VkExtensionProperties* properties,
    NgxExtensions* output)
{
    if (output == nullptr || (count != 0 && properties == nullptr))
        return kInvalidArgument;
    if (count > kMaxExtensions)
        return kTooManyExtensions;

    std::memset(output, 0, sizeof(*output));
    output->count = count;
    for (uint32_t i = 0; i < count; ++i)
    {
        const size_t length = std::strlen(properties[i].extensionName);
        if (length >= VK_MAX_EXTENSION_NAME_SIZE)
            return kInvalidArgument;
        std::memcpy(output->names[i], properties[i].extensionName, length + 1);
    }
    return 0;
}

bool validImage(const NgxVkImage& image)
{
    return image.image != 0 && image.view != 0 && image.width != 0 && image.height != 0
        && image.format != 0 && image.subresourceRange.aspectMask != 0
        && image.subresourceRange.levelCount != 0 && image.subresourceRange.layerCount != 0;
}

bool validReflectionGuide(const NgxDlssRrResources& resources)
{
    if (!validImage(resources.reflectionGuide))
        return false;
    if (resources.reflectionGuideKind == kSpecularMotion)
        return true;
    return resources.reflectionGuideKind == kSpecularHitDistance
        && resources.reflectionGuide.format == static_cast<uint32_t>(VK_FORMAT_R32_SFLOAT);
}

bool validDlssRrQuality(uint32_t quality)
{
    return quality <= kDlssRrQualityDlaa;
}

NVSDK_NGX_PerfQuality_Value dlssRrQuality(uint32_t quality)
{
    switch (quality)
    {
    case kDlssRrQualityQuality:
        return NVSDK_NGX_PerfQuality_Value_MaxQuality;
    case kDlssRrQualityDlaa:
        return NVSDK_NGX_PerfQuality_Value_DLAA;
    default:
        return NVSDK_NGX_PerfQuality_Value_Balanced;
    }
}

NVSDK_NGX_Resource_VK ngxResource(const NgxVkImage& image, bool readWrite)
{
    return NVSDK_NGX_Create_ImageView_Resource_VK(
        reinterpret_cast<VkImageView>(static_cast<uintptr_t>(image.view)),
        reinterpret_cast<VkImage>(static_cast<uintptr_t>(image.image)),
        image.subresourceRange,
        static_cast<VkFormat>(image.format),
        image.width,
        image.height,
        readWrite);
}

NVSDK_NGX_Result releaseFeature(NgxContext* context)
{
    if (context->feature == nullptr)
        return NVSDK_NGX_Result_Success;
    const NVSDK_NGX_Result result = NVSDK_NGX_VULKAN_ReleaseFeature(context->feature);
    if (result == NVSDK_NGX_Result_Success)
    {
        context->feature = nullptr;
        context->renderWidth = 0;
        context->renderHeight = 0;
        context->outputWidth = 0;
        context->outputHeight = 0;
        context->quality = kDlssRrQualityBalanced;
    }
    return result;
}

NVSDK_NGX_Result ensureFeature(
    NgxContext* context,
    VkCommandBuffer commandBuffer,
    const NgxDlssRrResources& resources,
    uint32_t quality)
{
    const bool dimensionsMatch = context->feature != nullptr
        && context->renderWidth == resources.inputColor.width
        && context->renderHeight == resources.inputColor.height
        && context->outputWidth == resources.outputColor.width
        && context->outputHeight == resources.outputColor.height
        && context->quality == quality;
    if (dimensionsMatch)
        return NVSDK_NGX_Result_Success;

    NVSDK_NGX_Result result = releaseFeature(context);
    if (result != NVSDK_NGX_Result_Success)
        return result;

    NVSDK_NGX_Parameter_SetUI(
        context->parameters,
        NVSDK_NGX_Parameter_RayReconstruction_Hint_Render_Preset_Balanced,
        NVSDK_NGX_RayReconstruction_Hint_Render_Preset_E);
    NVSDK_NGX_Parameter_SetUI(
        context->parameters,
        NVSDK_NGX_Parameter_RayReconstruction_Hint_Render_Preset_Quality,
        NVSDK_NGX_RayReconstruction_Hint_Render_Preset_E);
    NVSDK_NGX_Parameter_SetUI(
        context->parameters,
        NVSDK_NGX_Parameter_RayReconstruction_Hint_Render_Preset_DLAA,
        NVSDK_NGX_RayReconstruction_Hint_Render_Preset_E);

    NVSDK_NGX_DLSSD_Create_Params create{};
    create.InDenoiseMode = NVSDK_NGX_DLSS_Denoise_Mode_DLUnified;
    create.InRoughnessMode = NVSDK_NGX_DLSS_Roughness_Mode_Packed;
    create.InUseHWDepth = context->config.depth == 2
        ? NVSDK_NGX_DLSS_Depth_Type_Linear : NVSDK_NGX_DLSS_Depth_Type_HW;
    create.InWidth = resources.inputColor.width;
    create.InHeight = resources.inputColor.height;
    create.InTargetWidth = resources.outputColor.width;
    create.InTargetHeight = resources.outputColor.height;
    create.InPerfQualityValue = dlssRrQuality(quality);
    create.InFeatureCreateFlags =
        (context->config.color == 0 ? NVSDK_NGX_DLSS_Feature_Flags_IsHDR : 0)
        | (context->config.motionResolution == 0 ? NVSDK_NGX_DLSS_Feature_Flags_MVLowRes : 0)
        | (context->config.depth == 0 ? NVSDK_NGX_DLSS_Feature_Flags_DepthInverted : 0);
    create.InEnableOutputSubrects = false;
    result = NGX_VULKAN_CREATE_DLSSD_EXT1(
        context->device,
        commandBuffer,
        1,
        1,
        &context->feature,
        context->parameters,
        &create);
    if (result == NVSDK_NGX_Result_Success)
    {
        context->renderWidth = resources.inputColor.width;
        context->renderHeight = resources.inputColor.height;
        context->outputWidth = resources.outputColor.width;
        context->outputHeight = resources.outputColor.height;
        context->quality = quality;
    }
    return result;
}
}

extern "C" int32_t nvidia_dlss_ngx_instance_extensions(
    const NgxIdentity* identity,
    const char* applicationDataPath,
    const char* runtimePath,
    NgxExtensions* output) noexcept
{
    return guarded([&]() {
        if (!validIdentity(identity) || !validString(applicationDataPath)
            || !validString(runtimePath) || output == nullptr)
        {
            return kInvalidArgument;
        }

        DiscoveryInfo info(*identity, applicationDataPath, runtimePath);
        uint32_t count = 0;
        VkExtensionProperties* properties = nullptr;
        const NVSDK_NGX_Result result =
            NVSDK_NGX_VULKAN_GetFeatureInstanceExtensionRequirements(
                &info.discovery,
                &count,
                &properties);
        if (result != NVSDK_NGX_Result_Success)
            return resultCode(result);
        return copyExtensions(count, properties, output);
    });
}

extern "C" int32_t nvidia_dlss_ngx_device_extensions(
    const NgxIdentity* identity,
    const char* applicationDataPath,
    const char* runtimePath,
    VkInstance instance,
    VkPhysicalDevice physicalDevice,
    NgxExtensions* output) noexcept
{
    return guarded([&]() {
        if (!validIdentity(identity) || !validString(applicationDataPath)
            || !validString(runtimePath) || instance == nullptr || physicalDevice == nullptr
            || output == nullptr)
        {
            return kInvalidArgument;
        }

        DiscoveryInfo info(*identity, applicationDataPath, runtimePath);
        NVSDK_NGX_FeatureRequirement requirement{};
        NVSDK_NGX_Result result = NVSDK_NGX_VULKAN_GetFeatureRequirements(
            instance,
            physicalDevice,
            &info.discovery,
            &requirement);
        if (result != NVSDK_NGX_Result_Success)
            return resultCode(result);
        if (requirement.FeatureSupported != NVSDK_NGX_FeatureSupportResult_Supported)
            return kFeatureUnsupported;

        uint32_t count = 0;
        VkExtensionProperties* properties = nullptr;
        result = NVSDK_NGX_VULKAN_GetFeatureDeviceExtensionRequirements(
            instance,
            physicalDevice,
            &info.discovery,
            &count,
            &properties);
        if (result != NVSDK_NGX_Result_Success)
            return resultCode(result);
        return copyExtensions(count, properties, output);
    });
}

extern "C" int32_t nvidia_dlss_ngx_init(
    const NgxIdentity* identity,
    const char* applicationDataPath,
    const char* runtimePath,
    VkInstance instance,
    VkPhysicalDevice physicalDevice,
    VkDevice device,
    PFN_vkGetInstanceProcAddr getInstanceProcAddr,
    PFN_vkGetDeviceProcAddr getDeviceProcAddr,
    const NgxDlssRrConfig* config,
    NgxContext** output) noexcept
{
    return guarded([&]() {
        if (!validIdentity(identity) || !validString(applicationDataPath)
            || !validString(runtimePath) || instance == nullptr || physicalDevice == nullptr
            || device == nullptr || getInstanceProcAddr == nullptr || getDeviceProcAddr == nullptr
            || output == nullptr || config == nullptr || !validConfig(*config))
        {
            return kInvalidArgument;
        }
        *output = nullptr;

        auto context = std::make_unique<NgxContext>(
            std::filesystem::u8path(applicationDataPath).wstring(),
            std::filesystem::u8path(runtimePath).wstring(),
            device);
        context->config = *config;

        NVSDK_NGX_Result result;
        if (identity->applicationId != 0)
        {
            result = NVSDK_NGX_VULKAN_Init(
                identity->applicationId,
                context->applicationDataPath.c_str(),
                instance,
                physicalDevice,
                device,
                getInstanceProcAddr,
                getDeviceProcAddr,
                &context->common,
                NVSDK_NGX_Version_API);
        }
        else
        {
            result = NVSDK_NGX_VULKAN_Init_with_ProjectID(
                identity->projectId,
                NVSDK_NGX_ENGINE_TYPE_CUSTOM,
                identity->engineVersion,
                context->applicationDataPath.c_str(),
                instance,
                physicalDevice,
                device,
                getInstanceProcAddr,
                getDeviceProcAddr,
                &context->common,
                NVSDK_NGX_Version_API);
        }
        if (result != NVSDK_NGX_Result_Success)
            return resultCode(result);

        result = NVSDK_NGX_VULKAN_GetCapabilityParameters(&context->parameters);
        if (result == NVSDK_NGX_Result_Success)
        {
            int available = 0;
            result = NVSDK_NGX_Parameter_GetI(
                context->parameters,
                NVSDK_NGX_Parameter_SuperSamplingDenoising_Available,
                &available);
            if (result == NVSDK_NGX_Result_Success && available == 0)
                result = NVSDK_NGX_Result_FAIL_FeatureNotSupported;
        }
        if (result != NVSDK_NGX_Result_Success)
        {
            if (context->parameters != nullptr)
                NVSDK_NGX_VULKAN_DestroyParameters(context->parameters);
            NVSDK_NGX_VULKAN_Shutdown1(device);
            return resultCode(result);
        }

        *output = context.release();
        return 0;
    });
}

extern "C" int32_t nvidia_dlss_ngx_optimal_settings(
    NgxContext* context,
    uint32_t outputWidth,
    uint32_t outputHeight,
    uint32_t quality,
    NgxExtent* output) noexcept
{
    return guarded([&]() {
        if (context == nullptr || context->parameters == nullptr || output == nullptr
            || outputWidth == 0 || outputHeight == 0 || !validDlssRrQuality(quality))
        {
            return kInvalidArgument;
        }

        uint32_t maxWidth = 0;
        uint32_t maxHeight = 0;
        uint32_t minWidth = 0;
        uint32_t minHeight = 0;
        float sharpness = 0.0f;
        const NVSDK_NGX_Result result = NGX_DLSSD_GET_OPTIMAL_SETTINGS(
            context->parameters,
            outputWidth,
            outputHeight,
            dlssRrQuality(quality),
            &output->width,
            &output->height,
            &maxWidth,
            &maxHeight,
            &minWidth,
            &minHeight,
            &sharpness);
        if (result != NVSDK_NGX_Result_Success)
            return resultCode(result);
        if (output->width == 0 || output->height == 0)
            return kInvalidArgument;
        return 0;
    });
}

extern "C" int32_t nvidia_dlss_ngx_evaluate(
    NgxContext* context,
    VkCommandBuffer commandBuffer,
    uint32_t quality,
    const NgxFrameConstants* frame,
    const NgxDlssRrResources* resources) noexcept
{
    return guarded([&]() {
        if (context == nullptr || context->parameters == nullptr || commandBuffer == nullptr
            || frame == nullptr || resources == nullptr || !validDlssRrQuality(quality)
            || !validConfig(context->config))
        {
            return kInvalidArgument;
        }
        if (!validImage(resources->inputColor)
            || !validImage(resources->outputColor)
            || !validImage(resources->depth)
            || !validImage(resources->motion)
            || !validImage(resources->diffuseAlbedo)
            || !validImage(resources->specularAlbedo)
            || !validImage(resources->normalRoughness)
            || !validReflectionGuide(*resources))
        {
            return kInvalidArgument;
        }

        // RR guide buffers stay at render resolution, including specular motion.
        // Only the primary motion field follows the MVLowRes creation flag.
        for (const auto* input : {&resources->inputColor, &resources->depth,
                 &resources->motion, &resources->diffuseAlbedo, &resources->specularAlbedo,
                 &resources->normalRoughness, &resources->reflectionGuide})
        {
            const auto& extent = input == &resources->motion && context->config.motionResolution == 1
                ? resources->outputColor : resources->inputColor;
            if (!(input->usage & VK_IMAGE_USAGE_SAMPLED_BIT)
                || input->state != VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL
                || input->width != extent.width || input->height != extent.height)
                return kInvalidArgument;
        }
        if (!(resources->outputColor.usage & VK_IMAGE_USAGE_STORAGE_BIT)
            || resources->outputColor.state != VK_IMAGE_LAYOUT_GENERAL)
            return kInvalidArgument;
        if (quality == kDlssRrQualityDlaa
            && (resources->inputColor.width != resources->outputColor.width
                || resources->inputColor.height != resources->outputColor.height))
            return kInvalidArgument;

        NVSDK_NGX_Result result = ensureFeature(context, commandBuffer, *resources, quality);
        if (result != NVSDK_NGX_Result_Success)
            return resultCode(result);

        NVSDK_NGX_Resource_VK nativeResources[] = {
            ngxResource(resources->inputColor, false),
            ngxResource(resources->outputColor, true),
            ngxResource(resources->depth, false),
            ngxResource(resources->motion, false),
            ngxResource(resources->diffuseAlbedo, false),
            ngxResource(resources->specularAlbedo, false),
            ngxResource(resources->normalRoughness, false),
            ngxResource(resources->reflectionGuide, false),
        };
        NVSDK_NGX_VK_DLSSD_Eval_Params evaluate{};
        evaluate.pInColor = &nativeResources[0];
        evaluate.pInOutput = &nativeResources[1];
        evaluate.pInDepth = &nativeResources[2];
        evaluate.pInMotionVectors = &nativeResources[3];
        evaluate.pInDiffuseAlbedo = &nativeResources[4];
        evaluate.pInSpecularAlbedo = &nativeResources[5];
        evaluate.pInNormals = &nativeResources[6];
        if (resources->reflectionGuideKind == kSpecularMotion)
        {
            evaluate.pInMotionVectorsReflections = &nativeResources[7];
        }
        else
        {
            evaluate.pInSpecularHitDistance = &nativeResources[7];
            evaluate.pInWorldToViewMatrix = const_cast<float*>(frame->worldToCameraView);
            evaluate.pInViewToClipMatrix = const_cast<float*>(frame->cameraViewToClip);
        }
        evaluate.InJitterOffsetX = frame->jitterOffset[0];
        evaluate.InJitterOffsetY = frame->jitterOffset[1];
        evaluate.InRenderSubrectDimensions = {
            resources->inputColor.width,
            resources->inputColor.height,
        };
        evaluate.InReset = frame->reset != 0 ? 1 : 0;
        // The SDK helper converts each zero scale to unity before setting NGX parameters.
        evaluate.InMVScaleX = frame->mvecScale[0];
        evaluate.InMVScaleY = frame->mvecScale[1];
        result = NGX_VULKAN_EVALUATE_DLSSD_EXT(
            commandBuffer,
            context->feature,
            context->parameters,
            &evaluate);
        return resultCode(result);
    });
}

extern "C" int32_t nvidia_dlss_ngx_shutdown(NgxContext* context) noexcept
{
    return guarded([&]() {
        if (context == nullptr)
            return kInvalidArgument;

        std::unique_ptr<NgxContext> owned(context);
        NVSDK_NGX_Result result = releaseFeature(context);
        if (context->parameters != nullptr)
        {
            const NVSDK_NGX_Result destroyResult =
                NVSDK_NGX_VULKAN_DestroyParameters(context->parameters);
            context->parameters = nullptr;
            if (result == NVSDK_NGX_Result_Success)
                result = destroyResult;
        }
        const NVSDK_NGX_Result shutdownResult = NVSDK_NGX_VULKAN_Shutdown1(context->device);
        if (result == NVSDK_NGX_Result_Success)
            result = shutdownResult;
        return resultCode(result);
    });
}

extern "C" const char* nvidia_dlss_ngx_result_name(int32_t result) noexcept
{
    switch (result)
    {
    case 0: return "ok";
    case kInvalidArgument: return "invalid argument";
    case kOutOfMemory: return "out of host memory";
    case kTooManyExtensions: return "too many required Vulkan extensions";
    case kFeatureUnsupported: return "feature unsupported";
    case kUnexpectedException: return "unexpected native exception";
    default: break;
    }

    switch (static_cast<NVSDK_NGX_Result>(static_cast<uint32_t>(result)))
    {
    case NVSDK_NGX_Result_Success: return "success";
    case NVSDK_NGX_Result_Fail: return "failure";
    case NVSDK_NGX_Result_FAIL_FeatureNotSupported: return "feature not supported";
    case NVSDK_NGX_Result_FAIL_PlatformError: return "platform error";
    case NVSDK_NGX_Result_FAIL_FeatureAlreadyExists: return "feature already exists";
    case NVSDK_NGX_Result_FAIL_FeatureNotFound: return "feature not found";
    case NVSDK_NGX_Result_FAIL_InvalidParameter: return "invalid parameter";
    case NVSDK_NGX_Result_FAIL_ScratchBufferTooSmall: return "scratch buffer too small";
    case NVSDK_NGX_Result_FAIL_NotInitialized: return "not initialized";
    case NVSDK_NGX_Result_FAIL_UnsupportedInputFormat: return "unsupported input format";
    case NVSDK_NGX_Result_FAIL_RWFlagMissing: return "read-write flag missing";
    case NVSDK_NGX_Result_FAIL_MissingInput: return "missing input";
    case NVSDK_NGX_Result_FAIL_UnableToInitializeFeature: return "unable to initialize feature";
    case NVSDK_NGX_Result_FAIL_OutOfDate: return "driver or runtime out of date";
    case NVSDK_NGX_Result_FAIL_OutOfGPUMemory: return "out of GPU memory";
    case NVSDK_NGX_Result_FAIL_UnsupportedFormat: return "unsupported format";
    case NVSDK_NGX_Result_FAIL_UnableToWriteToAppDataPath: return "unable to write application data";
    case NVSDK_NGX_Result_FAIL_UnsupportedParameter: return "unsupported parameter";
    case NVSDK_NGX_Result_FAIL_Denied: return "denied";
    case NVSDK_NGX_Result_FAIL_NotImplemented: return "not implemented";
    default: return "unknown NGX result";
    }
}
