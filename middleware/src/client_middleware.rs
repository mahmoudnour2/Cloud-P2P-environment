use std::net::{UdpSocket, SocketAddr};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use rand::seq::SliceRandom;
use rand::thread_rng;

pub struct Middleware {
    client_ip: String,
    cloud_ips: Vec<String>,
    load_balancer: LoadBalancer,
    max_retries: u32, // For fault tolerance
}

impl Middleware {
    pub fn new(client_ip: String, cloud_ips: Vec<String>, strategy: LoadBalancingStrategy) -> Self {
        Middleware {
            client_ip,
            cloud_ips: cloud_ips.clone(),
            load_balancer: LoadBalancer::new(cloud_ips, strategy),
            max_retries: 3,
        }
    }

    // Multicasting Thread: Spawn a thread to send requests to all cloud nodes
    pub fn multicast_request(&self, request: &str) {
        let cloud_ips = self.cloud_ips.clone();
        let client_ip = self.client_ip.clone();
        let request = request.to_string();

        thread::spawn(move || {
            let socket = UdpSocket::bind(&client_ip).expect("Couldn't bind to address");
            for ip in &cloud_ips {
                let addr: SocketAddr = ip.parse().expect("Invalid IP address");
                socket.send_to(request.as_bytes(), addr).expect("Couldn't send data");
                println!("Multicasting request to cloud node at {}", ip);
            }
        });
    }

    // Request Listener: Listen for responses in a separate thread
    pub fn listen_for_responses(&self, expected_responses: usize, timeout_secs: u64) -> Vec<String> {
        let socket = UdpSocket::bind(&self.client_ip).expect("Couldn't bind to address");
        socket.set_read_timeout(Some(Duration::new(timeout_secs, 0))).expect("Failed to set timeout");

        let responses = Arc::new(Mutex::new(Vec::new()));
        let response_clone = Arc::clone(&responses);
        
        thread::spawn(move || {
            let mut buf = [0; 1024];
            for _ in 0..expected_responses {
                match socket.recv_from(&mut buf) {
                    Ok((amt, src)) => {
                        let response = String::from_utf8_lossy(&buf[..amt]).to_string();
                        println!("Received response from {}: {}", src, response);
                        let mut res = response_clone.lock().unwrap();
                        res.push(response);
                    }
                    Err(_) => {
                        println!("No response received within timeout.");
                        break;
                    }
                }
            }
        });

        thread::sleep(Duration::from_secs(timeout_secs)); // Wait for listener to finish
        Arc::try_unwrap(responses).unwrap().into_inner().unwrap() // Return collected responses
    }

    // Send Request with Load Balancer and Retry for Fault Tolerance
// Send Request with Load Balancer and Retry for Fault Tolerance
    pub fn send_with_load_balancer(&mut self, request: &str) -> Result<(), String> {
        for _ in 0..self.max_retries {
            let selected_node = self.load_balancer.select_node();
            println!("Sending request to {}", selected_node);

            let socket = UdpSocket::bind(&self.client_ip).expect("Couldn't bind to address");
            let addr: SocketAddr = selected_node.parse().expect("Invalid IP address");
            socket.send_to(request.as_bytes(), addr).expect("Couldn't send data");

            let mut buf = [0; 1024];
            socket.set_read_timeout(Some(Duration::new(5, 0))).expect("Couldn't set timeout");
            match socket.recv_from(&mut buf) {
                Ok((amt, src)) => {
                    println!("Received response from {}: {}", src, String::from_utf8_lossy(&buf[..amt]));
                    return Ok(());
                }
                Err(_) => {
                    println!("No response from {}, retrying...", selected_node);
                }
            }
        }
        Err("Request failed after retries".to_string())
    }
}

#[derive(Clone)]
pub enum LoadBalancingStrategy {
    RoundRobin,
    Random,
}

pub struct LoadBalancer {
    cloud_ips: Vec<String>,
    strategy: LoadBalancingStrategy,
    index: usize,
}

impl LoadBalancer {
    pub fn new(cloud_ips: Vec<String>, strategy: LoadBalancingStrategy) -> Self {
        LoadBalancer {
            cloud_ips,
            strategy,
            index: 0,
        }
    }

    // Select node based on the strategy (round-robin or random)
    pub fn select_node(&mut self) -> String {
        match self.strategy {
            LoadBalancingStrategy::RoundRobin => {
                let selected = self.cloud_ips[self.index].clone();
                self.index = (self.index + 1) % self.cloud_ips.len();
                selected
            }
            LoadBalancingStrategy::Random => {
                let mut rng = thread_rng();
                let selected = self.cloud_ips.choose(&mut rng).unwrap().clone();
                selected
            }
        }
    }
}
