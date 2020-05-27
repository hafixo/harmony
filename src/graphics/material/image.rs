use std::{fs, io, path::PathBuf, sync::Arc};
use serde::{ Deserialize, Serialize };
use io::ErrorKind;

#[derive(Eq, PartialEq, Hash, Debug, Copy, Clone, Serialize, Deserialize)]
pub enum ImageFormat {
    SRGB,
    RGB,
    HDR16,
    HDR32,
}

impl Into<wgpu::TextureFormat> for ImageFormat {
    fn into(self) -> wgpu::TextureFormat {
        match self {
            ImageFormat::HDR16 => wgpu::TextureFormat::Rgba16Float,
            ImageFormat::HDR32 => wgpu::TextureFormat::Rgba32Float,
            ImageFormat::RGB => wgpu::TextureFormat::Rgba8Unorm,
            ImageFormat::SRGB => wgpu::TextureFormat::Rgba8UnormSrgb,
        }
    }
}

#[derive(Eq, PartialEq, Hash, Debug, Clone, Serialize, Deserialize)]
pub struct ImageInfo {
    /// Relative to where the ron file is located.
    pub file: String, 
    pub format: ImageFormat,
}

pub(crate) struct ImageBuilder {
    pub bytes: Vec<u8>,
}

fn create_texture(device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, width: u32, height: u32, format: wgpu::TextureFormat, bytes: Vec<u8>) -> (wgpu::Texture, wgpu::Sampler, wgpu::Extent3d) {
    let texture_extent = wgpu::Extent3d {
        width,
        height,
        depth: 1,
    };
    
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        size: texture_extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsage::SAMPLED | wgpu::TextureUsage::COPY_DST,
        label: None,
    });

    let temp_buf = device.create_buffer_with_data(&bytes, wgpu::BufferUsage::COPY_SRC);

    encoder.copy_buffer_to_texture(
        wgpu::BufferCopyView {
            buffer: &temp_buf,
            layout: wgpu::TextureDataLayout {
                offset: 0,
                // TODO: Figure out a better method of detecting bytes per row.
                bytes_per_row: if format == wgpu::TextureFormat::Rgba8UnormSrgb
                    || format == wgpu::TextureFormat::Rgba8Unorm
                {
                    4 * texture_extent.width
                } else {
                    (4 * 4) * texture_extent.width
                },
                rows_per_image: 0,
            }
        },
        wgpu::TextureCopyView {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
        },
        texture_extent,
    );

    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: None,
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        address_mode_w: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Linear,
        lod_min_clamp: -100.0,
        lod_max_clamp: 100.0,
        compare: wgpu::CompareFunction::Undefined,
    });

    (texture, sampler, texture_extent)
}

impl ImageBuilder {
    pub fn build(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, image_info: Arc<ImageInfo>) -> Image {
        let (image_bytes, width, height) = match image_info.format {
            ImageFormat::HDR16 |
            ImageFormat::HDR32 => {
                let decoder = image::hdr::HdrDecoder::new(self.bytes.as_slice()).unwrap();
                let metadata = decoder.metadata();
                let decoded = decoder.read_image_hdr().unwrap();

                let image_data = decoded
                    .iter()
                    .flat_map(|pixel| vec![pixel[0], pixel[1], pixel[2], 1.0])
                    .collect::<Vec<_>>();

                let image_bytes = unsafe {
                    std::slice::from_raw_parts(image_data.as_ptr() as *const u8, image_data.len() * 4)
                }
                .to_vec();

                (image_bytes, metadata.width, metadata.height)
            },
            ImageFormat::RGB | ImageFormat::SRGB => {
                let image = image::load_from_memory(&self.bytes).unwrap().to_rgba();
                let (width, height) = image.dimensions();

                (image.into_raw(), width, height)
            },
            _ => panic!(""),
        };

        let format: wgpu::TextureFormat = image_info.format.into();

        let (texture, sampler, extent) = create_texture(device, encoder, width, height, format, image_bytes);

        let view = texture.create_default_view();

        Image {
            image_info,
            extent,
            texture,
            sampler,
            view,
            format,
        }
    }
}

pub struct Image {
    pub image_info: Arc<ImageInfo>, 
    pub extent: wgpu::Extent3d,
    pub texture: wgpu::Texture,
    pub sampler: wgpu::Sampler,
    pub view: wgpu::TextureView,
    pub format: wgpu::TextureFormat,
}

