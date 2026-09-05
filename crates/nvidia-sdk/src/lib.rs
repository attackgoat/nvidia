//! Verified acquisition and runtime staging for pinned NVIDIA SDK releases.

#![forbid(unsafe_code)]

use {
    anyhow::{Context as _, Result, bail, ensure},
    fs2::FileExt as _,
    serde::{Deserialize, Serialize},
    sha2::{Digest as _, Sha256},
    std::{
        env,
        ffi::OsStr,
        fs::{self, File, OpenOptions},
        io::{self, Read as _, Write as _},
        path::{Component, Path, PathBuf},
        str::FromStr,
    },
    zip::ZipArchive,
};

const LOCK_MANIFEST: &str = include_str!("../sdk-lock.toml");

fn cache_dir() -> Result<PathBuf> {
    if let Some(path) = env::var_os("NVIDIA_SDK_CACHE").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(path));
    }

    if cfg!(windows) {
        return env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|path| path.join("nvidia-sdk"))
            .context("set NVIDIA_SDK_CACHE or LOCALAPPDATA");
    }

    if let Some(path) = env::var_os("XDG_CACHE_HOME").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(path).join("nvidia-sdk"));
    }

    env::var_os("HOME")
        .map(PathBuf::from)
        .map(|path| path.join(".cache/nvidia-sdk"))
        .context("set NVIDIA_SDK_CACHE or HOME")
}

fn download(url: &str, destination: &Path) -> Result<()> {
    let response = ureq::get(url)
        .set(
            "User-Agent",
            concat!("nvidia-sdk/", env!("CARGO_PKG_VERSION")),
        )
        .call()
        .with_context(|| format!("downloading {url}"))?;
    let mut source = response.into_reader();
    let mut destination_file = File::create(destination)
        .with_context(|| format!("creating download file {}", destination.display()))?;
    io::copy(&mut source, &mut destination_file)
        .with_context(|| format!("downloading {url} to {}", destination.display()))?;
    destination_file
        .flush()
        .with_context(|| format!("flushing download file {}", destination.display()))?;

    Ok(())
}

fn extract_zip(archive_path: &Path, output: &Path) -> Result<()> {
    fs::create_dir_all(output)
        .with_context(|| format!("creating extraction directory {}", output.display()))?;
    let file = File::open(archive_path)
        .with_context(|| format!("opening archive {}", archive_path.display()))?;
    let mut archive = ZipArchive::new(file)
        .with_context(|| format!("reading zip archive {}", archive_path.display()))?;

    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).with_context(|| {
            format!(
                "reading entry {index} in archive {}",
                archive_path.display()
            )
        })?;
        let relative = entry.enclosed_name().with_context(|| {
            format!(
                "unsafe path in entry {index} of archive {}",
                archive_path.display()
            )
        })?;
        safe_relative_path(&relative).with_context(|| {
            format!(
                "validating entry {index} in archive {}",
                archive_path.display()
            )
        })?;
        let destination = output.join(relative);

        if entry.is_dir() {
            fs::create_dir_all(&destination)
                .with_context(|| format!("creating archive directory {}", destination.display()))?;

            continue;
        }

        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating extraction parent {}", parent.display()))?;
        }

        let mut file = File::create(&destination)
            .with_context(|| format!("creating extracted file {}", destination.display()))?;
        io::copy(&mut entry, &mut file).with_context(|| {
            format!(
                "extracting entry {index} from {} to {}",
                archive_path.display(),
                destination.display()
            )
        })?;
    }

    Ok(())
}

fn verify_hash(path: &Path, expected: &str) -> Result<()> {
    let mut file = File::open(path)
        .with_context(|| format!("opening archive for hashing {}", path.display()))?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher)
        .with_context(|| format!("hashing archive {}", path.display()))?;
    let actual = format!("{:x}", hasher.finalize());

    ensure!(
        actual == expected,
        "archive hash mismatch for {}: expected {expected}, got {actual}",
        path.display()
    );

    Ok(())
}

