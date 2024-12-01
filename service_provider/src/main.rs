use quinn::{Endpoint, ServerConfig, TransportConfig, ClientConfig};
use rand::seq::index;
use rustls::server;
use std::error::Error;
use std::net::SocketAddr;
use remote_trait_object::{Context, Service, ServiceToExport, Config};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use tonic::{transport::Server, Request, Response, Status};
use futures::{FutureExt, StreamExt};
use tokio::sync::{Mutex};
use std::collections::{HashMap, VecDeque};
mod cloud_leader_election;
use cloud_leader_election::{State, VoteReason, SystemMetrics, Node};
use steganography::*;

use image_steganographer::encoder_server::{Encoder, EncoderServer};
use image_steganographer::{EncodeRequest, EncodeReply};
use port_grabber::port_grabber_server::{PortGrabber, PortGrabberServer};
use port_grabber::{PortRequest, PortReply};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime, PrivatePkcs8KeyDer};

use std::sync::Arc;
mod quinn_utils;
use quinn_utils::*;
use std::time::SystemTime;
use stegano_core::SteganoCore;
use std::time::UNIX_EPOCH;
use rand::Rng;
use image::ImageFormat;
use std::fs::File;
use std::io::{Read, Write};
use local_ip_address::local_ip;
use std::net::IpAddr;
use std::net::UdpSocket;
use mac_address;
use tokio::time::sleep;
use tokio::time::timeout;
use std::time::Duration;
use serde::{Serialize, Deserialize};

mod dos;
use dos::{add_dos_entry, delete_dos_entry, setup_drive_hub, get_all_entries};


pub mod port_grabber {
    tonic::include_proto!("port_grabber");
}

pub mod image_steganographer {
    tonic::include_proto!("steganography");
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct dos_heartbeat{
    pub mac_address: String,
    pub ip_address: String,
    pub resources: String,
}


pub static CURRENT_LEADER_ID: AtomicU64 = AtomicU64::new(0);
pub static PERSONAL_ID: AtomicU64 = AtomicU64::new(0);



#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Setup Quinn endpoints for Node

    let hub = setup_drive_hub().await?;
    let filename = "DoS.tsv";
    let drivename = "DoS";

    // get all entries usage

    // match get_all_entries(&hub, filename).await {
    //     Ok(entries) => {
    //         for entry in entries {
    //             println!("{:?}", entry);
    //         }
    //     }
    //     Err(e) => eprintln!("Error retrieving entries: {}", e),
    // }


