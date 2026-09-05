use {
    nvidia_sdk::{ResolveOptions, Sdk},
    std::{env, error::Error, fs, io, path::Path},
};

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rustc-check-cfg=cfg(nvidia_omm_native)");
    println!("cargo:rustc-check-cfg=cfg(nvidia_omm_static)");
    println!("cargo:rerun-if-env-changed=DOCS_RS");

    if env::var_os("DOCS_RS").is_some() {
        return Ok(());
    }

    println!("cargo:rerun-if-env-changed=OMM_SDK");
    println!("cargo:rerun-if-env-changed=NVIDIA_SDK_ROOT");
    println!("cargo:rerun-if-env-changed=NVIDIA_SDK_CACHE");
    println!("cargo:rerun-if-env-changed=NVIDIA_SDK_OFFLINE");
    println!("cargo:rerun-if-env-changed=CARGO_NET_OFFLINE");
    println!("cargo:rerun-if-changed=native/omm.cpp");

    if env::var_os("CARGO_FEATURE_NATIVE").is_none() || env::var("HOST") != env::var("TARGET") {
        return Ok(());
    }

    let target = env::var("TARGET").map_err(|error| format!("reading TARGET: {error}"))?;

    if !matches!(
        target.as_str(),
        "x86_64-unknown-linux-gnu"
            | "x86_64-pc-windows-msvc"
            | "x86_64-apple-darwin"
            | "aarch64-apple-darwin"
    ) {
        return Ok(());
    }

    let mut sdk = Sdk::Omm.resolve(&ResolveOptions::for_cargo()?)?;
    sdk.path = dunce::canonicalize(&sdk.path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "canonicalizing omm sdk directory {}: {error}",
                sdk.path.display()
            ),
        )
    })?;

    println!("cargo:rerun-if-changed={}", sdk.path.display());

    if target.ends_with("apple-darwin") {
        return build_macos_source(&sdk.path);
    }

    let include = sdk.path.join("include");
    let mut build = cc::Build::new();
    build
        .cpp(true)
        .file("native/omm.cpp")
        .include(include)
        .flag_if_supported("-std=c++17")
        .flag_if_supported("/std:c++17")
        .flag_if_supported("/EHsc");

    let backend = if target == "x86_64-unknown-linux-gnu" {
        build.define("NVIDIA_OMM_LINUX", None);

        println!("cargo:rustc-link-lib=dylib=dl");

        sdk.path.join("lib/libomm-lib.so.1.9.2")
    } else {
        build.define("NVIDIA_OMM_WINDOWS", None);
        sdk.path.join("bin/omm-lib.dll")
    };
    build.compile("nvidia_omm_native");

    validate_library_architecture(&backend, &target)?;
    let backend = dunce::canonicalize(&backend).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("canonicalizing omm backend {}: {error}", backend.display()),
        )
    })?;
    let backend = backend
        .to_str()
        .ok_or_else(|| format!("omm backend path is not valid utf-8: {}", backend.display()))?;

    println!("cargo:rustc-env=NVIDIA_OMM_BACKEND={backend}");
    println!("cargo:rustc-cfg=nvidia_omm_native");

    Ok(())
}

fn build_macos_source(sdk: &Path) -> Result<(), Box<dyn Error>> {
    let source = sdk.join("source");
    let omm = source.join("omm-lib");
    let third_party = source.join("third-party");
    let allocator_path = omm.join("src/std_allocator.h");
    let allocator = fs::read_to_string(&allocator_path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "reading omm allocator {}: {error}",
                allocator_path.display()
            ),
        )
    })?;

    if !allocator.contains("#elif defined(__linux__) || defined(__APPLE__)") {
        return Err(format!("omm source requires the macos aligned-allocator patch at {}; see crates/nvidia-sdk/SOURCE-SDKS.md", allocator_path.display()).into());
    }

    // Emit dependent archives first in the linker command, before their dependencies.
    cc::Build::new()
        .cpp(true)
        .files([
            omm.join("src/bake.cpp"),
            omm.join("src/bake_cpu_impl.cpp"),
            omm.join("src/bake_gpu_impl.cpp"),
            omm.join("src/debug_impl.cpp"),
            omm.join("src/serialize_impl.cpp"),
            omm.join("src/shader_bindings.cpp"),
            omm.join("src/stb_lib.cpp"),
            omm.join("src/texture_impl.cpp"),
            Path::new("native/omm.cpp").to_owned(),
        ])
        .include(sdk.join("include"))
        .include(omm.join("src"))
        .include(omm.join("shaders"))
        .include(third_party.join("glm"))
        .include(third_party.join("lz4"))
        .include(third_party.join("stb"))
        .include(third_party.join("xxhash"))
        .define("NVIDIA_OMM_STATIC", None)
        .define("OMM_ONLY_SPIRV_SHADERS_AVAILABLE", "1")
        .define("OMM_VK_S_SHIFT", "100")
        .define("OMM_VK_T_SHIFT", "200")
        .define("OMM_VK_B_SHIFT", "300")
        .define("OMM_VK_U_SHIFT", "400")
        .flag_if_supported("-std=c++20")
        .warnings(false)
        .compile("nvidia_omm_native");

    for library in ["lz4", "xxhash"] {
        let directory = third_party.join(library);
        cc::Build::new()
            .file(directory.join(format!("{library}.c")))
            .include(directory)
            .warnings(false)
            .compile(&format!("nvidia_omm_{library}"));
    }

    let sdk = sdk
        .to_str()
        .ok_or_else(|| format!("omm sdk path is not valid utf-8: {}", sdk.display()))?;

    println!("cargo:rustc-env=NVIDIA_OMM_BACKEND={sdk}");
    println!("cargo:rustc-cfg=nvidia_omm_native");
    println!("cargo:rustc-cfg=nvidia_omm_static");

    Ok(())
}

fn validate_library_architecture(path: &Path, target: &str) -> Result<(), Box<dyn Error>> {
    let bytes = fs::read(path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("reading omm shared library {}: {error}", path.display()),
        )
    })?;
    let valid = if target.contains("linux") {
        bytes.get(..4) == Some(b"\x7fELF")
            && bytes.get(4) == Some(&2)
            && bytes.get(5) == Some(&1)
            && bytes.get(18..20) == Some(&[0x3e, 0x00])
    } else {
        bytes
            .get(0x3c..0x40)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u32::from_le_bytes)
            .is_some_and(|offset| {
                let offset = offset as usize;
                bytes.get(offset..offset + 4) == Some(b"PE\0\0")
                    && bytes.get(offset + 4..offset + 6) == Some(&[0x64, 0x86])
            })
    };

    if !valid {
        return Err(format!(
            "omm shared library is not a native x86-64 binary: {}",
            path.display()
        )
        .into());
    }

    Ok(())
}
