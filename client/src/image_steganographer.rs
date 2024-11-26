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
use tracing_subscriber::fmt::format;
use std::collections::HashMap;
use std::{env, thread};

use std::io::{Read, Write};
use std::sync::{Arc, Barrier};
use serde::{Deserialize, Serialize};
use std::error::Error;
use stegano_core::{SteganoCore,SteganoEncoder, CodecOptions};
use std::io::Cursor;
use minifb::{Window, WindowOptions, Key};

#[derive(Serialize, Deserialize, Debug)]
struct AccessMetadata {
    owner_id: String,
    requester_id: String,
    access_rights: u32,
}

#[remote_trait_object_macro::service]
pub trait ImageSteganographer: Send + Sync {
    fn encode(&self, secret_image: &[u8], output_path: &str, file_name: &str) -> Result<Vec<u8>, String>;
    fn decode(&self, encoded_image: &[u8], decoded_image_path: &str, file_name: &str) -> Result<Vec<u8>, String>;
    
    fn encode_with_access_rights(&self, 
        secret_image: &[u8], 
        owner_id: &str,
        requester_id: &str,
        access_rights: u32,
        output_path: &str
    ) -> Result<Vec<u8>, String>;
    
    fn decode_with_access_check(&self, 
        encoded_image: &[u8],
        requester_id: &str
    ) -> Result<Vec<u8>, String>;
    
    fn view_decoded_image_temp(&self, 
        encoded_image: &[u8],
        requester_id: &str
    ) -> Result<(), String>;
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


    fn encode(&self, secret_image: &[u8], output_path: &str, file_name: &str) -> Result<Vec<u8>, String> {
        
        println!("Beginning Encoding");
        
        // Save the secret image to a temporary file
        let temp_secret_path = format!("/tmp/{}",file_name);
        let mut temp_secret_file = File::create(&temp_secret_path).map_err(|e| e.to_string())?;
        temp_secret_file.write_all(secret_image).map_err(|e| e.to_string())?;
        temp_secret_file.flush().map_err(|e| e.to_string())?;


        let carrier_path = "carrier.png";


        SteganoCore::encoder()
            .hide_file(&temp_secret_path)
            .use_media(&carrier_path).unwrap()
            .write_to(output_path)
            .hide();
        


        println!("Encoded image saved to {}", output_path);

        let encoded_image = image::open(output_path).unwrap();


        let mut buffer = Vec::new();
        encoded_image.write_to(&mut buffer, ImageFormat::PNG).map_err(|e| e.to_string())?;
        
        // Delete the temporary secret image file
        std::fs::remove_file(&temp_secret_path).map_err(|e| e.to_string())?;
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
        std::fs::remove_file(&temp_enc_path).map_err(|e| e.to_string())?;
        Ok(buffer)
    }

    fn encode_with_access_rights(&self,
        encoded_image: &[u8],
        owner_id: &str,
        requester_id: &str,
        access_rights: u32,
        output_path: &str
    ) -> Result<Vec<u8>, String> {
        // Create metadata
        let metadata = AccessMetadata {
            owner_id: owner_id.to_string(),
            requester_id: requester_id.to_string(),
            access_rights: access_rights,
        };
        
        // Serialize metadata to JSON
        let metadata_json = serde_json::to_string(&metadata)
            .map_err(|e| format!("Failed to serialize metadata: {}", e))?;
        
        encode_and_unveil(encoded_image, &metadata_json, requester_id)?;

        // Read the final encoded image
        println!("Reading the final encoded image from: {}", output_path);
        let final_image = image::open(output_path)
            .map_err(|e| format!("Failed to open final encoded image: {}", e))?;
        let mut buffer = Vec::new();
        final_image.write_to(&mut buffer, ImageFormat::PNG)
            .map_err(|e| format!("Failed to write final encoded image to buffer: {}", e))?;
        println!("Final encoded image read successfully.");

        Ok(buffer)
    }

