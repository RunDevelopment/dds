use crate::{
    header::{DxgiFormat, FourCC, Header},
    Format, SizeMultiple,
};

#[derive(Debug)]
#[non_exhaustive]
pub enum FormatError {
    UnsupportedDxgiFormat(DxgiFormat),
    UnsupportedFourCC(FourCC),
    UnsupportedPixelFormat,
}
impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FormatError::UnsupportedDxgiFormat(format) => {
                write!(f, "DXGI format {:?} is not supported for decoding", format)
            }
            FormatError::UnsupportedFourCC(four_cc) => {
                write!(f, "Unsupported {:?} in DX10 header extension", four_cc)
            }
            FormatError::UnsupportedPixelFormat => {
                write!(f, "Unsupported pixel format in the DDS header")
            }
        }
    }
}
impl std::error::Error for FormatError {}

#[derive(Debug)]
#[non_exhaustive]
pub enum LayoutError {
    /// The decoder only supports up to 255 mipmaps.
    ///
    /// In practice, texture will have at most 32 mipmaps, so this limitation
    /// should only affect invalid/malicious files.
    TooManyMipMaps(u32),
    /// A volume/texture 3D without a depth.
    MissingDepth,
    /// The width, height, or depth of the texture is zero.
    ZeroDimension,
    /// The `array_size` field is too large.
    ///
    /// It either exceeds a user-defined limit or causes an overflow for cube map faces.
    ArraySizeTooBig(u32),
    /// The header of the DDS file describes a data section that is too large.
    ///
    /// I.e. it is possible for the header to describe a texture that requires >2^64
    /// bytes of memory.
    DataLayoutTooBig,
    /// The faces of a cube map must always be 2D textures.
    InvalidCubeMapFaces,
}
impl std::fmt::Display for LayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LayoutError::TooManyMipMaps(mipmaps) => {
                write!(
                    f,
                    "Too many mipmaps ({}), the maximum supported is 255",
                    mipmaps
                )
            }
            LayoutError::MissingDepth => {
                write!(f, "Missing depth for a texture 3D/volume")
            }
            LayoutError::ZeroDimension => {
                write!(f, "The width, height, or depth of the texture is zero")
            }
            LayoutError::ArraySizeTooBig(size) => {
                write!(f, "Array size {} is too large", size)
            }
            LayoutError::DataLayoutTooBig => {
                write!(f, "Data layout described by the header is too large")
            }
            LayoutError::InvalidCubeMapFaces => {
                write!(f, "Cube map faces must be 2D textures")
            }
        }
    }
}
impl std::error::Error for LayoutError {}

#[derive(Debug)]
#[non_exhaustive]
pub enum DecodeError {
    /// When decoding a rectangle, the rectangle is out of bounds of the size
    /// of the image.
    RectOutOfBounds,
    /// When decoding a rectangle, the row pitch is too small.
    ///
    /// A row pitch must be at least `color.bytes_per_pixel() * rect.width` bytes.
    RowPitchTooSmall {
        required_minimum: usize,
    },
    /// When decoding a rectangle, the buffer is too small.
    ///
    /// A buffer much have at least `row_pitch * rect.height` bytes.
    RectBufferTooSmall {
        required_minimum: usize,
    },

    /// Returned by [`crate::Decoder::read_surface`] when the user tries to
    /// decode a surface into an image that is not the same size as the
    /// surface.
    UnexpectedSurfaceSize,
    /// When decoding a volume texture, it is not allowed to skip mipmaps
    /// within a volume.
    ///
    /// See [`crate::Decoder::skip_mipmaps`] for more details.
    CannotSkipMipmapsInVolume,
    /// There are no further surfaces to decode.
    NoMoreSurfaces,
    /// This error is returned by [`crate::Decoder::read_cube_map`] when the
    /// user tries to read a DDS file that isn't a cube map.
    NotACubeMap,

    /// The decoder has exceeded its memory limit.
    MemoryLimitExceeded,

    Layout(LayoutError),
    Format(FormatError),
    Header(HeaderError),
    Io(std::io::Error),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::RectOutOfBounds => {
                write!(f, "Rectangle is out of bounds of the image size")
            }
            DecodeError::RowPitchTooSmall { required_minimum } => {
                write!(
                    f,
                    "Row pitch too small: Must be at least `color.bytes_per_pixel() * rect.width` == {} bytes",
                    required_minimum
                )
            }
            DecodeError::RectBufferTooSmall { required_minimum } => {
                write!(
                    f,
                    "Buffer too small for rectangle: required at least {} bytes",
                    required_minimum
                )
            }
            DecodeError::UnexpectedSurfaceSize => {
                write!(f, "Unexpected size of the surface")
            }
            DecodeError::CannotSkipMipmapsInVolume => {
                write!(f, "Cannot skip mipmaps within a volume texture")
            }
            DecodeError::NoMoreSurfaces => {
                write!(f, "No more surfaces to decode")
            }
            DecodeError::NotACubeMap => {
                write!(f, "The DDS file is not a cube map")
            }

            DecodeError::MemoryLimitExceeded => {
                write!(f, "Memory limit exceeded")
            }

            DecodeError::Layout(error) => write!(f, "{}", error),
            DecodeError::Format(error) => write!(f, "{}", error),
            DecodeError::Header(error) => write!(f, "Header error: {}", error),
            DecodeError::Io(error) => write!(f, "I/O error: {}", error),
        }
    }
}

