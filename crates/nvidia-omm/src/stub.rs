use {
    crate::{AlphaMip, BakeInput, BakeOutput},
    anyhow::{Result, bail},
    std::{marker::PhantomData, path::Path},
};

pub struct AlphaTexture<'a> {
    _baker: PhantomData<&'a Baker>,
}

pub struct Baker;

impl Baker {
    /// Creates a CPU baker.
    ///
    /// # Errors
    /// Always returns an error because the native backend is unavailable.
    pub fn new(_library_path: &Path) -> Result<Self> {
        bail!(
            "native nvidia omm is unavailable; enable the nvidia-omm native feature on a supported host"
        )
    }

    /// Copies alpha mips into an SDK texture.
    ///
    /// # Errors
    /// Always returns an error because the native backend is unavailable.
    pub fn create_texture<'a>(&'a self, _mips: &[AlphaMip<'_>]) -> Result<AlphaTexture<'a>> {
        bail!("native nvidia omm is unavailable on this build")
    }

    /// Bakes opacity micromaps.
    ///
    /// # Errors
    /// Always returns an error because the native backend is unavailable.
    pub fn bake(&self, _texture: &AlphaTexture<'_>, _input: BakeInput<'_>) -> Result<BakeOutput> {
        bail!("native nvidia omm is unavailable on this build")
    }
}