    fn decode_with_access_check(&self,
        encoded_image: &[u8],
        requester_id: &str
    ) -> Result<Vec<u8>, String> {
        // Save the encoded image temporarily
        let temp_encoded_path = format!("/tmp/temp_encoded_{}.png", requester_id);
        std::fs::write(&temp_encoded_path, encoded_image)
            .map_err(|e| format!("Failed to write temp encoded image: {}", e))?;

        // First extract the metadata
        let temp_metadata_path = format!("/tmp/extracted_metadata_{}.txt", requester_id);
        println!("Extracting metadata to: {}", temp_metadata_path);
        let _result = unveil(
            &Path::new(&temp_encoded_path),
            &Path::new(&temp_metadata_path),
            &CodecOptions::default());
        println!("Metadata extraction result: {:?}", _result);

        // Read and parse metadata
        let metadata_str = std::fs::read_to_string(&temp_metadata_path)
            .map_err(|e| format!("Failed to read metadata: {}", e))?;
        let mut metadata: AccessMetadata = serde_json::from_str(&metadata_str)
            .map_err(|e| format!("Failed to parse metadata: {}", e))?;

        // Check access rights
        if metadata.requester_id != requester_id {
            return Err("Unauthorized access: requester ID mismatch".to_string());
        }
        if metadata.access_rights == 0 {
            return Err("No more access rights remaining".to_string());
        }

        // Decode the actual image
        let temp_output_path = format!("/tmp/decoded_{}.png", requester_id);
        let _result = unveil(
            &Path::new(&temp_encoded_path),
            &Path::new(&temp_output_path),
            &CodecOptions::default());

        // Decrement access rights and save updated metadata
        metadata.access_rights -= 1;
        let updated_metadata = serde_json::to_string(&metadata)
            .map_err(|e| format!("Failed to serialize updated metadata: {}", e))?;
        std::fs::write(&temp_metadata_path, updated_metadata)
            .map_err(|e| format!("Failed to write updated metadata: {}", e))?;

        // Re-encode with updated metadata
        let final_encoded_path = format!("/tmp/final_encoded_{}.png", requester_id);
        SteganoCore::encoder()
            .hide_file(&temp_metadata_path)
            .use_media(&temp_output_path).unwrap()
            .write_to(&final_encoded_path)
            .hide();

        // Read the decoded image into memory
        let decoded_image = image::open(&temp_output_path)
            .map_err(|e| format!("Failed to open decoded image: {}", e))?;
        let mut buffer = Vec::new();
        decoded_image.write_to(&mut buffer, ImageFormat::PNG)
            .map_err(|e| format!("Failed to write to buffer: {}", e))?;

        // Cleanup temporary files
        std::fs::remove_file(&temp_encoded_path).ok();
        std::fs::remove_file(&temp_metadata_path).ok();
        std::fs::remove_file(&temp_output_path).ok();
        std::fs::remove_file(&final_encoded_path).ok();

        Ok(buffer)
    }

    fn view_decoded_image_temp(&self, 
        encoded_image: &[u8],
        requester_id: &str
    ) -> Result<(), String> {
        // First decode the image and check access rights
        let decoded_buffer = self.decode_with_access_check(encoded_image, requester_id)?;
        
        // Load the image from memory
        let img = image::load_from_memory(&decoded_buffer)
            .map_err(|e| format!("Failed to load decoded image: {}", e))?;
        
        // Convert to RGB
        let rgb_img = img.to_rgb();
        let (width, height) = rgb_img.dimensions();
        
        // Convert image data to u32 buffer for minifb
        let buffer: Vec<u32> = rgb_img.pixels()
            .map(|p| {
                let (r, g, b) = (p[0] as u32, p[1] as u32, p[2] as u32);
                (r << 16) | (g << 8) | b
            })
            .collect();

        // Create window
        let mut window = Window::new(
            "Temporary Image Viewer - Press Escape to close",
            width as usize,
            height as usize,
            WindowOptions {
                resize: true,
                scale: minifb::Scale::X1,
                ..WindowOptions::default()
            },
        ).map_err(|e| format!("Failed to create window: {}", e))?;

        // Set window update speed
        window.limit_update_rate(Some(std::time::Duration::from_micros(16600)));

        // Update window while it's open
        while window.is_open() && !window.is_key_down(Key::Escape) {
            window
                .update_with_buffer(&buffer, width as usize, height as usize)
                .map_err(|e| format!("Failed to update window: {}", e))?;
        }

        Ok(())
    }
}

fn encode_and_unveil(encoded_image: &[u8], metadata_json: &str, requester_id: &str) -> Result<(), String> {
    // Create a temporary file for the metadata
    let temp_metadata_path = format!("/tmp/metadata_{}.txt", requester_id);
    println!("Creating metadata file at: {}", temp_metadata_path);
    std::fs::write(&temp_metadata_path, metadata_json)
        .map_err(|e| format!("Failed to write metadata: {}", e))?;
    println!("Metadata file created successfully.");

    // Save the already encoded image to a temporary file
    let temp_encoded_path = format!("/tmp/encoded_{}.png", requester_id);
    println!("Creating encoded image file at: {}", temp_encoded_path);
    std::fs::write(&temp_encoded_path, encoded_image)
        .map_err(|e| format!("Failed to write encoded image: {}", e))?;
    println!("Encoded image file created successfully.");

    // Encode the metadata into the already encoded image
    let final_encoded_path = format!("/tmp/final_encoded_{}.png", requester_id);
    println!("Encoding metadata into the encoded image.");
    SteganoCore::encoder()
        .hide_file(&temp_metadata_path)
        .use_media(&temp_encoded_path).unwrap()
        .write_to(&final_encoded_path)
        .hide();
    println!("Metadata encoded successfully into the image.");

    // Verify that the files exist before unveiling
    if !Path::new(&final_encoded_path).exists() {
        return Err(format!("Final encoded image does not exist: {}", final_encoded_path));
    }
    if !Path::new(&temp_metadata_path).exists() {
        return Err(format!("Metadata file does not exist: {}", temp_metadata_path));
    }

    // Unveil the metadata from the encoded image
    let _result = unveil(
        &Path::new(&final_encoded_path),
        &Path::new(&temp_metadata_path),
        &CodecOptions::default());
    println!("Metadata extraction result: {:?}", _result);

    Ok(())
}