impl Image {
    pub fn new<T>(
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        path: T,
        file_name: T,
    ) -> Self
    where
        T: Into<String>,
    {
        let path = path.into();

        let (image_bytes, texture_extent, format) = if path.ends_with(".hdr") {
            Self::create_hdr_image(path)
        } else if path.to_lowercase().contains("_normal")
            || path.to_lowercase().contains("metallic")
        {
            Self::create_normal_image(path)
        } else {
            Self::create_color_image(path)
        };

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            size: texture_extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsage::SAMPLED | wgpu::TextureUsage::COPY_DST,
            label: None,
        });

        let temp_buf = device.create_buffer_with_data(&image_bytes, wgpu::BufferUsage::COPY_SRC);

        encoder.copy_buffer_to_texture(
            wgpu::BufferCopyView {
                buffer: &temp_buf,
                layout: wgpu::TextureDataLayout {
                    offset: 0,
                    bytes_per_row: if format == wgpu::TextureFormat::Rgba8UnormSrgb
                        || format == wgpu::TextureFormat::Rgba8Unorm
                    {
                        4 * texture_extent.width
                    } else {
                        (4 * 4) * texture_extent.width
                    },
                    rows_per_image: 0,
                },
            },
            wgpu::TextureCopyView {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
            },
            texture_extent,
        );

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: None,
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            lod_min_clamp: -100.0,
            lod_max_clamp: 100.0,
            compare: wgpu::CompareFunction::Undefined,
        });

        let view = texture.create_default_view();

        let file_name =  file_name.into();
        Self {
            image_info: Arc::new(ImageInfo {
                file: file_name.clone(),
                format: ImageFormat::SRGB,
            }),
            extent: texture_extent,
            texture,
            sampler,
            view,
            format,
        }
    }

    fn create_normal_image(path: String) -> (Vec<u8>, wgpu::Extent3d, wgpu::TextureFormat) {
        let img = image::open(&path)
            .unwrap_or_else(|_| panic!("Image: Unable to open the file: {}", path))
            .to_rgba();
        let (width, height) = img.dimensions();
        let texture_extent = wgpu::Extent3d {
            width,
            height,
            depth: 1,
        };

        let image_bytes: Vec<u8> = img.into_raw();

        (image_bytes, texture_extent, wgpu::TextureFormat::Rgba8Unorm)
    }

    fn create_color_image(path: String) -> (Vec<u8>, wgpu::Extent3d, wgpu::TextureFormat) {
        let img = image::open(&path)
            .unwrap_or_else(|_| panic!("Image: Unable to open the file: {}", path))
            .to_rgba();
        let (width, height) = img.dimensions();
        let texture_extent = wgpu::Extent3d {
            width,
            height,
            depth: 1,
        };

        let image_bytes: Vec<u8> = img.into_raw();

        // TODO: Fix loading of images. We should use SRGB for textures and Unorm for roughness/normal maps/etc.
        // Should be done with a material loader perhaps?
        (
            image_bytes,
            texture_extent,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        )
    }

    fn create_hdr_image(path: String) -> (Vec<u8>, wgpu::Extent3d, wgpu::TextureFormat) {
        // Load the image
        let decoder =
            image::hdr::HdrDecoder::new(io::BufReader::new(fs::File::open(&path).unwrap()))
                .unwrap();
        let metadata = decoder.metadata();
        let decoded = decoder.read_image_hdr().unwrap();

        let (w, h) = (metadata.width, metadata.height);

        let texture_extent = wgpu::Extent3d {
            width: w,
            height: h,
            depth: 1,
        };

        let image_data = decoded
            .iter()
            .flat_map(|pixel| vec![pixel[0], pixel[1], pixel[2], 1.0])
            .collect::<Vec<_>>();

        let image_bytes = unsafe {
            std::slice::from_raw_parts(image_data.as_ptr() as *const u8, image_data.len() * 4)
        }
        .to_vec();

        (
            image_bytes,
            texture_extent,
            wgpu::TextureFormat::Rgba32Float,
        )
    }
}

impl assetmanage_rs::Asset for ImageBuilder {
    fn decode(bytes: &[u8]) -> Result<Self, io::Error> {
        Ok(ImageBuilder {
            bytes: bytes.to_vec(),
        })
    }
}

impl assetmanage_rs::Asset for ImageInfo {
    fn decode(bytes: &[u8]) -> Result<Self, io::Error> {
        ron::de::from_bytes::<ImageInfo>(bytes)
            .map_err(|e| std::io::Error::new(ErrorKind::InvalidData, e))
    }
}
