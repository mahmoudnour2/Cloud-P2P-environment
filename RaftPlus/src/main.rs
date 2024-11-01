use tokio;
use std::time::Duration;
use crate::node::{Node, SystemMetrics, State};
use tokio::sync::mpsc;
use std::collections::HashMap;
use crate::node::NodeMessage;

mod node;

#[tokio::main]
async fn main() {
    println!("Starting RaftPlus simulation with 3 nodes...\n");

    // Create channels for each node
    let mut node_channels: HashMap<u64, mpsc::Sender<NodeMessage>> = HashMap::new();
    let mut receivers: HashMap<u64, mpsc::Receiver<NodeMessage>> = HashMap::new();

    for &node_id in &[1, 2, 3] {
        let (tx, rx) = mpsc::channel(100);
        node_channels.insert(node_id, tx);
        receivers.insert(node_id, rx);
    }

    // Create nodes with strategic initial metrics
    let mut node1 = Node::new(1, node_channels.clone(), receivers.remove(&1).unwrap());
    let mut node2 = Node::new(2, node_channels.clone(), receivers.remove(&2).unwrap());
    let mut node3 = Node::new(3, node_channels.clone(), receivers.remove(&3).unwrap());

    // Initial setup - Node 1 starts as leader with good metrics
    customize_metrics(&mut node1.metrics, 30.0, 40.0, 800.0);
    customize_metrics(&mut node2.metrics, 45.0, 50.0, 600.0);
    customize_metrics(&mut node3.metrics, 35.0, 45.0, 700.0);
    node1.state = State::Leader;

    println!("\n=== Starting Simulation ===\n");
    println!("Initial state: Node 1 is leader");

    // Spawn nodes
    let node1_handle = tokio::spawn(async move { node1.run().await });
    let node2_handle = tokio::spawn(async move { node2.run().await });
    let node3_handle = tokio::spawn(async move { node3.run().await });

    // Simulation timeline
    tokio::time::sleep(Duration::from_secs(5)).await;
    println!("\n=== Scenario 1: Leader Performance Degradation ===");
    
    // Degrade leader's performance (Node 1)
    if let Some(leader_tx) = node_channels.get(&1) {
        let degraded_metrics = SystemMetrics {
            cpu_load: 90.0,
            memory_usage: 85.0,
            network_bandwidth: 200.0,
            disk_io: 500.0,
            request_latency: 100.0,
            connection_count_for_node: 500,
        };
        
        // Send a special message to update the node's metrics
        let _ = leader_tx.send(NodeMessage::UpdateMetrics(degraded_metrics.clone())).await;
        
        println!("Degraded Node 1's metrics: ...");
    }

    tokio::time::sleep(Duration::from_secs(10)).await;
    println!("\n=== Scenario 2: Network Partition ===");
    // Actually stop the messages for 6 seconds
    tokio::time::sleep(Duration::from_secs(6)).await;
    node_channels.clear(); // This will simulate network partition

    tokio::time::sleep(Duration::from_secs(10)).await;
    println!("\n=== Scenario 3: System Recovery ===");
    // System should stabilize with a new leader

    tokio::time::sleep(Duration::from_secs(5)).await;
    println!("\n=== Simulation completed ===");

    let _ = tokio::join!(node1_handle, node2_handle, node3_handle);
}

fn customize_metrics(metrics: &mut SystemMetrics, cpu: f64, memory: f64, network: f64) {
    metrics.cpu_load = cpu;
    metrics.memory_usage = memory;
    metrics.network_bandwidth = network;
    metrics.request_latency = if cpu > 70.0 { 50.0 } else { 10.0 };
    metrics.connection_count_for_node = if memory > 70.0 { 200 } else { 50 };
    
    println!("Node metrics initialized:");
    println!("  CPU Load: {:.1}%", metrics.cpu_load);
    println!("  Memory Usage: {:.1}%", metrics.memory_usage);
    println!("  Network Bandwidth: {:.1} Mbps", metrics.network_bandwidth);
    println!("  Request Latency: {:.1}ms", metrics.request_latency);
    println!("  Connection Count: {}", metrics.connection_count_for_node);
}