fn remove_path(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || metadata.is_file() => {
            fs::remove_file(path).with_context(|| format!("removing file {}", path.display()))?;
        }
        Ok(metadata) if metadata.is_dir() => fs::remove_dir_all(path)
            .with_context(|| format!("removing directory {}", path.display()))?,
        Ok(_) => bail!("unsupported filesystem object at {}", path.display()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("reading metadata for {}", path.display()));
        }
    }

    Ok(())
}

fn safe_relative_path(path: &Path) -> Result<()> {
    ensure!(!path.as_os_str().is_empty(), "empty relative path");

    ensure!(
        !path.is_absolute(),
        "absolute path is not allowed: {}",
        path.display()
    );

    ensure!(
        path.components()
            .all(|component| matches!(component, Component::Normal(_))),
        "unsafe relative path: {}",
        path.display()
    );

    Ok(())
}

#[derive(Debug, Deserialize, Serialize)]
struct Artifact {
    assembly: Option<SourceAssembly>,
    id: String,
    required: Vec<PathBuf>,
    runtime: Option<Vec<RuntimeFile>>,
    sha256: String,
    targets: Vec<String>,
    url: String,
}

impl Artifact {
    fn acquire(&self, path: &Path, sdk: Sdk, entry: &SdkEntry) -> Result<()> {
        ensure!(
            self.url.starts_with("https://")
                && self.sha256.len() == 64
                && self.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "invalid acquisition metadata for {} {}",
            sdk.name(),
            self.id
        );

        let parent = path
            .parent()
            .with_context(|| format!("sdk cache path has no parent: {}", path.display()))?;
        fs::create_dir_all(parent)
            .with_context(|| format!("creating cache directory {}", parent.display()))?;
        let key = self.cache_key()?;
        let lock_path = parent.join(format!("{key}.lock"));
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .with_context(|| format!("opening cache lock {}", lock_path.display()))?;
        lock.lock_exclusive()
            .with_context(|| format!("locking cache file {}", lock_path.display()))?;

        if self.validate_cached(path).is_ok() {
            return Ok(());
        }

        eprintln!(
            "acquiring nvidia {} sdk {} from {}\nlicense: {} ({})",
            sdk.name(),
            entry.version,
            self.url,
            entry.license,
            entry.license_url
        );

        let temporary = parent.join(format!(".{key}.{}.tmp", std::process::id()));
        remove_path(&temporary)?;
        fs::create_dir_all(&temporary)
            .with_context(|| format!("creating staging directory {}", temporary.display()))?;
        let result = (|| -> Result<()> {
            let archive_path = temporary.join("archive.zip");
            download(&self.url, &archive_path)?;
            verify_hash(&archive_path, &self.sha256)?;
            let extracted = temporary.join("extracted");
            extract_zip(&archive_path, &extracted)?;
            let root = if let Some(assembly) = &self.assembly {
                safe_relative_path(&assembly.root)?;
                let root = extracted.join(&assembly.root);

                ensure!(
                    root.is_dir(),
                    "missing source archive root {}",
                    root.display()
                );

                assembly.prepare(&root, &temporary)?;

                root
            } else {
                self.locate_root(&extracted)?
            };
            self.validate(&root)?;

            if self.assembly.is_some() {
                let stamp = root.join(".nvidia-sdk-source");
                fs::write(&stamp, &key).with_context(|| {
                    format!("writing source assembly stamp {}", stamp.display())
                })?;
            }

            remove_path(path)?;
            fs::rename(&root, path).with_context(|| {
                format!(
                    "installing downloaded sdk from {} to {}",
                    root.display(),
                    path.display()
                )
            })?;

            Ok(())
        })();
        let cleanup = remove_path(&temporary);
        result?;

        cleanup
    }

