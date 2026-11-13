//! KTX2 texture files (Khronos KTX 2.0) for the material library (spec
//! §4.6, Phase 8 T1): 2D textures with a full mip chain in a GPU block
//! format, no supercompression. Writing is ours (a few dozen lines, so no
//! external tool); reading and the data format descriptor come from the
//! `ktx2` crate, the one Bevy's own KTX2 loader uses.

use ::ktx2 as k;

/// The texture formats the library uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TexFormat {
    /// BC3 (DXT5) sRGB: albedo in RGB, roughness in alpha (linear).
    Bc3Srgb,
    /// BC5: two channels (normal x, y).
    Bc5,
    /// Uncompressed RGBA, sRGB (tests, tools).
    Rgba8Srgb,
    /// Uncompressed RGBA, linear.
    Rgba8,
}

impl TexFormat {
    pub fn vk(self) -> k::Format {
        match self {
            TexFormat::Bc3Srgb => k::Format::BC3_SRGB_BLOCK,
            TexFormat::Bc5 => k::Format::BC5_UNORM_BLOCK,
            TexFormat::Rgba8Srgb => k::Format::R8G8B8A8_SRGB,
            TexFormat::Rgba8 => k::Format::R8G8B8A8_UNORM,
        }
    }

    pub fn from_vk(f: k::Format) -> Option<Self> {
        [
            TexFormat::Bc3Srgb,
            TexFormat::Bc5,
            TexFormat::Rgba8Srgb,
            TexFormat::Rgba8,
        ]
        .into_iter()
        .find(|t| t.vk() == f)
    }

    /// Bytes of one 4 × 4 block (compressed) or one texel.
    pub fn block_bytes(self) -> usize {
        match self {
            TexFormat::Bc3Srgb | TexFormat::Bc5 => 16,
            TexFormat::Rgba8Srgb | TexFormat::Rgba8 => 4,
        }
    }

    pub fn is_compressed(self) -> bool {
        matches!(self, TexFormat::Bc3Srgb | TexFormat::Bc5)
    }

    /// Bytes of a `w × h` level.
    pub fn level_bytes(self, w: u32, h: u32) -> usize {
        let (w, h) = (w.max(1) as usize, h.max(1) as usize);
        if self.is_compressed() {
            w.div_ceil(4) * h.div_ceil(4) * self.block_bytes()
        } else {
            w * h * self.block_bytes()
        }
    }
}

/// A 2D texture with its mip chain (level 0 = full size first).
#[derive(Debug, Clone, PartialEq)]
pub struct Texture {
    pub format: TexFormat,
    pub width: u32,
    pub height: u32,
    pub levels: Vec<Vec<u8>>,
    /// Key/value metadata (written sorted by key, as KTX2 requires).
    pub metadata: Vec<(String, String)>,
}

impl Texture {
    /// Size of level `i`.
    pub fn level_size(&self, i: usize) -> (u32, u32) {
        ((self.width >> i).max(1), (self.height >> i).max(1))
    }

