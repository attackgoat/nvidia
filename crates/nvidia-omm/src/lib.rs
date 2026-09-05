#[cfg(any(nvidia_omm_native, test))]
mod ffi;

#[cfg(nvidia_omm_native)]
mod native;

#[cfg(not(nvidia_omm_native))]
mod stub;

#[cfg(nvidia_omm_native)]
pub use native::{AlphaTexture, Baker};

#[cfg(not(nvidia_omm_native))]
pub use stub::{AlphaTexture, Baker};

#[cfg(any(nvidia_omm_native, test))]
use anyhow::{Context as _, Result, ensure};

pub const ALPHA_CUTOFF: f32 = 0.5;
pub const FOUR_STATE_FORMAT: u16 = 2;
pub const SDK_VERSION: &str = "1.9.2";

#[derive(Clone, Copy, Debug)]
pub struct AlphaMip<'a> {
    /// Must not exceed the preceding mip's width (equal dimensions are allowed).
    pub width: u32,
    /// Must not exceed the preceding mip's height (equal dimensions are allowed).
    pub height: u32,
    /// Bytes between row starts; zero means `width`. Final-row padding is optional.
    pub row_pitch: u32,
    pub data: &'a [u8],
}

#[cfg(any(nvidia_omm_native, test))]
impl AlphaMip<'_> {
    fn validate_all(mips: &[Self]) -> Result<()> {
        ensure!(
            !mips.is_empty(),
            "nvidia omm texture requires at least one mip"
        );
        ensure!(
            mips.windows(2)
                .all(|pair| pair[1].width <= pair[0].width && pair[1].height <= pair[0].height),
            "nvidia omm mip dimensions must be non-increasing"
        );

        for mip in mips {
            ensure!(
                mip.width > 0 && mip.height > 0,
                "nvidia omm mip extent must be nonzero"
            );

            let row_pitch = if mip.row_pitch == 0 {
                mip.width
            } else {
                mip.row_pitch
            };

            ensure!(
                row_pitch >= mip.width,
                "nvidia omm mip row pitch is smaller than its width"
            );

            let pitch = usize::try_from(row_pitch).context("nvidia omm row pitch overflow")?;
            let preceding_rows =
                usize::try_from(mip.height - 1).context("nvidia omm mip height overflow")?;
            let width = usize::try_from(mip.width).context("nvidia omm mip width overflow")?;
            let required = preceding_rows
                .checked_mul(pitch)
                .and_then(|offset| offset.checked_add(width))
                .context("nvidia omm mip byte size overflow")?;

            ensure!(
                mip.data.len() >= required,
                "nvidia omm mip data is truncated"
            );
        }

        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BakeConfig {
    /// Must be positive with a finite, normal `f32` square. Baking also rejects
    /// geometry whose derived subdivision arithmetic is unsafe in OMM 1.9.2.
    pub dynamic_subdivision_scale: f32,
    pub rejection_threshold: f32,
    pub max_subdivision_level: u8,
    pub max_array_data_size: u32,
    pub max_workload_size: u64,
    pub internal_threads: bool,
    pub validation: bool,
}

impl Default for BakeConfig {
    fn default() -> Self {
        Self {
            dynamic_subdivision_scale: 2.0,
            rejection_threshold: 0.0,
            max_subdivision_level: 8,
            max_array_data_size: u32::MAX,
            max_workload_size: u64::MAX,
            internal_threads: true,
            validation: cfg!(debug_assertions),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct BakeInput<'a> {
    /// Finite UVs, including wrapped coordinates outside [0, 1]. Baking
    /// conservatively validates both SDK subdivision heuristics and requires
    /// referenced mip-0 texel coordinates to have magnitude below 2^30.
    /// Texel bounding box products, with raster headroom, must fit in `i32`.
    pub texture_coordinates: &'a [[f32; 2]],
    pub indices: &'a [u32],
    pub config: BakeConfig,
}

#[cfg(any(nvidia_omm_native, test))]
impl BakeInput<'_> {
    fn validate(self, texture_size: [u32; 2]) -> Result<()> {
        ensure!(
            !self.indices.is_empty() && self.indices.len().is_multiple_of(3),
            "nvidia omm indices must contain complete triangles"
        );
        ensure!(
            !self.texture_coordinates.is_empty(),
            "nvidia omm texture coordinates must not be empty"
        );
        ensure!(
            self.indices
                .iter()
                .all(|&index| (index as usize) < self.texture_coordinates.len()),
            "nvidia omm index exceeds the texture-coordinate count"
        );
        ensure!(
            self.config.dynamic_subdivision_scale.is_finite()
                && self.config.dynamic_subdivision_scale > 0.0,
            "nvidia omm dynamic subdivision scale must be finite and positive"
        );
        ensure!(
            (0.0..=1.0).contains(&self.config.rejection_threshold),
            "nvidia omm rejection threshold must be in [0, 1]"
        );
        ensure!(
            self.config.max_subdivision_level <= 12,
            "nvidia omm subdivision level exceeds 12"
        );

        // OMM 1.9.2 bake_cpu_impl.cpp ComputeAreaHeuristic casts the ratio to
        // uint32_t BEFORE clamping. Use f32 throughout, including length(cross)
        // (not abs(cross)), so validation catches the SDK's intermediate overflow.
        let scale = self.config.dynamic_subdivision_scale;
        let target_area = scale * scale;

        ensure!(
            target_area.is_normal(),
            "nvidia omm squared subdivision scale must be finite and normal"
        );

        #[expect(
            clippy::cast_precision_loss,
            reason = "match the SDK's uint2 to float2 conversion"
        )]
        let size = texture_size.map(|value| value as f32);

        for uv in self.texture_coordinates {
            ensure!(
                uv.iter().all(|value| value.is_finite()),
                "nvidia omm texture coordinates must be finite"
            );
        }

        for indices in self.indices.chunks_exact(3) {
            let uv = [indices[0], indices[1], indices[2]]
                .map(|index| self.texture_coordinates[index as usize]);
            let p = uv.map(|uv| [uv[0] * size[0], uv[1] * size[1]]);
            let a = [p[2][0] - p[0][0], p[2][1] - p[0][1]];
            let b = [p[1][0] - p[0][0], p[1][1] - p[0][1]];
            let cross = a[0] * b[1] - a[1] * b[0];
            let pixel_area = 0.5 * (cross * cross).sqrt();
            let ratio = pixel_area / target_area;

            ensure!(
                ratio.is_finite() && (0.0..4_294_967_296.0).contains(&ratio),
                "nvidia omm dynamic subdivision area ratio is outside uint32 range"
            );

            // Degenerate triangles use ComputeEdgeHeuristic instead. Validate
            // both paths rather than depend on the SDK's degeneracy rounding.
            for (start, end) in [(0, 1), (0, 2), (1, 2)] {
                let edge = [
                    (uv[end][0] - uv[start][0]) * size[0],
                    (uv[end][1] - uv[start][1]) * size[1],
                ];
                let length_squared = edge[0] * edge[0] + edge[1] * edge[1];

                ensure!(
                    length_squared.is_finite(),
                    "nvidia omm dynamic subdivision edge length must be finite"
                );

                // Finite f32 lengths and a positive normal squared scale bound
                // log2(length_squared)/2 - log2(scale) well inside int32 range.
            }

            // Even a zero-area triangle can have huge finite coordinates. The
            // SDK later casts UVs/texel positions to int32; leave headroom for
            // interpolation and pixel offsets rather than letting it reach UB.
            ensure!(
                p.iter()
                    .flatten()
                    .all(|value| value.abs() < 1_073_741_824.0),
                "nvidia omm texture coordinates exceed the safe raster range"
            );

            // ComputeWorkloadSize multiplies int32 AABB dimensions BEFORE its
            // uint64 cast. Match its f32 subtract-then-scale rounding, also bound
            // raster scale-then-subtract, and leave room for floor/ceil, the
            // half-texel offset and inclusive pixel endpoints. Use f64 for the
            // padded product so i32::MAX is not rounded up to 2^31.
            let extent = [0, 1].map(|axis| {
                let min_uv = uv[0][axis].min(uv[1][axis]).min(uv[2][axis]);
                let max_uv = uv[0][axis].max(uv[1][axis]).max(uv[2][axis]);
                let workload_extent = (max_uv - min_uv) * size[axis];
                let min_p = p[0][axis].min(p[1][axis]).min(p[2][axis]);
                let max_p = p[0][axis].max(p[1][axis]).max(p[2][axis]);
                f64::from(workload_extent)
                    .max(f64::from(max_p) - f64::from(min_p))
                    .ceil()
                    + 4.0
            });

            ensure!(
                extent[0] * extent[1] <= f64::from(i32::MAX),
                "nvidia omm texel bounding box product exceeds the safe int32 range"
            );
        }

        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BakeOutput {
    pub array_data: Box<[u8]>,
    pub descriptors: Box<[OpacityMicromapDescriptor]>,
    pub descriptor_usage: Box<[OpacityMicromapUsage]>,
    pub indices: Box<[i32]>,
    pub index_usage: Box<[OpacityMicromapUsage]>,
    pub statistics: Statistics,
}

#[cfg(nvidia_omm_native)]
impl BakeOutput {
    unsafe fn copy<T: Copy>(pointer: *const T, count: u32) -> Result<Box<[T]>> {
        if count == 0 {
            return Ok(Box::new([]));
        }

        ensure!(
            !pointer.is_null(),
            "nvidia omm returned a null output pointer"
        );

        Ok(unsafe { std::slice::from_raw_parts(pointer, count as usize) }.into())
    }

    unsafe fn from_native(output: ffi::BakeResult) -> Result<Self> {
        Ok(Self {
            array_data: unsafe { Self::copy(output.array_data, output.array_data_size) }?,
            descriptors: unsafe { Self::copy(output.descriptors, output.descriptor_count) }?,
            descriptor_usage: unsafe {
                Self::copy(output.descriptor_usage, output.descriptor_usage_count)
            }?,
            indices: unsafe { Self::copy(output.indices, output.index_count) }?,
            index_usage: unsafe { Self::copy(output.index_usage, output.index_usage_count) }?,
            statistics: output.statistics,
        })
    }
}

impl Baker {
    /// Returns the bundled native backend path, when available.
    #[must_use]
    pub fn bundled_backend_path() -> Option<&'static std::path::Path> {
        option_env!("NVIDIA_OMM_BACKEND").map(std::path::Path::new)
    }

    /// Reports whether this build includes the native backend.
    #[must_use]
    pub const fn native_backend_available() -> bool {
        cfg!(nvidia_omm_native)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct OpacityMicromapDescriptor {
    pub data_offset: u32,
    pub subdivision_level: u16,
    pub format: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct OpacityMicromapUsage {
    pub count: u32,
    pub subdivision_level: u16,
    pub format: u16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct Statistics {
    pub opaque: u64,
    pub transparent: u64,
    pub unknown_transparent: u64,
    pub unknown_opaque: u64,
    pub fully_opaque: u32,
    pub fully_transparent: u32,
    pub fully_unknown_opaque: u32,
    pub fully_unknown_transparent: u32,
    pub known_area: f32,
}

#[cfg(test)]
mod test {
    use {
        super::*,
        std::mem::{align_of, offset_of, size_of},
    };

    #[test]
    fn invalid_geometry_is_rejected() {
        let texture_coordinates = [[0.0, 0.0]; 3];

        assert!(
            BakeInput {
                texture_coordinates: &texture_coordinates,
                indices: &[0, 1],
                config: BakeConfig::default(),
            }
            .validate([2, 2])
            .is_err()
        );
    }

    #[test]
    fn invalid_texture_is_rejected() {
        assert!(
            AlphaMip::validate_all(&[AlphaMip {
                width: 2,
                height: 2,
                row_pitch: 1,
                data: &[0; 4],
            }])
            .is_err()
        );
    }

    #[test]
    fn mip_data_only_requires_pixels_in_the_final_row() {
        #[cfg(nvidia_omm_native)]
        let baker = Baker::new(Baker::bundled_backend_path().unwrap()).unwrap();

        for (width, height, row_pitch, required) in [
            (2, 2, 4, 6),
            (2, 1, u32::MAX, 2),
            (2, 2, 0, 4),
            (2, 2, 2, 4),
        ] {
            let data = vec![0; required];
            let mip = AlphaMip {
                width,
                height,
                row_pitch,
                data: &data,
            };
            AlphaMip::validate_all(&[mip]).unwrap();

            #[cfg(nvidia_omm_native)]
            {
                let texture = baker.create_texture(&[mip]).unwrap();
                let output = baker
                    .bake(
                        &texture,
                        BakeInput {
                            texture_coordinates: &[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
                            indices: &[0, 1, 2],
                            config: BakeConfig::default(),
                        },
                    )
                    .unwrap();
                assert_eq!(output.indices.as_ref(), &[-1]);
            }

            let truncated = AlphaMip {
                data: &data[..required - 1],
                ..mip
            };
            assert_eq!(
                AlphaMip::validate_all(&[truncated])
                    .unwrap_err()
                    .to_string(),
                "nvidia omm mip data is truncated"
            );
            #[cfg(nvidia_omm_native)]
            assert!(baker.create_texture(&[truncated]).is_err());
        }
    }

    #[test]
    fn oversized_mip_data_is_rejected_without_wrapping() {
        let error = AlphaMip::validate_all(&[AlphaMip {
            width: u32::MAX,
            height: u32::MAX,
            row_pitch: u32::MAX,
            data: &[],
        }])
        .unwrap_err();

        if usize::BITS >= 64 {
            assert_eq!(error.to_string(), "nvidia omm mip data is truncated");
        } else {
            assert_eq!(error.to_string(), "nvidia omm mip byte size overflow");
        }
    }

    #[test]
    fn mip_dimensions_must_be_non_increasing() {
        #[cfg(nvidia_omm_native)]
        let baker = Baker::new(Baker::bundled_backend_path().unwrap()).unwrap();

        for dimensions in [
            [[2, 2], [4, 2], [1, 1]],
            [[2, 2], [2, 4], [1, 1]],
            [[4, 4], [1, 2], [2, 1]],
            [[4, 4], [2, 1], [1, 2]],
        ] {
            let mips = dimensions.map(|[width, height]| AlphaMip {
                width,
                height,
                row_pitch: 0,
                data: &[0; 16],
            });
            let error = AlphaMip::validate_all(&mips).unwrap_err().to_string();

            assert_eq!(error, "nvidia omm mip dimensions must be non-increasing");

            #[cfg(nvidia_omm_native)]
            assert_eq!(
                baker.create_texture(&mips).err().unwrap().to_string(),
                error
            );
        }

        for dimensions in [
            [[4, 4], [2, 2], [1, 1]],
            [[4, 4], [4, 4], [4, 4]],
            [[4, 3], [3, 3], [3, 1]],
        ] {
            let mips = dimensions.map(|[width, height]| AlphaMip {
                width,
                height,
                row_pitch: 0,
                data: &[0; 16],
            });
            AlphaMip::validate_all(&mips).unwrap();
        }
    }

    #[test]
    fn unsafe_texel_bounding_box_products_are_rejected() {
        #[cfg(nvidia_omm_native)]
        let baker = Baker::new(Baker::bundled_backend_path().unwrap()).unwrap();

        #[cfg(nvidia_omm_native)]
        let texture = baker
            .create_texture(&[AlphaMip {
                width: 2,
                height: 2,
                row_pitch: 0,
                data: &[0; 4],
            }])
            .unwrap();

        for uv in [
            [[0.0, 0.0], [32768.0, 32768.0], [16384.0, 16384.0]],
            [[-16384.0, -16384.0], [16384.0, 16384.0], [0.0, 0.0]],
            // The unpadded product fits, but raster rounding/headroom does not.
            [[0.0, 0.0], [23169.0, 23169.0], [0.0, 0.0]],
            [[0.0, 0.0], [23168.25, 23168.25], [0.0, 0.0]],
        ] {
            for validation in [false, true] {
                let input = BakeInput {
                    texture_coordinates: &uv,
                    indices: &[0, 1, 2],
                    config: BakeConfig {
                        dynamic_subdivision_scale: 2.0,
                        max_subdivision_level: 0,
                        max_workload_size: 1,
                        validation,
                        ..BakeConfig::default()
                    },
                };
                let error = input.validate([2, 2]).unwrap_err().to_string();

                assert_eq!(
                    error,
                    "nvidia omm texel bounding box product exceeds the safe int32 range"
                );

                #[cfg(nvidia_omm_native)]
                assert_eq!(baker.bake(&texture, input).unwrap_err().to_string(), error);
            }
        }

        for end in [[23168.0, 23168.0], [50000.0, 1.0]] {
            BakeInput {
                texture_coordinates: &[[0.0, 0.0], end, [0.0, 0.0]],
                indices: &[0, 1, 2],
                config: BakeConfig::default(),
            }
            .validate([2, 2])
            .unwrap();
        }
    }

    #[test]
    fn unsafe_subdivision_arithmetic_is_rejected() {
        let triangle = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];

        #[cfg(nvidia_omm_native)]
        let baker = Baker::new(Baker::bundled_backend_path().unwrap()).unwrap();

        #[cfg(nvidia_omm_native)]
        let texture = baker
            .create_texture(&[AlphaMip {
                width: 2,
                height: 2,
                row_pitch: 0,
                data: &[0; 4],
            }])
            .unwrap();

        for (name, uv, scale) in [
            ("squared scale underflow", triangle, 1e-30),
            ("subnormal squared scale", triangle, 1e-20),
            ("squared scale overflow", triangle, f32::MAX),
            (
                "finite ratio beyond uint32 limit",
                triangle,
                2.0_f32.powi(-16),
            ),
            (
                "finite ratio at uint32 limit",
                [triangle[0], triangle[1], [0.0, 0.5]],
                2.0_f32.powi(-16),
            ),
            (
                "ratio overflow",
                [[0.0, 0.0], [2.0, 0.0], [0.0, 2.0]],
                1.1e-19,
            ),
            ("zero scale", triangle, 0.0),
            ("negative scale", triangle, -1.0),
            ("nan scale", triangle, f32::NAN),
            ("infinite scale", triangle, f32::INFINITY),
            ("nan uv", [[f32::NAN, 0.0], triangle[1], triangle[2]], 2.0),
            (
                "infinite uv",
                [[0.0, f32::INFINITY], triangle[1], triangle[2]],
                2.0,
            ),
            ("negative infinite uv", [[f32::NEG_INFINITY, 0.0]; 3], 2.0),
            ("texel overflow", [[f32::MAX, 0.0]; 3], 2.0),
            (
                "edge overflow",
                [[-1e20, 0.0], [1e20, 0.0], [0.0, 0.0]],
                2.0,
            ),
            (
                "cross squared overflow",
                [[0.0, 0.0], [1e10, 0.0], [0.0, 1e10]],
                1e10,
            ),
            ("huge translated point", [[1e30, 1e30]; 3], 2.0),
        ] {
            // Neither SDK validation nor a zero subdivision cap prevents the cast.
            for validation in [false, true] {
                let input = BakeInput {
                    texture_coordinates: &uv,
                    indices: &[0, 1, 2],
                    config: BakeConfig {
                        dynamic_subdivision_scale: scale,
                        max_subdivision_level: 0,
                        validation,
                        ..BakeConfig::default()
                    },
                };
                let error = input.validate([2, 2]).expect_err(name).to_string();

                assert!(error.starts_with("nvidia omm"), "{name}: {error}");

                #[cfg(nvidia_omm_native)]
                assert_eq!(
                    baker.bake(&texture, input).unwrap_err().to_string(),
                    error,
                    "{name}"
                );
            }
        }
    }

    #[test]
    fn ordinary_subdivision_arithmetic_is_accepted() {
        for uv in [
            [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            [[0.0, 0.0], [0.0, 1.0], [1.0, 0.0]],
            [[-1.0, -1.0], [2.0, -1.0], [-1.0, 2.0]],
            [[0.0, 0.0]; 3],
            [[0.0, 0.0], [0.5, 0.5], [1.0, 1.0]],
        ] {
            for scale in [2.0, 1e19] {
                BakeInput {
                    texture_coordinates: &uv,
                    indices: &[0, 1, 2],
                    config: BakeConfig {
                        dynamic_subdivision_scale: scale,
                        ..BakeConfig::default()
                    },
                }
                .validate([2, 2])
                .unwrap();
            }
        }

        let input = BakeInput {
            texture_coordinates: &[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            indices: &[0, 1, 2],
            config: BakeConfig {
                dynamic_subdivision_scale: 2.0_f32.powi(-15),
                ..BakeConfig::default()
            },
        };

        assert!(input.validate([2, 2]).is_ok());
        assert!(input.validate([4, 4]).is_err());
    }

    #[cfg(nvidia_omm_native)]
    #[test]
    fn native_baker_returns_owned_special_indices() {
        let baker = Baker::new(Baker::bundled_backend_path().unwrap()).unwrap();
        let texture = baker
            .create_texture(&[AlphaMip {
                width: 2,
                height: 2,
                row_pitch: 0,
                data: &[0; 4],
            }])
            .unwrap();
        let output = baker
            .bake(
                &texture,
                BakeInput {
                    texture_coordinates: &[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
                    indices: &[0, 1, 2],
                    config: BakeConfig::default(),
                },
            )
            .unwrap();

        assert_eq!(output.indices.as_ref(), &[-1]);
        assert_eq!(output.statistics.fully_transparent, 1);
    }

    #[test]
    fn native_ffi_layouts_are_stable() {
        assert_eq!(align_of::<OpacityMicromapDescriptor>(), 4);
        assert_eq!(offset_of!(OpacityMicromapDescriptor, subdivision_level), 4);
        assert_eq!(size_of::<OpacityMicromapDescriptor>(), 8);
        assert_eq!(size_of::<OpacityMicromapUsage>(), 8);
        assert_eq!(size_of::<Statistics>(), 56);
        assert_eq!(size_of::<ffi::TextureMip>(), 24);
        assert_eq!(size_of::<ffi::BakeInput>(), 64);
        assert_eq!(size_of::<ffi::BakeResult>(), 136);
    }
}
