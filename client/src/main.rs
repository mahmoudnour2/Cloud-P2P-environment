use quinn::{ClientConfig, Connecting, Endpoint, ServerConfig, TransportConfig};
use rustls::{client, server};
use core::num;
use std::error::Error;
use std::net::SocketAddr;
use remote_trait_object::{Context, Service, Config, ServiceToImport};
use std::sync::Arc;
use std::time::Duration;
use std::panic::AssertUnwindSafe;

mod transport;
mod quinn_utils;
//use image_steganographer::{ImageSteganographer};
use transport::{create, TransportEnds};
use quinn_utils::*;
use quinn_proto::crypto::rustls::QuicClientConfig;
use image;
use steganography::{self, util::file_to_bytes};
use tokio::task;
use tokio::time::timeout;
use std::env;
use std::process::{Command, exit};
use std::fs::OpenOptions;
use std::io::Write;
use tokio::sync::Semaphore;

use image_steganographer::encoder_client::EncoderClient;
use image_steganographer::EncodeRequest;
use image_steganographer::EncodeReply;

pub mod image_steganographer {
    tonic::include_proto!("steganography");
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    
    
    // Setup Quinn endpoints
    let server_addrs: Vec<&str> = vec![
        "fe80::4ed7:17ff:fe7f:d11b:5017",
    ];  // Connect to server's ports


    // Load all secret images from the secret_images folder
    let secret_images_path = "secret_images";
    let secret_images = std::fs::read_dir(secret_images_path).map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_file())
        .map(|entry| entry.path())
        .collect::<Vec<_>>();

    // Split secret images into 3 vectors
    let chunk_size = (secret_images.len() + 2) / 3;
    let mut secret_images_chunks: Vec<Vec<_>> = Vec::new();

    for chunk in secret_images.chunks(chunk_size) {
        secret_images_chunks.push(chunk.to_vec());
    }

    while secret_images_chunks.len() < 3 {
        secret_images_chunks.push(Vec::new());
    }

    let mut stego_portions = vec![];
    let semaphore = Arc::new(Semaphore::new(5)); // Limit to 5 concurrent requests
    let process_start_time = std::time::Instant::now();

    for chunk in secret_images_chunks.into_iter() {
        let secret_images = chunk.clone();
        let server_addrs = server_addrs.clone();
        //let semaphore = semaphore.clone();

        let stego_portion = tokio::spawn(async move {
            for (index, entry) in secret_images.iter().enumerate() {
                let secret_path = entry;
                let secret_file_name = secret_path.file_name()
                    .ok_or("Failed to get filename")?
                    .to_str()
                    .ok_or("Failed to convert filename to string")?
                    .to_string();
            
                println!("Processing secret image {}: {}", index, secret_file_name);
            
                let secret_file = std::fs::File::open(&secret_path)
                    .map_err(|e| format!("Failed to open secret file: {}", e))?;
                let secret_image = file_to_bytes(secret_file);
                let secret_image_bytes = &secret_image;
            
                // Generate unique output paths for each image
                let stego_path = format!("encoded_images/stego_{}.png", secret_file_name);
                let finale_path = format!("decoded_images");
                let mut handles = vec![];

                for server_addr in &server_addrs {
                    let secret_image_bytes = secret_image_bytes.clone();
                    let stego_path = stego_path.clone();
                    let secret_file_name = secret_file_name.clone();
                    let server_addr = server_addr.to_string();

                    let handle = tokio::spawn(async move {
                        let uri = format!("http://{}", server_addr);
                        let mut client = EncoderClient::connect(uri).await.map_err(|e| e.to_string())?;
                        println!("Encoding secret image {} on server {}...", index, server_addr);
                        let request = tonic::Request::new(EncodeRequest {
                            image: secret_image_bytes,
                            output_path: stego_path,
                            file_name: secret_file_name,
                        });

                        client.encode(request).await.map_err(|e| e.to_string())
                    });

                    handles.push(handle);
                }

                let mut successful_response = None;
                for handle in handles {
                    match handle.await {
                        Ok(Ok(response)) => {
                            successful_response = Some(response);
                            break;
                        }
                        Ok(Err(e)) => {
                            println!("Error encoding on server: {}", e);
                        }
                        Err(e) => {
                            println!("Task join error: {}", e);
                        }
                    }
                }


                if successful_response.is_none() {
                    return Err("Failed to encode image on all servers".to_string());
                }
                let uri = format!("http://{}", server_addrs[0]);
                let mut client = EncoderClient::connect(uri).await.map_err(|e| e.to_string())?;
                println!("Encoding secret image {}...", index);
                let start_time = std::time::Instant::now();
                let request = tonic::Request::new(EncodeRequest {
                    image: secret_image_bytes.clone(),
                    output_path: stego_path.clone(),
                    file_name: secret_file_name.clone(),
                });
            
                let response = client.encode(request).await.map_err(|e| e.to_string())?;
            }
            Ok::<(), String>(())
        });

        stego_portions.push(stego_portion);
    }

    let results: Vec<_> = futures::future::join_all(stego_portions).await;

    for result in results {
        match result {
            Ok(_) => {
                println!("Secret images processed successfully");
            },
            Err(e) => {
                println!("Error processing secret images: {}", e);
            }
        }
    }

    let process_duration = process_start_time.elapsed();
    let csv_path = "process_times.csv";
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(csv_path)
        .map_err(|e| format!("Failed to open CSV file: {}", e))?;

    writeln!(file, "{}", process_duration.as_secs())
        .map_err(|e| format!("Failed to write to CSV file: {}", e))?;

    

    
    

    println!("Shutting down server...");
    tokio::signal::ctrl_c().await.expect("Failed to listen for ctrl_c signal");

    // let _ = tokio::join!(steg_handle);

    Ok(())
} 