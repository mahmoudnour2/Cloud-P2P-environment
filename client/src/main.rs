use quinn::{ClientConfig, Endpoint, TransportConfig};
use rustls::{client};
use std::error::Error;
use std::net::SocketAddr;
use remote_trait_object::{Context, Config, ServiceToImport};
use std::sync::Arc;
use std::time::Duration;
use image_steganographer::{ImageSteganographer, SomeImageSteganographer};
use transport::{create, TransportEnds};
use quinn_utils::*;
use tokio::sync::Mutex;
use quinn_proto::crypto::rustls::QuicClientConfig;
use steganography::util::file_to_bytes;
use tokio::time::timeout;
use std::panic::{self, AssertUnwindSafe};

mod transport;
mod image_steganographer;
mod quinn_utils;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Setup Quinn endpoints
    let server_addrs: Vec<SocketAddr> = vec![
        "127.0.0.1:5011".parse()?,
    ];  // Connect to server's ports
    let client_addr: SocketAddr = "127.0.0.1:4800".parse()?;  // Listen on this port

    println!("Quinn endpoints setup beginning.");

    // Configure client
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

    // Continuously try to find and communicate with the leader
    loop {
        println!("Attempting to connect to servers to find the leader...");

        let mut leader_ends = None;

        // Attempt to connect to each server, looking for the one that accepts the connection
        for addr in &server_addrs {
            match timeout(Duration::from_secs(5), create(client_endpoint.clone(), *addr)).await {
                Ok(Ok(ends)) => {
                    println!("Leader server found at address: {}", addr);
                    leader_ends = Some(ends);
                    break; // Stop once a leader has been found
                }
                Ok(Err(e)) => {
                    println!("Failed to connect to {}: {}", addr, e);
                }
                Err(_) => {
                    println!("Connection attempt to {} timed out.", addr);
                }
            }
        }

        // If no leader was found, retry the loop
        let leader_ends = match leader_ends {
            Some(ends) => ends,
            None => {
                println!("No leader server available. Retrying...");
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };


        // Load all secret images from the secret_images folder
        let secret_images_path = "secret_images";
        let secret_images = std::fs::read_dir(secret_images_path)
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_file())
            .collect::<Vec<_>>();
            
        
        let (context_user, image_steganographer): (Context, ServiceToImport<dyn ImageSteganographer>) = Context::with_initial_service_import
        (
            Config::default_setup(),
            leader_ends.send.clone(),
            leader_ends.recv.clone()
        );
        context_user.disable_garbage_collection();
        let context_user = Arc::new(Mutex::new(context_user));
        let context_user_clone = context_user.clone();
        // let image_steganographer = Arc::new(Mutex::new(image_steganographer));
        // let image_steganographer_clone = Arc::clone(&image_steganographer);

        // let image_steganographer_lock = image_steganographer.lock().await;
        let image_steganographer_proxy: Box<dyn ImageSteganographer> = image_steganographer.into_proxy();
        let image_steganographer_proxy = Arc::new(Mutex::new(image_steganographer_proxy));
        let image_steganographer_proxy_clone = Arc::clone(&image_steganographer_proxy);
        tokio::spawn(async move {
            loop {
                // Attempt to perform an operation to check connection status.
                match leader_ends.send.connection().closed().await {
                    e => {
                        println!("Connection closed gracefully by server.");
                        {
                            let mut context_user_clone_lock = context_user_clone.lock().await;
                            context_user_clone_lock.clear_service_registry();
                        }
                        drop(context_user_clone);
                        // drop(context_user);
                        drop(image_steganographer_proxy_clone);

                        break;
                    }
                }
            }
        });
        for (index, entry) in secret_images.iter().enumerate() {
            let secret_path = entry.path();
            let secret_file_name = secret_path.file_name().unwrap().to_str().unwrap().to_string();

            println!("Processing secret image {}: {}", index, secret_file_name);

            let secret_file = std::fs::File::open(&secret_path).unwrap();
            let secret_image = file_to_bytes(secret_file);
            let secret_image_bytes = &secret_image;

            // Generate unique output paths for each image
            let stego_path = format!("stego_{}.png", index);
            let finale_path = "decoded_images".to_string();

            println!("Encoding secret image {}...", index);
            
            let image_steganographer_proxy_lock = image_steganographer_proxy.lock().await;
            let result = panic::catch_unwind(AssertUnwindSafe(|| {
                match image_steganographer_proxy_lock.encode(&secret_image_bytes, &stego_path, &secret_file_name) {
                    Ok(encoded_bytes) => Ok(encoded_bytes),
                    Err(e) => {
                        println!("Error during encoding: {:?}", e);
                        Err(e)
                    }
                }
            })).unwrap_or_else(|e| {
                println!("Connection error occurred: {:?}", e);
                Err("Connection lost during encoding".to_string())
            });
            
            // Handle the result
            match result {
                Ok(_) => println!("Encoding completed successfully"),
                Err(e) => {
                    println!("Failed to encode: {:?}", e);
                    // context_user_clone.clear_service_registry();
                    // drop(context_user_clone);
                    // drop(image_steganographer_proxy_clone);
                    break;
                }
            }
        }

        // unsafe {
        //     remote_trait_object::port_thread_local::PORT.with(|k| {
        //         k.borrow_mut().clear();
        //     });
        // }
        println!("Finished processing requests with the leader.");
    }
}