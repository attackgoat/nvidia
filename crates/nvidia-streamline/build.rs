use std::{
    env,
    error::Error,
    fs::{self, File},
    io::{self, Read as _},
    path::{Path, PathBuf},
};

const HEADERS: [&str; 12] = [
    "sl.h",
    "sl_appidentity.h",
    "sl_consts.h",
    "sl_core_api.h",
    "sl_core_types.h",
    "sl_device_wrappers.h",
    "sl_dlss.h",
    "sl_dlss_d.h",
    "sl_result.h",
    "sl_security.h",
    "sl_struct.h",
    "sl_version.h",
];
const RUNTIME_FILES: [&str; 6] = [
    "sl.interposer.dll",
    "sl.common.dll",
    "sl.pcl.dll",
    "sl.dlss_d.dll",
    "NvLowLatencyVk.dll",
    "nvngx_dlssd.dll",
];
const TARGET: &str = "x86_64-pc-windows-msvc";

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rustc-check-cfg=cfg(nvidia_streamline_native)");
    println!("cargo:rerun-if-env-changed=DOCS_RS");

    if env::var_os("DOCS_RS").is_some() {
        return Ok(());
    }

    println!("cargo:rerun-if-env-changed=STREAMLINE_SDK");
    println!("cargo:rerun-if-changed=native/streamline.cpp");

    if !is_native_target() {
        return Ok(());
    }

    build_native()
}

fn is_native_target() -> bool {
    env::var("TARGET").as_deref() == Ok(TARGET)
        && env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("x86_64")
        && env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
}

fn build_native() -> Result<(), Box<dyn Error>> {
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

    let manifest_dir = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR")
            .ok_or_else(|| io::Error::other("cargo did not provide CARGO_MANIFEST_DIR"))?,
    );
    let sdk_dir = sdk_dir()?;
    let include_dir = sdk_dir.join("include");
    let bin_dir = sdk_dir.join("bin/x64");

    for header in HEADERS {
        let path = include_dir.join(header);
        require_complete_file(&path, 160, "streamline 2.9.0 header")?;

        rerun_if_changed(&path);
    }

    validate_version(&include_dir.join("sl_version.h"))?;

    let runtime_sources = RUNTIME_FILES.map(|name| bin_dir.join(name));

    for path in &runtime_sources {
        require_complete_file(path, 4096, "streamline 2.9.0 runtime")?;

        rerun_if_changed(path);
    }

    let notice_sources = [
        (sdk_dir.join("license.txt"), "streamline-license.txt"),
        (
            sdk_dir.join("3rd-party-licenses.md"),
            "streamline-3rd-party-licenses.md",
        ),
        (
            bin_dir.join("nvngx_dlss.license.txt"),
            "nvngx_dlss.license.txt",
        ),
        (bin_dir.join("reflex.license.txt"), "reflex.license.txt"),
        (
            manifest_dir.join("DLSS-THIRD-PARTY-NOTICES.txt"),
            "dlss-third-party-notices.txt",
        ),
    ];

    for (path, _) in &notice_sources {
        require_complete_file(path, 256, "streamline runtime notice")?;

        rerun_if_changed(path);
    }

    cc::Build::new()
        .cpp(true)
        .file(manifest_dir.join("native/streamline.cpp"))
        .include(&include_dir)
        .flag_if_supported("/std:c++17")
        .flag_if_supported("/EHsc")
        .compile("nvidia_streamline_native");

    let runtime_dir = PathBuf::from(
        env::var_os("OUT_DIR").ok_or_else(|| io::Error::other("cargo did not provide OUT_DIR"))?,
    )
    .join("streamline");
    prepare_directory(&runtime_dir)?;

    for (source, name) in runtime_sources.iter().zip(RUNTIME_FILES) {
        copy(source, &runtime_dir.join(name))?;
    }

    for (source, name) in notice_sources {
        copy(&source, &runtime_dir.join(name))?;
    }

    let runtime_dir = runtime_dir.to_str().ok_or_else(|| {
        io::Error::other(format!(
            "streamline runtime path is not valid unicode: {}",
            runtime_dir.display()
        ))
    })?;

    println!("cargo:rustc-cfg=nvidia_streamline_native");
    println!("cargo:rustc-env=NVIDIA_STREAMLINE_RUNTIME_DIR={runtime_dir}");

    Ok(())
}

