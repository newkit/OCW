//! The swapchain: which colour format to ask the runtime for, and how
//! large the eye images are.
//!
//! Runtimes list the formats they can composite, best first by their own
//! taste; we take the first of ours that they offer ([`choose_format`]):
//!
//! 1. **8-bit sRGB, RGBA** – what the renderer writes by default (Bevy's
//!    `Rgba8UnormSrgb`), gamma-correct for the compositor;
//! 2. 8-bit sRGB, BGRA (some Windows runtimes list it first);
//! 3. 16-bit float – linear, also correct, twice the bandwidth.
//!
//! Plain UNORM formats would need our own sRGB encoding and are refused.
//! The numbers are Vulkan's `VkFormat` (Linux, Windows, Android – the
//! Vulkan path of X1) and DXGI's (a later D3D12 path), so the choice is the
//! same code on every platform.

/// A colour format both the renderer and a runtime can use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorFormat {
    Rgba8UnormSrgb,
    Bgra8UnormSrgb,
    Rgba16Float,
    /// Linear 8-bit (not chosen: the colours would be too dark).
    Rgba8Unorm,
    Bgra8Unorm,
}

/// Ours, best first.
pub const PREFERRED: [ColorFormat; 3] = [
    ColorFormat::Rgba8UnormSrgb,
    ColorFormat::Bgra8UnormSrgb,
    ColorFormat::Rgba16Float,
];

impl ColorFormat {
    const ALL: [Self; 5] = [
        Self::Rgba8UnormSrgb,
        Self::Bgra8UnormSrgb,
        Self::Rgba16Float,
        Self::Rgba8Unorm,
        Self::Bgra8Unorm,
    ];

    /// Its `VkFormat`.
    pub fn vk(self) -> i64 {
        match self {
            Self::Rgba8Unorm => 37,
            Self::Rgba8UnormSrgb => 43,
            Self::Bgra8Unorm => 44,
            Self::Bgra8UnormSrgb => 50,
            Self::Rgba16Float => 97,
        }
    }

    /// Its `DXGI_FORMAT`.
    pub fn dxgi(self) -> i64 {
        match self {
            Self::Rgba16Float => 10,
            Self::Rgba8Unorm => 28,
            Self::Rgba8UnormSrgb => 29,
            Self::Bgra8Unorm => 87,
            Self::Bgra8UnormSrgb => 91,
        }
    }

    pub fn from_vk(f: i64) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.vk() == f)
    }

    pub fn from_dxgi(f: i64) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.dxgi() == f)
    }

    pub fn is_srgb(self) -> bool {
        matches!(self, Self::Rgba8UnormSrgb | Self::Bgra8UnormSrgb)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Rgba8UnormSrgb => "RGBA8 sRGB",
            Self::Bgra8UnormSrgb => "BGRA8 sRGB",
            Self::Rgba16Float => "RGBA16F",
            Self::Rgba8Unorm => "RGBA8",
            Self::Bgra8Unorm => "BGRA8",
        }
    }
}

/// Which graphics API's format numbers a runtime speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatSpace {
    Vulkan,
    Dxgi,
}

/// The format to create the swapchain with, from the runtime's list (in
/// its numbers); `None` if it offers none of ours.
pub fn choose_format(offered: &[i64], space: FormatSpace) -> Option<ColorFormat> {
    let known: Vec<ColorFormat> = offered
        .iter()
        .filter_map(|&f| match space {
            FormatSpace::Vulkan => ColorFormat::from_vk(f),
            FormatSpace::Dxgi => ColorFormat::from_dxgi(f),
        })
        .collect();
    PREFERRED.into_iter().find(|p| known.contains(p))
}

/// The size of each eye's image: the runtime's recommendation scaled by
/// `scale` (the quality preset's render scale), at most its maximum, at
/// least 16 px, rounded to a multiple of 4 (friendlier to the GPU's tiles
/// and to the compositor's reprojection).
pub fn eye_extent(recommended: (u32, u32), max: (u32, u32), scale: f32) -> (u32, u32) {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let one = |r: u32, m: u32| {
        let m = m.max(16);
        let v = ((r as f32 * scale) / 4.0).round() as u32 * 4;
        v.clamp(16, m - m % 4)
    };
    (one(recommended.0, max.0), one(recommended.1, max.1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_rgba_first_then_bgra_then_float() {
        use ColorFormat::*;
        let v = |fs: &[ColorFormat]| fs.iter().map(|f| f.vk()).collect::<Vec<_>>();
        // A runtime's own order does not matter, ours does.
        assert_eq!(
            choose_format(
                &v(&[Bgra8UnormSrgb, Rgba16Float, Rgba8UnormSrgb]),
                FormatSpace::Vulkan
            ),
            Some(Rgba8UnormSrgb)
        );
        assert_eq!(
            choose_format(&v(&[Rgba8Unorm, Bgra8UnormSrgb]), FormatSpace::Vulkan),
            Some(Bgra8UnormSrgb)
        );
        assert_eq!(
            choose_format(&v(&[Rgba16Float, Rgba8Unorm]), FormatSpace::Vulkan),
            Some(Rgba16Float)
        );
        // Linear 8-bit only, or formats we do not know: none.
        assert_eq!(choose_format(&[37, 44, 123_456], FormatSpace::Vulkan), None);
        assert_eq!(choose_format(&[], FormatSpace::Vulkan), None);
        // DXGI numbers (a D3D12 runtime).
        assert_eq!(
            choose_format(&[91, 29], FormatSpace::Dxgi),
            Some(Rgba8UnormSrgb)
        );
        assert_eq!(
            choose_format(&[91], FormatSpace::Dxgi),
            Some(Bgra8UnormSrgb)
        );
    }

    #[test]
    fn format_numbers_round_trip() {
        // VK_FORMAT_R8G8B8A8_SRGB = 43, DXGI_FORMAT_R8G8B8A8_UNORM_SRGB = 29.
        assert_eq!(ColorFormat::Rgba8UnormSrgb.vk(), 43);
        assert_eq!(ColorFormat::Rgba8UnormSrgb.dxgi(), 29);
        for f in ColorFormat::ALL {
            assert_eq!(ColorFormat::from_vk(f.vk()), Some(f));
            assert_eq!(ColorFormat::from_dxgi(f.dxgi()), Some(f));
            assert!(!f.name().is_empty());
        }
        assert!(ColorFormat::Bgra8UnormSrgb.is_srgb() && !ColorFormat::Rgba16Float.is_srgb());
    }

    #[test]
    fn eye_images_follow_the_recommendation_and_the_scale() {
        // The Quest 1 over Link recommends about its panel size.
        assert_eq!(eye_extent((1440, 1600), (4096, 4096), 1.0), (1440, 1600));
        assert_eq!(eye_extent((1440, 1600), (4096, 4096), 0.8), (1152, 1280));
        // Never beyond the maximum, never below 16 px, multiples of 4.
        assert_eq!(eye_extent((1440, 1600), (1500, 1599), 1.5), (1500, 1596));
        assert_eq!(eye_extent((1440, 1600), (4096, 4096), 0.001), (16, 16));
        assert_eq!(
            eye_extent((1441, 1601), (4096, 4096), f32::NAN),
            (1440, 1600)
        );
    }
}
