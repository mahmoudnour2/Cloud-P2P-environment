use quinn::{Endpoint, ServerConfig, TransportConfig, ClientConfig};
use std::error::Error;
use std::net::SocketAddr;
use remote_trait_object::{Context, Service, ServiceToExport, Config};
use std::sync::Arc;

mod transport;
mod image_steganographer;
mod quinn_utils;
use image_steganographer::{ImageSteganographer, SomeImageSteganographer};
use image;
use transport::{create, TransportEnds};
use quinn_utils::*;
use quinn_proto::crypto::rustls::QuicClientConfig;
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    
    // Setup Quinn endpoints
    let server_addrs: Vec<SocketAddr> = vec![
        "10.7.16.71:5000".parse()?,
        "10.7.16.71:5001".parse()?,
        "10.7.16.71:5002".parse()?
    ];  // Connect to server's ports
    let client_addr: SocketAddr = "10.7.16.80:4800".parse()?;  // Listen on this port

    println!("Quinn endpoints setup beginning.");

    
    let mut server_endpoints = Vec::new();
    for addr in server_addrs {
        let (endpoint, _cert) = make_server_endpoint(addr).unwrap();
        server_endpoints.push(endpoint);
    }

    println!("Quinn endpoints setup successfully.");
    // Create transport ends
    println!("Creating transport ends.");
    let mut transport_ends_vec = Vec::new();
    for endpoint in server_endpoints {
        let ends = create(endpoint).await?;
        transport_ends_vec.push(ends);
    }
    println!("Transport ends created successfully.");
    
    // Create RTO context
    println!("Creating RTO context.");
    let mut contexts = Vec::new();
    for ends in transport_ends_vec {
        let context = Context::with_initial_service_export(
            Config::default_setup(),
            ends.send,
            ends.recv,
            ServiceToExport::new(Box::new(SomeImageSteganographer::new(75,10)) as Box<dyn ImageSteganographer>),
        );
        contexts.push(context);
    }
    println!("RTO context created successfully.");
    
    // Create and register the steganographer service
    let steganographer = SomeImageSteganographer::new(90, 10);


    // Keep the server running
    tokio::signal::ctrl_c().await?;
    println!("Shutting down server...");

    Ok(())
} 