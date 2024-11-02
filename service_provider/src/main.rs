use quinn::{Endpoint, ServerConfig, TransportConfig, ClientConfig};
use std::error::Error;
use std::net::SocketAddr;
use remote_trait_object::{Context, Service, ServiceToExport, Config};
use std::sync::Arc;

mod transport;
mod image_steganographer;
mod quinn_utils;
mod cloud_leader_election;
use image_steganographer::{ImageSteganographer, SomeImageSteganographer};
use image;
use transport::{create, TransportEnds};
use quinn_utils::*;
use quinn_proto::crypto::rustls::QuicClientConfig;
use cloud_leader_election::{State, VoteReason, SystemMetrics, Node};
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    
    /*
    // Setup Quinn endpoints
    let server_addr: SocketAddr = "10.7.16.71:5000".parse()?;  // Connect to server's port
    let client_addr: SocketAddr = "10.7.16.80:4800".parse()?;  // Listen on this port

    println!("Quinn endpoints setup beginning.");

    let (server_endpoint, _server_cert) = make_server_endpoint(server_addr).unwrap();

    println!("Quinn endpoints setup successfully.");
    // Create transport ends
    println!("Creating transport ends.");
    let transport_ends = create(server_endpoint).await?;
    println!("Transport ends created successfully.");
    
    // Create RTO context
    println!("Creating RTO context.");
    let _context_steganographer = Context::with_initial_service_export(
        Config::default_setup(),
        transport_ends.send,
        transport_ends.recv,
        ServiceToExport::new(Box::new(SomeImageSteganographer::new(75,10)) as Box<dyn ImageSteganographer>),
    );
    println!("RTO context created successfully.");
    
    // Create and register the steganographer service
    let steganographer = SomeImageSteganographer::new(90, 10);
    */

    // Setup Quinn endpoints
    let server_addr: SocketAddr = "10.7.16.71:5000".parse()?;  // Connect to server's port
    let client_addresses: Vec<SocketAddr> = vec![
        "10.7.19.137:5000".parse()?,
    ];
    println!("Quinn endpoints setup beginning.");
    let mut quinn_node = Node::new(5,server_addr,client_addresses).await?;
    quinn_node.run().await;
    println!("Quinn endpoints setup successfully.");
    // Keep the server running
    tokio::signal::ctrl_c().await?;
    println!("Shutting down server...");

    Ok(())
} 