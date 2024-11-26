use std::path::Path;
use std::fs::File;
use image::{DynamicImage, GenericImageView, ImageBuffer, ImageFormat, Rgb, Rgba};
use stegano_core::commands::unveil;
use steganography::util::{file_as_dynamic_image, save_image_buffer};
use steganography::{encoder::*, decoder::*};
use serde;
use serde_json;

use crossbeam::channel::{unbounded, Receiver, Sender};
use tracing_subscriber::fmt::format;
use std::collections::HashMap;
use std::{env, thread};

use std::io::{Read, Write};
use std::sync::{Arc, Barrier};
use serde::{Deserialize, Serialize};
use std::error::Error;
use stegano_core::{SteganoCore,SteganoEncoder, CodecOptions};

use crate::quinn_utils::*;
use quinn_proto::crypto::rustls::QuicClientConfig;
use quinn::{ClientConfig, Connecting, Endpoint, ServerConfig, TransportConfig, Connection};

use std::time::Duration;
use std::net::SocketAddr;


#[derive(Serialize, Deserialize, Debug)]
pub struct ImageData {
    pub image: Vec<u8>,
    pub file_name: String,
    pub output_path: String,
}

pub struct ImageSteganographer{
    server_addrs: Vec<SocketAddr>,
    client_addr: SocketAddr,
}

impl ImageSteganographer {

    pub fn new(server_addrs: Vec<SocketAddr>, client_addr: SocketAddr) -> Result<Self, Box<dyn Error>> {

        Ok(ImageSteganographer {
            client_addr: client_addr,
            server_addrs: server_addrs,
        })
    }

