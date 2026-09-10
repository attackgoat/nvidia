#![forbid(unsafe_code)]

use std::fs;

#[path = "../sdk.rs"]
mod sdk;

#[test]
fn sdk_watches_sources_not_generated_outputs() {
    let temporary = tempfile::tempdir().unwrap();
    let sdk = temporary.path();
    let sources = [
        "CMakeLists.txt",
        "Include/NRD.h",
        "Integration/NRDIntegration.hpp",
        "Resources/Version.h",
        "Source/Wrapper.cpp",
        "Source/Denoisers/Relax_Diffuse.hpp",
        "Shaders/NRD.hlsli",
        "Shaders/Shaders.cfg",
        "Shaders/Include/RELAX_Common.hlsli",
        "deps/nri/CMakeLists.txt",
        "deps/nri/Source/VK/ImplVK.cpp",
        "deps/shadermake/ShaderMake/ShaderMake.cpp",
        "deps/mathlib/MathLib.hlsli",
        "deps/sse2neon/sse2neon.h",
        "deps/vulkan-headers/include/vulkan/vulkan.h",
        "deps/vma/include/vk_mem_alloc.h",
    ];

    for relative in sources {
        let path = sdk.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "source").unwrap();
    }

    let mut expected = sources.map(|relative| sdk.join(relative)).to_vec();
    expected.sort();
    assert_eq!(sdk::sdk_inputs(sdk).unwrap(), expected);

    for relative in [
        "Shaders/NRDConfig.hlsli",
        ".nvidia-sdk-source",
        ".git/index",
        "_Build/CMakeCache.txt",
        "_Shaders/Clear.cs.spirv.h",
        "deps/nri/.git",
        "deps/shadermake/.git/index",
        "deps/nri/_Bin/libNRI.a",
        "deps/nri/_Build/CMakeCache.txt",
        "deps/nri/_Shaders/generated.h",
    ] {
        let path = sdk.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "generated").unwrap();
    }

    assert_eq!(sdk::sdk_inputs(sdk).unwrap(), expected);

    fs::write(sdk.join("Shaders/NRDConfig.hlsli"), "regenerated").unwrap();
    fs::write(sdk.join("Shaders/NRD.hlsli"), "changed source").unwrap();

    assert_eq!(sdk::sdk_inputs(sdk).unwrap(), expected);
}