    fn cache_key(&self) -> Result<String> {
        // Source recipes, dependency pins, and patches are part of the cache identity.
        if self.assembly.is_some() {
            Ok(format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(self).with_context(|| format!(
                    "serializing cache identity for artifact {}",
                    self.id
                ))?)
            ))
        } else {
            Ok(self.sha256.clone())
        }
    }

    fn validate_cached(&self, path: &Path) -> Result<()> {
        self.validate(path)?;

        if self.assembly.is_some() {
            let stamp = path.join(".nvidia-sdk-source");

            ensure!(
                fs::read_to_string(&stamp).with_context(|| format!(
                    "reading source assembly stamp {}",
                    stamp.display()
                ))? == self.cache_key()?,
                "source sdk assembly is incomplete or stale: {}",
                stamp.display()
            );
        }

        Ok(())
    }

    fn locate_root(&self, extracted: &Path) -> Result<PathBuf> {
        if self.validate(extracted).is_ok() {
            return Ok(extracted.to_owned());
        }

        let mut candidates = fs::read_dir(extracted)
            .with_context(|| format!("reading extracted directory {}", extracted.display()))?
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .map(|entry| entry.path())
            .filter(|path| self.validate(path).is_ok());
        let root = candidates.next().with_context(|| {
            format!(
                "archive does not contain the expected sdk layout: {}",
                extracted.display()
            )
        })?;

        ensure!(
            candidates.next().is_none(),
            "archive contains multiple possible sdk roots: {}",
            extracted.display()
        );

        Ok(root)
    }

    fn supports(&self, target: &str) -> bool {
        self.targets
            .iter()
            .any(|candidate| candidate == "*" || candidate == target)
    }

    fn validate(&self, path: &Path) -> Result<()> {
        ensure!(
            path.is_dir(),
            "sdk directory is missing: {}",
            path.display()
        );

        for relative in &self.required {
            safe_relative_path(relative)?;
            let required = path.join(relative);
            let metadata = fs::metadata(&required).with_context(|| {
                format!(
                    "reading metadata for required sdk file {}",
                    required.display()
                )
            })?;

            ensure!(
                metadata.is_file(),
                "sdk input is not a file: {}",
                required.display()
            );

            ensure!(
                metadata.len() > 0,
                "sdk input is empty: {}",
                required.display()
            );

            let mut prefix = [0_u8; 128];
            let count = File::open(&required)
                .with_context(|| format!("opening required sdk file {}", required.display()))?
                .read(&mut prefix)
                .with_context(|| format!("reading required sdk file {}", required.display()))?;

            ensure!(
                !prefix[..count].starts_with(b"version https://git-lfs.github.com/spec/v1"),
                "sdk input is a git lfs pointer: {}",
                required.display()
            );
        }

        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct LockManifest {
    schema: u32,
    sdks: SdkEntries,
}

impl LockManifest {
    fn load() -> Result<Self> {
        let manifest: Self = toml::from_str(LOCK_MANIFEST).context("parsing sdk-lock.toml")?;

        ensure!(manifest.schema == 1, "unsupported sdk lock schema");

        Ok(manifest)
    }
}

/// A validated SDK tree and its pinned provenance.
#[derive(Clone, Debug, Serialize)]
pub struct ResolvedSdk {
    pub artifact: String,
    pub license: String,
    pub license_url: String,
    pub path: PathBuf,
    pub sdk: Sdk,
    pub source_url: Option<String>,
    pub target: String,
    pub version: String,
}

impl ResolvedSdk {
    fn new(
        sdk: Sdk,
        target: &str,
        entry: &SdkEntry,
        artifact: &Artifact,
        path: PathBuf,
        source_url: Option<String>,
    ) -> Self {
        Self {
            artifact: artifact.id.clone(),
            license: entry.license.clone(),
            license_url: entry.license_url.clone(),
            path,
            sdk,
            source_url,
            target: target.to_owned(),
            version: entry.version.clone(),
        }
    }

    /// Copies this SDK's selected redistributable runtime set into `output`.
    ///
    /// # Errors
    ///
    /// Returns an error if the resolved artifact has no runtime manifest or a
    /// required file cannot be copied safely.
    pub fn stage_runtime(&self, output: &Path, profile: RuntimeProfile) -> Result<Vec<StagedFile>> {
        let manifest = LockManifest::load()?;
        let entry = manifest.sdks.get(self.sdk);
        let artifact = entry
            .artifacts
            .iter()
            .find(|artifact| artifact.id == self.artifact)
            .context("resolved artifact is absent from the embedded lock manifest")?;
        let runtime = artifact
            .runtime
            .as_deref()
            .context("sdk artifact does not define a redistributable runtime")?;
        fs::create_dir_all(output)
            .with_context(|| format!("creating runtime directory {}", output.display()))?;

        let mut staged = Vec::new();

        for file in runtime.iter().filter(|file| {
            file.profile
                .as_deref()
                .is_none_or(|candidate| candidate == profile.name())
        }) {
            safe_relative_path(&file.source)?;
            safe_relative_path(&file.destination)?;
            let source = self.path.join(&file.source);
            let destination = output.join(&file.destination);

            ensure!(
                source.is_file(),
                "missing runtime file {}",
                source.display()
            );

            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("creating runtime parent {}", parent.display()))?;
            }

            fs::copy(&source, &destination).with_context(|| {
                format!(
                    "copying runtime file {} to {}",
                    source.display(),
                    destination.display()
                )
            })?;
            staged.push(StagedFile {
                path: destination,
                source,
            });
        }

        Ok(staged)
    }
}

