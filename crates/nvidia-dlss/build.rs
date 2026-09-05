use std::{
    env,
    error::Error,
    fs::{self, File},
    io::{self, Read as _},
    path::{Path, PathBuf},
};

const LFS_POINTER_PREFIX: &[u8] = b"version https://git-lfs.github.com/spec/v1";
const NATIVE_TARGET: &str = "x86_64-unknown-linux-gnu";
const RUNTIME_FILE: &str = "libnvidia-ngx-dlssd.so.310.4.0";

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rustc-check-cfg=cfg(nvidia_dlss_native)");
    println!("cargo:rerun-if-env-changed=DOCS_RS");

    if env::var_os("DOCS_RS").is_some() {
        return Ok(());
    }

    if env::var("TARGET").as_deref() != Ok(NATIVE_TARGET) {
        return Ok(());
    }

    configure_native()
}

fn configure_native() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-env-changed=DLSS_SDK");

    for variable in [
        "NVIDIA_SDK_ROOT",
        "NVIDIA_SDK_CACHE",
        "NVIDIA_SDK_OFFLINE",
        "CARGO_NET_OFFLINE",
        "XDG_CACHE_HOME",
        "HOME",
        "LOCALAPPDATA",
    ] {
        println!("cargo:rerun-if-env-changed={variable}");
    }

    println!("cargo:rerun-if-changed=native/ngx.cpp");

    let manifest_dir = PathBuf::from(
        env::var("CARGO_MANIFEST_DIR")
            .map_err(|error| build_error(format!("reading CARGO_MANIFEST_DIR: {error}")))?,
    );
    let sdk = nvidia_sdk::Sdk::Dlss.resolve(&nvidia_sdk::ResolveOptions::for_cargo()?)?;
    let sdk_dir = sdk.path.canonicalize().map_err(|error| {
        build_error(format!(
            "resolving dlss sdk directory {}: {error}",
            sdk.path.display()
        ))
    })?;
    let include_dir = sdk_dir.join("include");
    let library_dir = sdk_dir.join("lib/Linux_x86_64");

    for header in [
        "nvsdk_ngx.h",
        "nvsdk_ngx_defs.h",
        "nvsdk_ngx_defs_dlssd.h",
        "nvsdk_ngx_helpers.h",
        "nvsdk_ngx_helpers_dlssd.h",
        "nvsdk_ngx_helpers_dlssd_vk.h",
        "nvsdk_ngx_helpers_vk.h",
        "nvsdk_ngx_params.h",
        "nvsdk_ngx_params_dlssd.h",
        "nvsdk_ngx_vk.h",
    ] {
        require_file(&include_dir.join(header), "dlss sdk header", None)?;
    }

    let static_library = library_dir.join("libnvsdk_ngx.a");
    require_file(&static_library, "dlss sdk static library", Some(1024))?;

    let runtime_profile = if env::var_os("CARGO_FEATURE_DLSS_RELEASE_RUNTIME").is_some() {
        "rel"
    } else {
        "dev"
    };
    let runtime = library_dir.join(runtime_profile).join(RUNTIME_FILE);
    require_file(&runtime, "dlssd runtime", Some(1024))?;

    let license = sdk_dir.join("LICENSE.txt");
    require_file(&license, "dlss sdk license", None)?;
    let shared_notice = manifest_dir.join("DLSS-THIRD-PARTY-NOTICES.txt");
    require_file(&shared_notice, "dlss shared third-party notice", None)?;

    cc::Build::new()
        .cpp(true)
        .file(manifest_dir.join("native/ngx.cpp"))
        .include(&include_dir)
        .flag_if_supported("-std=c++17")
        .flag_if_supported("-Wno-missing-field-initializers")
        .compile("nvidia_dlss_ngx_bridge");
    let output_dir = PathBuf::from(
        env::var("OUT_DIR").map_err(|error| build_error(format!("reading OUT_DIR: {error}")))?,
    );
    let native_dir = output_dir.join("native");
    fs::create_dir_all(&native_dir).map_err(|error| {
        build_error(format!(
            "creating dlss native library directory {}: {error}",
            native_dir.display()
        ))
    })?;
    stage_file(&static_library, &native_dir.join("libnvsdk_ngx.a"))?;

    println!("cargo:rustc-link-search=native={}", native_dir.display());
    println!("cargo:rustc-link-lib=static=nvsdk_ngx");
    println!("cargo:rustc-link-lib=dylib=dl");

    let runtime_dir = output_dir.join("runtime");
    fs::create_dir_all(&runtime_dir).map_err(|error| {
        build_error(format!(
            "creating dlss runtime directory {}: {error}",
            runtime_dir.display()
        ))
    })?;
    stage_file(&runtime, &runtime_dir.join(RUNTIME_FILE))?;
    stage_file(&license, &runtime_dir.join("nvidia-rtx-sdk-license.txt"))?;
    stage_file(
        &shared_notice,
        &runtime_dir.join("dlss-third-party-notices.txt"),
    )?;

    println!(
        "cargo:rustc-env=NVIDIA_DLSS_RUNTIME_DIR={}",
        runtime_dir.display()
    );
    println!("cargo:rustc-cfg=nvidia_dlss_native");

    Ok(())
}

fn require_file(path: &Path, description: &str, minimum_size: Option<u64>) -> io::Result<()> {
    let metadata = fs::metadata(path).map_err(|error| {
        build_error(format!(
            "{description} is missing at {}: {error}",
            path.display()
        ))
    })?;

    if !metadata.is_file() {
        return Err(build_error(format!(
            "{description} is not a file: {}",
            path.display()
        )));
    }

    if metadata.len() == 0 {
        return Err(build_error(format!(
            "{description} is empty: {}",
            path.display()
        )));
    }

    let mut prefix = [0_u8; 128];
    let mut file = File::open(path).map_err(|error| {
        build_error(format!("opening {description} {}: {error}", path.display()))
    })?;
    let count = file.read(&mut prefix).map_err(|error| {
        build_error(format!("reading {description} {}: {error}", path.display()))
    })?;

    if prefix[..count].starts_with(LFS_POINTER_PREFIX) {
        return Err(build_error(format!(
            "{description} is a git lfs pointer; fetch lfs content for {}",
            path.display()
        )));
    }

    if minimum_size.is_some_and(|minimum| metadata.len() < minimum) {
        return Err(build_error(format!(
            "{description} is incomplete: {} ({} bytes)",
            path.display(),
            metadata.len()
        )));
    }

    println!("cargo:rerun-if-changed={}", path.display());

    Ok(())
}

fn stage_file(source: &Path, destination: &Path) -> io::Result<()> {
    if destination.exists() {
        fs::remove_file(destination).map_err(|error| {
            build_error(format!(
                "removing staged dlss file {}: {error}",
                destination.display()
            ))
        })?;
    }

    fs::copy(source, destination).map_err(|error| {
        build_error(format!(
            "staging dlss file {} as {}: {error}",
            source.display(),
            destination.display()
        ))
    })?;
    Ok(())
}

fn build_error(message: impl Into<String>) -> io::Error {
    io::Error::other(message.into())
}
