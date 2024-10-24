mod client_middleware;
use client_middleware::{Middleware, LoadBalancingStrategy};

fn main() {
    let client_ip = "127.0.0.1:3400".to_string(); // Client's IP and port
    let cloud_ips = vec![
        "127.0.0.1:3401".to_string(),  // Cloud node 1
        "127.0.0.1:3402".to_string(),  // Cloud node 2
        "127.0.0.1:3403".to_string(),  // Cloud node 3
    ];

    let mut middleware = Middleware::new(client_ip, cloud_ips, LoadBalancingStrategy::RoundRobin);

    // Multicast request
    middleware.multicast_request("Encrypt Image Request");

    // Listen for responses
    let responses = middleware.listen_for_responses(3, 30);
    println!("Received responses: {:?}", responses);

    // Send request using load balancer and retry if necessary
    if let Err(e) = middleware.send_with_load_balancer("Single Node Request") {
        println!("Failed after retries: {}", e);
    }
}