    let server_addr_leader_election: SocketAddr = "0.0.0.0:0".parse()?;
    let directory_of_service_addr: SocketAddr = "10.7.19.117:5020".parse()?;
    let peer_servers_leader_election: Vec<SocketAddr> = vec![
        // "10.7.16.154:5016".parse()?,
        // "10.7.16.71:5016".parse()?,
    ];
    
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let cert_der = CertificateDer::from(cert.cert);
    let priv_key = PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der());

    let mut directory_of_service_server_config =
        ServerConfig::with_single_cert(vec![cert_der.clone()], priv_key.into())?;
    let directory_of_service_server_transport_config = Arc::get_mut(&mut directory_of_service_server_config.transport).unwrap();
    directory_of_service_server_transport_config.max_concurrent_uni_streams(0_u8.into());
    // directory_of_service_server_transport_config.keep_alive_interval(Some(Duration::from_secs(5)));
    // directory_of_service_server_transport_config.max_idle_timeout(Some(std::time::Duration::from_secs(10).try_into().unwrap()));
    

    // Spawn a new task to listen for connection requests on the directory of service endpoint
    println!("Quin node is beginning setup");
    let my_id = 2; // Make sure this matches your node ID
    PERSONAL_ID.store(my_id as u64, AtomicOrdering::Relaxed);


    let connections: Arc<Mutex<Vec<quinn::Connection>>> = Arc::new(Mutex::new(Vec::new()));
    let connections_clone = Arc::clone(&connections);
    let directory_service = tokio::spawn(async move {
        let (directory_of_service_addr_endpoint,_cert) = make_server_endpoint(directory_of_service_addr).unwrap();
        let heartbeat_tracker: Arc<Mutex<HashMap<dos_heartbeat, std::time::Instant>>> = Arc::new(Mutex::new(HashMap::new()));
        
        loop {
            //println!("Listening!!!");
            if CURRENT_LEADER_ID.load(AtomicOrdering::Relaxed) != PERSONAL_ID.load(AtomicOrdering::Relaxed) {
                // Ignore connections if not the leader
                continue;
            }
            match timeout(Duration::from_millis(100), directory_of_service_addr_endpoint.accept()).await {
                Ok(Some(incoming)) => {
                    if let Ok(conn) = incoming.await {
                        println!("{}", conn.remote_address());
                        if let Ok((mut send, mut recv)) = conn.accept_bi().await {
                           println!("Connection Accepted");
                            
                            let mut buffer: Vec<u8> = Vec::new();
                            if let Ok(buffer) = recv.read_to_end(1024 * 1024).await {
                               // println!("Message Received");
                                println!("Message length: {}", buffer.len());
                                
                                if let Ok(msg) = bincode::deserialize::<dos_heartbeat>(&buffer) {
                                    println!("Message: {}", msg.mac_address);
                                    let mut heartbeat_map = heartbeat_tracker.lock().await;
                                    heartbeat_map.insert(msg.clone(), std::time::Instant::now());
                                    if let Err(e) = add_dos_entry(&hub, &msg.clone().ip_address ,  &msg.mac_address, &msg.resources, &drivename,&filename).await {
                                        println!("Failed to add DoS entry: {}", e);
                                    }
                                    println!("Heartbeat Tracker: {:?}", heartbeat_map);
                                    // Update or add the MAC address to the heartbeat tracker
                                    if let Ok(response) = bincode::serialize(&"MAC address received".to_string()) {
                                        if let Err(e) = send.write_all(&response).await {
                                            println!("Failed to send response: {}", e);
                                        }
                                        send.finish();
                                        sleep(Duration::from_secs(1)).await;
                                    }
                                    else {
                                        println!("FAILED TO SERIALIZE RESPONSE");
                                    }
                                } else {
                                    println!("MESSAGE NEVER DECODED");
                                }
                            } else {
                                println!("MESSAGE NEVER RECEIVED");
                            }
                        } else {
                            println!("CONNECTION NEVER ACCEPTED");
                        }
                    }
                }
                Ok(None) => {
                    // No incoming connection within the timeout duration
                }
                Err(_) => {
                    // println!("Timeout occurred while waiting for incoming connections");
                }
            }
            // Check for stale heartbeats (over 30 seconds old)
            let mut heartbeat_map = heartbeat_tracker.lock().await;
            let now = std::time::Instant::now();
            let mut to_remove = Vec::new();
            for (mac_addr, last_heartbeat) in heartbeat_map.iter() {
                if now.duration_since(*last_heartbeat).as_secs() > 30 {
                    println!("Node {} has not sent a heartbeat in over 30 seconds", mac_addr.mac_address);
                    if let Err(e) = add_dos_entry(&hub, "0.0.0.1", "new_client0", "dummy.jpg", &drivename, &filename).await {
                        println!("Failed to add DoS entry: {}", e);
                    }
                    if let Err(e) = delete_dos_entry(&hub, &mac_addr.ip_address, &drivename, &filename).await {
                        println!("Failed to delete DoS entry: {}", e);
                    }
                    to_remove.push(mac_addr.clone());
                }
            }
            for mac_addr in to_remove {
                heartbeat_map.remove(&mac_addr);
            }
            
            sleep(Duration::from_millis(100)).await;
        }
    });
    // Monitor connections for failures
    // tokio::spawn(async move {
    //     loop {
    //         let mut connections = connections_clone.lock().await;
    //         connections.retain(|conn| {
    //             conn.close(0u32.into(), &[]);
    //             if conn.close_reason().is_some() {
    //                 println!("Connection closed: {:?}", conn.remote_address());
    //                 // Handle the closed connection here
    //                 false
    //             } else {
    //                 true
    //             }
    //         });
    //         tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    //     }
    // });
    

    // Setup your Quinn node for leader election and message passing
    let mut quinn_node = Node::new(my_id, server_addr_leader_election, peer_servers_leader_election).await?;
    
    // Spawn Quinn node task
    let node_handle = tokio::spawn(async move {
        quinn_node.run().await;
        tokio::signal::ctrl_c().await.map_err(|e| Box::new(e) as Box<dyn Error + Send>)?;
        println!("Shutting down node...");
        Ok::<(), Box<dyn Error + Send>>(())
    });

    // Create and start gRPC server
    let grpc_addr = "[::]:50051".parse::<SocketAddr>()?;
    let port_grabber_service = PortGrabberService::default();
    Server::builder()
        .add_service(PortGrabberServer::new(port_grabber_service))
        .serve(grpc_addr)
        .await?;

    // Wait for Ctrl-C signal to shutdown the server gracefully
    tokio::signal::ctrl_c().await?;
    println!("Shutting down server...");

    // Await both handles to ensure clean shutdown
    let _ = tokio::join!(node_handle, directory_service);

    Ok(())
}

