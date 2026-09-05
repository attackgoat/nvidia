/*
This software contains source code provided by NVIDIA Corporation.
Copyright (c) 2022, NVIDIA CORPORATION. All rights reserved.
*/

#include <algorithm>
#include <cstdio>
#include <cstdint>
#include <cstring>
#include <exception>
#include <new>

#include "NRI.h"
#include "Extensions/NRIHelper.h"
#include "Extensions/NRIRayTracing.h"
#include "Extensions/NRIWrapperVK.h"
#include "NRD.h"
#include "NRDIntegration.h"
#include "NRDIntegration.hpp"

static_assert(NRD_VERSION_MAJOR == 4 && NRD_VERSION_MINOR == 17 && NRD_VERSION_BUILD == 3,
    "NRD SDK must be version 4.17.3");

namespace {

constexpr nrd::Identifier kRelax = 1;
constexpr const char* kRequiredDeviceExtensions[] = {"VK_KHR_push_descriptor"};

struct NrdImage {
    uint64_t image;
    int32_t format;
    int32_t layout;
};

struct NrdResources {
    NrdImage motion;
    NrdImage normal_roughness;
    NrdImage view_z;
    NrdImage diffuse_sh0;
    NrdImage diffuse_sh1;
    NrdImage specular_sh0;
    NrdImage specular_sh1;
    NrdImage output_diffuse_sh0;
    NrdImage output_diffuse_sh1;
    NrdImage output_specular_sh0;
    NrdImage output_specular_sh1;
};

struct NrdFrame {
    float view_to_clip[16];
    float view_to_clip_previous[16];
    float world_to_view[16];
    float world_to_view_previous[16];
    float motion_scale[3];
    float jitter[2];
    float jitter_previous[2];
    float denoising_range;
    uint32_t width;
    uint32_t height;
    uint32_t frame_index;
    uint32_t reset;
    struct {
        uint32_t diffuse_max_accumulated_frame_num;
        uint32_t diffuse_max_fast_accumulated_frame_num;
        uint32_t history_fix_frame_num;
        uint32_t atrous_iteration_num;
        float diffuse_prepass_blur_radius;
        float min_hit_distance_weight;
        float diffuse_phi_luminance;
        float depth_threshold;
        uint32_t hit_distance_reconstruction_mode;
        uint32_t enable_anti_firefly;
    } settings;
};

static_assert(sizeof(NrdImage) == 16);
static_assert(sizeof(NrdResources) == 176);
static_assert(sizeof(NrdFrame) == 344);

struct Context {
    nrd::Integration integration;
    nri::QueueFamilyVKDesc queue_family = {};
    nri::DeviceCreationVKDesc device = {};
    nrd::RelaxSettings settings = {};
    uint16_t width = 0;
    uint16_t height = 0;
    uint8_t queued_evaluations = 0;
    bool ready = false;
};

void message_callback(nri::Message type, const char*, uint32_t, const char* message, void*) {
    if (type == nri::Message::ERROR)
        std::fprintf(stderr, "NRI error: %s\n", message);
}

nrd::Resource image_resource(const NrdImage& image, bool writable) {
    nrd::Resource resource = {};
    resource.vk.image = image.image;
    resource.vk.format = image.format;
    resource.state.access = writable ? nri::AccessBits::SHADER_RESOURCE_STORAGE : nri::AccessBits::SHADER_RESOURCE;
    resource.state.layout = writable ? nri::Layout::SHADER_RESOURCE_STORAGE : nri::Layout::SHADER_RESOURCE;
    resource.state.stages = nri::StageBits::COMPUTE_SHADER;

    return resource;
}

int32_t recreate(Context& context, uint32_t width, uint32_t height) {
    if (width == 0 || height == 0 || width > UINT16_MAX || height > UINT16_MAX)
        return -2;
    if (context.ready && context.width == width && context.height == height)
        return 0;

    nrd::DenoiserDesc denoiser = {kRelax, nrd::Denoiser::RELAX_DIFFUSE_SPECULAR_SH};
    nrd::InstanceCreationDesc instance = {};
    instance.denoisers = &denoiser;
    instance.denoisersNum = 1;

    nrd::IntegrationCreationDesc integration = {};
    std::memcpy(integration.name, "Retro NRD RELAX SH", sizeof("Retro NRD RELAX SH"));
    integration.resourceWidth = static_cast<uint16_t>(width);
    integration.resourceHeight = static_cast<uint16_t>(height);
    integration.queuedFrameNum = context.queued_evaluations;
    integration.autoWaitForIdle = true;

    // RecreateVK destroys the old integration even if replacement fails or throws.
    context.ready = false;
    nrd::Result result = context.integration.RecreateVK(integration, instance, context.device);
    if (result != nrd::Result::SUCCESS)
        return static_cast<int32_t>(result) + 1;

    context.width = static_cast<uint16_t>(width);
    context.height = static_cast<uint16_t>(height);
    context.ready = true;

    return 0;
}

} // namespace

