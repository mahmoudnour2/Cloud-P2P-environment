use std::path::Path;
use std::fs::File;
use image::FilterType::Lanczos3;
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
use tempfile::NamedTempFile;

#[derive(Serialize, Deserialize, Debug)]
struct AccessMetadata {
    owner_id: String,
    requester_id: String,
    access_rights: u32,
    file_name: String,
}

#[remote_trait_object_macro::service]
pub trait ImageSteganographer: Send + Sync {
    fn encode(&self, secret_image: &[u8], output_path: &str, file_name: &str, owner_id: &str) -> Result<Vec<u8>, String>;
    //fn decode(&self, encoded_image: &[u8], decoded_image_path: &str, file_name: &str) -> Result<Vec<u8>, String>;
    
    fn encode_with_access_rights(&self, 
        secret_image: &[u8], 
        owner_id: &str,
        requester_id: &str,
        access_rights: u32,
        output_path: &str
    ) -> Result<Vec<u8>, String>;
    
    fn decode_with_access_check(&self, 
        encoded_image: &[u8],
        requester_id: &str,
        output_path: &str
    ) -> Result<Vec<u8>, String>;
    
    fn view_decoded_image_temp(&self, 
        encoded_image: &[u8],
        requester_id: &str,
        output_path: &str
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


    fn encode(&self, secret_image: &[u8], output_path: &str, file_name: &str, owner_id: &str) -> Result<Vec<u8>, String> {
        
        println!("Beginning Encoding");
        


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

    // fn decode(&self, encoded_image: &[u8], decoded_file_path: &str, _file_name: &str) -> Result<Vec<u8>, String> {
    //     let encoded_image = image::load_from_memory(encoded_image).map_err(|e| format!("Failed to load image from memory: {}", e))?;
        
    //     // Ensure the directory for the temporary file exists
    //     let temp_enc_path = "tmp/encoded_image.png";
    //     if let Some(parent) = Path::new(temp_enc_path).parent() {
    //         std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create directory: {}", e))?;
    //     }
    //     encoded_image.save(temp_enc_path).map_err(|e| format!("Failed to save encoded image: {}", e))?;

    //     let mut trimmed_decoded_file_path = decoded_file_path.trim_end_matches(".png");
    //     trimmed_decoded_file_path = trimmed_decoded_file_path.trim_end_matches(".jpg");
    //     let formatted_decoded_file_path = format!("{}.txt", trimmed_decoded_file_path);
    //     println!("Formatted decoded file path: {}", formatted_decoded_file_path);

    //     // Create a temporary file for the decoded text
    //     let mut temp_decoded_file = NamedTempFile::new().map_err(|e| format!("Failed to create temporary file: {}", e))?;

    //     // Extract metadata
    //     println!("Extracting metadata from the encoded image.");
    //     let result = unveil(
    //         &Path::new(&temp_enc_path),
    //         temp_decoded_file.path(),
    //         &CodecOptions::default(),
    //     ).map_err(|e| format!("Failed to extract metadata: {}", e));
    //     println!("Metadata extraction result: {:?}", result);

    //     if let Err(e) = result {
    //         return Err(format!("Failed to extract metadata: {}", e));
    //     }

    //     // Read and parse metadata
    //     let mut metadata_str = String::new();
    //     temp_decoded_file.read_to_string(&mut metadata_str).map_err(|e| format!("Failed to read metadata: {}", e))?;
    //     let metadata: AccessMetadata = serde_json::from_str(&metadata_str).map_err(|e| format!("Failed to parse metadata: {}", e))?;

    //     // Write the metadata to the final file
    //     let mut final_file = File::create(&formatted_decoded_file_path).map_err(|e| format!("Failed to create decoded file: {}", e))?;
    //     final_file.write_all(metadata_str.as_bytes()).map_err(|e| format!("Failed to write to decoded file: {}", e))?;

    //     // Read the decoded text file into memory
    //     let mut buffer = Vec::new();
    //     final_file.read_to_end(&mut buffer).map_err(|e| format!("Failed to read decoded text file: {}", e))?;

    //     Ok(buffer)
    // }

    fn encode_with_access_rights(&self,
        encoded_image: &[u8],
        owner_id: &str,
        requester_id: &str,
        access_rights: u32,
        output_path: &str
    ) -> Result<Vec<u8>, String> {
        // Remove the file extension from the output path
        let carrier_path = "carrier.png";
        let mut trimmed_output_path = output_path.trim_end_matches(".png");
        let file_name = output_path;
        trimmed_output_path = trimmed_output_path.trim_end_matches(".jpg");

        // Ensure the output path has a .png extension
        let output_path_with_extension = format!("{}.png", trimmed_output_path);

        // Create necessary directories
        std::fs::create_dir_all("/tmp/stegano_temp")
            .map_err(|e| format!("Failed to create temp directory: {}", e))?;

        // Create metadata
        let metadata = AccessMetadata {
            owner_id: owner_id.to_string(),
            requester_id: requester_id.to_string(),
            access_rights,
            file_name: file_name.to_string(),
        };
        
        // Use the new directory for temporary files
        let temp_metadata_path = format!("/tmp/stegano_temp/metadata_{}.txt", requester_id);
        let temp_metadata_path = temp_metadata_path.as_str();
        let temp_secret_path = format!("/tmp/stegano_temp/secret_{}.png", requester_id);
        let temp_secret_path = temp_secret_path.as_str();

        // Serialize metadata to JSON
        let metadata_json = serde_json::to_string(&metadata)
            .map_err(|e| format!("Failed to serialize metadata: {}", e))?;
        
        std::fs::write(&temp_metadata_path, metadata_json)
            .map_err(|e| format!("Failed to write metadata: {}", e))?;

        // Save the secret image temporarily
        std::fs::write(&temp_secret_path, encoded_image)
            .map_err(|e| format!("Failed to write secret image: {}", e))?;
        
        let files_vec: Vec<&str> = vec![&temp_metadata_path, &temp_secret_path];
        // Hide both metadata and secret image
        SteganoCore::encoder()
            .hide_files(files_vec)
            .use_media(&carrier_path).unwrap()
            .write_to(&output_path_with_extension)
            .hide();

        // Read the encoded image into memory
        let encoded_image = image::open(&output_path_with_extension)
            .map_err(|e| format!("Failed to open encoded image: {}", e))?;
        let mut buffer = Vec::new();
        encoded_image.write_to(&mut buffer, ImageFormat::PNG)
            .map_err(|e| format!("Failed to write to buffer: {}", e))?;

        // Cleanup temporary files
        std::fs::remove_file(&temp_metadata_path).ok();
        std::fs::remove_file(&temp_secret_path).ok();

        Ok(buffer)
    }

    fn decode_with_access_check(&self,
        encoded_image: &[u8],
        requester_id: &str,
        output_path: &str
    ) -> Result<Vec<u8>, String> {
        let carrier_path = "carrier.png";
        // Remove the file extension from the output path
        let mut trimmed_output_path = output_path.trim_end_matches(".png");
        trimmed_output_path = trimmed_output_path.trim_end_matches(".jpg");

        // Ensure the output path has a .png extension
        let output_path_with_extension = format!("{}.png", trimmed_output_path);
        // Create necessary directories
        let temp_dir = format!("/tmp/stegano_temp_{}", requester_id);
        std::fs::create_dir_all(&temp_dir).map_err(|e| format!("Failed to create temp directory: {}", e))?;
       
        // Use the new directory for temporary files
        let temp_encoded_path = format!("{}/temp_encoded.png", temp_dir);
        let temp_metadata_path = format!("{}/metadata_{}.txt", temp_dir, requester_id);
        let temp_secret_path = format!("{}/secret_{}.png", temp_dir, requester_id);
       
        // Save the encoded image temporarily
        std::fs::write(&temp_encoded_path, encoded_image).map_err(|e| format!("Failed to write temp encoded image: {}", e))?;
       
        // Extract metadata and secret image
        println!("Extracting metadata and secret image from the encoded image.");
        let result = unveil(
            &Path::new(&temp_encoded_path),
            &Path::new(&temp_dir),
            &CodecOptions::default()
        ).map_err(|e| format!("Failed to extract metadata and secret image: {}", e));
        println!("Metadata and secret image extraction result: {:?}", result);
       
        if let Err(e) = result {
            return Err(format!("Failed to extract metadata and secret image: {}", e));
        }
       
        // Read and parse metadata
        let metadata_str = std::fs::read_to_string(&temp_metadata_path).map_err(|e| format!("Failed to read metadata: {}", e))?;
        let mut metadata: AccessMetadata = serde_json::from_str(&metadata_str).map_err(|e| format!("Failed to parse metadata: {}", e))?;
       
        // Check access rights
        if metadata.requester_id != requester_id {
            return Err("Unauthorized access: requester ID mismatch".to_string());
        }
        if metadata.access_rights == 0 {
            return Err("No more access rights remaining".to_string());
        }
        
        
        // Decode the actual image
        let temp_decoded_dir = format!("{}/temp_decoded", temp_dir);
        std::fs::create_dir_all(&temp_decoded_dir).map_err(|e| format!("Failed to create temporary decoded directory: {}", e))?;
        let result = unveil(
            &Path::new(&temp_secret_path),
            &Path::new(&temp_decoded_dir),
            &CodecOptions::default()
        ).map_err(|e| format!("Failed to extract secret image: {}", e));
        println!("Secret image extraction result: {:?}", result);
        
        if let Err(e) = result {
            return Err(format!("Failed to extract secret image: {}", e));
        }
       
        // Decrement access rights and save updated metadata
        metadata.access_rights -= 1;
        let updated_metadata = serde_json::to_string(&metadata).map_err(|e| format!("Failed to serialize updated metadata: {}", e))?;
        std::fs::write(&temp_metadata_path, updated_metadata).map_err(|e| format!("Failed to write updated metadata: {}", e))?;
        
        let files_vec: Vec<&str> = vec![&temp_metadata_path, &temp_secret_path].iter().map(|s| s.as_str()).collect();

        SteganoCore::encoder()
            .hide_files(files_vec)
            .use_media(&carrier_path).unwrap()
            .write_to(&output_path_with_extension)
            .hide();

        // Read the decoded image into memory

        let mut temp_decoded_path = format!("{}/{}_{}", temp_decoded_dir,metadata.owner_id,metadata.file_name);
        // Remove the file extension from the output path
        temp_decoded_path = temp_decoded_path.trim_end_matches(".png").to_string();
        temp_decoded_path = temp_decoded_path.trim_end_matches(".jpg").to_string();

        // Ensure the output path has a .png extension
        temp_decoded_path = format!("{}.jpg", temp_decoded_path);

        println!("Opening decoded image: {}", temp_decoded_path);
        let decoded_image = image::open(&temp_decoded_path)
            .map_err(|e| format!("Failed to open decoded image: {}", e))?;
        let mut buffer = Vec::new();
        decoded_image.write_to(&mut buffer, ImageFormat::PNG)
            .map_err(|e| format!("Failed to write to buffer: {}", e))?;

        // Cleanup temporary files
        std::fs::remove_file(&temp_encoded_path).ok();
        std::fs::remove_file(&temp_metadata_path).ok();
        std::fs::remove_file(&temp_secret_path).ok();
        std::fs::remove_file(&temp_decoded_path).ok();

        Ok(buffer)
    }

    fn view_decoded_image_temp(&self, 
        encoded_image: &[u8],
        requester_id: &str,
        output_path: &str
    ) -> Result<(), String> {
        // First decode the image and check access rights
        let decoded_buffer = self.decode_with_access_check(encoded_image, requester_id,output_path)?;
        
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
