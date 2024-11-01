use std::path::Path;
use std::fs::File;
use image::{DynamicImage, GenericImageView, ImageBuffer, ImageFormat, Rgb, Rgba};
use steganography::{encoder::*, decoder::*};
use remote_trait_object::*;
use remote_trait_object_macro::service;
use serde;
use serde_json;

use crossbeam::channel::{unbounded, Receiver, Sender};
use std::collections::HashMap;
use std::{env, thread};

use std::io::{Read, Write};
use std::sync::{Arc, Barrier};
use serde::{Deserialize, Serialize};
use std::error::Error;



#[remote_trait_object_macro::service]
pub trait ImageSteganographer: Send + Sync {
    //fn new(&self, compression_quality: u8, max_pixel_diff: u8) -> Self;
    fn encode(&self, secret_path: &str, carrier_path: &str, output_path: &str) -> Result<Vec<u8>, String>;
    fn decode(&self, encoded_image_path: &str, decoded_image_path: &str) -> Result<Vec<u8>, String>;
}
impl Service for dyn ImageSteganographer {}

pub struct SomeImageSteganographer {
    compression_quality: u8,  // For JPEG output (1-100)
    max_pixel_diff: u8,      // Max RGB difference allowed per pixel
}

impl SomeImageSteganographer {
    pub fn new(compression_quality: u8, max_pixel_diff: u8) -> Self {
        Self {
            compression_quality: compression_quality.clamp(1, 100),
            max_pixel_diff: max_pixel_diff.clamp(1, 255),
        }
    }
}



impl ImageSteganographer for SomeImageSteganographer {


    fn encode(&self, secret_path: &str, carrier_path: &str, output_path: &str) -> Result<Vec<u8>, String> {
        let secret = File::open(secret_path).map_err(|e| e.to_string())?;
        let carrier = image::open(carrier_path).map_err(|e| e.to_string())?;

        let carrier = if carrier_path.ends_with(".jpg") || carrier_path.ends_with(".jpeg") {
            let mut buffer = Vec::new();
            carrier.write_to(&mut buffer, ImageFormat::PNG).map_err(|e| e.to_string())?;
            let carrier_bytes = buffer;
            image::load_from_memory(&carrier_bytes).unwrap()
        } else {
            carrier
        };
        
        let secret_bytes = steganography::util::file_to_bytes(secret);
        let secret_bytes: &[u8] = &secret_bytes;
        
        let encoder = Encoder::new(secret_bytes,carrier);
        
        let encoded_buffer = encoder.encode_alpha();
        let encoded_image = DynamicImage::ImageRgba8(ImageBuffer::from_raw(encoded_buffer.width(), encoded_buffer.height(), encoded_buffer.into_vec()).unwrap());

        encoded_image.save(output_path).unwrap();
        println!("Encoded image saved to {}", output_path);


        let mut buffer = Vec::new();
        encoded_image.write_to(&mut buffer, ImageFormat::PNG).map_err(|e| e.to_string())?;
        Ok(buffer)
    }


    fn decode(&self, encoded_image_path: &str, decoded_image_path: &str) -> Result<Vec<u8>, String> {
        
        let encoded_image = image::open(encoded_image_path).map_err(|e| e.to_string())?;

        let encoded_bytes = encoded_image.to_rgba();
        let decoder = Decoder::new(encoded_bytes);
        let decoded_bytes = decoder.decode_alpha();
        let decoded_bytes: &[u8]= &decoded_bytes;
        
        let decoded_image = image::load_from_memory(decoded_bytes).unwrap();
        decoded_image.save(decoded_image_path).map_err(|e| e.to_string())?;
        println!("Decoded image saved to {}", decoded_image_path);
        let mut buffer = Vec::new();
        decoded_image.write_to(&mut buffer, ImageFormat::PNG).map_err(|e| e.to_string())?;
        Ok(buffer)
    }
}
