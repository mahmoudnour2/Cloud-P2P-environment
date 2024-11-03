use quinn::{Endpoint, ServerConfig, TransportConfig, ClientConfig};
use std::error::Error;
use std::net::SocketAddr;
use remote_trait_object::{Context, Service, Config, ServiceToImport};
use std::sync::Arc;

mod transport;
mod image_steganographer;
mod quinn_utils;
use image_steganographer::{ImageSteganographer, SomeImageSteganographer};
use transport::{create, TransportEnds};
use quinn_utils::*;
use quinn_proto::crypto::rustls::QuicClientConfig;
use image;
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    
    // Setup Quinn endpoints
    let server_addrs: Vec<SocketAddr> = vec![
        "10.7.16.71:5001".parse()?,
        "10.7.16.71:5002".parse()?,
        "10.7.16.71:5003".parse()?
    ];  // Connect to server's ports
    let client_addr: SocketAddr = "10.7.16.80:4800".parse()?;  // Listen on this port

    println!("Quinn endpoints setup beginning.");

    //let (server_endpoint, _server_cert) = make_server_endpoint(server_addr).unwrap();
    
    
    let mut client_endpoint = quinn::Endpoint::client(client_addr)?;
    client_endpoint.set_default_client_config(ClientConfig::new(Arc::new(QuicClientConfig::try_from(
        rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(SkipServerVerification::new())
            .with_no_client_auth(),
    )?)));


    println!("Quinn endpoints setup successfully.");
    // Create transport ends
    println!("Creating transport ends.");
    let mut transport_ends_vec = Vec::new();
    for address in server_addrs {
        let ends = create(client_endpoint.clone(),address).await?;
        transport_ends_vec.push(ends);
    }
    println!("Transport ends created successfully.");

    
    // Process each transport end
    for (index, transport_end) in transport_ends_vec.iter().enumerate() {
        println!("Processing transport end {}", index);
        
        // Create RTO context
        println!("Creating RTO context for endpoint {}", index);
        let (_context_user, image_steganographer): (_, ServiceToImport<dyn ImageSteganographer>) =
            Context::with_initial_service_import(Config::default_setup(), transport_end.send.clone(), transport_end.recv.clone());
        let image_steganographer_proxy: Box<dyn ImageSteganographer> = image_steganographer.into_proxy();
        println!("RTO context created successfully for endpoint {}", index);

        // Load the secret image
        let secret_image = tokio::task::block_in_place(move || {
            let img = image::open("/home/magdeldin@auc.egy/Cloud-P2P-environment/client/secret.jpg").unwrap();
            let mut buffer = Vec::new();
            img.write_to(&mut buffer, image::ImageFormat::PNG).unwrap();
            buffer
        });

        let secret_image_bytes: &[u8] = &secret_image;
        println!("Secret image loaded successfully for endpoint {}", index);

        // Generate unique output paths for each endpoint
        let stego_path = format!("/home/magdeldin@auc.egy/stego_{}.png", index);
        let finale_path = format!("/home/magdeldin@auc.egy/finale_{}.png", index);

        let stegano = image_steganographer_proxy.encode(secret_image_bytes, &stego_path).unwrap();
        println!("Encode method invoked successfully for endpoint {}", index);

        let local_steganogragrapher = SomeImageSteganographer::new(100, 10);
        let _finale = local_steganogragrapher.decode(&stegano, &finale_path).unwrap();
        println!("Decode method invoked successfully for endpoint {}", index);
    }



    // Keep the server running
    tokio::signal::ctrl_c().await?;
    println!("Shutting down server...");

    Ok(())
} 