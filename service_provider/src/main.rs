use quinn::{Endpoint, ServerConfig, TransportConfig, ClientConfig};
use std::error::Error;
use std::net::SocketAddr;
use remote_trait_object::{Context, Service, ServiceToExport, Config};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

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

pub static CURRENT_LEADER_ID: AtomicU64 = AtomicU64::new(0);

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Setup Quinn endpoints for Node
    let server_addr: SocketAddr = "10.7.16.154:5016".parse()?;
    let client_addresses: Vec<SocketAddr> = vec![
        "10.7.19.117:5016".parse()?,
        "10.7.16.71:5016".parse()?,
    ];

    // Setup Quinn endpoints for steganographer
    let server_addrs: Vec<SocketAddr> = vec![
        "10.7.16.154:5017".parse()?,
    ];

    println!("Quinn endpoints setup beginning.");

    // Spawn the Node task
    let node_handle = tokio::spawn(async move {
        let mut quinn_node = Node::new(1, server_addr, client_addresses).await?;
        quinn_node.run().await;
        Ok::<(), Box<dyn Error + Send>>(())
    });

    // Spawn the steganographer service task
    let steg_handle = tokio::spawn(async move {
        let my_id = 1; // Make sure this matches your node ID
        let mut server_endpoints = Vec::new();
        for addr in server_addrs {
            let (endpoint, _cert) = make_server_endpoint(addr).unwrap();
            server_endpoints.push(endpoint);
        }

        let mut transport_ends_vec = Vec::new();
        for endpoint in server_endpoints {
            let ends = create(endpoint).await?;
            transport_ends_vec.push(ends);
        }

        let mut contexts = Vec::new();
        for ends in transport_ends_vec {
            // Only create and export the service if this node is the leader
            if CURRENT_LEADER_ID.load(AtomicOrdering::SeqCst) == my_id {
                let context = Context::with_initial_service_export(
                    Config::default_setup(),
                    ends.send,
                    ends.recv,
                    ServiceToExport::new(Box::new(SomeImageSteganographer::new(75,10)) as Box<dyn ImageSteganographer>),
                );
                contexts.push(context);
                println!("Steganographer service started - this node is the leader");
            } else {
                println!("Steganographer service not started - this node is not the leader");
            }
        }

        let _steganographer = SomeImageSteganographer::new(90, 10);
        Ok::<(), Box<dyn Error + Send>>(())
    });

    // Wait for Ctrl-C
    tokio::signal::ctrl_c().await?;
    println!("Shutting down server...");

    // Await both handles to ensure clean shutdown
    let _ = tokio::join!(node_handle, steg_handle);

    Ok(())
} 