/// Resolver settings used by build scripts and packaging tools.
#[derive(Clone, Debug)]
pub struct ResolveOptions {
    pub cache_dir: Option<PathBuf>,
    pub offline: bool,
    pub target: String,
}

impl ResolveOptions {
    fn environment_flag(name: &str) -> bool {
        env::var_os(name).is_some_and(|value| {
            !value.is_empty() && value != OsStr::new("0") && value != OsStr::new("false")
        })
    }

    /// Constructs options for a Cargo build script.
    ///
    /// # Errors
    ///
    /// Returns an error when Cargo did not provide the target triple.
    pub fn for_cargo() -> Result<Self> {
        let target = env::var("TARGET").context("cargo did not provide TARGET")?;

        Ok(Self {
            cache_dir: None,
            offline: Self::environment_flag("NVIDIA_SDK_OFFLINE")
                || Self::environment_flag("CARGO_NET_OFFLINE"),
            target,
        })
    }

    #[must_use]
    pub fn for_target(target: impl Into<String>) -> Self {
        Self {
            cache_dir: None,
            offline: Self::environment_flag("NVIDIA_SDK_OFFLINE")
                || Self::environment_flag("CARGO_NET_OFFLINE"),
            target: target.into(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct RuntimeFile {
    destination: PathBuf,
    profile: Option<String>,
    source: PathBuf,
}

/// Runtime flavor selected from an SDK archive.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RuntimeProfile {
    #[default]
    Release,
    Development,
}

impl RuntimeProfile {
    const fn name(self) -> &'static str {
        match self {
            Self::Release => "release",
            Self::Development => "development",
        }
    }
}

impl FromStr for RuntimeProfile {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "release" => Ok(Self::Release),
            "development" => Ok(Self::Development),
            _ => bail!("unknown runtime profile {value:?}"),
        }
    }
}

/// A supported NVIDIA SDK integration.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Sdk {
    Dlss,
    Nrd,
    Omm,
    Sharc,
    Streamline,
}

impl Sdk {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Dlss => "dlss",
            Self::Nrd => "nrd",
            Self::Omm => "omm",
            Self::Sharc => "sharc",
            Self::Streamline => "streamline",
        }
    }

    /// Resolves and validates this SDK for one target.
    ///
    /// # Errors
    ///
    /// Returns an error if the target is unsupported, a configured SDK is invalid,
    /// or a required archive cannot be securely acquired and validated.
    pub fn resolve(self, options: &ResolveOptions) -> Result<ResolvedSdk> {
        let manifest = LockManifest::load()?;
        let entry = manifest.sdks.get(self);
        let artifact = entry
            .artifacts
            .iter()
            .find(|artifact| artifact.supports(&options.target))
            .with_context(|| {
                format!(
                    "{} {} has no pinned artifact for {}",
                    self.name(),
                    entry.version,
                    options.target
                )
            })?;

        if let Some(path) = env::var_os(&entry.environment).filter(|value| !value.is_empty()) {
            let path = PathBuf::from(path);
            artifact.validate(&path).with_context(|| {
                format!(
                    "{} points to an invalid {} {} sdk",
                    entry.environment,
                    self.name(),
                    entry.version
                )
            })?;

            return Ok(ResolvedSdk::new(
                self,
                &options.target,
                entry,
                artifact,
                path,
                None,
            ));
        }

        if let Some(root) = env::var_os("NVIDIA_SDK_ROOT").filter(|value| !value.is_empty()) {
            let path = PathBuf::from(root).join(self.name()).join(&entry.version);

            if path.exists() {
                artifact.validate(&path)?;

                return Ok(ResolvedSdk::new(
                    self,
                    &options.target,
                    entry,
                    artifact,
                    path,
                    None,
                ));
            }
        }

        let default_cache;
        let cache = if let Some(cache) = &options.cache_dir {
            cache
        } else {
            default_cache = cache_dir()?;
            &default_cache
        };
        let path = cache
            .join(self.name())
            .join(&entry.version)
            .join(artifact.cache_key()?);

        if artifact.validate_cached(&path).is_ok() {
            return Ok(ResolvedSdk::new(
                self,
                &options.target,
                entry,
                artifact,
                path,
                Some(artifact.url.clone()),
            ));
        }

        ensure!(
            !options.offline,
            "{} {} is not cached and downloads are disabled",
            self.name(),
            entry.version
        );

        artifact.acquire(&path, self, entry)?;

        Ok(ResolvedSdk::new(
            self,
            &options.target,
            entry,
            artifact,
            path,
            Some(artifact.url.clone()),
        ))
    }
}

