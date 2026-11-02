// CPU-only regression test: run the real SDK helpers, intercept only GPU entrypoints.
#include <vulkan/vulkan.h>
#include <nvsdk_ngx_vk.h>
#include <nvsdk_ngx_defs_dlssd.h>
#include <any>
#include <cassert>
#include <map>
#include <string>

struct Parameters : NVSDK_NGX_Parameter
{
    std::map<std::string, std::any> values;
#define PARAMETER_TYPE(T) \
    void Set(const char* name, T value) override { values[name] = value; } \
    NVSDK_NGX_Result Get(const char* name, T* value) const override { \
        const auto found = values.find(name); \
        if (found == values.end()) return NVSDK_NGX_Result_FAIL_InvalidParameter; \
        const auto* stored = std::any_cast<T>(&found->second); \
        if (!stored) return NVSDK_NGX_Result_FAIL_InvalidParameter; \
        *value = *stored; return NVSDK_NGX_Result_Success; \
    }
    PARAMETER_TYPE(unsigned long long)
    PARAMETER_TYPE(float)
    PARAMETER_TYPE(double)
    PARAMETER_TYPE(unsigned int)
    PARAMETER_TYPE(int)
    PARAMETER_TYPE(ID3D11Resource*)
    PARAMETER_TYPE(ID3D12Resource*)
    PARAMETER_TYPE(void*)
#undef PARAMETER_TYPE
    void Reset() override { values.clear(); }
};

static float expectedScale[2];
static int expectedFlags;
static unsigned expectedDepth;
static unsigned expectedGuide;
static int expectedQuality;
static unsigned evaluations;
static unsigned creations;

template<typename T>
static T parameter(const NVSDK_NGX_Parameter* params, const char* name)
{
    T value{};
    assert(params->Get(name, &value) == NVSDK_NGX_Result_Success);
    return value;
}

static NVSDK_NGX_Result captureCreation(
    VkCommandBuffer, NVSDK_NGX_Feature feature, const NVSDK_NGX_Parameter* params,
    NVSDK_NGX_Handle** handle)
{
    assert(feature == NVSDK_NGX_Feature_RayReconstruction);
    assert(parameter<int>(params, NVSDK_NGX_Parameter_DLSS_Feature_Create_Flags) == expectedFlags);
    assert(parameter<unsigned>(params, NVSDK_NGX_Parameter_Use_HW_Depth) == expectedDepth);
    assert(parameter<unsigned>(params, NVSDK_NGX_Parameter_Width) == 64);
    assert(parameter<unsigned>(params, NVSDK_NGX_Parameter_Height) == 32);
    assert(parameter<unsigned>(params, NVSDK_NGX_Parameter_OutWidth) == 128);
    assert(parameter<unsigned>(params, NVSDK_NGX_Parameter_OutHeight) == 64);
    assert(parameter<int>(params, NVSDK_NGX_Parameter_PerfQualityValue) == expectedQuality);
    *handle = reinterpret_cast<NVSDK_NGX_Handle*>(2);
    ++creations;
    return NVSDK_NGX_Result_Success;
}

static NVSDK_NGX_Result captureCreation1(
    VkDevice, VkCommandBuffer cmd, NVSDK_NGX_Feature feature,
    const NVSDK_NGX_Parameter* params, NVSDK_NGX_Handle** handle)
{
    return captureCreation(cmd, feature, params, handle);
}

static NVSDK_NGX_Result captureEvaluation(
    VkCommandBuffer, const NVSDK_NGX_Handle*, const NVSDK_NGX_Parameter* params,
    PFN_NVSDK_NGX_ProgressCallback_C)
{
    assert(parameter<float>(params, NVSDK_NGX_Parameter_MV_Scale_X) == expectedScale[0]);
    assert(parameter<float>(params, NVSDK_NGX_Parameter_MV_Scale_Y) == expectedScale[1]);
    auto* resource = static_cast<NVSDK_NGX_Resource_VK*>(parameter<void*>(params, NVSDK_NGX_Parameter_Depth));
    const auto& depth = resource->Resource.ImageViewInfo.SubresourceRange;
    assert(depth.aspectMask == VK_IMAGE_ASPECT_DEPTH_BIT);
    assert(depth.baseMipLevel == 2 && depth.levelCount == 1);
    assert(depth.baseArrayLayer == 3 && depth.layerCount == 2);
    assert(!resource->ReadWrite);
    resource = static_cast<NVSDK_NGX_Resource_VK*>(parameter<void*>(params, NVSDK_NGX_Parameter_Output));
    assert(resource->ReadWrite);
    assert((parameter<void*>(params, NVSDK_NGX_Parameter_GBuffer_SpecularMvec) != nullptr) == (expectedGuide == 0));
    assert((parameter<void*>(params, NVSDK_NGX_Parameter_DLSSD_SpecularHitDistance) != nullptr) == (expectedGuide == 1));
    ++evaluations;
    return NVSDK_NGX_Result_Success;
}

#define NVSDK_NGX_VULKAN_CreateFeature captureCreation
#define NVSDK_NGX_VULKAN_CreateFeature1 captureCreation1
#define NVSDK_NGX_VULKAN_EvaluateFeature_C captureEvaluation
#include "ngx.cpp"
#undef NVSDK_NGX_VULKAN_CreateFeature
#undef NVSDK_NGX_VULKAN_CreateFeature1
#undef NVSDK_NGX_VULKAN_EvaluateFeature_C