impl From<LayoutError> for DecodeError {
    fn from(error: LayoutError) -> Self {
        DecodeError::Layout(error)
    }
}
impl From<FormatError> for DecodeError {
    fn from(error: FormatError) -> Self {
        DecodeError::Format(error)
    }
}
impl From<HeaderError> for DecodeError {
    fn from(error: HeaderError) -> Self {
        DecodeError::Header(error)
    }
}
impl From<std::io::Error> for DecodeError {
    fn from(error: std::io::Error) -> Self {
        DecodeError::Io(error)
    }
}

impl std::error::Error for DecodeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DecodeError::Layout(error) => Some(error),
            DecodeError::Format(error) => Some(error),
            DecodeError::Header(error) => Some(error),
            DecodeError::Io(error) => Some(error),
            _ => None,
        }
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub enum HeaderError {
    InvalidMagicBytes([u8; 4]),
    InvalidHeaderSize(u32),
    InvalidPixelFormatSize(u32),
    InvalidRgbBitCount(u32),
    InvalidDxgiFormat(u32),
    InvalidResourceDimension(u32),
    InvalidAlphaMode(u32),
    InvalidArraySizeForTexture3D(u32),

    Io(std::io::Error),
}

impl std::fmt::Display for HeaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HeaderError::InvalidMagicBytes(bytes) => {
                write!(
                    f,
                    "Invalid magic bytes {:?}, expected {:?} (ASCII: 'DDS ')",
                    bytes,
                    Header::MAGIC
                )
            }
            HeaderError::InvalidHeaderSize(size) => {
                write!(f, "Invalid DDS header size of {}, expected 124", size)
            }
            HeaderError::InvalidPixelFormatSize(size) => {
                write!(
                    f,
                    "Invalid DDS header pixel format size of {}, expected 32",
                    size
                )
            }
            HeaderError::InvalidRgbBitCount(count) => {
                write!(
                    f,
                    "Invalid DDS header pixel format rgb_bit_count of {}, expected 8, 16, 24, or 32",
                    count
                )
            }
            HeaderError::InvalidDxgiFormat(format) => {
                write!(f, "Invalid DXGI format {} in DX10 header extension", format)
            }
            HeaderError::InvalidResourceDimension(dimension) => {
                let label = match dimension {
                    0 => " (Unknown)",
                    1 => " (Buffer)",
                    2 => " (Texture1D)",
                    3 => " (Texture2D)",
                    4 => " (Texture3D)",
                    _ => "",
                };
                write!(
                    f,
                    "Invalid resource dimension {}{} in DX10 header extension",
                    dimension, label
                )
            }
            HeaderError::InvalidAlphaMode(mode) => {
                write!(f, "Invalid alpha mode {} in DX10 header extension", mode)
            }
            HeaderError::InvalidArraySizeForTexture3D(array_size) => {
                write!(
                    f,
                    "Invalid array size {} for a texture 3D in DX10 header extension",
                    array_size
                )
            }

            HeaderError::Io(error) => write!(f, "I/O error: {}", error),
        }
    }
}

impl From<std::io::Error> for HeaderError {
    fn from(error: std::io::Error) -> Self {
        HeaderError::Io(error)
    }
}
impl std::error::Error for HeaderError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            HeaderError::Io(error) => Some(error),
            _ => None,
        }
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub enum EncodeError {
    UnsupportedFormat(Format),
    InvalidSize(SizeMultiple),
    /// Returned by [`crate::encode()`] when the user tries to write a surface
    /// with width or height of 0.
    EmptySurface,

    /// Returned by [`crate::Encoder`] when the user tries to write a surface
    /// with a size that is different from the size declared in the header.
    UnexpectedSurfaceSize,
    /// Returned by [`crate::Encoder`] when the encoder has already written all
    /// surfaces declared in the header, but the user attempts to write
    /// additional surfaces.
    TooManySurfaces,
    /// Returned by [`crate::Encoder::finish()`] when the encoder has not
    /// written all surfaces declared in the header.
    MissingSurfaces,

    Layout(LayoutError),
    Io(std::io::Error),
}

impl std::fmt::Display for EncodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            EncodeError::UnsupportedFormat(format) => {
                write!(f, "Unsupported format: {:?}", format)
            }
            EncodeError::InvalidSize(size) => {
                write!(f, "Size is not a multiple of {:?}", size)
            }
            EncodeError::EmptySurface => write!(f, "Surface has a width or height of 0"),

            EncodeError::UnexpectedSurfaceSize => {
                write!(f, "Unexpected size of the surface")
            }
            EncodeError::TooManySurfaces => write!(f, "Too many surfaces are attempted to written"),
            EncodeError::MissingSurfaces => write!(f, "Not enough surfaces have been written"),

            EncodeError::Layout(err) => write!(f, "Layout error: {}", err),
            EncodeError::Io(err) => write!(f, "IO error: {}", err),
        }
    }
}
impl std::error::Error for EncodeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            EncodeError::Layout(err) => Some(err),
            EncodeError::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<LayoutError> for EncodeError {
    fn from(err: LayoutError) -> Self {
        EncodeError::Layout(err)
    }
}
impl From<std::io::Error> for EncodeError {
    fn from(err: std::io::Error) -> Self {
        EncodeError::Io(err)
    }
}
