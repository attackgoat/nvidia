// SPDX-License-Identifier: MIT OR Apache-2.0

#include <omm.h>

#include <cmath>
#include <cstddef>
#include <cstdint>
#include <filesystem>
#include <memory>
#include <new>

#if defined(NVIDIA_OMM_STATIC)
#elif defined(NVIDIA_OMM_WINDOWS)
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#else
#include <dlfcn.h>
#endif

namespace {

constexpr int BRIDGE_INVALID_ARGUMENT = -1;
constexpr int BRIDGE_LOAD_FAILED = -2;
constexpr int BRIDGE_SYMBOL_MISSING = -3;
constexpr int BRIDGE_VERSION_MISMATCH = -4;
constexpr int BRIDGE_OUT_OF_MEMORY = -5;
constexpr int BRIDGE_EXCEPTION = -6;
constexpr int BRIDGE_INVALID_OUTPUT = -7;

struct NvidiaOmmTextureMip {
    uint32_t width;
    uint32_t height;
    uint32_t rowPitch;
    uint32_t reserved;
    const uint8_t* data;
};

struct NvidiaOmmBakeInput {
    ommCpuTexture texture;
    const float* textureCoordinates;
    uint32_t textureCoordinateCount;
    const uint32_t* indices;
    uint32_t indexCount;
    float dynamicSubdivisionScale;
    float rejectionThreshold;
    uint32_t maxArrayDataSize;
    uint64_t maxWorkloadSize;
    uint8_t maxSubdivisionLevel;
    uint8_t internalThreads;
    uint8_t validation;
    uint8_t reserved[5];
};

struct NvidiaOmmStatistics {
    uint64_t opaque;
    uint64_t transparent;
    uint64_t unknownTransparent;
    uint64_t unknownOpaque;
    uint32_t fullyOpaque;
    uint32_t fullyTransparent;
    uint32_t fullyUnknownOpaque;
    uint32_t fullyUnknownTransparent;
    float knownArea;
};

struct NvidiaOmmBakeResult {
    const uint8_t* arrayData;
    uint32_t arrayDataSize;
    const ommCpuOpacityMicromapDesc* descriptors;
    uint32_t descriptorCount;
    const ommCpuOpacityMicromapUsageCount* descriptorUsage;
    uint32_t descriptorUsageCount;
    const int32_t* indices;
    uint32_t indexCount;
    const ommCpuOpacityMicromapUsageCount* indexUsage;
    uint32_t indexUsageCount;
    NvidiaOmmStatistics statistics;
};

static_assert(sizeof(NvidiaOmmTextureMip) == 24);
static_assert(offsetof(NvidiaOmmTextureMip, data) == 16);
static_assert(offsetof(NvidiaOmmBakeInput, maxWorkloadSize) == 48);
static_assert(offsetof(NvidiaOmmStatistics, knownArea) == 48);
static_assert(ommSpecialIndex_FullyUnknownOpaque == -4);
static_assert(OMM_VERSION_MAJOR == 1 && OMM_VERSION_MINOR == 9 && OMM_VERSION_BUILD == 2,
    "OMM SDK must be version 1.9.2");
static_assert(sizeof(NvidiaOmmBakeInput) == 64);
static_assert(sizeof(NvidiaOmmStatistics) == 56);
static_assert(sizeof(NvidiaOmmBakeResult) == 136);
static_assert(sizeof(ommCpuOpacityMicromapDesc) == 8);
static_assert(sizeof(ommCpuOpacityMicromapUsageCount) == 8);

class Module {
public:
    bool load(const char* path) {
#if defined(NVIDIA_OMM_STATIC)
        return path != nullptr;
#else
#if defined(NVIDIA_OMM_WINDOWS)
        handle_ = LoadLibraryW(std::filesystem::u8path(path).c_str());
#else
        handle_ = dlopen(path, RTLD_NOW | RTLD_LOCAL);
#endif
        return handle_ != nullptr;
#endif
    }

    void* symbol(const char* name) const {
#if defined(NVIDIA_OMM_STATIC)
        return nullptr;
#elif defined(NVIDIA_OMM_WINDOWS)
        return reinterpret_cast<void*>(GetProcAddress(handle_, name));
#else
        return dlsym(handle_, name);
#endif
    }

