use quinn::{Endpoint, ServerConfig, TransportConfig, ClientConfig};
use std::error::Error;
use std::net::SocketAddr;
use remote_trait_object::{Context, Service, ServiceToExport, Config};
use std::sync::Arc;

mod transport;
mod image_steganographer;
mod quinn_utils;
use image_steganographer::{ImageSteganographer, SomeImageSteganographer};
use transport::{create, TransportEnds};
use quinn_utils::*;
use quinn_proto::crypto::rustls::QuicClientConfig;
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Setup Quinn endpoints
    let server_addr: SocketAddr = "0.0.0.0:5000".parse()?;
    let client_addr: SocketAddr = "127.0.0.1:0".parse()?;

    let (server_endpoint, _server_cert) = make_server_endpoint(server_addr).unwrap();
    
    
    let mut client_endpoint = quinn::Endpoint::client(client_addr)?;
    client_endpoint.set_default_client_config(ClientConfig::new(Arc::new(QuicClientConfig::try_from(
        rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(SkipServerVerification::new())
            .with_no_client_auth(),
    )?)));

    // Create transport ends
    let transport_ends = create(server_endpoint, client_endpoint).await?;

    // Create RTO context
    let (_context_user, image_steganographer): (_, ServiceToImport<dyn ImageSteganographer>) =
        Context::with_initial_service_import(Config::default_setup(), send2, recv2);
    let image_steganographer_proxy: Box<dyn ImageSteganographer> = image_steganographer.into_proxy();

    // Test the encode method
    image_steganographer_proxy.encode(secret_path.clone(), carrier_path.clone(), output_path1.clone()).unwrap();
    println!("Encode method invoked successfully.");


    // Keep the server running
    tokio::signal::ctrl_c().await?;
    println!("Shutting down server...");

    Ok(())
} 