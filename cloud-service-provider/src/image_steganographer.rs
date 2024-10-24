use std::path::Path;
use image::{DynamicImage, GenericImageView, ImageBuffer, ImageFormat, Rgba};
use steganography::{encoder::*, decoder::*};

pub struct ImageSteganographer {
    compression_quality: u8,  // For JPEG output (1-100)
    max_pixel_diff: u8,      // Max RGB difference allowed per pixel
}

impl ImageSteganographer {
    pub fn new(compression_quality: u8, max_pixel_diff: u8) -> Self {
        Self {
            compression_quality: compression_quality.clamp(1, 100),
            max_pixel_diff: max_pixel_diff.clamp(1, 255),
        }
    }

    pub fn hide_image(&self, carrier_path: &str, secret_path: &str, output_path: &str) 
        -> Result<std::fs::File, Box<dyn std::error::Error>> {
        // Load images and validate dimensions
        let carrier = image::open(carrier_path)?;
        let secret = image::open(secret_path)?;
        
        // Calculate required capacity
        let (secret_width, secret_height) = secret.dimensions();
        let (carrier_width, carrier_height) = carrier.dimensions();
        
        // Check if carrier is large enough (needs 8x pixels for RGB data)
        if carrier_width * carrier_height < (secret_width * secret_height * 8) {
            return Err("Carrier image too small".into());
        }

        // Prepare secret image data
        let mut secret_data = Vec::new();
        
        // Store dimensions for later recovery
        secret_data.extend_from_slice(&secret_width.to_le_bytes());
        secret_data.extend_from_slice(&secret_height.to_le_bytes());
        
        // Convert secret image to bytes with optimized encoding
        let secret_bytes = self.optimize_image_data(&secret)?;
        secret_data.extend_from_slice(&secret_bytes);

        // Create and configure encoder
        let encoder = Encoder::new(&secret_data, carrier);
        
        // Use alpha channel encoding for better quality
        let encoded = encoder.encode_alpha();

        // Save with appropriate format and settings
        self.save_encoded_image(&encoded, output_path)?;

        // Return the file object
        let file = std::fs::File::open(output_path)?;
        Ok(file)
    }

    pub fn reveal_image(&self, encoded_path: &str, output_path: &str) 
        -> Result<(), Box<dyn std::error::Error>> {
        let encoded = image::open(encoded_path)?;
        let decoder = Decoder::new(encoded.to_rgba());
        
        let decoded_data = decoder.decode_alpha();
        
        // Extract dimensions (first 8 bytes)
        let width = u32::from_le_bytes(decoded_data[0..4].try_into()?);
        let height = u32::from_le_bytes(decoded_data[4..8].try_into()?);
        
        // Reconstruct image from remaining data
        let img_data = &decoded_data[8..];
        let reconstructed = self.reconstruct_image(img_data, width, height)?;
        
        // Save with original dimensions
        reconstructed.save(output_path)?;
        
        Ok(())
    }

    fn optimize_image_data(&self, image: &DynamicImage) 
        -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let mut optimized = Vec::new();
        
        // Convert to RGB and apply delta encoding
        for pixel in image.to_rgb().pixels() {
            for &channel in pixel.data.iter() {
                let adjusted = channel - (channel % self.max_pixel_diff);
                optimized.push(adjusted);
            }
        }
        
        Ok(optimized)
    }

    fn save_encoded_image(
        &self,
        image: &ImageBuffer<Rgba<u8>, Vec<u8>>,
        path: &str
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Handle the Result returned by guess_format
        let format = image::guess_format(image)?;

        match format {
            ImageFormat::PNG => {
                image.save(path)?;
            },
            ImageFormat::JPEG => {
                // Convert to RGB and save with quality setting
                let rgb = DynamicImage::ImageRgba8(image.clone()).to_rgb();
                rgb.save(path);
            },
            _ => return Err("Unsupported output format".into())
        }

        Ok(())
    }

    fn reconstruct_image(
        &self,
        data: &[u8],
        width: u32,
        height: u32
    ) -> Result<DynamicImage, Box<dyn std::error::Error>> {
        let mut img_buffer = ImageBuffer::new(width, height);
        
        let mut i = 0;
        for y in 0..height {
            for x in 0..width {
                if i + 2 >= data.len() {
                    break;
                }
                
                let pixel = Rgba([
                    data[i],
                    data[i + 1],
                    data[i + 2],
                    255
                ]);
                
                img_buffer.put_pixel(x, y, pixel);
                i += 3;
            }
        }
        
        Ok(DynamicImage::ImageRgba8(img_buffer))
    }
}