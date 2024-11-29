use quinn::{Endpoint, ServerConfig, TransportConfig, ClientConfig};
use rand::seq::index;
use rustls::server;
use std::error::Error;
use std::net::SocketAddr;
use remote_trait_object::{Context, Service, ServiceToExport, Config};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

mod transport;
mod image_steganographer;
mod quinn_utils;
mod cloud_leader_election;
mod dos;
use dos::{add_dos_entry, delete_dos_entry, setup_drive_hub, get_dos_entry_by_ip};
use image_steganographer::{ImageSteganographer, SomeImageSteganographer};
use image;
use transport::{create, TransportEnds};
use quinn_utils::*;
use quinn_proto::crypto::rustls::QuicClientConfig;
use cloud_leader_election::{State, VoteReason, SystemMetrics, Node};
use futures::{FutureExt, StreamExt};
use tokio::time::{timeout, Duration};
use tokio::task::spawn_blocking;
use tokio::sync::{Mutex, Semaphore};
use std::collections::{HashMap, VecDeque};

pub static CURRENT_LEADER_ID: AtomicU64 = AtomicU64::new(0);
pub static PERSONAL_ID: AtomicU64 = AtomicU64::new(0);

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {

    // Setup DOS Start
    let hub = setup_drive_hub().await?;
    let filename = "DoS.tsv";
    let drivename = "DoS";
    //The IP you want to add
    let client_ip = "192.168.1.100";

    // Get the DOS entry for this IP
    if let Some(entry) = get_dos_entry_by_ip(client_ip) {
        // Parameters for add_dos_entry:
        // hub, client_ip, client_id, resources, drive_id, tsv_filename
        add_dos_entry(
            &hub, 
            &entry.Client_IP,    // IP from the entry
            &entry.Client_ID,    // Client ID from the entry
            &entry.resources,    // Resources from the entry
            drivename,  // Replace with your actual Drive/Folder ID
            filename    // Your desired filename
        ).await?;
    } else {
        println!("No predefined entry found for IP: {}", client_ip);
    }
    add_dos_entry(&hub, "192.168.1.2" ,  "new_client2", "d1.jpg", &drivename,&filename).await?;
    add_dos_entry(&hub, "192.168.1.3" ,  "new_client3", "d1.jpg", &drivename,&filename).await?;

    // delete_dos_entry(&hub, client_ip, &drivename, &filename).await?;
    // delete_dos_entry(&hub, "192.168.1.3", &drivename, &filename).await?;

    //delete_dos_entry(&hub, "192.168.1.100", &drivename, &filename).await?;


    // add_dos_entry(&hub, "192.168.1.2" ,  "new_client2", "d1.jpg", &drivename,&filename).await?;
    // add_dos_entry(&hub, "192.168.1.3" ,  "new_client3", "d1.jpg", &drivename,&filename).await?;
    // add_dos_entry(&hub, "192.168.1.4" ,  "new_client4", "d1.jpg", &drivename,&filename).await?;

    
    // delete_dos_entry(&hub, "192.168.1.3", &drivename, &filename).await?;
    // delete_dos_entry(&hub, "192.168.1.4", &drivename, &filename).await?;


    /*
    


     */

    // DOS END

    // Setup Quinn endpoints for Node
    let server_addr_leader_election: SocketAddr = "192.168.1.14:5016".parse()?;
    let peer_servers_leader_election: Vec<SocketAddr> = vec![
        // "10.7.16.154:5016".parse()?,
        // "10.7.16.71:5016".parse()?,
    ];

    // Setup Quinn endpoints for steganographer
    let server_addrs: Vec<SocketAddr> = vec![
        "192.168.1.14:5017".parse()?,
    ];

    println!("Quin node is beginning setup");
    let my_id = 2; // Make sure this matches your node ID
    PERSONAL_ID.store(my_id as u64, AtomicOrdering::Relaxed);

    let mut quinn_node = Node::new(my_id, server_addr_leader_election, peer_servers_leader_election).await?;
    // Spawn the Node task
    let node_handle = tokio::spawn(async move {
        quinn_node.run().await;
        tokio::signal::ctrl_c().await.map_err(|e| Box::new(e) as Box<dyn Error + Send>)?;
        println!("Shutting down node...");
        Ok::<(), Box<dyn Error + Send>>(())
    });

    
    let server_endpoints = Arc::new({
        let mut endpoints = Vec::new();
        for addr in server_addrs {
            let (endpoint, _cert) = make_server_endpoint(addr).unwrap();
            endpoints.push(endpoint);
        }
        endpoints
    });
    let server_endpoints_clone = Arc::clone(&server_endpoints);
    println!("Steganography service endpoints are setup");

    let transport_ends_vec = Arc::new(Mutex::new(Vec::new()));
    let transport_ends_vec_clone = Arc::clone(&transport_ends_vec);

    // Limit the number of concurrent connections
    let max_connections = 10;
    let semaphore = Arc::new(Semaphore::new(max_connections));
    let request_queue = Arc::new(Mutex::new(VecDeque::new()));

    let connection_handle = tokio::spawn(async move {
        loop {
            let server_endpoints_clone = Arc::clone(&server_endpoints_clone); // Clone the Arc for use within this iteration
            // Check if this node is the current leader
            if CURRENT_LEADER_ID.load(AtomicOrdering::SeqCst) != my_id {
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
    
            for endpoint in server_endpoints_clone.iter() {
                match semaphore.try_acquire() {
                    Ok(_) => {
                        // Acquired a connection slot
                        let semaphore_clone = semaphore.clone();
                        let transport_ends_vec_clone = Arc::clone(&transport_ends_vec_clone);
                        let endpoint = endpoint.clone(); // Clone the endpoint to move into the task
                        tokio::spawn(async move {
                            match timeout(Duration::from_secs(1000), endpoint.accept()).await {
                                Ok(Some(incoming)) => {
                                    println!("Received a connection request from client");
                                    match incoming.await {
                                        Ok(conn) => {
                                            let ends = match create(conn).await {
                                                Ok(ends) => ends,
                                                Err(e) => {
                                                    eprintln!("Failed to create transport ends: {}", e);
                                                    return;
                                                }
                                            };
                                            let mut vec = transport_ends_vec_clone.lock().await;
                                            vec.push(ends);
                                        }
                                        Err(e) => {
                                            eprintln!("Failed to accept incoming connection: {}", e);
                                        }
                                    }
                                }
                                Ok(None) => {
                                    println!("Server endpoint has stopped accepting new connections");
                                }
                                Err(e) => {
                                    eprintln!("Error accepting incoming connection: {}", e);
                                }
                            }
                            drop(semaphore_clone); // Release the semaphore when done
                        });
                    }
                    Err(_) => {
                        println!("Max connections reached, queuing the request...");
                        let mut queue = request_queue.lock().await;
                        queue.push_back(endpoint.clone());
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        continue;
                    }
                }
            }

            {
                let mut queue = request_queue.lock().await;
                while let Some(endpoint) = queue.pop_front() {
                    match semaphore.try_acquire() {
                        Ok(_) => {
                            let semaphore_clone = semaphore.clone();
                            let transport_ends_vec_clone = Arc::clone(&transport_ends_vec_clone);
                            tokio::spawn(async move {
                                match timeout(Duration::from_secs(1000), endpoint.accept()).await {
                                    Ok(Some(incoming)) => {
                                        println!("Received a connection request from client");
                                        match incoming.await {
                                            Ok(conn) => {
                                                let ends = match create(conn).await {
                                                    Ok(ends) => ends,
                                                    Err(e) => {
                                                        eprintln!("Failed to create transport ends: {}", e);
                                                        return;
                                                    }
                                                };
                                                let mut vec = transport_ends_vec_clone.lock().await;
                                                vec.push(ends);
                                            }
                                            Err(e) => {
                                                eprintln!("Failed to accept incoming connection: {}", e);
                                            }
                                        }
                                    }
                                    Ok(None) => {
                                        println!("Server endpoint has stopped accepting new connections");
                                    }
                                    Err(e) => {
                                        eprintln!("Error accepting incoming connection: {}", e);
                                    }
                                }
                                drop(semaphore_clone); // Release the semaphore when done
                            });
                        }
                        Err(_) => {
                            queue.push_front(endpoint);
                            break;
                        }
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
    
        println!("Shutting down connection handle...");
        Ok::<(), String>(())
    });
    
    


    let contexts: Arc<Mutex<HashMap<TransportEnds, Context>>> = Arc::new(Mutex::new(HashMap::new()));

    // Spawn the steganographer service task
    let steg_handle = tokio::spawn(async move {
        
        loop {
            if CURRENT_LEADER_ID.load(AtomicOrdering::SeqCst) != my_id {
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
            let mut contexts = contexts.lock().await;
            println!("{}",contexts.len());
            let mut vec = transport_ends_vec.lock().await;
            vec.retain(|ends| {
                if ends.is_active() {
                    
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
