use std::io::Write;

use crate::{
    header::Header,
    iter::{SurfaceInfo, SurfaceIterator},
    resize::{Aligner, ResizeState},
    split_encode, ColorFormat, DataLayout, EncodeError, EncodeOptions, Format, ImageView, Size,
};

pub struct Encoder<W> {
    writer: W,
    format: Format,
    layout: DataLayout,
    iter: SurfaceIterator,
    pub options: EncodeOptions,
    resize: Option<Box<(Aligner, ResizeState)>>,
}
impl<W> Encoder<W> {
    pub fn new(mut writer: W, format: Format, header: &Header) -> Result<Self, EncodeError>
    where
        W: Write,
    {
        if format.encoding_support().is_none() {
            return Err(EncodeError::UnsupportedFormat(format));
        }

        let layout = DataLayout::from_header_with(header, format.into())?;

        header.write(&mut writer)?;

        Ok(Self {
            writer,
            format,
            layout,
            iter: SurfaceIterator::new(layout),
            options: EncodeOptions::default(),
            resize: None,
        })
    }

    pub fn format(&self) -> Format {
        self.format
    }
    pub fn layout(&self) -> DataLayout {
        self.layout
    }

    /// The size of the level 0 object.
    ///
    /// For single textures and texture arrays, this will return the size of the
    /// texture (mipmap level 0). For cube maps, this will return the size of
    /// the individual faces (mipmap level 0). For volume textures, this will
    /// return the size of the first depth slice (mipmap level 0).
    pub fn main_size(&self) -> Size {
        self.layout.main_size()
    }
    /// The native color of the DDS file.
    ///
    /// See [`Format::precision`] for more information about the precision of
    /// the color format.
    pub fn native_color(&self) -> ColorFormat {
        self.format().color()
    }

    pub fn into_writer(self) -> W {
        self.writer
    }

    /// Finishes writing the DDS file.
    ///
    /// This will verify that all surfaces have been written and flush the
    /// writer.
    pub fn finish(mut self) -> Result<(), EncodeError>
    where
        W: Write,
    {
        if self.surface_info().is_some() {
            return Err(EncodeError::MissingSurfaces);
        }
        self.writer.flush()?;
        Ok(())
    }

    /// Returns information about the surface about to be written.
    ///
    /// The returned value is not valid after calling `write_surface`.
    ///
    /// If there are no more surfaces, `None` is returned.
    pub fn surface_info(&self) -> Option<SurfaceInfo<'_>> {
        self.iter.current()
    }

    /// Writes the next surface.
    ///
    /// The next surface is determined by the data layout of the DDS file. For
    /// volume textures, this function will write the next depth slice.
    ///
    /// See [`Self::surface_info`] for more information about the surface.
    pub fn write_surface(&mut self, image: ImageView) -> Result<(), EncodeError>
    where
        W: Write,
    {
        self.write_surface_impl(image, ProgressToken::none(), &WriteOptions::default())
    }

    /// Writes the next surface.
    ///
    /// The next surface is determined by the data layout of the DDS file. For
    /// volume textures, this function will write the next depth slice.
    ///
    /// See [`Self::surface_info`] for more information about the surface.
    pub fn write_surface_with(
        &mut self,
        image: ImageView,
        mut progress: impl FnMut(f32),
        options: &WriteOptions,
    ) -> Result<(), EncodeError>
    where
        W: Write,
    {
        self.write_surface_impl(image, ProgressToken::new(&mut progress), options)
    }

    fn write_surface_impl(
        &mut self,
        image: ImageView,
        mut progress: ProgressToken,
        options: &WriteOptions,
    ) -> Result<(), EncodeError>
    where
        W: Write,
    {
        progress.report(0.0);

        let current = self.iter.current().ok_or(EncodeError::TooManySurfaces)?;
        if current.size() != image.size() {
            return Err(EncodeError::UnexpectedSurfaceSize);
        }
        split_encode(&mut self.writer, image, self.format, &self.options)?;
        self.iter.advance();

        if options.generate_mipmaps
            && self.layout.volume().is_none()
            && self.iter.current().map_or(false, |c| c.is_mipmap())
        {
            let (align, resize) = Self::get_or_init(&mut self.resize);
            let src = align.align(image);

            let mut count = 0;
            while let Some(current) = self.iter.current() {
                if !current.is_mipmap() {
                    debug_assert!(count > 0);
                    break;
                }

                count += 1;
                progress.report(1.0 - 0.3_f32.powi(count));

                let mipmap_size = current.size();
                let mip_data = resize.resize(
                    &src,
                    mipmap_size,
                    options.resize_straight_alpha,
                    options.resize_filter,
                );
                let mip =
                    ImageView::new(mip_data, mipmap_size, image.color).expect("invalid mipmap");

                split_encode(&mut self.writer, mip, self.format, &self.options)?;
                self.iter.advance();
            }
        }

        progress.report(1.0);

        Ok(())
    }

    fn get_or_init(
        resize: &mut Option<Box<(Aligner, ResizeState)>>,
    ) -> &mut (Aligner, ResizeState) {
        if resize.is_none() {
            *resize = Some(Box::new((Aligner::new(), ResizeState::new())));
        }
        resize.as_mut().unwrap()
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ResizeFilter {
    Nearest,
    Box,
    Triangle,
    Mitchell,
    Lanczos3,
}
impl Default for ResizeFilter {
    fn default() -> Self {
        Self::Box
    }
}

#[derive(Debug, Clone, Copy)]
pub struct WriteOptions {
    /// Whether to generate mipmaps for the texture.
    ///
    /// Since the encoder knows exactly how many mipmaps are needed, it will
    /// generate all mipmaps until the next level 0 object or EOF.
    ///
    /// Note: Generating mipmaps for volume depth slices is not supported. This
    /// will **NOT** result in an error and instead the encoder will silently
    /// ignore the option.
    ///
    /// Default: `false`
    pub generate_mipmaps: bool,
    /// Whether the alpha channel (if any) is straight alpha.
    ///
    /// This is important when generating mipmaps. Resizing RGBA with straight
    /// alpha requires that the alpha channel is premultiplied into the color
    /// channels before resizing and then unpremultiplied after resizing. This
    /// is necessary to avoid color bleeding.
    ///
    /// If the alpha channel is premultiplied alpha or custom (e.g. like in
    /// channel-packed textures), this option should be set to `false`.
    ///
    /// If this option is set to `false`, all channels will be resized
    /// independently of each other.
    ///
    /// Default: `true`
    pub resize_straight_alpha: bool,
    /// The filter to use when resizing the texture to generate mipmaps.
    ///
    /// Default: [`ResizeFilter::Box`]
    pub resize_filter: ResizeFilter,
}
impl Default for WriteOptions {
    fn default() -> Self {
        Self {
            generate_mipmaps: false,
            resize_straight_alpha: true,
            resize_filter: ResizeFilter::Box,
        }
    }
}

struct ProgressToken<'a> {
    reporter: Option<&'a mut dyn FnMut(f32)>,
    offset: f32,
    scale: f32,
}
impl<'a> ProgressToken<'a> {
    fn none() -> Self {
        Self {
            reporter: None,
            offset: 0.0,
            scale: 1.0,
        }
    }

    fn new(reporter: &'a mut dyn FnMut(f32)) -> Self {
        Self {
            reporter: Some(reporter),
            offset: 0.0,
            scale: 1.0,
        }
    }

    pub fn report(&mut self, mut progress: f32) {
        if let Some(reporter) = &mut self.reporter {
            progress = self.offset + progress * self.scale;
            (reporter)(progress);
        }
    }
}
