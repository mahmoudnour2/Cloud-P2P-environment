use std::path::Path;
use std::fs::File;
use image::{DynamicImage, GenericImageView, ImageBuffer, ImageFormat, Rgb, Rgba};
use stegano_core::commands::unveil;
use steganography::util::{file_as_dynamic_image, save_image_buffer};
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
use stegano_core::{SteganoCore,SteganoEncoder, CodecOptions};





#[remote_trait_object_macro::service]
pub trait ImageSteganographer: Send + Sync {
    //fn new(&self, compression_quality: u8, max_pixel_diff: u8) -> Self;
    fn encode(&self, secret_image: &[u8], output_path: &str) -> Result<Vec<u8>, String>;
    fn decode(&self, encoded_image: &[u8], decoded_image_path: &str, file_name: &str) -> Result<Vec<u8>, String>;
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


    fn encode(&self, secret_image: &[u8], output_path: &str) -> Result<Vec<u8>, String> {
        
        println!("Beginning Encoding");

        
        // Save the secret image to a temporary file
        let temp_secret_path = "/tmp/secret.jpg";
        let mut temp_secret_file = File::create(temp_secret_path).map_err(|e| e.to_string())?;
        temp_secret_file.write_all(secret_image).map_err(|e| e.to_string())?;
        temp_secret_file.flush().map_err(|e| e.to_string())?;

        // Load the secret image from the temporary file
        //let secret_image = File::open(temp_secret_path).map_err(|e| e.to_string())?;
        

        // Load the carrier image
        //let carrier_path = "/home/magdeldin/Cloud-P2P-environment/service_provider/carrier.jpg";
        let carrier_path = "carrier.png";
        //let carrier = file_as_dynamic_image(carrier_path.to_string());

        // let carrier = if carrier_path.ends_with(".jpg") || carrier_path.ends_with(".jpeg") {
        //     let mut buffer = Vec::new();
        //     carrier.write_to(&mut buffer, ImageFormat::PNG).map_err(|e| e.to_string())?;
        //     let carrier_bytes = buffer;
        //     image::load_from_memory(&carrier_bytes).unwrap()
        // } else {
        //     carrier
        // };

        SteganoCore::encoder()
            .hide_file(&temp_secret_path)
            .use_media(&carrier_path).unwrap()
            .write_to(output_path)
            .hide();
        
        // let encoder = Encoder::new(secret_image,carrier);
        
        // let encoded_buffer = encoder.encode_alpha();
        
        // save_image_buffer(encoded_buffer.clone(), output_path.to_string());


        // let encoded_image = DynamicImage::ImageRgba8(ImageBuffer::from_raw(encoded_buffer.width(), encoded_buffer.height(), encoded_buffer.into_vec()).unwrap());

        // encoded_image.save(output_path).unwrap();
        println!("Encoded image saved to {}", output_path);

        let encoded_image = image::open(output_path).unwrap();


        let mut buffer = Vec::new();
        encoded_image.write_to(&mut buffer, ImageFormat::PNG).map_err(|e| e.to_string())?;
        // Delete the temporary secret image file
        //std::fs::remove_file(temp_secret_path).map_err(|e| e.to_string())?;
        println!("Buffer length: {}", buffer.len());
        
        Ok(buffer)
    }


    fn decode(&self, encoded_image: &[u8], decoded_image_path: &str, file_name: &str) -> Result<Vec<u8>, String> {
        
        let encoded_image = image::load_from_memory(encoded_image).unwrap();
        // let encoded_bytes = encoded_image.to_rgba();
        // let decoder = Decoder::new(encoded_bytes);
        // let decoded_bytes = decoder.decode_alpha();
        // let decoded_bytes: &[u8]= &decoded_bytes;
        
        // let decoded_image = image::load_from_memory(decoded_bytes).unwrap();
        // decoded_image.save(decoded_image_path).map_err(|e| e.to_string())?;

        // Save the encoded image to a temporary file
        let temp_enc_path = "/tmp/encoded_image.png";
        encoded_image.save(temp_enc_path).unwrap();
        let _result = unveil(
            &Path::new(temp_enc_path),
            &Path::new(decoded_image_path),
            &CodecOptions::default());

        println!("Decoded image saved to {}", decoded_image_path);
        
        let new_decoded_image_path = decoded_image_path.to_owned()+"/"+file_name;
        let decoded_image = image::open(new_decoded_image_path).unwrap();
        
        let mut buffer = Vec::new();
        decoded_image.write_to(&mut buffer, ImageFormat::PNG).map_err(|e| e.to_string())?;
        // Delete the temporary encoded image file
        std::fs::remove_file(temp_enc_path).map_err(|e| e.to_string())?;
        Ok(buffer)
    }
}