    ~Module() {
#if !defined(NVIDIA_OMM_STATIC)
        if (handle_ == nullptr)
            return;
#if defined(NVIDIA_OMM_WINDOWS)
        FreeLibrary(handle_);
#else
        dlclose(handle_);
#endif
#endif
    }

private:
#if defined(NVIDIA_OMM_STATIC)
#elif defined(NVIDIA_OMM_WINDOWS)
    HMODULE handle_ = nullptr;
#else
    void* handle_ = nullptr;
#endif
};

struct Api {
    decltype(&ommGetLibraryDesc) getLibraryDesc = nullptr;
    decltype(&ommCreateBaker) createBaker = nullptr;
    decltype(&ommDestroyBaker) destroyBaker = nullptr;
    decltype(&ommCpuCreateTexture) createTexture = nullptr;
    decltype(&ommCpuDestroyTexture) destroyTexture = nullptr;
    decltype(&ommCpuBake) bake = nullptr;
    decltype(&ommCpuGetBakeResultDesc) getBakeResultDesc = nullptr;
    decltype(&ommCpuDestroyBakeResult) destroyBakeResult = nullptr;
    decltype(&ommDebugGetStats2) getStats = nullptr;
};

struct Context {
    Module module;
    Api api;
    ommBaker baker = nullptr;
};

struct Result {
    Context* context;
    ommCpuBakeResult result;
};

template <typename T>
bool loadFunction(const Module& module, const char* name, T& output) {
    output = reinterpret_cast<T>(module.symbol(name));
    return output != nullptr;
}

bool loadApi(Context& context) {
#if defined(NVIDIA_OMM_STATIC)
    context.api = {
        &ommGetLibraryDesc,
        &ommCreateBaker,
        &ommDestroyBaker,
        &ommCpuCreateTexture,
        &ommCpuDestroyTexture,
        &ommCpuBake,
        &ommCpuGetBakeResultDesc,
        &ommCpuDestroyBakeResult,
        &ommDebugGetStats2,
    };
    return true;
#else
    return loadFunction(context.module, "ommGetLibraryDesc", context.api.getLibraryDesc)
        && loadFunction(context.module, "ommCreateBaker", context.api.createBaker)
        && loadFunction(context.module, "ommDestroyBaker", context.api.destroyBaker)
        && loadFunction(context.module, "ommCpuCreateTexture", context.api.createTexture)
        && loadFunction(context.module, "ommCpuDestroyTexture", context.api.destroyTexture)
        && loadFunction(context.module, "ommCpuBake", context.api.bake)
        && loadFunction(context.module, "ommCpuGetBakeResultDesc", context.api.getBakeResultDesc)
        && loadFunction(context.module, "ommCpuDestroyBakeResult", context.api.destroyBakeResult)
        && loadFunction(context.module, "ommDebugGetStats2", context.api.getStats);
#endif
}

void messageCallback(ommMessageSeverity, const char*, void*) {}

template <typename F>
int ffiBoundary(F&& function) noexcept {
    try {
        return function();
    } catch (const std::bad_alloc&) {
        return BRIDGE_OUT_OF_MEMORY;
    } catch (...) {
        return BRIDGE_EXCEPTION;
    }
}

} // namespace

extern "C" int nvidia_omm_create(const char* libraryPath, Context** output) noexcept {
    return ffiBoundary([&] {
        if (libraryPath == nullptr || libraryPath[0] == '\0' || output == nullptr)
            return BRIDGE_INVALID_ARGUMENT;
        *output = nullptr;
        std::unique_ptr<Context> context(new Context());
        if (!context->module.load(libraryPath))
            return BRIDGE_LOAD_FAILED;
        if (!loadApi(*context))
            return BRIDGE_SYMBOL_MISSING;
        const ommLibraryDesc library = context->api.getLibraryDesc();
        if (library.versionMajor != 1 || library.versionMinor != 9 || library.versionBuild != 2)
            return BRIDGE_VERSION_MISMATCH;
        ommBakerCreationDesc desc = ommBakerCreationDescDefault();
        desc.type = ommBakerType_CPU;
        desc.messageInterface.messageCallback = messageCallback;
        const ommResult result = context->api.createBaker(&desc, &context->baker);
        if (result != ommResult_SUCCESS)
            return static_cast<int>(result);
        *output = context.release();
        return static_cast<int>(ommResult_SUCCESS);
    });
}

