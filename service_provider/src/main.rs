use quinn::{Endpoint, ServerConfig, TransportConfig, ClientConfig};
use rand::seq::index;
use rustls::server;
use std::error::Error;
use std::net::SocketAddr;
use remote_trait_object::{Context, Service, ServiceToExport, Config};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use tonic::{transport::Server, Request, Response, Status};
use futures::{FutureExt, StreamExt};
use tokio::sync::{Mutex, Semaphore};
use std::collections::{HashMap, VecDeque};
use cloud_leader_election::{State, VoteReason, SystemMetrics, Node};
use image_steganographer::{ImageSteganographer, SomeImageSteganographer};
use std::sync::Arc;

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

    // Setup Quinn endpoints for steganography (These can be your server IPs for steganography service)
    let server_addrs: Vec<&str> = vec![
        "fe80::4ed7:17ff:fe7f:d11b",  // Example address
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

    // Setup gRPC service (for image steganography)
    let image_steg_service = ImageSteganographyService {
        semaphore: Arc::new(Semaphore::new(10)),  // Limit to 10 concurrent requests
    };

    // Create and start gRPC server
    let grpc_addr = "[::]:50051".parse::<SocketAddr>()?;
    Server::builder()
        .add_service(image_steg_service)
        .serve(grpc_addr)
        .await?;

    // Wait for Ctrl-C signal to shutdown the server gracefully
    tokio::signal::ctrl_c().await?;
    println!("Shutting down server...");

    // Await both handles to ensure clean shutdown
    let _ = tokio::join!(node_handle);

    Ok(())
}

// Define your gRPC service implementation
#[derive(Debug, Default)]
pub struct ImageSteganographyService {
    semaphore: Arc<Semaphore>,  // Semaphore to control concurrency
}

#[tonic::async_trait]
impl ImageSteganographer for ImageSteganographyService {
    async fn encode(
        &self,
        request: Request<EncodeRequest>,
    ) -> Result<Response<EncodeReply>, Status> {
        let _permit = self.semaphore.acquire().await.unwrap();  // Acquire semaphore permit for concurrent requests

        let encode_request = request.into_inner();
        let secret_image = encode_request.secret_image;
        let output_path = encode_request.output_path;
        let file_name = encode_request.file_name;

        // Instantiate the steganographer (the encoder)
        let encoder = SomeImageSteganographer::new(90, 5);  // Example compression quality and pixel diff
        match encoder.encode(&secret_image, &output_path, &file_name) {
            Ok(encoded_image) => {
                let reply = EncodeReply {
                    encoded_image,  // Return the encoded image buffer
                };
                Ok(Response::new(reply))
            }
            Err(err) => Err(Status::internal(err)),
        }
    }

    async fn decode(
        &self,
        request: Request<DecodeRequest>,
    ) -> Result<Response<DecodeReply>, Status> {
        let _permit = self.semaphore.acquire().await.unwrap();  // Acquire semaphore permit

        let decode_request = request.into_inner();
        let encoded_image = decode_request.encoded_image;
        let decoded_image_path = decode_request.decoded_image_path;
        let file_name = decode_request.file_name;

        // Instantiate the steganographer (the decoder)
        let decoder = SomeImageSteganographer::new(90, 5);  // Example compression quality and pixel diff
        match decoder.decode(&encoded_image, &decoded_image_path, &file_name) {
            Ok(decoded_image) => {
                let reply = DecodeReply {
                    decoded_image,  // Return the decoded image buffer
                };
                Ok(Response::new(reply))
            }
            Err(err) => Err(Status::internal(err)),
        }
    }
}