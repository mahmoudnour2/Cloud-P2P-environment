use quinn::{ClientConfig, Connecting, Endpoint, ServerConfig, TransportConfig};
use rustls::{client, server};
use core::num;
use std::error::Error;
use std::net::SocketAddr;
use remote_trait_object::{Context, Service, Config, ServiceToImport};
use std::sync::Arc;
use std::time::Duration;
use std::panic::AssertUnwindSafe;

mod transport;
mod image_steganographer;
mod quinn_utils;
use image_steganographer::{ImageSteganographer, SomeImageSteganographer};
use transport::{create, TransportEnds};
use quinn_utils::*;
use quinn_proto::crypto::rustls::QuicClientConfig;
use image;
use steganography::{self, util::file_to_bytes};
use tokio::task;
use tokio::time::timeout;
use std::env;
use std::process::{Command, exit};
use std::fs::OpenOptions;
use std::io::Write;
use tokio::sync::Semaphore;
use tokio::time::sleep;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    
    // Add near the start of main
    std::fs::create_dir_all("encoded_images")
    .map_err(|e| format!("Failed to create encoded_images directory: {}", e))?;
    std::fs::create_dir_all("decoded_images")
    .map_err(|e| format!("Failed to create decoded_images directory: {}", e))?;
    // Setup Quinn endpoints
    let server_addrs: Vec<SocketAddr> = vec![
        "10.7.17.170:5017".parse()?,
    ];  // Connect to server's ports
    let client_addr: SocketAddr = "10.7.19.179:0".parse()?;  // Listen on this port

    let client_owner_addr: SocketAddr = "10.7.19.179:9000".parse()?;


    let (client_owner_endpoint, _client_owner_cert) = make_server_endpoint(client_owner_addr).unwrap();

    tokio::spawn(async move {
        loop {
            match timeout(Duration::from_secs(1), client_owner_endpoint.accept()).await {
                Ok(Some(incoming)) => {
                    match incoming.await {
                        Ok(conn) => {
                            match conn.accept_bi().await {
                                Ok((mut send, mut recv)) => {
                                    match recv.read_to_end(64 * 1024).await {
                                        Ok(msg_bytes) => {
                                            match bincode::deserialize::<String>(&msg_bytes) {
                                                Ok(msg) => {
                                                    let parts: Vec<&str> = msg.split(',').collect();
                                                    if parts.len() == 3 {
                                                        let requester_id = parts[0].to_string();
                                                        let file_name = parts[1].to_string();
                                                        let num_accesses: u32 = parts[2].parse().unwrap_or(0);
    
                                                        println!("Connection detected on client_owner_addr.");
                                                        // Open a new window with a prompt asking for a yes/no response
                                                        let question = format!(
                                                            "Client {} asks to access {} for {} times. Accept?",
                                                            requester_id, file_name, num_accesses
                                                        );
                                                        let output = Command::new("sh")
                                                            .arg("-c")
                                                            .arg(format!("zenity --question --text='{}'", question))
                                                            .output()
                                                            .expect("Failed to execute command");
    
                                                        if output.status.success() {
                                                            println!("User accepted the connection.");
                                                            // Handle the accepted connection here
                                                            let secret_images_path = "encoded_images";
                                                            let secret_image_path = format!("{}/{}", secret_images_path, file_name);
                                                            let secret_image = std::fs::read(&secret_image_path)
                                                                .map_err(|e| {
                                                                    println!("Failed to read secret image: {:?}", e);
                                                                    return;
                                                                }).unwrap_or_else(|e| {
                                                                    println!("Failed to read secret image: {:?}", e);
                                                                    return vec![];
                                                                });
    
                                                            let stego_path = format!("encoded_images/{}", file_name);
                                                            let stego_with_rights_path = format!("{}", file_name);
    
                                                            let local_steganographer = SomeImageSteganographer::new(100, 10);
                                                            let owner_id = "owner123";
                                                            let requester_id = "requester456";
                                                            let initial_access_rights = num_accesses;
    
                                                            let encoded_image = local_steganographer.encode_with_access_rights(
                                                                &secret_image,
                                                                owner_id,
                                                                requester_id,
                                                                initial_access_rights,
                                                                &stego_with_rights_path,
                                                            ).unwrap_or_else(|e| {
                                                                println!("Failed to encode with access rights: {:?}", e);
                                                                return vec![];
                                                            });
                                                            
                                                            println!("Encoded image saved to {}", stego_with_rights_path);
                                                            if let Err(e) = send.write_all(&encoded_image).await {
                                                                println!("Failed to send encoded image: {:?}", e);
                                                            }
                                                            if let Err(e) = send.finish() {
                                                                println!("Failed to finish sending: {:?}", e);
                                                            }
                                                            sleep(Duration::from_secs(1)).await;
                                                            // Menu options
                                                            println!("Please choose an option:");
                                                            println!("1. Encode images");
                                                            println!("2. Request Images");
                                                            println!("3. View Borrowed Images");
                                                            println!("4. Exit");
                                                            } 
                                                            else {
                                                            println!("User denied the connection.");
                                                            // Handle the denied connection here
                                                            if !output.status.success() {
                                                                println!("User denied the connection.");
                                                                let response = "Request not granted. I will not give you the image.";
                                                                if let Err(e) = send.write_all(response.as_bytes()).await {
                                                                    println!("Failed to send denial message: {:?}", e);
                                                                }
                                                                if let Err(e) = send.finish() {
                                                                    println!("Failed to finish sending denial message: {:?}", e);
                                                                }
                                                                sleep(Duration::from_secs(1)).await;
                                                                
                                                                // Menu options
                                                                println!("Please choose an option:");
                                                                println!("1. Encode images");
                                                                println!("2. Request Images");
                                                                println!("3. View Borrowed Images");
                                                                println!("4. Exit");
                                                            }
                                                        }
                                                    } else {
                                                        println!("Invalid RequestAccess message format");
                                                    }
                                                }
                                                Err(e) => println!("Failed to deserialize message: {:?}", e),
                                            }
                                        }
                                        Err(e) => println!("Failed to read message: {:?}", e),
                                    }
                                }
                                Err(e) => println!("Failed to accept bidirectional stream: {:?}", e),
                            }
                        }
                        Err(e) => println!("Failed to accept connection: {:?}", e),
                    }
                }
                Ok(None) => {
                    // No incoming connection within the timeout duration
                }
                Err(_) => {},
            }
        }
    });

    println!("Quinn endpoints setup beginning.");

    //let (server_endpoint, _server_cert) = make_server_endpoint(server_addr).unwrap();
    

    let mut client_config = ClientConfig::new(Arc::new(QuicClientConfig::try_from(
        rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(SkipServerVerification::new())
            .with_no_client_auth(),
    )?));
    let mut transport_config = TransportConfig::default();
    transport_config.keep_alive_interval(Some(Duration::from_secs(5)));
    transport_config.max_idle_timeout(Some(quinn_proto::IdleTimeout::try_from(Duration::from_secs(200)).unwrap()));
    client_config.transport_config(Arc::new(transport_config));

    let mut client_endpoint = quinn::Endpoint::client(client_addr)?;
    client_endpoint.set_default_client_config(client_config);


    println!("Quinn endpoints setup successfully.");

    // Create transport ends
    println!("Creating transport ends.");
    loop{

    
        // Menu options
        println!("Please choose an option:");
        println!("1. Encode images");
        println!("2. Request Images");
        println!("3. View Borrowed Images");
        println!("4. Exit");

        let mut choice = String::new();
        std::io::stdin().read_line(&mut choice)?;
        let choice = loop {
            let trimmed = choice.trim();
            match trimmed.parse::<u32>() {
                Ok(num) => break num,
                Err(_) => {
                    println!("Invalid input. Please enter a valid number:");
                    choice.clear();
                    std::io::stdin().read_line(&mut choice)?;
                }
            }
        };

        match choice {
            1 => {
                println!("You chose to encode images.");
                // Call the function to encode images
                let secret_images_path = "secret_images";
                let secret_images = std::fs::read_dir(secret_images_path)
                    .map_err(|e| format!("Failed to read secret_images directory: {}", e))?
                    .filter_map(Result::ok)
                    .filter(|entry| entry.path().is_file())
                    .map(|entry| entry.path())
                    .collect::<Vec<_>>();

                println!("List of files in the secret_images folder:");
                for (index, path) in secret_images.iter().enumerate() {
                    println!("{}. {}", index + 1, path.display());
                }

                let selected_indices = loop {
                    println!("Enter the indices of the images you want to encode (e.g., 1,5-9,12):");
                    let mut indices_input = String::new();
                    std::io::stdin().read_line(&mut indices_input)?;
                    let indices_input = indices_input.trim();

                    let mut selected_indices = Vec::new();
                    let mut valid_input = true;

                    for part in indices_input.split(',') {
                        if part.contains('-') {
                            let range_parts: Vec<&str> = part.split('-').collect();
                            if range_parts.len() == 2 {
                                let start = range_parts[0].parse::<usize>();
                                let end = range_parts[1].parse::<usize>();
                                match (start, end) {
                                    (Ok(start), Ok(end)) if start <= end && end <= secret_images.len() => {
                                        selected_indices.extend(start..=end);
                                    }
                                    _ => {
                                        println!("Invalid range: {}", part);
                                        valid_input = false;
                                        break;
                                    }
                                }
                            } else {
                                println!("Invalid range format: {}", part);
                                valid_input = false;
                                break;
                            }
                        } else {
                            match part.parse::<usize>() {
                                Ok(index) if index <= secret_images.len() => selected_indices.push(index),
                                _ => {
                                    println!("Invalid index: {}", part);
                                    valid_input = false;
                                    break;
                                }
                            }
                        }
                    }

                    if valid_input {
                        break selected_indices;
                    }
                };

                let selected_images: Vec<_> = selected_indices.iter().map(|&i| secret_images[i - 1].clone()).collect();
                println!("Selected images:");
                for (index, path) in selected_images.iter().enumerate() {
                    println!("{}. {}", index + 1, path.display());
                }

                // Proceed with encoding the selected images
                // Split secret images into 3 vectors
                let chunk_size = (selected_images.len() + 2) / 3;
                let mut secret_images_chunks: Vec<Vec<_>> = Vec::new();

                for chunk in selected_images.chunks(chunk_size) {
                    secret_images_chunks.push(chunk.to_vec());
                }

                while secret_images_chunks.len() < 3 {
                    secret_images_chunks.push(Vec::new());
                }

                let mut stego_portions = vec![];
                let semaphore = Arc::new(Semaphore::new(5)); // Limit to 5 concurrent requests
                let process_start_time = std::time::Instant::now();

                for chunk in secret_images_chunks.into_iter() {
                    let secret_images = chunk.clone();
                    let server_addrs = server_addrs.clone();
                    let client_endpoint = client_endpoint.clone();
                    let semaphore = semaphore.clone();

                    let stego_portion = tokio::spawn(async move {
                        for (index, entry) in secret_images.iter().enumerate() {
                        let secret_path = entry;
                        let secret_file_name = secret_path.file_name()
                            .ok_or("Failed to get filename")?
                            .to_str()
                            .ok_or("Failed to convert filename to string")?
                            .to_string();
                    
                        println!("Processing secret image {}: {}", index, secret_file_name);
                    
                        let secret_file = std::fs::File::open(&secret_path)
                            .map_err(|e| format!("Failed to open secret file: {}", e))?;
                        let secret_image = file_to_bytes(secret_file);
                        let secret_image_bytes = &secret_image;
                    
                        // Generate unique output paths for each image
                        let stego_path = format!("{}.png", secret_file_name);
                        let stego_with_rights_path = format!("{}.png", secret_file_name);
                        let finale_path = format!("decoded_images");
                    
                        println!("Encoding secret image {}...", index);
                        let start_time = std::time::Instant::now();
                        let mut success = false;
                        
                        let mut retries = 0;
                        let max_retries = 3;
                        let mut backoff_duration = Duration::from_secs(2);
                        while retries < max_retries && !success {
                            let mut handles = vec![];
                    
                            for addr in server_addrs.clone() {
                                
                                let client_endpoint = client_endpoint.clone();
                                let secret_image_bytes = secret_image_bytes.clone();
                                let stego_path = stego_path.clone();
                                let stego_with_rights_path = stego_with_rights_path.clone();
                                let finale_path = finale_path.clone();
                                let secret_file_name = secret_file_name.clone();
                                let semaphore = semaphore.clone();
                        
                                let handle = tokio::spawn(async move {
                                    let permit = semaphore.acquire().await.unwrap(); // Acquire a permit
                                    let ends = match timeout(Duration::from_secs(10), create(client_endpoint.clone(), addr)).await {
                                        Ok(Ok(ends)) => ends,
                                        Ok(Err(e)) => {
                                            retries += 1;
                                            backoff_duration *= 2;
                                            println!("Error creating transport ends: {}", e);
                                            return;
                                        }
                                        Err(_) => {
                                            retries += 1;
                                            backoff_duration *= 2;
                                            println!("Timeout occurred while creating transport ends");
                                            return;
                                        }
                                    };
                                    
                                    let (context_user, image_steganographer): (Context, ServiceToImport<dyn ImageSteganographer>) =
                                        Context::with_initial_service_import(Config::default_setup(), ends.send.clone(), ends.recv.clone());
                                    context_user.disable_garbage_collection();
                                    let image_steganographer_proxy: Box<dyn ImageSteganographer> = image_steganographer.into_proxy();
                                    println!("Encoding secret image {} with proxy", index);
                                    let stegano = timeout(Duration::from_secs(60), async {
                                        match std::panic::catch_unwind(AssertUnwindSafe(|| {
                                            let owner_id = "owner123";
                                            let requester_id = "requester456";
                                            let initial_access_rights = 3;

                                            // First encode the secret image remotely
                                            let encoded_image = image_steganographer_proxy.encode(
                                                &secret_image_bytes,
                                                &stego_path,
                                                &secret_file_name,
                                                &owner_id,
                                            )?;


                                            // Then add access rights locally
                                            // let local_steganographer = SomeImageSteganographer::new(100, 10);
                                            // let final_encoded = local_steganographer.encode_with_access_rights(
                                            //     &encoded_image,
                                            //     owner_id,
                                            //     requester_id,
                                            //     initial_access_rights,
                                            //     &stego_with_rights_path
                                            // )?;

                                            Ok(encoded_image)
                                        })) {
                                            Ok(Ok(result)) => Ok(result),
                                            Ok(Err(e)) => Err(e),
                                            Err(_) => Err(Box::new(std::io::Error::new(
                                                std::io::ErrorKind::Other,
                                                "Task panicked"
                                            )) as Box<dyn std::error::Error>)
                                        }
                                    }).await.unwrap_or_else(|_| Err(Box::new(std::io::Error::new(
                                        std::io::ErrorKind::TimedOut,
                                        "Timeout occurred during encoding"
                                    )) as Box<dyn std::error::Error>));
                                    
                                    // Handle the result
                                    match stegano {
                                        Ok(final_encoded) => {
                                            println!("Encoding completed successfully");
                                            let secret_file_name = format!("{}.png", secret_file_name.rsplit('.').nth(1).unwrap_or(&secret_file_name));
                                            let encoded_image_path = format!("encoded_images/{}", secret_file_name);
                                            if let Err(e) = std::fs::write(&encoded_image_path, &final_encoded) {
                                                println!("Failed to save encoded image: {}", e);
                                            }
                                            println!("Encoded image saved to {}", encoded_image_path);
                                            let stego_with_rights_path = stego_with_rights_path.clone();
                                            let local_steganographer = SomeImageSteganographer::new(100, 10);

                                            // if let Err(e) = local_steganographer.view_decoded_image_temp(
                                            //     &final_encoded,
                                            //     "requester456",
                                            //     &stego_with_rights_path
                                                
                                            // ) {
                                            //     println!("Error viewing decoded image: {}", e);
                                            // }
                                        },
                                    Err(e) => {
                                        retries += 1;
                                            backoff_duration *= 2;
                                            println!("Failed to encode: {:?}", e);
                                        }
                                    }
                                    drop(permit); // Release the permit
                                });
                                handles.push(handle);
                            }
                    
                            let results: Vec<_> = futures::future::join_all(handles).await;
                            for result in results {
                                match result {
                                    Ok(_) => {
                                        println!("Secret image {} processed successfully", index);
                                        success = true;
                                    },
                                    Err(e) => {
                                        println!("Error in task: {}", e);
                                    }
                                }
                            }
                        }
                        
                        }
                        Ok::<(), String>(())
                    });

                    stego_portions.push(stego_portion);
                }

                let results: Vec<_> = futures::future::join_all(stego_portions).await;

                for result in results {
                    match result {
                        Ok(_) => {
                            println!("Secret images processed successfully");
                        },
                        Err(e) => {
                            println!("Error processing secret images: {}", e);
                        }
                    }
                }

            },
            2 => {
                let peer_list = vec![
                    "10.7.19.179:9100"
                ];
                let resource_list = vec![
                    "a.png",
                    "b.png",
                    "c.png",
                ];
                println!("Available peers:");
                for (i, peer) in peer_list.iter().enumerate() {
                    println!("{}. {}", i + 1, peer);
                }

                println!("Enter the number of the peer you want to request an image from:");
                let mut peer_choice = String::new();
                std::io::stdin().read_line(&mut peer_choice)?;
                let peer_choice: usize = loop {
                    match peer_choice.trim().parse() {
                        Ok(num) if num > 0 && num <= peer_list.len() => break num,
                        _ => {
                            println!("Invalid choice. Please enter a valid number:");
                            peer_choice.clear();
                            std::io::stdin().read_line(&mut peer_choice)?;
                        }
                    }
                };

                let selected_peer = &peer_list[peer_choice - 1];

                println!("Available resources:");
                for (i, resource) in resource_list.iter().enumerate() {
                    println!("{}. {}", i + 1, resource);
                }
                println!("Enter the number of the resource you want to request:");
                let mut resource_choice = String::new();
                std::io::stdin().read_line(&mut resource_choice)?;
                let resource_choice: usize = loop {
                    match resource_choice.trim().parse() {
                        Ok(num) if num > 0 && num <= resource_list.len() => break num,
                        _ => {
                            println!("Invalid choice. Please enter a valid number:");
                            resource_choice.clear();
                            std::io::stdin().read_line(&mut resource_choice)?;
                        }
                    }
                };

                let selected_resource = &resource_list[resource_choice - 1];

                println!("Enter the number of times you want to access the resource:");
                let mut num_accesses = String::new();
                std::io::stdin().read_line(&mut num_accesses)?;
                let num_accesses: u32 = loop {
                    match num_accesses.trim().parse() {
                        Ok(num) if num > 0 => break num,
                        _ => {
                            println!("Invalid number of accesses. Must be greater than 0. Please enter a valid number:");
                            num_accesses.clear();
                            std::io::stdin().read_line(&mut num_accesses)?;
                        }
                    }
                };
            

                println!("Requesting resource '{}' from peer '{}'", selected_resource, selected_peer);

                // Implement the logic to request the selected resource from the selected peer
                let client_endpoint = client_endpoint.clone();
                let selected_peer = selected_peer.to_string();
                let selected_resource = selected_resource.to_string();
                let client_owner_addr = client_owner_addr.clone();
                
                let request_message = format!("{},{},{}", "requester456", selected_resource, num_accesses);
                let request_bytes = bincode::serialize(&request_message)?;

                let connecting = client_endpoint.connect(selected_peer.parse()?, "localhost")?;
                let connection = connecting.await?;
                let (mut send, mut recv) = connection.open_bi().await?;

                send.write_all(&request_bytes).await?;
                send.finish();
                sleep(Duration::from_secs(1)).await;
                match recv.read_to_end(500 * 1024 * 1024).await {
                    Ok(response_bytes) => {
                        // Check if the response is an actual image or an error message
                        if let Ok(response) = String::from_utf8(response_bytes.clone()) {
                            if response == "Request not granted. I will not give you the image." {
                                println!("{}", response);
                            } else {
                                // Write the received bytes directly to a file
                                let decoded_image_path = format!("borrowed_images/{}", selected_resource);
                                std::fs::write(&decoded_image_path, &response_bytes)?;
                                println!("Received and saved the image to {}", decoded_image_path);
                            }
                        } else {
                            // Write the received bytes directly to a file
                            let decoded_image_path = format!("borrowed_images/{}", selected_resource);
                            std::fs::write(&decoded_image_path, &response_bytes)?;
                            println!("Received and saved the image to {}", decoded_image_path);
                        }
                    }
                    Err(e) => println!("Failed to receive the image: {:?}", e),
                }
            },
            3 => {
                view_borrowed_images().await?;
            },
            4 => {
                println!("Goodbye!");
                break;
            },
            _ => {
                println!("Invalid choice. Please try again.");
            }
        }
    }

    // Wait for the user to press Enter before shutting down the server
    println!("Shutting down server...");
    tokio::signal::ctrl_c().await.expect("Failed to listen for ctrl_c signal");

    // let _ = tokio::join!(steg_handle);

    Ok(())
}