extern "C" int nvidia_omm_destroy(Context* context) noexcept {
    return ffiBoundary([&] {
        if (context == nullptr)
            return BRIDGE_INVALID_ARGUMENT;
        const ommResult result = context->api.destroyBaker(context->baker);
        delete context;
        return static_cast<int>(result);
    });
}

extern "C" int nvidia_omm_create_texture(
    Context* context,
    const NvidiaOmmTextureMip* mips,
    uint32_t mipCount,
    ommCpuTexture* output) noexcept {
    return ffiBoundary([&] {
        if (context == nullptr || mips == nullptr || mipCount == 0 || output == nullptr)
            return BRIDGE_INVALID_ARGUMENT;
        std::unique_ptr<ommCpuTextureMipDesc[]> sdkMips(new ommCpuTextureMipDesc[mipCount]);
        for (uint32_t index = 0; index < mipCount; ++index) {
            const uint32_t pitch = mips[index].rowPitch == 0 ? mips[index].width : mips[index].rowPitch;
            if (mips[index].width == 0 || mips[index].height == 0
                || pitch < mips[index].width || mips[index].data == nullptr)
                return BRIDGE_INVALID_ARGUMENT;
            sdkMips[index] = ommCpuTextureMipDescDefault();
            sdkMips[index].width = mips[index].width;
            sdkMips[index].height = mips[index].height;
            sdkMips[index].rowPitch = mips[index].rowPitch;
            sdkMips[index].textureData = mips[index].data;
        }
        ommCpuTextureDesc desc = ommCpuTextureDescDefault();
        desc.format = ommCpuTextureFormat_UNORM8;
        desc.mips = sdkMips.get();
        desc.mipCount = mipCount;
        desc.alphaCutoff = 0.5f;
        return static_cast<int>(context->api.createTexture(context->baker, &desc, output));
    });
}

extern "C" int nvidia_omm_destroy_texture(Context* context, ommCpuTexture texture) noexcept {
    return ffiBoundary([&] {
        if (context == nullptr || texture == nullptr)
            return BRIDGE_INVALID_ARGUMENT;
        return static_cast<int>(context->api.destroyTexture(context->baker, texture));
    });
}