fn sdk_dir() -> Result<PathBuf, Box<dyn Error>> {
    let sdk = nvidia_sdk::Sdk::Streamline.resolve(&nvidia_sdk::ResolveOptions::for_cargo()?)?;
    Ok(dunce::canonicalize(&sdk.path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "canonicalizing streamline 2.9.0 sdk directory {}: {error}",
                sdk.path.display()
            ),
        )
    })?)
}

fn require_complete_file(path: &Path, minimum_size: u64, description: &str) -> io::Result<()> {
    let metadata = fs::metadata(path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "reading metadata for {description} {}: {error}",
                path.display()
            ),
        )
    })?;

    if !metadata.is_file() {
        return Err(io::Error::other(format!(
            "{description} is not a file: {}",
            path.display()
        )));
    }

    let mut file = File::open(path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("opening {description} {}: {error}", path.display()),
        )
    })?;
    let mut prefix = [0_u8; 128];
    let prefix_len = file.read(&mut prefix).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("reading {description} {}: {error}", path.display()),
        )
    })?;

    if prefix[..prefix_len].starts_with(b"version https://git-lfs.github.com/spec/v1") {
        return Err(io::Error::other(format!(
            "{description} is a git lfs pointer; fetch the sdk file: {}",
            path.display()
        )));
    }

    if metadata.len() < minimum_size {
        return Err(io::Error::other(format!(
            "{description} is incomplete ({} bytes, expected at least {minimum_size}): {}",
            metadata.len(),
            path.display()
        )));
    }

    Ok(())
}

fn validate_version(path: &Path) -> io::Result<()> {
    let contents = fs::read_to_string(path).map_err(|error| {
        io::Error::new(error.kind(), format!("reading {}: {error}", path.display()))
    })?;
    let version = (
        cpp_define(&contents, "SL_VERSION_MAJOR")?,
        cpp_define(&contents, "SL_VERSION_MINOR")?,
        cpp_define(&contents, "SL_VERSION_PATCH")?,
    );

    if version != (2, 9, 0) {
        return Err(io::Error::other(format!(
            "STREAMLINE_SDK must point to streamline 2.9.0, found {}.{}.{}",
            version.0, version.1, version.2
        )));
    }

    Ok(())
}

fn cpp_define(contents: &str, name: &str) -> io::Result<u32> {
    contents
        .lines()
        .find_map(|line| {
            let mut fields = line.split_whitespace();
            (fields.next() == Some("#define") && fields.next() == Some(name))
                .then(|| fields.next())
                .flatten()
        })
        .ok_or_else(|| io::Error::other(format!("missing {name} in sl_version.h")))?
        .parse()
        .map_err(|error| io::Error::other(format!("parsing {name} in sl_version.h: {error}")))
}

fn prepare_directory(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(io::Error::other(format!(
                "refusing to replace streamline runtime directory symlink {}",
                path.display()
            )));
        }
        Ok(metadata) if metadata.is_dir() => fs::remove_dir_all(path).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("removing runtime directory {}: {error}", path.display()),
            )
        })?,
        Ok(_) => fs::remove_file(path).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("removing runtime file {}: {error}", path.display()),
            )
        })?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(io::Error::new(
                error.kind(),
                format!(
                    "reading runtime directory metadata {}: {error}",
                    path.display()
                ),
            ));
        }
    }

    fs::create_dir_all(path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("creating runtime directory {}: {error}", path.display()),
        )
    })
}

fn copy(source: &Path, destination: &Path) -> io::Result<()> {
    fs::copy(source, destination).map(|_| ()).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "copying {} to {}: {error}",
                source.display(),
                destination.display()
            ),
        )
    })
}

fn rerun_if_changed(path: &Path) {
    println!("cargo:rerun-if-changed={}", path.display());
}
