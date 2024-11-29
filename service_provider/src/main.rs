use quinn::{Endpoint, ServerConfig, TransportConfig, ClientConfig};
use rand::seq::index;
use rustls::{client, server};
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
use tokio::sync::{Mutex, Semaphore};
use std::collections::{HashMap, VecDeque};
use laminar::{Socket, Packet, SocketEvent};
use std::thread;
use local_ip_address::local_ip;
use std::net::IpAddr;
use std::net::UdpSocket;
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StegoRequest {
    pub secret_image: Vec<u8>,
    pub output_path: String,
    pub file_name: String,
}

pub static CURRENT_LEADER_ID: AtomicU64 = AtomicU64::new(0);
pub static PERSONAL_ID: AtomicU64 = AtomicU64::new(0);

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {

    // Setup Quinn endpoints for Node
    let server_addr_leader_election: SocketAddr = "10.7.19.117:5016".parse()?;
    let peer_servers_leader_election: Vec<SocketAddr> = vec![
        "10.7.16.154:5016".parse()?,
        "10.7.16.71:5016".parse()?,
    ];

    // Setup Quinn endpoints for steganographer
    let server_addrs: Vec<SocketAddr> = vec![
        "10.7.19.117:5017".parse()?,
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

    
    let mut server_endpoints = Vec::new();
    for addr in server_addrs.clone() {
        let (endpoint, _cert) = make_server_endpoint(addr).unwrap();
        server_endpoints.push(endpoint);
    }
    println!("Steganography service endpoints are setup");



    // Spawn the steganographer service task
    let steg_handle = tokio::spawn(async move {
        let server_addrs = server_addrs.clone();

        // Creates the socket
        let mut socket = Socket::bind(server_addrs[0]).map_err(|e| e.to_string())?;
        let event_receiver = socket.get_event_receiver();
        let packet_sender = socket.get_packet_sender();
        // Starts the socket, which will start a poll mechanism to receive and send messages.
        let _thread = thread::spawn(move || socket.start_polling());



        

        

        loop {
            // Waits until a socket event occurs
            let result = event_receiver.recv();

            match result {
                Ok(socket_event) => {
                    match socket_event {
                        SocketEvent::Packet(packet) => {
                            let endpoint: SocketAddr = packet.addr();
                            let received_data: &[u8] = packet.payload();

                            let local_ip: IpAddr = local_ip().unwrap();
                            let local_addr = SocketAddr::new(local_ip, 0);

                            let socket = UdpSocket::bind(local_addr).map_err(|e| e.to_string())?;
                            let actual_addr = socket.local_addr().map_err(|e| e.to_string())?;
                            println!("Assigned port: {}", actual_addr.port());
                            std::mem::drop(socket);

                            let addr = format!("{}", actual_addr);
                            let response_packet = Packet::reliable_ordered(endpoint, addr.as_bytes().to_vec(), None);
                            packet_sender.send(response_packet).map_err(|e| e.to_string())?;

                            let steg_handle = tokio::spawn(async move {
                                
                                // Creates the socket
                                let mut socket = Socket::bind(actual_addr).map_err(|e| e.to_string());
                                let mut socket = match socket {
                                    Ok(socket) => socket,
                                    Err(e) => {
                                        println!("Error binding socket: {:?}", e);
                                        return;
                                    }
                                };
                                let event_receiver_steg = socket.get_event_receiver();
                                let packet_sender_steg = socket.get_packet_sender();
                                // Starts the socket, which will start a poll mechanism to receive and send messages.
                                let _thread = thread::spawn(move || socket.start_polling());

                                let result = event_receiver_steg.recv();

                                match result {
                                    Ok(socket_event) => {
                                        match socket_event {
                                            SocketEvent::Packet(packet) => {
                                                let endpoint: SocketAddr = packet.addr();
                                                let received_data: &[u8] = packet.payload();

                                                let stego_request = bincode::deserialize(received_data).map_err(|e| e.to_string());
                                                let stego_request: StegoRequest = match stego_request {
                                                    Ok(request) => request,
                                                    Err(e) => {
                                                        println!("Error deserializing request: {:?}", e);
                                                        return;
                                                    }
                                                };
                                                let steganographer = SomeImageSteganographer::new(90,90);
                                                let secret_image = stego_request.secret_image;
                                                let output_path = stego_request.output_path;
                                                let file_name = stego_request.file_name;
                                                let encoded_buffer = steganographer.encode(&secret_image, &output_path, &file_name).map_err(|e| e.to_string());
                                                let encoded_buffer = match encoded_buffer {
                                                    Ok(buffer) => buffer,
                                                    Err(e) => {
                                                        println!("Error encoding image: {:?}", e);
                                                        Vec::new()
                                                    }
                                                };

                                                let response_packet = Packet::reliable_ordered(endpoint, encoded_buffer, None);
                                                packet_sender_steg.send(response_packet).map_err(|e| e.to_string());
                                            }
                                            SocketEvent::Connect(connect_event) => { /* a client connected */ }
                                            SocketEvent::Timeout(timeout_event) => { /* a client timed out */ }
                                            SocketEvent::Disconnect(disconnect_event) => { /* a client disconnected */ }
                                        }
                                    }
                                    Err(e) => {
                                        println!("Something went wrong when receiving, error: {:?}", e);
                                    }
                                }
                            });

                            // Spawn a task to delete the thread after 200 seconds
                            tokio::spawn(async move {
                                tokio::time::sleep(Duration::from_secs(200)).await;
                                steg_handle.abort();
                            });
                        }
                        SocketEvent::Connect(connect_event) => { /* a client connected */ }
                        SocketEvent::Timeout(timeout_event) => { /* a client timed out */ }
                        SocketEvent::Disconnect(disconnect_event) => { /* a client disconnected */ }
                    }

                    

                }
                Err(e) => {
                    println!("Something went wrong when receiving, error: {:?}", e);
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
    let _ = tokio::join!(node_handle, steg_handle);

    Ok(())
} 
