use quinn::{Endpoint, ServerConfig, TransportConfig, ClientConfig};
use rustls::client;
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
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {

    // Load the secret image

    
    // let l_steganographer = SomeImageSteganographer::new(100, 10);
    // // Load the secret image
    // let secret_path = "/home/magdeldin/Cloud-P2P-environment/client/secret.jpg";
    // let secret_file_name = std::path::Path::new(secret_path)
    //     .file_name()
    //     .and_then(|name| name.to_str())
    //     .unwrap_or("default_secret.jpg");
    // println!("Secret file name: {}", secret_file_name);
    // let secret_file = std::fs::File::open(secret_path).unwrap();
    
    // let secret_image = file_to_bytes(secret_file);

    // let secret_image_bytes: &[u8] = &secret_image;
    // println!("Secret image loaded successfully.");

    // // Generate unique output paths for each endpoint
    // let stego_path = format!("/home/magdeldin/stego.png");
    // let finale_path = format!("/home/magdeldin/Pictures");

    // let stegano = l_steganographer.encode(secret_image_bytes, &stego_path).unwrap();
    // println!("Encode method invoked successfully.");

    // let _finale = l_steganographer.decode(&stegano, &finale_path, &secret_file_name).unwrap();
    // println!("Decode method invoked successfully.");

    
    // // Setup Quinn enpoints
    // let server_addr: SocketAddr = "127.0.0.1:5000".parse()?;

    // let client_addr: SocketAddr = "127.0.0.1:4800".parse()?;

    // let mut client_endpoint = quinn::Endpoint::client(client_addr)?;
    // client_endpoint.set_default_client_config(ClientConfig::new(Arc::new(QuicClientConfig::try_from(
    //     rustls::ClientConfig::builder()
    //         .dangerous()
    //         .with_custom_certificate_verifier(SkipServerVerification::new())
    //         .with_no_client_auth(),
    // )?)));

    // println!("Quinn endpoints setup beginning.");

    // // Create transport ends
    // println!("Creating transport ends.");
    // let transport_ends = create(client_endpoint.clone(), server_addr).await?;
    // println!("Transport ends created successfully.");

    // // Create RTO context
    // println!("Creating RTO context for endpoint");
    // let (context_user, image_steganographer): (Context, ServiceToImport<dyn ImageSteganographer>) =
    //     Context::with_initial_service_import(Config::default_setup(), transport_ends.send.clone(), transport_ends.recv.clone());
    // let image_steganographer_proxy: Box<dyn ImageSteganographer> = image_steganographer.into_proxy();
    // println!("RTO context created successfully for endpoint");

    // // Load the secret image

    // let secret_path = "/home/magdeldin/Cloud-P2P-environment/client/secret.jpg";

    // let secret_file = std::fs::File::open(secret_path).unwrap();
    
    // let secret_image = file_to_bytes(secret_file);

    // let secret_file_name = "secret.jpg";

    // let secret_image_bytes: &[u8] = &secret_image;
    // println!("Secret image loaded successfully for endpoint");

    // // Generate unique output paths for each endpoint
    // let stego_path = format!("/home/magdeldin/stego.png");
    // let finale_path = format!("/home/magdeldin");

    // println!("Encoding secret image...");
    // let stegano = image_steganographer_proxy.encode(secret_image_bytes, &stego_path).unwrap();
    // println!("Encode method invoked successfully for endpoint");

    // let local_steganogragrapher = SomeImageSteganographer::new(100, 10);
    // let _finale = local_steganogragrapher.decode(&stegano, &finale_path, &secret_file_name).unwrap();
    // println!("Decode method invoked successfully for endpoint");


    // Setup Quinn endpoints
    let server_addrs: Vec<SocketAddr> = vec![
        "127.0.0.1:5001".parse()?,
        "127.0.0.1:5002".parse()?,
        "127.0.0.1:5050".parse()?
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
    let mut transport_ends_vec = Vec::new();
    for address in server_addrs {
        let ends = create(client_endpoint.clone(),address).await?;
        transport_ends_vec.push(ends);
    }
    println!("Transport ends created successfully.");

    let mut context_vector = Vec::new();
    let mut image_steganographer_proxy_vector = Vec::new();

    
    // Process each transport end
    for (index, transport_end) in transport_ends_vec.iter().enumerate() {
        println!("Processing transport end {}", index);
        
        // Create RTO context
        println!("Creating RTO context for endpoint {}", index);
        let (context_user, image_steganographer): (Context, ServiceToImport<dyn ImageSteganographer>) =
            Context::with_initial_service_import(Config::default_setup(), transport_end.send.clone(), transport_end.recv.clone());
        
        //image_steganographer_vector.push(image_steganographer);
        let image_steganographer_proxy: Box<dyn ImageSteganographer> = image_steganographer.into_proxy();
        context_vector.push(context_user);
        
        image_steganographer_proxy_vector.push(image_steganographer_proxy);

        println!("RTO context created successfully for endpoint {}", index);

        // // Load the secret image
        // let secret_path = "/home/magdeldin/Cloud-P2P-environment/client/secret.jpg";

        // let secret_file = std::fs::File::open(secret_path).unwrap();
        
        // let secret_image = file_to_bytes(secret_file);

        // let secret_file_name = "secret.jpg";

        // let secret_image_bytes: &[u8] = &secret_image;
        // println!("Secret image loaded successfully for endpoint {}", index);

        // // Generate unique output paths for each endpoint
        // let stego_path = format!("/home/magdeldin/stego.png");
        // let finale_path = format!("/home/magdeldin/Pictures");

        // println!("Encoding secret image for endpoint {}...", index);
        // let stegano = image_steganographer_proxy.encode(secret_image_bytes, &stego_path);
        // let stegano = match stegano {
        //     Ok(s) => s,
        //     Err(e) => {
        //         println!("Error encoding secret image for endpoint {}: {}", index, e);
        //         continue;
        //     }
        // };
        // println!("Encode method invoked successfully for endpoint {}", index);

        // let local_steganogragrapher = SomeImageSteganographer::new(100, 10);
        // let _finale = local_steganogragrapher.decode(&stegano, &finale_path, &secret_file_name).unwrap();
        // println!("Decode method invoked successfully for endpoint {}", index);
        // println!("Transport end {} processed successfully.", index);
    }
    
    // Load all secret images from the secret_images folder
    let secret_images_path = "secret_images";
    let secret_images = std::fs::read_dir(secret_images_path)?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_file())
        .collect::<Vec<_>>();


    for i in 0..10 {
        println!("Iteration {}", i);
        for (index, entry) in secret_images.iter().enumerate() {
            let secret_path = entry.path();
            let secret_file_name = secret_path.file_name().unwrap().to_str().unwrap();
    
            println!("Processing secret image {}: {}", index, secret_file_name);
    
            let secret_file = std::fs::File::open(&secret_path).unwrap();
            let secret_image = file_to_bytes(secret_file);
            let secret_image_bytes: &[u8] = &secret_image;
    
            // Generate unique output paths for each image
            let stego_path = format!("encoded_images/stego_{}.png", index);
            let finale_path = format!("decoded_images");
    
            println!("Encoding secret image {}...", index);
            let start_time = std::time::Instant::now();
            let stegano = image_steganographer_proxy_vector[index % image_steganographer_proxy_vector.len()].encode(secret_image_bytes, &stego_path);
            let duration = start_time.elapsed();
            println!("Encoding time for secret image {}: {:.2?} seconds", index, duration.as_secs_f64());

            let stegano = match stegano {
                Ok(s) => s,
                Err(e) => {
                    println!("Error encoding secret image {}: {}", index, e);
                    continue;
                }
            };
            println!("Encode method invoked successfully for secret image {}", index);
    
            let local_steganogragrapher = SomeImageSteganographer::new(100, 10);
            let _finale = local_steganogragrapher.decode(&stegano, &finale_path, &secret_file_name).unwrap();
            println!("Decode method invoked successfully for secret image {}", index);
        }
    }

    
    

    // Keep the server running
    tokio::signal::ctrl_c().await?;
    println!("Shutting down server...");

    Ok(())
} 