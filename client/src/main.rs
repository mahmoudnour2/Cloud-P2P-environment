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
mod image_steganographer;
mod quinn_utils;
use image_steganographer::{ImageSteganographer, SomeImageSteganographer};
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
use laminar::{Packet, Socket, SocketEvent};
use std::thread;
use local_ip_address::local_ip;
use std::net::IpAddr;
use std::net::UdpSocket;
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StegoRequest {
    pub secret_image: Vec<u8>,
    pub output_path: String,
    pub file_name: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    
    
    // Setup Quinn endpoints
    let server_addrs: Vec<SocketAddr> = vec![
        "10.7.19.117:5017".parse()?,
        "10.7.16.154:5017".parse()?,
        "10.7.16.71:5017".parse()?,
    ];  // Connect to server's ports
    let client_addr: SocketAddr = "10.7.17.170:0".parse()?;  // Listen on this port



    // Create transport ends
    println!("Creating transport ends.");


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

    let process_start_time = std::time::Instant::now();

    for chunk in secret_images_chunks.into_iter() {
        let secret_images = chunk.clone();
        let server_addrs = server_addrs.clone();


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
        
            println!("Encoding secret image {}...", index);
            let start_time = std::time::Instant::now();
            let mut success = false;
            
            let mut retries = 0;
            let max_retries = 3;
            let mut backoff_duration = Duration::from_secs(2);
            while retries < max_retries && !success {
                let mut handles = vec![];
        
                for addr in server_addrs.clone() {
                    
                    

                    let secret_image_bytes = secret_image_bytes.clone();
                    let stego_path = stego_path.clone();
                    let finale_path = finale_path.clone();
                    let secret_file_name = secret_file_name.clone();

                    // Creates the socket
                    let mut socket = Socket::bind("127.0.0.1:12345")?;
                    let packet_sender = socket.get_packet_sender();
                    // Starts the socket, which will start a poll mechanism to receive and send messages.
                    let _thread = thread::spawn(move || socket.start_polling());

                    // Bytes to sent
                    let bytes = 
            
                    let handle = tokio::spawn(async move {
                        
                        
                        
                        


                    });
                    handles.push(handle);
                }
        
                let results: Vec<_> = futures::future::join_all(handles).await;
                for result in results {
                    match result {
                        Ok(_) => {
                            println!("Secret image {} processed successfully", index);
                            success = true;
                        },
                        Err(e) => {
                            println!("Error in task: {}", e);
                        }
                    }
                }
                tokio::time::sleep(backoff_duration).await;
            }
            
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

    // let steg_handle = tokio::spawn(async move{


    //     // Load all secret images from the secret_images folder
    //     let secret_images_path = "secret_images";
    //     let secret_images = std::fs::read_dir(secret_images_path).map_err(|e| e.to_string())?
    //         .filter_map(Result::ok)
    //         .filter(|entry| entry.path().is_file())
    //         .collect::<Vec<_>>();
        
        
    //     for (index, entry) in secret_images.iter().enumerate() {
    //         let secret_path = entry.path();
    //         let secret_file_name = secret_path.file_name().unwrap().to_str().unwrap().to_string();
    
    //         println!("Processing secret image {}: {}", index, secret_file_name);
    
    //         let secret_file = std::fs::File::open(&secret_path).unwrap();
    //         let secret_image = file_to_bytes(secret_file);
    //         let secret_image_bytes = &secret_image;
    
    //         // Generate unique output paths for each image
    //         let stego_path = format!("encoded_images/stego_{}.png", index);
    //         let finale_path = format!("decoded_images");

    
    //         println!("Encoding secret image {}...", index);
    //         let start_time = std::time::Instant::now();
    //         let mut success = false;
            
    //         // let stegano = image_steganographer_proxy_vector[index % image_steganographer_proxy_vector.len()].encode(secret_image_bytes, &stego_path);
            
            

    //         while start_time.elapsed().as_secs() < 120 && !success {
    //             let mut handles = vec![];

    //             for addr in server_addrs.clone() {
    //                 let ends = create(client_endpoint.clone(),addr).await?;

    //                 let secret_image_bytes = secret_image_bytes.clone();
    //                 let stego_path = stego_path.clone();
    //                 let finale_path = finale_path.clone();
    //                 let secret_file_name = secret_file_name.clone().to_string();

    //                 let handle = tokio::spawn(async move {

    //                     let (context_user, image_steganographer): (Context, ServiceToImport<dyn ImageSteganographer>) =
    //                             Context::with_initial_service_import(Config::default_setup(), ends.send.clone(), ends.recv.clone());
    //                         let image_steganographer_proxy: Box<dyn ImageSteganographer> = image_steganographer.into_proxy();
    //                         println!("Encoding secret image {} with proxy", index);
    //                         let stegano = image_steganographer_proxy.encode(&secret_image_bytes, &stego_path, &secret_file_name);
    //                         match stegano {
    //                             Ok(stegano_vec) => {
    //                                 println!("Secret image {} encoded successfully", index);
    //                                 let local_steganogragrapher = SomeImageSteganographer::new(100, 10);
    //                                 let _finale = local_steganogragrapher.decode(&stegano_vec, &finale_path, &secret_file_name).unwrap();

    //                             },
    //                             Err(e) => {
    //                                 println!("Error encoding secret image {}: {}", index, e);
    //                             }
    //                         }
                            
                        
    //                 });
    //                 handles.push(handle);
    //             }
    
    //             let results: Vec<_> = futures::future::join_all(handles).await;
    //             for result in results {
    //                 match result {
    //                     Ok(_) => {
    //                         println!("Secret image {} processed successfully", index);
    //                         success = true;
    //                     },
    //                     Err(e) => {
    //                         println!("Error processing secret image {}: {}", index, e);
    //                     }
    //                 }
    //             }
    //         }
           
            
    //         println!("Decode method invoked successfully for secret image {}", index);
    //     }
    //     println!("Shutting down steg handle...");

    //     Ok::<(), String>(())
    // });

    
    

    println!("Shutting down server...");
    tokio::signal::ctrl_c().await.expect("Failed to listen for ctrl_c signal");

    // let _ = tokio::join!(steg_handle);

    Ok(())
} 