async fn view_borrowed_images() -> Result<(), Box<dyn Error>> {
    let borrowed_dir = "borrowed_images";
    
    // Get list of borrowed images
    let entries = std::fs::read_dir(borrowed_dir)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry.path().extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext == "png" || ext == "jpg")
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    
    if entries.is_empty() {
        println!("No borrowed images found.");
        println!("\nPress Enter to continue...");
        std::io::stdin().read_line(&mut String::new())?;
        return Ok(());
    }
    
    println!("\nAvailable borrowed images:");
    for (i, entry) in entries.iter().enumerate() {
        println!("{}. {}", i + 1, entry.file_name().to_string_lossy());
    }
    
    println!("\nEnter the number of the image you want to view (or 0 to go back):");
    std::io::stdout().flush()?;
    
    let mut choice = String::new();
    std::io::stdin().read_line(&mut choice)?;
    let choice: usize = match choice.trim().parse() {
        Ok(num) if num == 0 => return Ok(()),
        Ok(num) if num <= entries.len() => num,
        _ => {
            println!("Invalid choice.");
            println!("\nPress Enter to continue...");
            std::io::stdin().read_line(&mut String::new())?;
            return Ok(());
        }
    };
    
    let selected_image = &entries[choice - 1];
    let image_path = selected_image.path();
    
    // Read the image file
    let image_data = std::fs::read(&image_path)?;
    
    // Create steganographer instance
    let steganographer = SomeImageSteganographer::new(100, 10);
    
    // View the decoded image
    match steganographer.view_decoded_image_temp(
        &image_data,
        "requester456", // Using default requester ID
        &image_path.to_string_lossy()
    ) {
        Ok(_) => println!("Image viewing completed."),
        Err(e) => println!("Error viewing image: {}", e)
    }
    
    println!("\nPress Enter to continue...");
    std::io::stdin().read_line(&mut String::new())?;
    
    Ok(())
} 