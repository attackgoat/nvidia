use {
    nvidia_sdk::{ResolveOptions, Sdk},
    std::{env, error::Error, io, path::PathBuf},
};

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rustc-check-cfg=cfg(nvidia_nrd_native)");
    println!("cargo:rerun-if-env-changed=DOCS_RS");

    if env::var_os("DOCS_RS").is_some() {
        return Ok(());
    }

    for name in [
        "NRD_SDK",
        "NVIDIA_SDK_ROOT",
        "NVIDIA_SDK_CACHE",
        "NVIDIA_SDK_OFFLINE",
        "CARGO_NET_OFFLINE",
        "VULKAN_SDK",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }

    println!("cargo:rerun-if-changed=native");

    let target =
        env::var("TARGET").map_err(|error| io::Error::other(format!("reading TARGET: {error}")))?;

    if env::var_os("CARGO_FEATURE_NATIVE").is_none()
        || !matches!(
            target.as_str(),
            "x86_64-pc-windows-msvc" | "x86_64-unknown-linux-gnu" | "aarch64-apple-darwin"
        )
    {
        return Ok(());
    }

    let sdk = Sdk::Nrd.resolve(&ResolveOptions::for_cargo()?)?;
    let sdk = dunce::canonicalize(&sdk.path).map_err(|error| {
        io::Error::other(format!(
            "resolving nrd sdk directory {}: {error}",
            sdk.path.display()
        ))
    })?;

    println!("cargo:rerun-if-changed={}", sdk.display());

    let manifest =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").ok_or("missing CARGO_MANIFEST_DIR")?);
    let output = cmake::Config::new(manifest.join("native"))
        .define("NRD_SOURCE_DIR", sdk)
        .define("CMAKE_BUILD_TYPE", "Release")
        .profile("Release")
        .build();

    println!(
        "cargo:rustc-link-search=native={}",
        output.join("lib").display()
    );

    for library in [
        "nvidia_nrd",
        "NRD",
        "NRI",
        "NRI_Shared",
        "NRI_VK",
        "ShaderMakeBlob",
    ] {
        println!("cargo:rustc-link-lib=static={library}");
    }

    if target.contains("linux") {
        println!("cargo:rustc-link-lib=dl");
        println!("cargo:rustc-link-lib=pthread");
    }

    if target.contains("apple") {
        println!("cargo:rustc-link-lib=dylib=c++");

        for framework in ["Metal", "QuartzCore", "Foundation"] {
            println!("cargo:rustc-link-lib=framework={framework}");
        }
    } else if target.contains("windows") {
        println!("cargo:rustc-link-lib=dylib=user32");
    } else {
        println!("cargo:rustc-link-lib=dylib=stdc++");
    }

    println!("cargo:rustc-cfg=nvidia_nrd_native");

    Ok(())
}
