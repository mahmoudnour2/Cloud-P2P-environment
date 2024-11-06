use quinn::{Endpoint, ServerConfig, TransportConfig, ClientConfig};
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
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    
    // Setup Quinn endpoints
    let server_addr: SocketAddr = "127.0.0.1:5000".parse()?;  // Connect to server's port
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
    let transport_ends = create(client_endpoint, server_addr).await?;
    println!("Transport ends created successfully.");

    // Create RTO context
    println!("Creating RTO context.");
    let (_context_user, image_steganographer): (_, ServiceToImport<dyn ImageSteganographer>) =
        Context::with_initial_service_import(Config::default_setup(), transport_ends.send, transport_ends.recv);
    let image_steganographer_proxy: Box<dyn ImageSteganographer> = image_steganographer.into_proxy();
    println!("RTO context created successfully.");
    // Test the encode method

    // load the secret image
    let secret_image = tokio::task::spawn_blocking(move || {
        let img = image::open("/home/magdeldin/Cloud-P2P-environment/client/secret.jpg").unwrap();
        let mut buffer = Vec::new();
        img.write_to(&mut buffer, image::ImageFormat::PNG).unwrap();
        buffer
    }).await?;
    // Convert the secret image buffer to a byte slice
    let secret_image_bytes: &[u8] = &secret_image;

    println!("Secret image loaded successfully.");

    let stegano = image_steganographer_proxy.encode(secret_image_bytes, "/home/magdeldin/stego.png").unwrap();
    println!("Encode method invoked successfully.");

    // Test the decode method
    let local_steganogragrapher = SomeImageSteganographer::new(100, 10);
    let finale = local_steganogragrapher.decode(&stegano,"/home/magdeldin/finale.png").unwrap();
    println!("Decode method invoked successfully.");



    // Keep the server running
    tokio::signal::ctrl_c().await?;
    println!("Shutting down server...");

    Ok(())
} 