extern "C" int32_t nvidia_nrd_create(
    void* vk_instance,
    void* vk_physical_device,
    void* vk_device,
    uint32_t queue_family_index,
    uint8_t vulkan_minor_version,
    uint8_t queued_evaluations,
    void** output) noexcept
{
    if (!vk_instance || !vk_physical_device || !vk_device || !output || vulkan_minor_version < 3 || queued_evaluations == 0)
        return -2;

    try {
        Context* context = new Context();
        context->queued_evaluations = queued_evaluations;
        context->queue_family.queueNum = 1;
        context->queue_family.queueType = nri::QueueType::GRAPHICS;
        context->queue_family.familyIndex = queue_family_index;
        context->device.callbackInterface.MessageCallback = message_callback;
        context->device.vkInstance = vk_instance;
        context->device.vkPhysicalDevice = vk_physical_device;
        context->device.vkDevice = vk_device;
        context->device.queueFamilies = &context->queue_family;
        context->device.queueFamilyNum = 1;
        context->device.minorVersion = vulkan_minor_version;

        if (vulkan_minor_version < 4) {
            context->device.vkExtensions.deviceExtensions = kRequiredDeviceExtensions;
            context->device.vkExtensions.deviceExtensionNum = 1;
        }

        *output = context;

        return 0;
    } catch (const std::bad_alloc&) {
        return -3;
    } catch (...) {
        return -1;
    }
}