impl FromStr for Sdk {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "dlss" => Ok(Self::Dlss),
            "nrd" => Ok(Self::Nrd),
            "omm" => Ok(Self::Omm),
            "sharc" => Ok(Self::Sharc),
            "streamline" => Ok(Self::Streamline),
            _ => bail!("unknown nvidia sdk {value:?}"),
        }
    }
}

#[derive(Debug, Deserialize)]
struct SdkEntries {
    dlss: SdkEntry,
    nrd: SdkEntry,
    omm: SdkEntry,
    sharc: SdkEntry,
    streamline: SdkEntry,
}

impl SdkEntries {
    fn get(&self, sdk: Sdk) -> &SdkEntry {
        match sdk {
            Sdk::Dlss => &self.dlss,
            Sdk::Nrd => &self.nrd,
            Sdk::Omm => &self.omm,
            Sdk::Sharc => &self.sharc,
            Sdk::Streamline => &self.streamline,
        }
    }
}

#[derive(Debug, Deserialize)]
struct SdkEntry {
    artifacts: Vec<Artifact>,
    environment: String,
    license: String,
    license_url: String,
    version: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct SourceAssembly {
    dependencies: Vec<SourceDependency>,
    #[serde(default)]
    patches: Vec<SourcePatch>,
    #[serde(default)]
    relocations: Vec<SourceRelocation>,
    root: PathBuf,
}

impl SourceAssembly {
    fn prepare(&self, root: &Path, temporary: &Path) -> Result<()> {
        for relocation in &self.relocations {
            relocation.apply(root, root)?;
        }

        for (index, dependency) in self.dependencies.iter().enumerate() {
            ensure!(
                dependency.url.starts_with("https://")
                    && dependency.sha256.len() == 64
                    && dependency
                        .sha256
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit()),
                "invalid source dependency acquisition metadata"
            );

            let archive = temporary.join(format!("dependency-{index}.zip"));
            download(&dependency.url, &archive)?;
            verify_hash(&archive, &dependency.sha256)?;
            let extracted = temporary.join(format!("dependency-{index}"));
            extract_zip(&archive, &extracted)?;

            if let Some(files) = &dependency.files {
                safe_relative_path(&dependency.root)?;
                safe_relative_path(&dependency.destination)?;
                let source = extracted.join(&dependency.root);
                let destination = root.join(&dependency.destination);

                ensure!(
                    !destination.exists(),
                    "source dependency destination already exists: {}",
                    destination.display()
                );

                ensure!(!files.is_empty(), "empty source dependency file selection");

                for relative in files {
                    safe_relative_path(relative)?;
                    let output = destination.join(relative);
                    let parent = output.parent().with_context(|| {
                        format!("dependency path has no parent: {}", output.display())
                    })?;
                    fs::create_dir_all(parent).with_context(|| {
                        format!("creating dependency directory {}", parent.display())
                    })?;
                    let input = source.join(relative);
                    fs::copy(&input, &output).with_context(|| {
                        format!(
                            "copying source dependency {} to {}",
                            input.display(),
                            output.display()
                        )
                    })?;
                }
            } else {
                SourceRelocation {
                    source: dependency.root.clone(),
                    destination: dependency.destination.clone(),
                }
                .apply(&extracted, root)?;
            }

            remove_path(&extracted)?;
            fs::remove_file(&archive)
                .with_context(|| format!("removing dependency archive {}", archive.display()))?;
        }

