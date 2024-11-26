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
use tokio::sync::Semaphore;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    
    // Add near the start of main
std::fs::create_dir_all("encoded_images")
.map_err(|e| format!("Failed to create encoded_images directory: {}", e))?;
std::fs::create_dir_all("decoded_images")
.map_err(|e| format!("Failed to create decoded_images directory: {}", e))?;
    // Setup Quinn endpoints
    let server_addrs: Vec<SocketAddr> = vec![
        "10.7.19.102:5017".parse()?,
    ];  // Connect to server's ports
    let client_addr: SocketAddr = "10.7.19.179:0".parse()?;  // Listen on this port

    println!("Quinn endpoints setup beginning.");

    //let (server_endpoint, _server_cert) = make_server_endpoint(server_addr).unwrap();
    

    let mut client_config = ClientConfig::new(Arc::new(QuicClientConfig::try_from(
        rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(SkipServerVerification::new())
            .with_no_client_auth(),
    )?));
    let mut transport_config = TransportConfig::default();
    transport_config.keep_alive_interval(Some(Duration::from_secs(5)));
    client_config.transport_config(Arc::new(transport_config));

    let mut client_endpoint = quinn::Endpoint::client(client_addr)?;
    client_endpoint.set_default_client_config(client_config);


    println!("Quinn endpoints setup successfully.");

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
    let semaphore = Arc::new(Semaphore::new(5)); // Limit to 5 concurrent requests
    let process_start_time = std::time::Instant::now();

    for chunk in secret_images_chunks.into_iter() {
        let secret_images = chunk.clone();
        let server_addrs = server_addrs.clone();
        let client_endpoint = client_endpoint.clone();
        let semaphore = semaphore.clone();

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
            let stego_path = format!("stego_{}.png", secret_file_name);
            let stego_with_rights_path = format!("stego_with_access_rights_{}.png", secret_file_name);
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
                    
                    
                    let client_endpoint = client_endpoint.clone();
                    let secret_image_bytes = secret_image_bytes.clone();
                    let stego_path = stego_path.clone();
                    let stego_with_rights_path = stego_with_rights_path.clone();
                    let finale_path = finale_path.clone();
                    let secret_file_name = secret_file_name.clone();
                    let semaphore = semaphore.clone();
            
                    let handle = tokio::spawn(async move {
                        let permit = semaphore.acquire().await.unwrap(); // Acquire a permit
                        let ends = match timeout(Duration::from_secs(10), create(client_endpoint.clone(), addr)).await {
                            Ok(Ok(ends)) => ends,
                            Ok(Err(e)) => {
                                retries += 1;
                                backoff_duration *= 2;
                                println!("Error creating transport ends: {}", e);
                                return;
                            }
                            Err(_) => {
                                retries += 1;
                                backoff_duration *= 2;
                                println!("Timeout occurred while creating transport ends");
                                return;
                            }
                        };
                        
                        let (context_user, image_steganographer): (Context, ServiceToImport<dyn ImageSteganographer>) =
                            Context::with_initial_service_import(Config::default_setup(), ends.send.clone(), ends.recv.clone());
                        context_user.disable_garbage_collection();
                        let image_steganographer_proxy: Box<dyn ImageSteganographer> = image_steganographer.into_proxy();
                        println!("Encoding secret image {} with proxy", index);
                        let stegano = timeout(Duration::from_secs(60), async {
                            match std::panic::catch_unwind(AssertUnwindSafe(|| {
                                let owner_id = "owner123";
                                let requester_id = "requester456";
                                let initial_access_rights = 3;

                                // First encode the secret image remotely
                                let encoded_image = image_steganographer_proxy.encode(
                                    &secret_image_bytes,
                                    &stego_path,
                                    &secret_file_name
                                )?;

                                // Then add access rights locally
                                let local_steganographer = SomeImageSteganographer::new(100, 10);
                                let final_encoded = local_steganographer.encode_with_access_rights(
                                    &encoded_image,
                                    owner_id,
                                    requester_id,
                                    initial_access_rights,
                                    &stego_with_rights_path
                                )?;

                                Ok(final_encoded)
                            })) {
                                Ok(Ok(result)) => Ok(result),
                                Ok(Err(e)) => Err(e),
                                Err(_) => Err(Box::new(std::io::Error::new(
                                    std::io::ErrorKind::Other,
                                    "Task panicked"
                                )) as Box<dyn std::error::Error>)
                            }
                        }).await.unwrap_or_else(|_| Err(Box::new(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            "Timeout occurred during encoding"
                        )) as Box<dyn std::error::Error>));
                        
                        // Handle the result
                        match stegano {
                            Ok(final_encoded) => {
                                println!("Encoding completed successfully");
                                // View the image temporarily
                                let local_steganographer = SomeImageSteganographer::new(100, 10);
                                if let Err(e) = local_steganographer.view_decoded_image_temp(
                                    &final_encoded,
                                    "requester456"
                                ) {
                                    println!("Error viewing image: {}", e);
                                }
                            },
                            Err(e) => {
                                retries += 1;
                                backoff_duration *= 2;
                                println!("Failed to encode: {:?}", e);
                            }
                        }
                        drop(permit); // Release the permit
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