extern "C" int32_t nvidia_nrd_evaluate(
    void* opaque,
    void* vk_command_buffer,
    const NrdFrame* frame,
    const NrdResources* resources) noexcept
{
    if (!opaque || !vk_command_buffer || !frame || !resources)
        return -2;

    try {
        Context& context = *static_cast<Context*>(opaque);
        int32_t result = recreate(context, frame->width, frame->height);
        if (result != 0)
            return result;

        nrd::CommonSettings common = {};
        std::memcpy(common.viewToClipMatrix, frame->view_to_clip, sizeof(common.viewToClipMatrix));
        std::memcpy(common.viewToClipMatrixPrev, frame->view_to_clip_previous, sizeof(common.viewToClipMatrixPrev));
        std::memcpy(common.worldToViewMatrix, frame->world_to_view, sizeof(common.worldToViewMatrix));
        std::memcpy(common.worldToViewMatrixPrev, frame->world_to_view_previous, sizeof(common.worldToViewMatrixPrev));
        std::memcpy(common.motionVectorScale, frame->motion_scale, sizeof(common.motionVectorScale));
        std::memcpy(common.cameraJitter, frame->jitter, sizeof(common.cameraJitter));
        std::memcpy(common.cameraJitterPrev, frame->jitter_previous, sizeof(common.cameraJitterPrev));
        common.resourceSize[0] = context.width;
        common.resourceSize[1] = context.height;
        common.resourceSizePrev[0] = context.width;
        common.resourceSizePrev[1] = context.height;
        common.rectSize[0] = context.width;
        common.rectSize[1] = context.height;
        common.rectSizePrev[0] = context.width;
        common.rectSizePrev[1] = context.height;
        common.denoisingRange = frame->denoising_range;
        common.frameIndex = frame->frame_index;
        common.accumulationMode = frame->reset ? nrd::AccumulationMode::RESTART : nrd::AccumulationMode::CONTINUE;
        context.integration.NewFrame();
        nrd::Result nrd_result = context.integration.SetCommonSettings(common);
        if (nrd_result != nrd::Result::SUCCESS)
            return static_cast<int32_t>(nrd_result) + 1;

        context.settings.diffuseMaxAccumulatedFrameNum = frame->settings.diffuse_max_accumulated_frame_num;
        context.settings.diffuseMaxFastAccumulatedFrameNum = frame->settings.diffuse_max_fast_accumulated_frame_num;
        context.settings.historyFixFrameNum = frame->settings.history_fix_frame_num;
        context.settings.atrousIterationNum = frame->settings.atrous_iteration_num;
        context.settings.diffusePrepassBlurRadius = frame->settings.diffuse_prepass_blur_radius;
        context.settings.minHitDistanceWeight = frame->settings.min_hit_distance_weight;
        context.settings.diffusePhiLuminance = frame->settings.diffuse_phi_luminance;
        context.settings.depthThreshold = frame->settings.depth_threshold;
        context.settings.hitDistanceReconstructionMode = static_cast<nrd::HitDistanceReconstructionMode>(
            frame->settings.hit_distance_reconstruction_mode);
        context.settings.enableAntiFirefly = frame->settings.enable_anti_firefly != 0;
        nrd_result = context.integration.SetDenoiserSettings(kRelax, &context.settings);
        if (nrd_result != nrd::Result::SUCCESS)
            return static_cast<int32_t>(nrd_result) + 1;

        nrd::ResourceSnapshot snapshot = {};
        snapshot.restoreInitialState = true;
        snapshot.SetResource(nrd::ResourceType::IN_MV, image_resource(resources->motion, false));
        snapshot.SetResource(nrd::ResourceType::IN_NORMAL_ROUGHNESS, image_resource(resources->normal_roughness, false));
        snapshot.SetResource(nrd::ResourceType::IN_VIEWZ, image_resource(resources->view_z, false));
        snapshot.SetResource(nrd::ResourceType::IN_DIFF_SH0, image_resource(resources->diffuse_sh0, false));
        snapshot.SetResource(nrd::ResourceType::IN_DIFF_SH1, image_resource(resources->diffuse_sh1, false));
        snapshot.SetResource(nrd::ResourceType::IN_SPEC_SH0, image_resource(resources->specular_sh0, false));
        snapshot.SetResource(nrd::ResourceType::IN_SPEC_SH1, image_resource(resources->specular_sh1, false));
        snapshot.SetResource(nrd::ResourceType::OUT_DIFF_SH0, image_resource(resources->output_diffuse_sh0, true));
        snapshot.SetResource(nrd::ResourceType::OUT_DIFF_SH1, image_resource(resources->output_diffuse_sh1, true));
        snapshot.SetResource(nrd::ResourceType::OUT_SPEC_SH0, image_resource(resources->output_specular_sh0, true));
        snapshot.SetResource(nrd::ResourceType::OUT_SPEC_SH1, image_resource(resources->output_specular_sh1, true));

        nri::CommandBufferVKDesc command = {};
        command.vkCommandBuffer = vk_command_buffer;
        command.queueType = nri::QueueType::GRAPHICS;
        context.integration.DenoiseVK(&kRelax, 1, command, snapshot);

        return 0;
    } catch (...) {
        return -1;
    }
}

extern "C" void nvidia_nrd_destroy(void* opaque) noexcept {
    try {
        delete static_cast<Context*>(opaque);
    } catch (...) {
    }
}
