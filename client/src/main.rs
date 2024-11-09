use quinn::{ClientConfig, Connecting, Endpoint, ServerConfig, TransportConfig};
use rustls::{client, server};
use core::num;
use std::error::Error;
use std::net::SocketAddr;
use remote_trait_object::{Context, Service, Config, ServiceToImport};
use std::sync::Arc;
use std::time::Duration;

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

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {




    // Setup Quinn endpoints
    let server_addrs: Vec<SocketAddr> = vec![
        "127.0.0.1:5000".parse()?,
        //"127.0.0.1:5002".parse()?,
        //"127.0.0.1:5050".parse()?
    ];  // Connect to server's ports
    let client_addr: SocketAddr = "127.0.0.1:4800".parse()?;  // Listen on this port

    println!("Quinn endpoints setup beginning.");

    //let (server_endpoint, _server_cert) = make_server_endpoint(server_addr).unwrap();
    

    let mut client_config = ClientConfig::new(Arc::new(QuicClientConfig::try_from(
        rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(SkipServerVerification::new())
            .with_no_client_auth(),
    )?));
    let mut transport_config = TransportConfig::default();
    transport_config.keep_alive_interval(Some(Duration::from_secs(10)));
    client_config.transport_config(Arc::new(transport_config));

    let mut client_endpoint = quinn::Endpoint::client(client_addr)?;
    client_endpoint.set_default_client_config(client_config);


    println!("Quinn endpoints setup successfully.");

    // Create transport ends
    println!("Creating transport ends.");




    // // Create transport ends
    // println!("Creating transport ends.");
    // let mut transport_ends_vec = Vec::new();
    // for address in server_addrs.clone() {
    //     let ends = create(client_endpoint.clone(),address).await?;
    //     transport_ends_vec.push(ends);
    // }
    // println!("Transport ends created successfully.");

    // let mut context_vector = Vec::new();
    // let mut image_steganographer_proxy_vector = Vec::new();

    
    // // Process each transport end
    // for (index, transport_end) in transport_ends_vec.iter().enumerate() {
    //     println!("Processing transport end {}", index);
        
    //     // Create RTO context
    //     println!("Creating RTO context for endpoint {}", index);
    //     let (context_user, image_steganographer): (Context, ServiceToImport<dyn ImageSteganographer>) =
    //         Context::with_initial_service_import(Config::default_setup(), transport_end.send.clone(), transport_end.recv.clone());
        
    //     //image_steganographer_vector.push(image_steganographer);
    //     let image_steganographer_proxy: Box<dyn ImageSteganographer> = image_steganographer.into_proxy();
    //     context_vector.push(context_user);
        
    //     image_steganographer_proxy_vector.push(image_steganographer_proxy);

    //     println!("RTO context created successfully for endpoint {}", index);

        
    // }



    let steg_handle = tokio::spawn(async move{


        // Load all secret images from the secret_images folder
        let secret_images_path = "secret_images";
        let secret_images = std::fs::read_dir(secret_images_path).map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_file())
            .collect::<Vec<_>>();
        
        
        for (index, entry) in secret_images.iter().enumerate() {
            let secret_path = entry.path();
            let secret_file_name = secret_path.file_name().unwrap().to_str().unwrap().to_string();
    
            println!("Processing secret image {}: {}", index, secret_file_name);
    
            let secret_file = std::fs::File::open(&secret_path).unwrap();
            let secret_image = file_to_bytes(secret_file);
            let secret_image_bytes = &secret_image;
    
            // Generate unique output paths for each image
            let stego_path = format!("encoded_images/stego_{}.png", index);
            let finale_path = format!("decoded_images");

    
            println!("Encoding secret image {}...", index);
            let start_time = std::time::Instant::now();
            
            // let stegano = image_steganographer_proxy_vector[index % image_steganographer_proxy_vector.len()].encode(secret_image_bytes, &stego_path);
            
            let mut handles = vec![];
            


            for addr in server_addrs.clone() {
                let ends = create(client_endpoint.clone(),addr).await?;

                let secret_image_bytes = secret_image_bytes.clone();
                let stego_path = stego_path.clone();
                let finale_path = finale_path.clone();
                let secret_file_name = secret_file_name.clone().to_string();

                let handle = tokio::spawn(async move {

                    let (context_user, image_steganographer): (Context, ServiceToImport<dyn ImageSteganographer>) =
                            Context::with_initial_service_import(Config::default_setup(), ends.send.clone(), ends.recv.clone());
                        let image_steganographer_proxy: Box<dyn ImageSteganographer> = image_steganographer.into_proxy();
                        println!("Encoding secret image {} with proxy", index);
                        let stegano = image_steganographer_proxy.encode(&secret_image_bytes, &stego_path);
                        let local_steganogragrapher = SomeImageSteganographer::new(100, 10);
                        let _finale = local_steganogragrapher.decode(&stegano.unwrap(), &finale_path, &secret_file_name).unwrap();
                    
                });
                handles.push(handle);
            }
    
            let results: Vec<_> = futures::future::join_all(handles).await;
            for result in results {
                match result {
                    Ok(_) => {
                        println!("Secret image {} processed successfully", index);
                    },
                    Err(e) => {
                        println!("Error processing secret image {}: {}", index, e);
                    }
                }
            }

            
            
            println!("Decode method invoked successfully for secret image {}", index);
        }

        for i in 0..10 {
            
        }
        println!("Shutting down steg handle...");

        Ok::<(), String>(())
    });

    // // Create transport ends
    // println!("Creating transport ends.");
    // let mut transport_ends_vec = Vec::new();
    // for address in server_addrs.clone() {
    //     let ends = create(client_endpoint.clone(),address).await?;
    //     transport_ends_vec.push(ends);
    // }
    // println!("Transport ends created successfully.");

    // let mut context_vector = Vec::new();
    // let mut image_steganographer_proxy_vector = Vec::new();

    
    // // Process each transport end
    // for (index, transport_end) in transport_ends_vec.iter().enumerate() {
    //     println!("Processing transport end {}", index);
        
    //     // Create RTO context
    //     println!("Creating RTO context for endpoint {}", index);
    //     let (context_user, image_steganographer): (Context, ServiceToImport<dyn ImageSteganographer>) =
    //         Context::with_initial_service_import(Config::default_setup(), transport_end.send.clone(), transport_end.recv.clone());
        
    //     //image_steganographer_vector.push(image_steganographer);
    //     let image_steganographer_proxy: Box<dyn ImageSteganographer> = image_steganographer.into_proxy();
    //     context_vector.push(context_user);
        
    //     image_steganographer_proxy_vector.push(image_steganographer_proxy);

    //     println!("RTO context created successfully for endpoint {}", index);

        
    // }
    
    // // Load all secret images from the secret_images folder
    // let secret_images_path = "secret_images";
    // let secret_images = std::fs::read_dir(secret_images_path)?
    //     .filter_map(Result::ok)
    //     .filter(|entry| entry.path().is_file())
    //     .collect::<Vec<_>>();

    
    // let proxies = image_steganographer_proxy_vector.lock().await;

    // for i in 0..10 {
    //     println!("Iteration {}", i);
    //     for (index, entry) in secret_images.iter().enumerate() {
    //         let secret_path = entry.path();
    //         let secret_file_name = secret_path.file_name().unwrap().to_str().unwrap();
    
    //         println!("Processing secret image {}: {}", index, secret_file_name);
    
    //         let secret_file = std::fs::File::open(&secret_path).unwrap();
    //         let secret_image = file_to_bytes(secret_file);
    //         let secret_image_bytes = &secret_image;
    
    //         // Generate unique output paths for each image
    //         let stego_path = format!("encoded_images/stego_{}.png", index);
    //         let finale_path = format!("decoded_images");

    
    //         println!("Encoding secret image {}...", index);
    //         let start_time = std::time::Instant::now();
            
    //         // let stegano = image_steganographer_proxy_vector[index % image_steganographer_proxy_vector.len()].encode(secret_image_bytes, &stego_path);
            
    //         let mut handles = vec![];
            


    //         for (i, proxy) in proxies.iter().enumerate() {
    //             let secret_image_bytes = secret_image_bytes.clone();
    //             let stego_path = stego_path.clone();
    //             let handle = tokio::spawn(async move {
    //                 proxy.encode(&secret_image_bytes, &stego_path)
    //             });
    //             handles.push(handle);
    //         }

    //         let results: Vec<_> = futures::future::join_all(handles).await;
    //         for result in results {
    //             match result {
    //                 Ok(Ok(stegano)) => {
    //                     println!("Encoding completed successfully.");
    //                     let duration = start_time.elapsed();
    //                     println!("Encoding time for secret image {}: {:.2?} seconds", index, duration.as_secs_f64());
    //                     println!("Encode method invoked successfully for secret image {}", index);
                
    //                     let local_steganogragrapher = SomeImageSteganographer::new(100, 10);
    //                     let _finale = local_steganogragrapher.decode(&stegano, &finale_path, &secret_file_name).unwrap();

    //                 }
    //                 Ok(Err(e)) => {
    //                     eprintln!("Encoding failed: {}", e);
    //                 }
    //                 Err(e) => {
    //                     eprintln!("Task join error: {}", e);
    //                 }
    //             }
    //         }
            
    //         println!("Decode method invoked successfully for secret image {}", index);
    //     }
    // }

    
    

    // Keep the server running
    tokio::signal::ctrl_c().await?;
    println!("Shutting down server...");

    let _ = tokio::join!(steg_handle);

    Ok(())
} 