        for patch in &self.patches {
            patch.apply(root)?;
        }

        Ok(())
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct SourceDependency {
    destination: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    files: Option<Vec<PathBuf>>,
    root: PathBuf,
    sha256: String,
    url: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct SourcePatch {
    after: String,
    before: String,
    path: PathBuf,
}

impl SourcePatch {
    fn apply(&self, root: &Path) -> Result<()> {
        safe_relative_path(&self.path)?;
        let path = root.join(&self.path);
        let source = fs::read_to_string(&path)
            .with_context(|| format!("reading source patch input {}", path.display()))?;

        ensure!(
            !self.before.is_empty() && source.matches(&self.before).count() == 1,
            "source patch preimage does not match exactly once: {}",
            path.display()
        );

        fs::write(&path, source.replacen(&self.before, &self.after, 1))
            .with_context(|| format!("writing patched source {}", path.display()))?;

        Ok(())
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct SourceRelocation {
    destination: PathBuf,
    source: PathBuf,
}

impl SourceRelocation {
    fn apply(&self, source_root: &Path, destination_root: &Path) -> Result<()> {
        safe_relative_path(&self.source)?;
        safe_relative_path(&self.destination)?;
        let source = source_root.join(&self.source);
        let destination = destination_root.join(&self.destination);

        ensure!(
            !destination.exists(),
            "source assembly destination already exists: {}",
            destination.display()
        );

        let parent = destination
            .parent()
            .with_context(|| format!("assembly path has no parent: {}", destination.display()))?;
        fs::create_dir_all(parent)
            .with_context(|| format!("creating assembly directory {}", parent.display()))?;
        fs::rename(&source, &destination).with_context(|| {
            format!(
                "assembling {} into {}",
                source.display(),
                destination.display()
            )
        })?;

        Ok(())
    }
}

/// A file copied into an application's runtime directory.
#[derive(Clone, Debug, Serialize)]
pub struct StagedFile {
    pub path: PathBuf,
    pub source: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lock_manifest() {
        let manifest = LockManifest::load().unwrap();

        assert_eq!(manifest.sdks.dlss.version, "310.4.0");
        assert_eq!(manifest.sdks.streamline.artifacts.len(), 1);
    }

    #[test]
    fn rejects_unsafe_paths() {
        assert!(safe_relative_path(Path::new("include/header.h")).is_ok());
        assert!(safe_relative_path(Path::new("../header.h")).is_err());
        assert!(safe_relative_path(Path::new("/tmp/header.h")).is_err());
    }

    #[test]
    fn shader_artifact_supports_all_targets_but_native_artifacts_remain_exact() {
        let manifest = LockManifest::load().unwrap();

        assert_eq!("sharc".parse::<Sdk>().unwrap(), Sdk::Sharc);
        assert_eq!(Sdk::Sharc.name(), "sharc");
        assert_eq!(serde_json::to_string(&Sdk::Sharc).unwrap(), "\"sharc\"");
        assert_eq!(manifest.sdks.sharc.version, "1.8.3");
        assert_eq!(manifest.sdks.sharc.environment, "SHARC_SDK");

        let artifact = &manifest.sdks.sharc.artifacts[0];

        assert!(artifact.assembly.is_none());
        assert!(artifact.runtime.is_none());

        for target in [
            "x86_64-unknown-linux-gnu",
            "x86_64-pc-windows-msvc",
            "aarch64-apple-darwin",
            "x86_64-apple-darwin",
            "wasm32-unknown-unknown",
            "future-target",
        ] {
            assert!(artifact.supports(target));

            for sdk in [Sdk::Dlss, Sdk::Nrd, Sdk::Omm, Sdk::Streamline] {
                for native in &manifest.sdks.get(sdk).artifacts {
                    assert!(!native.targets.iter().any(|target| target.contains('*')));
                    assert_eq!(
                        native.supports(target),
                        native.targets.iter().any(|candidate| candidate == target)
                    );
                }
            }
        }
    }

    #[test]
    fn sharc_requires_every_header_and_license_at_the_official_root() {
        let manifest = LockManifest::load().unwrap();
        let artifact = &manifest.sdks.sharc.artifacts[0];
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("SHARC-commit");
        fs::create_dir_all(root.join("include")).unwrap();

        assert_eq!(artifact.required.len(), 6);

        for relative in &artifact.required {
            fs::write(root.join(relative), "fixture").unwrap();
        }

        assert_eq!(artifact.locate_root(temporary.path()).unwrap(), root);

        artifact.validate_cached(&root).unwrap();

        for relative in &artifact.required {
            let path = root.join(relative);
            fs::remove_file(&path).unwrap();

            assert!(artifact.validate(&root).is_err());

            fs::write(&path, "").unwrap();

            assert!(artifact.validate(&root).is_err());

            fs::write(&path, "version https://git-lfs.github.com/spec/v1\n").unwrap();

            assert!(artifact.validate(&root).is_err());

            fs::write(&path, "fixture").unwrap();
        }

        assert!(artifact.validate(&root.join("include")).is_err());
    }

    #[test]
    fn source_artifacts_pin_the_complete_native_dependency_closure() {
        let manifest = LockManifest::load().unwrap();

        for (sdk, targets) in [
            (
                Sdk::Nrd,
                &[
                    "x86_64-unknown-linux-gnu",
                    "x86_64-pc-windows-msvc",
                    "aarch64-apple-darwin",
                ][..],
            ),
            (
                Sdk::Omm,
                &["x86_64-apple-darwin", "aarch64-apple-darwin"][..],
            ),
        ] {
            for target in targets {
                let artifact = manifest
                    .sdks
                    .get(sdk)
                    .artifacts
                    .iter()
                    .find(|artifact| artifact.supports(target))
                    .unwrap();
                let assembly = artifact.assembly.as_ref().unwrap();

                assert_eq!(
                    assembly.dependencies.len(),
                    if sdk == Sdk::Nrd { 6 } else { 4 }
                );
                assert!(artifact.url.starts_with("https://codeload.github.com/"));
                assert_eq!(artifact.sha256.len(), 64);

                for dependency in &assembly.dependencies {
                    assert!(dependency.url.starts_with("https://codeload.github.com/"));
                    assert_eq!(dependency.sha256.len(), 64);
                    assert!(
                        dependency
                            .sha256
                            .bytes()
                            .all(|byte| byte.is_ascii_hexdigit())
                    );

                    safe_relative_path(&dependency.root).unwrap();
                    safe_relative_path(&dependency.destination).unwrap();

                    if let Some(files) = &dependency.files {
                        assert!(!files.is_empty());

                        for file in files {
                            safe_relative_path(file).unwrap();
                        }
                    }
                }

                assert!(artifact.required.len() > 10);
            }
        }
    }

    #[test]
    fn all_archive_hashes_are_valid_including_streamline() {
        let manifest = LockManifest::load().unwrap();

        for sdk in [Sdk::Dlss, Sdk::Nrd, Sdk::Omm, Sdk::Sharc, Sdk::Streamline] {
            for artifact in &manifest.sdks.get(sdk).artifacts {
                assert_eq!(artifact.sha256.len(), 64);
                assert!(artifact.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()));
            }
        }

        assert_eq!(
            manifest.sdks.streamline.artifacts[0].sha256,
            "b7e4f31706cfacafba95d2d4abc2c9dcf2dc5fc58b2a6917c71f83051d271aa1"
        );
    }

    #[test]
    fn source_recipe_changes_invalidate_cache_identity() {
        let mut artifact = LockManifest::load().unwrap().sdks.omm.artifacts.remove(2);
        let original = artifact.cache_key().unwrap();
        artifact.assembly.as_mut().unwrap().dependencies[0].sha256 = "0".repeat(64);
        let dependency_changed = artifact.cache_key().unwrap();

        assert_ne!(original, dependency_changed);

        artifact.assembly.as_mut().unwrap().patches[0]
            .after
            .push('\n');

        assert_ne!(dependency_changed, artifact.cache_key().unwrap());
    }

    #[test]
    fn assembles_and_patches_only_staging_then_requires_completion_stamp() {
        let temporary = tempfile::tempdir().unwrap();
        let override_root = temporary.path().join("override");
        let staging = temporary.path().join("staging");
        fs::create_dir_all(&override_root).unwrap();
        fs::create_dir_all(staging.join("upstream")).unwrap();
        fs::write(override_root.join("header.h"), "#elif __linux__\n").unwrap();
        fs::copy(
            override_root.join("header.h"),
            staging.join("upstream/header.h"),
        )
        .unwrap();
        let artifact: Artifact = toml::from_str(
            r##"
id = "fixture"
url = "https://example.invalid/fixture.zip"
sha256 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
targets = ["fixture"]
required = ["header.h"]
[assembly]
root = "fixture"
dependencies = []
[[assembly.relocations]]
source = "upstream/header.h"
destination = "header.h"
[[assembly.patches]]
path = "header.h"
before = "#elif __linux__"
after = "#elif defined(__linux__) || defined(__APPLE__)"
"##,
        )
        .unwrap();
        // Overrides need neither a recipe stamp nor the acquired-source patches.
        artifact.validate(&override_root).unwrap();
        artifact
            .assembly
            .as_ref()
            .unwrap()
            .prepare(&staging, temporary.path())
            .unwrap();

        assert_eq!(
            fs::read_to_string(override_root.join("header.h")).unwrap(),
            "#elif __linux__\n"
        );
        assert!(
            fs::read_to_string(staging.join("header.h"))
                .unwrap()
                .contains("__APPLE__")
        );
        assert!(artifact.validate_cached(&staging).is_err());

        fs::write(
            staging.join(".nvidia-sdk-source"),
            artifact.cache_key().unwrap(),
        )
        .unwrap();
        artifact.validate_cached(&staging).unwrap();
        fs::write(staging.join(".nvidia-sdk-source"), "stale").unwrap();

        assert!(artifact.validate_cached(&staging).is_err());
    }

    #[test]
    fn rejects_patch_drift_unsafe_relocations_and_wrong_hashes() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("header.h");
        fs::write(&path, "abc").unwrap();
        verify_hash(
            &path,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        )
        .unwrap();
        let error = verify_hash(&path, &"0".repeat(64)).unwrap_err().to_string();

        assert!(error.contains("archive hash mismatch"));
        assert!(error.contains(path.to_str().unwrap()));

        let source_patch = SourcePatch {
            path: "header.h".into(),
            before: "missing".into(),
            after: "patched".into(),
        };

        assert!(source_patch.apply(temporary.path()).is_err());

        fs::write(&path, "missing missing").unwrap();

        assert!(source_patch.apply(temporary.path()).is_err());

        let relocation = SourceRelocation {
            source: "header.h".into(),
            destination: "../escaped".into(),
        };

        assert!(
            relocation
                .apply(temporary.path(), temporary.path())
                .is_err()
        );

        let relocation = SourceRelocation {
            source: "../escaped".into(),
            destination: "header.h".into(),
        };

        assert!(
            relocation
                .apply(temporary.path(), temporary.path())
                .is_err()
        );
    }

    #[test]
    fn file_and_zip_errors_name_operation_and_preserve_path_case() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("MixedCase.zip");
        let output = temporary.path().join("extracted");
        let error = verify_hash(&path, &"0".repeat(64)).unwrap_err().to_string();

        assert!(error.contains("opening archive for hashing"));
        assert!(error.contains(path.to_str().unwrap()));

        let error = extract_zip(&path, &output).unwrap_err().to_string();

        assert!(error.contains("opening archive"));
        assert!(error.contains(path.to_str().unwrap()));

        fs::write(&path, "not a zip archive").unwrap();
        let error = extract_zip(&path, &output).unwrap_err().to_string();

        assert!(error.contains("reading zip archive"));
        assert!(error.contains(path.to_str().unwrap()));
    }
}