    pub async fn encode(&self, secret_image: &[u8], output_path: &str, file_name: &str) -> Result<Vec<u8>, String> {
        

        let barrier = Arc::new(Barrier::new(self.server_addrs.len()));
        let mut handles = vec![];

        for (i, addr) in self.server_addrs.iter().enumerate() {
            let barrier = Arc::clone(&barrier);
            let server_addrs = self.server_addrs.clone();
            let addr = addr.clone();
            let client_addr = self.client_addr.clone();
            let secret_image = secret_image.to_vec().clone();
            let output_path = output_path.to_string().clone();
            let file_name = file_name.to_string().clone();
            
            let handle = tokio::spawn(async move {
                // Simulate some work with the endpoint
                println!("Thread {} started", i);
            
                // Perform encoding work here
                // For example, you can call a function to encode the image
                println!("Beginning Encoding");

                let quic_client_config = match QuicClientConfig::try_from(
                    rustls::ClientConfig::builder()
                    .dangerous()
                    .with_custom_certificate_verifier(SkipServerVerification::new())
                    .with_no_client_auth(),
                ) {
                    Ok(config) => config,
                    Err(e) => {
                    eprintln!("Error creating QuicClientConfig: {:?}", e);
                    return Vec::<u8>::new();
                    }
                };
                let mut client_config = ClientConfig::new(Arc::new(quic_client_config));
                let mut transport_config = TransportConfig::default();
                transport_config.keep_alive_interval(Some(Duration::from_secs(5)));
                client_config.transport_config(Arc::new(transport_config));
                
                let mut client_endpoint = match quinn::Endpoint::client(client_addr) {
                    Ok(endpoint) => endpoint,
                    Err(e) => {
                    eprintln!("Error creating client endpoint: {:?}", e);
                    return Vec::new();
                    }
                };
                client_endpoint.set_default_client_config(client_config);
            
                // Establish connections
                let client_connecting = client_endpoint.connect(
                    server_addrs[i],
                    "localhost",
                ).map_err(|e| {
                    eprintln!("Error connecting: {:?}", e);
                    return Vec::<u8>::new();
                }).ok();
                
                let client_conn = client_connecting;
                let client_conn = match client_conn {
                    Some(conn) => conn,
                    None => {
                    return Vec::<u8>::new();
                    }
                };
            
                let client_conn = tokio::time::timeout(Duration::from_secs(100), client_conn).await
                    .map_err(|e| {
                    eprintln!("Connection timed out: {:?}", e);
                    return Vec::<u8>::new();
                    })
                    .and_then(|res| res.map_err(|e| {
                    eprintln!("Error connecting: {:?}", e);
                    return Vec::new();
                    }))
                    .ok();
            
                let client_conn = match client_conn {
                    Some(conn) => conn,
                    None => {
                    return Vec::new();
                    }
                };
                
            
                // Receive the server's IP address
            
                // let server_address = match client_conn.accept_bi().await {
                //     Ok((_, mut recv)) => {
                //     let max_size = 500 * 1024 * 1024; // 500MB max size, adjust as needed
                //     let buffer = recv.read_to_end(max_size).await
                //         .map_err(|e| {
                //         eprintln!("Error reading data: {:?}", e);
                //         e.to_string()
                //         }).unwrap_or_else(|e| {
                //         eprintln!("Error reading data: {:?}", e);
                //         Vec::new()
                //         });
            
                //     let server_ip = match String::from_utf8(buffer) {
                //         Ok(ip) => ip,
                //         Err(e) => {
                //         eprintln!("Error converting buffer to string: {:?}", e);
                //         return Vec::new();
                //         }
                //     };
                //     println!("Received server IP address: {}", server_ip);
                //     let server_address: SocketAddr = match server_ip.parse::<SocketAddr>() {
                //         Ok(addr) => addr,
                //         Err(e) => {
                //         eprintln!("Error parsing server address: {:?}", e);
                //         return Vec::new();
                //         }
                //     };
                //     Ok(server_address)
                //     },
                //     Err(e) => {
                //     eprintln!("Error accepting stream: {:?}", e);
                //     Err(e.to_string())
                //     }
                // };
            
                // let server_address = match server_address {
                //     Ok(addr) => addr,
                //     Err(e) => {
                //     eprintln!("Error parsing server address: {:?}", e);
                //     return Vec::new();
                //     }
                // };
            
            
                let new_client_connecting = client_endpoint.connect(
                    addr,
                    "localhost",
                ).map_err(|e| {
                    eprintln!("Error connecting: {:?}", e);
                    return Vec::<u8>::new();
                }).ok();
                
                let new_client_connecting = match new_client_connecting {
                    Some(conn) => conn,
                    None => {
                    return Vec::<u8>::new();
                    }
                };
            
                let new_client_conn = match new_client_connecting.await {
                    Ok(conn) => conn,
                    Err(e) => {
                    eprintln!("Error connecting: {:?}", e);
                    return Vec::<u8>::new();
                    }
                };
                
                println!("Connections established successfully.");
            
                let (mut send, mut recv) = match new_client_conn.open_bi().await {
                    Ok(stream) => stream,
                    Err(e) => {
                    eprintln!("Error opening bidirectional stream: {:?}", e);
                    return Vec::new();
                    }
                };

                let image_data = ImageData {
                    image: secret_image.clone(),
                    file_name: file_name.clone(),
                    output_path: output_path.clone(),
                };

                let image_data_bytes = serde_json::to_vec(&image_data).expect("Failed to serialize image data");

                println!("Sending image data to server");
                if let Err(e) = send.write_all(&image_data_bytes).await {
                    eprintln!("Error writing to stream: {:?}", e);
                    return Vec::new();
                }
                if let Err(e) = send.finish() {
                    eprintln!("Error finishing stream: {:?}", e);
                    return Vec::new();
                }
                
                println!("Receiving encoded image from server");
                let max_size = 500 * 1024 * 1024; // 500MB max size, adjust as needed
                let encoded_data = match recv.read_to_end(max_size).await {
                    Ok(data) => data,
                    Err(e) => {
                    eprintln!("Error reading from stream: {:?}", e);
                    return Vec::new();
                    }
                };
            
                println!("Thread {} finished", i);
                encoded_data
            });
            handles.push(handle);
        }

        // Wait for all tasks to complete and collect the results
        let mut encoded_images = Vec::new();
        for handle in handles {
            match handle.await {
            Ok(data) => encoded_images.push(data),
            Err(e) => eprintln!("Error in task: {:?}", e),
            }
        }

        // Perform the final encoding step
        let encoded_image = encoded_images[0].clone();

        Ok(encoded_image)

    }


    pub fn decode(&self, encoded_image: &[u8], decoded_image_path: &str, file_name: &str) -> Result<Vec<u8>, String> {
        
        let encoded_image = image::load_from_memory(encoded_image).unwrap();
        // let encoded_bytes = encoded_image.to_rgba();
        // let decoder = Decoder::new(encoded_bytes);
        // let decoded_bytes = decoder.decode_alpha();
        // let decoded_bytes: &[u8]= &decoded_bytes;
        
        // let decoded_image = image::load_from_memory(decoded_bytes).unwrap();
        // decoded_image.save(decoded_image_path).map_err(|e| e.to_string())?;

        // Save the encoded image to a temporary file
        let temp_enc_path = "/tmp/encoded_image.png";
        encoded_image.save(temp_enc_path).unwrap();
        let _result = unveil(
            &Path::new(temp_enc_path),
            &Path::new(decoded_image_path),
            &CodecOptions::default());

        println!("Decoded image saved to {}", decoded_image_path);
        
        let new_decoded_image_path = decoded_image_path.to_owned()+"/"+file_name;
        let decoded_image = image::open(new_decoded_image_path).unwrap();
        
        let mut buffer = Vec::new();
        decoded_image.write_to(&mut buffer, ImageFormat::PNG).map_err(|e| e.to_string())?;
        std::fs::remove_file(&temp_enc_path).map_err(|e| e.to_string())?;
        Ok(buffer)
    }
}