    /// A metadata value.
    pub fn meta(&self, key: &str) -> Option<&str> {
        self.metadata
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// Checks the level sizes against the format.
    pub fn check(&self) -> Result<(), String> {
        if self.width == 0 || self.height == 0 || self.levels.is_empty() {
            return Err("empty texture".into());
        }
        for (i, l) in self.levels.iter().enumerate() {
            let (w, h) = self.level_size(i);
            let want = self.format.level_bytes(w, h);
            if l.len() != want {
                return Err(format!(
                    "level {i} ({w} × {h}) has {} bytes, {:?} needs {want}",
                    l.len(),
                    self.format
                ));
            }
        }
        Ok(())
    }
}

/// Number of levels of a full mip chain down to 1 × 1.
pub fn full_chain(width: u32, height: u32) -> u32 {
    32 - width.max(height).max(1).leading_zeros()
}

/// The key the writer names itself under (KTX2 `KTXwriter`).
pub const WRITER: &str = "ocw-assets (Open Cycling World)";

fn pad(b: &mut Vec<u8>, align: usize) {
    while !b.len().is_multiple_of(align) {
        b.push(0);
    }
}

/// Encodes `t` as a KTX2 file.
pub fn write(t: &Texture) -> Result<Vec<u8>, String> {
    t.check()?;
    let n = t.levels.len();
    let (basic, type_size) =
        k::dfd::Basic::from_format(t.format.vk()).map_err(|e| format!("{e:?}"))?;
    let dfd_block = k::dfd::Block::Basic(basic).to_vec();
    let mut dfd = Vec::new();
    dfd.extend(((dfd_block.len() + 4) as u32).to_le_bytes());
    dfd.extend(dfd_block);
    // Key/value data, sorted by key; KTXwriter always.
    let mut kv: Vec<(String, String)> = t
        .metadata
        .iter()
        .filter(|(k, _)| k != "KTXwriter")
        .cloned()
        .collect();
    kv.push(("KTXwriter".into(), WRITER.into()));
    kv.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let mut kvd = Vec::new();
    for (key, value) in &kv {
        let len = key.len() + 1 + value.len() + 1;
        kvd.extend((len as u32).to_le_bytes());
        kvd.extend(key.as_bytes());
        kvd.push(0);
        kvd.extend(value.as_bytes());
        kvd.push(0);
        pad(&mut kvd, 4);
    }
    let level_index_at = k::Header::LENGTH;
    let dfd_at = level_index_at + n * k::LevelIndex::LENGTH;
    let kvd_at = dfd_at + dfd.len();
    let mut b = vec![0u8; kvd_at];
    b[dfd_at..kvd_at].copy_from_slice(&dfd);
    b.extend(&kvd);
    // Level data, smallest level first, each aligned to lcm(block, 4).
    let align = t.format.block_bytes().max(4);
    let mut index = vec![
        k::LevelIndex {
            byte_offset: 0,
            byte_length: 0,
            uncompressed_byte_length: 0,
        };
        n
    ];
    for i in (0..n).rev() {
        pad(&mut b, align);
        index[i] = k::LevelIndex {
            byte_offset: b.len() as u64,
            byte_length: t.levels[i].len() as u64,
            uncompressed_byte_length: t.levels[i].len() as u64,
        };
        b.extend(&t.levels[i]);
    }
    let header = k::Header {
        format: Some(t.format.vk()),
        type_size,
        pixel_width: t.width,
        pixel_height: t.height,
        pixel_depth: 0,
        layer_count: 0,
        face_count: 1,
        level_count: n as u32,
        supercompression_scheme: None,
        index: k::Index {
            dfd_byte_offset: dfd_at as u32,
            dfd_byte_length: dfd.len() as u32,
            kvd_byte_offset: kvd_at as u32,
            kvd_byte_length: kvd.len() as u32,
            sgd_byte_offset: 0,
            sgd_byte_length: 0,
        },
    };
    b[..k::Header::LENGTH].copy_from_slice(&header.as_bytes());
    for (i, l) in index.iter().enumerate() {
        let at = level_index_at + i * k::LevelIndex::LENGTH;
        b[at..at + k::LevelIndex::LENGTH].copy_from_slice(&l.as_bytes());
    }
    Ok(b)
}

/// Decodes a KTX2 file written by [`write`] (or any 2D, non-array,
/// non-supercompressed KTX2 file in one of the [`TexFormat`]s).
pub fn read(bytes: &[u8]) -> Result<Texture, String> {
    let r = k::Reader::new(bytes).map_err(|e| format!("KTX2: {e:?}"))?;
    let h = r.header();
    if h.supercompression_scheme.is_some() {
        return Err("KTX2: supercompressed files are not supported".into());
    }
    if h.layer_count > 1 || h.face_count != 1 || h.pixel_depth > 1 {
        return Err("KTX2: only plain 2D textures are supported".into());
    }
    let format = h
        .format
        .and_then(TexFormat::from_vk)
        .ok_or_else(|| format!("KTX2: format {:?} is not supported", h.format))?;
    let levels: Vec<Vec<u8>> = r.levels().map(|l| l.data.to_vec()).collect();
    let metadata = r
        .key_value_data()
        .map(|(k, v)| {
            let v = v.strip_suffix(&[0u8][..]).unwrap_or(v);
            (k.to_owned(), String::from_utf8_lossy(v).into_owned())
        })
        .collect();
    let t = Texture {
        format,
        width: h.pixel_width,
        height: h.pixel_height.max(1),
        levels,
        metadata,
    };
    t.check()?;
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texture(format: TexFormat, size: u32) -> Texture {
        let n = full_chain(size, size) as usize;
        let levels = (0..n)
            .map(|i| {
                let s = (size >> i).max(1);
                (0..format.level_bytes(s, s))
                    .map(|j| (j * 7 + i * 13) as u8)
                    .collect()
            })
            .collect();
        Texture {
            format,
            width: size,
            height: size,
            levels,
            metadata: vec![("OCWmaterial".into(), "meadow".into())],
        }
    }

    #[test]
    fn round_trip_every_format() {
        for f in [
            TexFormat::Bc3Srgb,
            TexFormat::Bc5,
            TexFormat::Rgba8Srgb,
            TexFormat::Rgba8,
        ] {
            let t = texture(f, 64);
            assert_eq!(t.levels.len(), 7);
            let b = write(&t).unwrap();
            let back = read(&b).unwrap();
            assert_eq!(back.levels, t.levels, "{f:?}");
            assert_eq!((back.width, back.height, back.format), (64, 64, f));
            assert_eq!(back.meta("OCWmaterial"), Some("meadow"));
            assert_eq!(back.meta("KTXwriter"), Some(WRITER));
        }
    }

    /// The file is standard KTX2: magic, a basic DFD for the format, level
    /// data aligned and stored smallest first (checked with the `ktx2`
    /// crate, Bevy's reader).
    #[test]
    fn the_file_is_valid_ktx2() {
        let t = texture(TexFormat::Bc3Srgb, 16);
        let b = write(&t).unwrap();
        assert_eq!(&b[..12], b"\xABKTX 20\xBB\r\n\x1A\n");
        let r = ::ktx2::Reader::new(&b[..]).unwrap();
        let dfd = r.basic_dfd().expect("basic DFD");
        assert_eq!(dfd.transfer_function, Some(::ktx2::TransferFunction::SRGB));
        assert_eq!(dfd.color_model, Some(::ktx2::ColorModel::BC3));
        let offsets: Vec<usize> = r
            .levels()
            .map(|l| l.data.as_ptr() as usize - b.as_ptr() as usize)
            .collect();
        assert!(offsets.iter().all(|o| o % 16 == 0), "{offsets:?}");
        assert!(offsets.windows(2).all(|w| w[0] > w[1]), "smallest last");
        let linear = write(&texture(TexFormat::Bc5, 8)).unwrap();
        let r = ::ktx2::Reader::new(&linear[..]).unwrap();
        assert_ne!(
            r.basic_dfd().unwrap().transfer_function,
            Some(::ktx2::TransferFunction::SRGB)
        );
    }

    #[test]
    fn bad_textures_are_rejected() {
        let mut t = texture(TexFormat::Bc5, 8);
        t.levels[1].pop();
        assert!(write(&t).unwrap_err().contains("level 1"));
        assert!(
            read(b"not a ktx2 file at all, sorry................................................")
                .is_err()
        );
        assert_eq!(full_chain(1024, 1024), 11);
        assert_eq!(full_chain(1, 1), 1);
        assert_eq!(TexFormat::Bc3Srgb.level_bytes(2, 2), 16);
        assert_eq!(TexFormat::Bc5.level_bytes(1024, 1024), 1024 * 1024);
    }
}
