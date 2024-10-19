use serde::{Deserialize, Serialize};
use reqwest::Client;
use serde_json::json;
use std::error::Error;

#[derive(Serialize, Deserialize, Debug, Clone)]
struct User {
    username: String,
    images: Vec<Image>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Image {
    id: String,
    encrypted_data: Vec<u8>,
    view_count: u32,
}

struct Peer {
    username: String,
    address: String,
}

struct EncryptedImage {
    data: Vec<u8>,
    views_remaining: u32,
    allowed_users: HashSet<String>,
}

impl User{
    async fn register_user(user: &User) -> Result<(), Box<dyn Error>> {
        let client = Client::new();
        let res = client.post("http://node1/discovery_service/register")
            .json(&user)
            .send()
            .await?;
        if res.status().is_success() {
            println!("User registered successfully: {:?}", res.text().await?);
        } else {
            println!("Failed to register user: {:?}", res.status());
        }
        Ok(())
    }
    
    async fn encrypt_image() --> Result<EncryptedImage, Box<dyn Error>> {
        let client = Client::new();
        
        let server1 = "http://node1.cloud/encrypt";
        let server2 = "http://node2.cloud/encrypt";
        let server3 = "http://node3.cloud/encrypt";
    
        let response1 = client.post(server1).send().await?;
        println!("Response from server 1: {:?}", response1.text().await?);
    
        let response2 = client.post(server2).send().await?;
        println!("Response from server 2: {:?}", response2.text().await?);
    
        let response3 = client.post(server3).send().await?;
        println!("Response from server 3: {:?}", response3.text().await?);
    
        Ok(())
    }
    
    async fn discover_peers(&self) -> Result<Vec<Peer>, Box<dyn Error>> {
        let client = Client::new();
        let discovery_service_url = "http://node1/discovery_service/peers";

        let response = client.get(discovery_service_url)
            .send()
            .await?
            .json::<Vec<Peer>>()
            .await?;

        for peer in &response {
            println!("Discovered peer: {} at {}", peer.username, peer.address);
        }

        Ok(response)
    }

    async fn share_image(&self, peer_address: &str, image_id: &str, num_views: u32) -> Result<(),Box<dyn Error>> {
        // Implement image sharing logic
    }
    fn update_image_quota(&mut self, image_id: &str, new_quota: u32) {
        // Implement logic to update the view quota of an image
    }
    
}
