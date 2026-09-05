#![forbid(unsafe_code)]

use std::{fs, path::Path, process::Command};

const SHA256: &str = "2fce446623df71f6cb6217da300422a0b4cfb7d7dca92d8111708dc030d60b4c";

fn resolver(cache: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_nvidia-sdk"));

    for name in [
        "SHARC_SDK",
        "NVIDIA_SDK_ROOT",
        "NVIDIA_SDK_OFFLINE",
        "CARGO_NET_OFFLINE",
    ] {
        command.env_remove(name);
    }

    command.env("NVIDIA_SDK_CACHE", cache).args([
        "path",
        "sharc",
        "--target",
        "wasm32-unknown-unknown",
        "--json",
    ]);

    command
}

#[test]
fn offline_sharc_override_root_cache_and_miss() {
    let temporary = tempfile::tempdir().unwrap();
    let cache = temporary.path().join("cache");

    for flag in ["NVIDIA_SDK_OFFLINE", "CARGO_NET_OFFLINE"] {
        let output = resolver(&cache).env(flag, "true").output().unwrap();

        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("downloads are disabled"));
        assert!(!cache.exists());
    }

    let root = temporary.path().join("root");
    let sdk = root.join("sharc/1.8.3");
    fs::create_dir_all(sdk.join("include")).unwrap();

    for relative in [
        "include/HashGridCommon.h",
        "include/HashGridTypes.h",
        "include/SharcCommon.h",
        "include/SharcGlslHelpers.h",
        "include/SharcTypes.h",
        "License.md",
    ] {
        fs::write(sdk.join(relative), "trusted local fixture").unwrap();
    }

    for (name, path) in [("SHARC_SDK", &sdk), ("NVIDIA_SDK_ROOT", &root)] {
        let output = resolver(&cache)
            .env(name, path)
            .arg("--offline")
            .output()
            .unwrap();

        assert!(output.status.success(), "{output:?}");

        let resolved: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

        assert_eq!(resolved["path"], sdk.to_str().unwrap());
        assert!(resolved["source_url"].is_null());
        assert!(!cache.exists());
    }

    let cached = cache.join("sharc/1.8.3").join(SHA256);
    fs::create_dir_all(cached.parent().unwrap()).unwrap();
    fs::rename(&sdk, &cached).unwrap();
    let output = resolver(&cache).arg("--offline").output().unwrap();

    assert!(output.status.success(), "{output:?}");

    let resolved: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(resolved["path"], cached.to_str().unwrap());
    assert!(resolved["source_url"].is_string());

    // Invalid explicit inputs must fail instead of falling back to a valid cache.
    let output = resolver(&cache)
        .env("SHARC_SDK", cached.join("include"))
        .arg("--offline")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("SHARC_SDK points to an invalid"));

    fs::remove_file(cached.join("License.md")).unwrap();

    assert!(
        !resolver(&cache)
            .arg("--offline")
            .output()
            .unwrap()
            .status
            .success()
    );
}

#[test]
#[ignore = "downloads the official sharc archive into a fresh temporary cache"]
fn fresh_sharc_acquisition_then_offline_reuse() {
    let temporary = tempfile::tempdir().unwrap();
    let cache = temporary.path().join("cache");
    let output = resolver(&cache).output().unwrap();

    assert!(output.status.success(), "{output:?}");

    let resolved: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let sdk = cache.join("sharc/1.8.3").join(SHA256);

    assert_eq!(resolved["path"], sdk.to_str().unwrap());
    assert_eq!(resolved["version"], "1.8.3");
    assert!(sdk.join("include/SharcCommon.h").is_file());
    assert!(sdk.join("License.md").is_file());

    for target in [
        "x86_64-pc-windows-msvc",
        "aarch64-apple-darwin",
        "future-target",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_nvidia-sdk"))
            .env_remove("SHARC_SDK")
            .env_remove("NVIDIA_SDK_ROOT")
            .env("NVIDIA_SDK_CACHE", &cache)
            .args(["path", "sharc", "--target", target, "--offline", "--json"])
            .output()
            .unwrap();

        assert!(output.status.success(), "{output:?}");

        let reused: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

        assert_eq!(reused["path"], resolved["path"]);
    }

    let output = resolver(&temporary.path().join("unused-cache"))
        .env("SHARC_SDK", &sdk)
        .arg("--offline")
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
}
