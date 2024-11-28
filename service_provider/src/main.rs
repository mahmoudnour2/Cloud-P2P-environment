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

use image_steganographer::encoder_server::{Encoder, EncoderServer};
use image_steganographer::{EncodeRequest, EncodeReply};
use port_grabber::port_grabber_server::{PortGrabber, PortGrabberServer};
use port_grabber::{PortRequest, PortReply};


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

pub mod port_grabber {
    tonic::include_proto!("port_grabber");
}

pub mod image_steganographer {
    tonic::include_proto!("steganography");
}

pub static CURRENT_LEADER_ID: AtomicU64 = AtomicU64::new(0);
pub static PERSONAL_ID: AtomicU64 = AtomicU64::new(0);





#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Setup Quinn endpoints for Node
    let server_addr_leader_election: SocketAddr = "0.0.0.0:0".parse()?;
    let peer_servers_leader_election: Vec<SocketAddr> = vec![
        // "10.7.16.154:5016".parse()?,
        // "10.7.16.71:5016".parse()?,
    ];



    println!("Quin node is beginning setup");
    let my_id = 2; // Make sure this matches your node ID
    PERSONAL_ID.store(my_id as u64, AtomicOrdering::Relaxed);

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
    let _ = tokio::join!(node_handle);

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
        println!("Assigned port: {}", final_addr.port());
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

        let encode_request = request.into_inner();
        let secret_image = encode_request.image;
        let output_path = encode_request.output_path;
        let file_name = encode_request.file_name;


        if (CURRENT_LEADER_ID.load(AtomicOrdering::Relaxed) != PERSONAL_ID.load(AtomicOrdering::Relaxed)) {
            return Err(Status::internal("Not the leader"));
        }

        println!("Beginning Encoding");



        // Save the secret image to a temporary file
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let random_num = rand::thread_rng().gen_range(0..1000);
        let temp_secret_path = format!("/tmp/{}_{}_{}", timestamp, random_num, file_name);
        let mut temp_secret_file = match File::create(&temp_secret_path) {
            Ok(file) => file,
            Err(e) => return Err(Status::internal(format!("Failed to create temp file: {}", e))),
        };
        if let Err(e) = temp_secret_file.write_all(&secret_image) {
            return Err(Status::internal(format!("Failed to write to temp file: {}", e)));
        }
        if let Err(e) = temp_secret_file.flush() {
            return Err(Status::internal(format!("Failed to flush temp file: {}", e)));
        }

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

        let encoded_image = match image::open(output_path) {
            Ok(img) => img,
            Err(e) => return Err(Status::internal(format!("Failed to open encoded image: {}", e))),
        };

        let mut buffer = Vec::new();
        if let Err(e) = encoded_image.write_to(&mut buffer, ImageFormat::PNG) {
            return Err(Status::internal(format!("Failed to write encoded image to buffer: {}", e)));
        }

        // Delete the temporary secret image file
        if let Err(e) = std::fs::remove_file(&temp_secret_path) {
            return Err(Status::internal(format!("Failed to delete temp file: {}", e)));
        }

        println!("Buffer length: {}", buffer.len());


        let reply = EncodeReply {
            image: buffer,  // Return the encoded image buffer
        };

        Ok(Response::new(reply))
    }
}