#[derive(Debug, Default)]
pub struct PortGrabberService {}

#[tonic::async_trait]
impl PortGrabber for PortGrabberService {
    async fn get_port(
        &self,
        request: Request<PortRequest>,
    ) -> Result<Response<PortReply>, Status> {
        let _port_request = request.into_inner();
        let local_ip: IpAddr = local_ip().unwrap();
        let local_addr = SocketAddr::new(local_ip, 0);

        let socket = UdpSocket::bind(local_addr).map_err(|e| Status::internal(e.to_string()))?;
        let final_addr = socket.local_addr().map_err(|e| Status::internal(e.to_string()))?;
        // println!("Assigned port: {}", final_addr.port());
        std::mem::drop(socket);

        // Spawn a new task for the ImageSteganographyService server
        tokio::spawn(async move {
            let final_addr = final_addr.clone();
            let grpc_addr = format!("[::]:{}", final_addr.port()).parse::<SocketAddr>().unwrap();
            let image_steg_service = ImageSteganographyService::default();
            let server = Server::builder()
            .add_service(EncoderServer::new(image_steg_service))
            .serve(grpc_addr);

            // Run the server with a timeout of 200 seconds
            let timeout = tokio::time::sleep(tokio::time::Duration::from_secs(200));
            tokio::select! {
            _ = server => {},
            _ = timeout => {
                println!("ImageSteganographyService server shutting down after 200 seconds");
            },
            }
        });

        let addr = format!("{}", final_addr);
        let reply = PortReply {
            port: addr,
        };

        Ok(Response::new(reply))
    }
}


#[derive(Debug, Default)]
pub struct ImageSteganographyService {}

#[tonic::async_trait]
impl Encoder for ImageSteganographyService {
    async fn encode(
        &self,
        request: Request<EncodeRequest>,
    ) -> Result<Response<EncodeReply>, Status> {

        if CURRENT_LEADER_ID.load(AtomicOrdering::Relaxed) != PERSONAL_ID.load(AtomicOrdering::Relaxed) {
            return Err(Status::internal("Not the leader"));
        }

        let encode_request = request.into_inner();
        let secret_image = encode_request.image;
        let output_path = encode_request.output_path;
        let file_name = encode_request.file_name;

        let temp_secret_path = format!("/tmp/{}",file_name);
        let mut temp_secret_file = File::create(&temp_secret_path).map_err(|e| Status::internal(e.to_string()))?;
        temp_secret_file.write_all(&secret_image).map_err(|e| Status::internal(e.to_string()))?;
        temp_secret_file.flush().map_err(|e| Status::internal(e.to_string()))?;


        let carrier_path = "carrier.png";

        let mut stegano_encoder = SteganoCore::encoder();

        let encoder = match stegano_encoder
            .hide_file(&temp_secret_path)
            .use_media(&carrier_path) {
                Ok(enc) => enc,
                Err(e) => return Err(Status::internal(format!("Failed to use media: {}", e))),
            };
        encoder
            .write_to(&output_path)
            .hide();
        


        println!("Encoded image saved to {}", output_path);

        let encoded_image = image::open(output_path).unwrap();


        let mut buffer = Vec::new();
        encoded_image.write_to(&mut buffer, ImageFormat::PNG).map_err(|e| Status::internal(e.to_string()))?;
        
        // Delete the temporary secret image file
        std::fs::remove_file(&temp_secret_path).map_err(|e| Status::internal(e.to_string()))?;
        println!("Buffer length: {}", buffer.len());


        let reply = EncodeReply {
            image: buffer,  // Return the encoded image buffer
        };

        Ok(Response::new(reply))
    }
}