int main()
{
    NgxVkImage image{};
    image.image = 1;
    image.view = 2;
    image.width = 64;
    image.height = 32;
    image.format = VK_FORMAT_R32_SFLOAT;
    image.state = VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL;
    image.usage = VK_IMAGE_USAGE_SAMPLED_BIT;
    image.subresourceRange = {VK_IMAGE_ASPECT_COLOR_BIT, 4, 2, 5, 3};
    for (bool readWrite : {false, true})
    {
        const auto resource = ngxResource(image, readWrite);
        const auto& range = resource.Resource.ImageViewInfo.SubresourceRange;
        assert(range.aspectMask == VK_IMAGE_ASPECT_COLOR_BIT);
        assert(range.baseMipLevel == 4 && range.levelCount == 2);
        assert(range.baseArrayLayer == 5 && range.layerCount == 3);
        assert(resource.ReadWrite == readWrite);
    }

    for (unsigned color : {0, 1})
    for (unsigned motion : {0, 1})
    for (unsigned depth : {0, 1, 2})
    for (const auto mode : {
             std::pair{kDlssRrQualityBalanced, NVSDK_NGX_PerfQuality_Value_Balanced},
             std::pair{kDlssRrQualityPerformance, NVSDK_NGX_PerfQuality_Value_MaxPerf},
             std::pair{kDlssRrQualityUltraPerformance, NVSDK_NGX_PerfQuality_Value_UltraPerformance}})
    {
        expectedQuality = mode.second;
        Parameters params;
        NgxContext context(L"", L"", nullptr);
        context.parameters = &params;
        context.config = {color, motion, depth};
        expectedFlags = (color == 0 ? NVSDK_NGX_DLSS_Feature_Flags_IsHDR : 0)
            | (motion == 0 ? NVSDK_NGX_DLSS_Feature_Flags_MVLowRes : 0)
            | (depth == 0 ? NVSDK_NGX_DLSS_Feature_Flags_DepthInverted : 0);
        expectedDepth = depth == 2 ? NVSDK_NGX_DLSS_Depth_Type_Linear : NVSDK_NGX_DLSS_Depth_Type_HW;
        NgxDlssRrResources resources{
            image, image, image, image, image, image, image, image, kSpecularMotion, 0};
        resources.outputColor.width = 128;
        resources.outputColor.height = 64;
        resources.outputColor.usage = VK_IMAGE_USAGE_STORAGE_BIT;
        resources.outputColor.state = VK_IMAGE_LAYOUT_GENERAL;
        if (motion == 1) {
            resources.motion.width = 128;
            resources.motion.height = 64;
        }
        resources.depth.format = VK_FORMAT_D32_SFLOAT;
        resources.depth.subresourceRange = {VK_IMAGE_ASPECT_DEPTH_BIT, 2, 1, 3, 2};
        NgxFrameConstants frame{};
        auto evaluate = [&]() {
            return nvidia_dlss_ngx_evaluate(&context, reinterpret_cast<VkCommandBuffer>(3),
                mode.first, &frame, &resources);
        };
        for (const auto scale : {std::pair{1.0f, 1.0f}, std::pair{64.0f, -32.0f},
                 std::pair{0.5f, 0.0f}, std::pair{0.0f, -0.5f}, std::pair{0.0f, -0.0f}})
        {
            frame.mvecScale[0] = scale.first;
            frame.mvecScale[1] = scale.second;
            expectedScale[0] = scale.first == 0 ? 1 : scale.first;
            expectedScale[1] = scale.second == 0 ? 1 : scale.second;
            for (unsigned kind : {kSpecularMotion, kSpecularHitDistance}) {
                expectedGuide = resources.reflectionGuideKind = kind;
                assert(evaluate() == 0);
            }
        }
        const auto before = evaluations;
        const auto created = creations;
        assert(nvidia_dlss_ngx_evaluate(&context, reinterpret_cast<VkCommandBuffer>(3),
            kDlssRrQualityDlaa, &frame, &resources) == kInvalidArgument);
        for (auto* input : {&resources.inputColor, &resources.outputColor, &resources.depth,
                 &resources.motion, &resources.diffuseAlbedo, &resources.specularAlbedo,
                 &resources.normalRoughness, &resources.reflectionGuide})
        {
            const auto original = *input;
            input->usage = input == &resources.outputColor ? VK_IMAGE_USAGE_SAMPLED_BIT : VK_IMAGE_USAGE_STORAGE_BIT;
            assert(evaluate() == kInvalidArgument);
            *input = original;
            input->state = VK_IMAGE_LAYOUT_UNDEFINED;
            assert(evaluate() == kInvalidArgument);
            *input = original;
            input->width = 0;
            assert(evaluate() == kInvalidArgument);
            *input = original;
            if (input != &resources.outputColor) {
                ++input->width;
                assert(evaluate() == kInvalidArgument);
                *input = original;
                ++input->height;
                assert(evaluate() == kInvalidArgument);
                *input = original;
            }
        }
        assert(evaluations == before && creations == created);
    }
    assert(creations == 36 && evaluations == 360);
    assert(!validConfig({2, 0, 0}));
    assert(!validConfig({0, 2, 0}));
    assert(!validConfig({0, 0, 3}));
}