extern "C" int nvidia_omm_bake(
    Context* context,
    const NvidiaOmmBakeInput* input,
    NvidiaOmmBakeResult* output,
    Result** outputResult) noexcept {
    return ffiBoundary([&] {
        if (context == nullptr || input == nullptr || output == nullptr || outputResult == nullptr
            || input->texture == nullptr || input->textureCoordinates == nullptr
            || input->textureCoordinateCount == 0 || input->indices == nullptr
            || input->indexCount == 0 || input->indexCount % 3 != 0
            || !std::isfinite(input->dynamicSubdivisionScale)
            || input->dynamicSubdivisionScale <= 0.0f
            || !std::isfinite(input->rejectionThreshold)
            || input->rejectionThreshold < 0.0f || input->rejectionThreshold > 1.0f
            || input->maxSubdivisionLevel > 12)
            return BRIDGE_INVALID_ARGUMENT;
        for (uint32_t index = 0; index < input->indexCount; ++index)
            if (input->indices[index] >= input->textureCoordinateCount)
                return BRIDGE_INVALID_ARGUMENT;

        ommCpuBakeInputDesc desc = ommCpuBakeInputDescDefault();
        desc.bakeFlags = static_cast<ommCpuBakeFlags>(ommCpuBakeFlags_Force32BitIndices
            | (input->internalThreads ? ommCpuBakeFlags_EnableInternalThreads : 0)
            | (input->validation ? ommCpuBakeFlags_EnableValidation : 0));
        desc.texture = input->texture;
        desc.runtimeSamplerDesc.addressingMode = ommTextureAddressMode_Wrap;
        desc.runtimeSamplerDesc.filter = ommTextureFilterMode_Linear;
        desc.alphaMode = ommAlphaMode_Test;
        desc.texCoordFormat = ommTexCoordFormat_UV32_FLOAT;
        desc.texCoords = input->textureCoordinates;
        desc.texCoordStrideInBytes = sizeof(float) * 2;
        desc.indexFormat = ommIndexFormat_UINT_32;
        desc.indexBuffer = input->indices;
        desc.indexCount = input->indexCount;
        desc.dynamicSubdivisionScale = input->dynamicSubdivisionScale;
        desc.rejectionThreshold = input->rejectionThreshold;
        desc.alphaCutoff = 0.5f;
        desc.nearDuplicateDeduplicationFactor = 0.0f;
        desc.alphaCutoffLessEqual = ommOpacityState_Transparent;
        desc.alphaCutoffGreater = ommOpacityState_Opaque;
        desc.format = ommFormat_OC1_4_State;
        desc.unknownStatePromotion = ommUnknownStatePromotion_ForceOpaque;
        desc.unresolvedTriState = ommSpecialIndex_FullyUnknownOpaque;
        desc.maxSubdivisionLevel = input->maxSubdivisionLevel;
        desc.maxArrayDataSize = input->maxArrayDataSize;
        desc.maxWorkloadSize = input->maxWorkloadSize;

        ommCpuBakeResult bakeResult = nullptr;
        ommResult result = context->api.bake(context->baker, &desc, &bakeResult);
        if (result != ommResult_SUCCESS)
            return static_cast<int>(result);
        const ommCpuBakeResultDesc* baked = nullptr;
        result = context->api.getBakeResultDesc(bakeResult, &baked);
        if (result != ommResult_SUCCESS || baked == nullptr || baked->indexFormat != ommIndexFormat_UINT_32) {
            context->api.destroyBakeResult(bakeResult);
            return result == ommResult_SUCCESS ? BRIDGE_INVALID_OUTPUT : static_cast<int>(result);
        }
        ommDebugStats statistics = ommDebugStatsDefault();
        result = context->api.getStats(context->baker, bakeResult, &statistics);
        if (result != ommResult_SUCCESS) {
            context->api.destroyBakeResult(bakeResult);
            return static_cast<int>(result);
        }
        output->arrayData = static_cast<const uint8_t*>(baked->arrayData);
        output->arrayDataSize = baked->arrayDataSize;
        output->descriptors = baked->descArray;
        output->descriptorCount = baked->descArrayCount;
        output->descriptorUsage = baked->descArrayHistogram;
        output->descriptorUsageCount = baked->descArrayHistogramCount;
        output->indices = static_cast<const int32_t*>(baked->indexBuffer);
        output->indexCount = baked->indexCount;
        output->indexUsage = baked->indexHistogram;
        output->indexUsageCount = baked->indexHistogramCount;
        output->statistics = {
            statistics.totalOpaque, statistics.totalTransparent,
            statistics.totalUnknownTransparent, statistics.totalUnknownOpaque,
            statistics.totalFullyOpaque, statistics.totalFullyTransparent,
            statistics.totalFullyUnknownOpaque, statistics.totalFullyUnknownTransparent,
            statistics.knownAreaMetric,
        };
        Result* ownedResult = new (std::nothrow) Result{context, bakeResult};
        if (ownedResult == nullptr) {
            context->api.destroyBakeResult(bakeResult);
            return BRIDGE_OUT_OF_MEMORY;
        }
        *outputResult = ownedResult;
        return static_cast<int>(ommResult_SUCCESS);
    });
}

extern "C" int nvidia_omm_destroy_bake_result(Result* result) noexcept {
    return ffiBoundary([&] {
        if (result == nullptr)
            return BRIDGE_INVALID_ARGUMENT;
        const ommResult code = result->context->api.destroyBakeResult(result->result);
        delete result;
        return static_cast<int>(code);
    });
}

extern "C" const char* nvidia_omm_result_name(int result) noexcept {
    switch (result) {
    case ommResult_SUCCESS: return "success";
    case ommResult_FAILURE: return "failure";
    case ommResult_INVALID_ARGUMENT: return "invalid argument";
    case ommResult_INSUFFICIENT_SCRATCH_MEMORY: return "insufficient scratch memory";
    case ommResult_NOT_IMPLEMENTED: return "not implemented";
    case ommResult_WORKLOAD_TOO_BIG: return "workload too big";
    case BRIDGE_INVALID_ARGUMENT: return "bridge invalid argument";
    case BRIDGE_LOAD_FAILED: return "loading OMM shared library failed";
    case BRIDGE_SYMBOL_MISSING: return "required OMM symbol is missing";
    case BRIDGE_VERSION_MISMATCH: return "OMM runtime version is not 1.9.2";
    case BRIDGE_OUT_OF_MEMORY: return "bridge allocation failed";
    case BRIDGE_EXCEPTION: return "unexpected bridge exception";
    case BRIDGE_INVALID_OUTPUT: return "OMM returned invalid output";
    default: return "unknown result";
    }
}
