use quinn::{ClientConfig, Endpoint, ServerConfig, TransportConfig};
use remote_trait_object::{Config, Context, Service, ServiceToExport};
use std::error::Error;
use std::net::SocketAddr;
use std::sync::Arc;

mod image_steganographer;
mod quinn_utils;
mod transport;
use image_steganographer::{ImageSteganographer, SomeImageSteganographer};
use quinn_proto::crypto::rustls::QuicClientConfig;
use quinn_utils::*;
use transport::{create, TransportEnds};
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Setup Quinn endpoints
    let server_addr: SocketAddr = "0.0.0.0:5000".parse()?;
    let client_addr: SocketAddr = "127.0.0.1:0".parse()?;

    let (server_endpoint, _server_cert) = make_server_endpoint(server_addr).unwrap();

    let mut client_endpoint = quinn::Endpoint::client(client_addr)?;
    client_endpoint.set_default_client_config(ClientConfig::new(Arc::new(
        QuicClientConfig::try_from(
            rustls::ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(SkipServerVerification::new())
                .with_no_client_auth(),
        )?,
    )));

    // Create transport ends
    let transport_ends = create(server_endpoint, client_endpoint).await?;

    // Create RTO context
    let _context_steganographer = Context::with_initial_service_export(
        Config::default_setup(),
        transport_ends.send1,
        transport_ends.recv1,
        ServiceToExport::new(
            Box::new(SomeImageSteganographer::new(75, 10)) as Box<dyn ImageSteganographer>
        ),
    );

    // Create and register the steganographer service
    let steganographer = SomeImageSteganographer::new(90, 10);

    // Keep the server running
    tokio::signal::ctrl_c().await?;
    println!("Shutting down server...");

    Ok(())
}
