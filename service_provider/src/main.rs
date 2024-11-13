use quinn::{Endpoint, ServerConfig, TransportConfig, ClientConfig};
use rand::seq::index;
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
use futures::{FutureExt, StreamExt};
use tokio::time::{timeout, Duration};
use tokio::task::spawn_blocking;
use tokio::sync::Mutex;
use std::collections::HashMap;

pub static CURRENT_LEADER_ID: AtomicU64 = AtomicU64::new(0);

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {

    // Setup Quinn endpoints for Node
    let server_addr: SocketAddr = "127.0.0.1:5016".parse()?;
    let client_addresses: Vec<SocketAddr> = vec![
        // "127.0.0.1:5010".parse()?,
    ];

    // Setup Quinn endpoints for steganographer
    let server_addrs: Vec<SocketAddr> = vec![
        "127.0.0.1:5011".parse()?,
    ];

    println!("Quin node is beginning setup");

    let mut quinn_node = Node::new(3, server_addr_leader_election, peer_servers_leader_election).await?;
    // Spawn the Node task
    let node_handle = tokio::spawn(async move {
        quinn_node.run().await;
        tokio::signal::ctrl_c().await.map_err(|e| Box::new(e) as Box<dyn Error + Send>)?;
        println!("Shutting down node...");
        Ok::<(), Box<dyn Error + Send>>(())
    });

    let my_id = 3; // Make sure this matches your node ID

    
    let mut server_endpoints = Vec::new();
    for addr in server_addrs {
        let (endpoint, _cert) = make_server_endpoint(addr).unwrap();
        server_endpoints.push(endpoint);
    }
    println!("Steganography service endpoints are setup");

    let transport_ends_vec = Arc::new(Mutex::new(Vec::new()));
    let transport_ends_vec_clone = Arc::clone(&transport_ends_vec);

    let connection_handle = tokio::spawn(async move {
        loop {
            // Check if this node is the current leader
            if CURRENT_LEADER_ID.load(AtomicOrdering::SeqCst) != my_id {
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
            for endpoint in &server_endpoints {
                match timeout(Duration::from_secs(1), endpoint.accept()).await {
                    Ok(Some(incoming)) => {
                        println!("Received a connection request from client");
                        //incoming.await accepts the connection
                        if let Ok(conn) = incoming.await {
                            let ends = match create(conn).await {
                                Ok(ends) => ends,
                                Err(e) => {
                                    eprintln!("Failed to create transport ends: {}", e);
                                    continue;
                                }
                            };
                            let mut vec = transport_ends_vec_clone.lock().await;
                            vec.push(ends);
                        }
                    }
                    Ok(None) => {
                        println!("Server endpoint has stopped accepting new connections");
                        // No incoming connection within the timeout duration
                    }
                    Err(_) => {
                        println!("No incoming connection within the timeout duration");
                    }
                }
            }
            let ctrl_c_timeout = Duration::from_secs(1);
            match timeout(ctrl_c_timeout, tokio::signal::ctrl_c()).await {
                Ok(Ok(())) => {
                    break;
                }
                Ok(Err(e)) => {
                    println!("Error waiting for Ctrl+C: {}", e);
                }
                Err(_) => {
                    // Timeout occurred, continue the loop
                }
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }

        // Wait for Ctrl-C
        tokio::signal::ctrl_c().await.map_err(|e| e.to_string())?;
        println!("Shutting down connection handle...");
        Ok::<(), String>(())
    });


    let contexts: Arc<Mutex<HashMap<TransportEnds, Context>>> = Arc::new(Mutex::new(HashMap::new()));

    // Spawn the steganographer service task
    let steg_handle = tokio::spawn(async move {
        
        loop {
            let mut contexts = contexts.lock().await;
            println!("{}",contexts.len());
            let mut vec = transport_ends_vec.lock().await;
            vec.retain(|ends| {
                if ends.is_active() {
                    if CURRENT_LEADER_ID.load(AtomicOrdering::SeqCst) == my_id {
                        // Only create and export the service if this node is the leader and the context doesn’t already exist
                        if !contexts.contains_key(ends) {
                            let context = Context::with_initial_service_export(
                                Config::default_setup(),
                                ends.send.clone(),
                                ends.recv.clone(),
                                ServiceToExport::new(Box::new(SomeImageSteganographer::new(75, 10)) as Box<dyn ImageSteganographer>),
                            );
                            contexts.insert(ends.clone(), context);
                            println!("Steganographer service started for client {:?}", ends.get_remote_address());
                        }
                    } else {
                        println!("This node is not the leader; no new context created.");
                    }
                    true
                } else {
                    // Remove context if the connection is no longer active
                    if contexts.remove(ends).is_some() {
                        println!("Context removed for inactive connection {:?}", ends);
                    }
                    false
                }
            });
            drop(vec); // Release the lock before sleeping

            let ctrl_c_timeout = Duration::from_secs(1);
            match timeout(ctrl_c_timeout, tokio::signal::ctrl_c()).await {
                Ok(Ok(())) => {
                    break;
                }
                Ok(Err(e)) => {
                    println!("Error waiting for Ctrl+C: {}", e);
                }
                Err(_) => {
                    // Timeout occurred, continue the loop
                }
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }

        // Wait for Ctrl-C
        tokio::signal::ctrl_c().await.map_err(|e| e.to_string())?;
        println!("Shutting down steg handle...");

        Ok::<(), String>(())
    });

    // Wait for Ctrl-C

    tokio::signal::ctrl_c().await?;
    println!("Shutting down server...");

    // Await both handles to ensure clean shutdown
    let _ = tokio::join!(node_handle, steg_handle,connection_handle);

    Ok(())
} 
