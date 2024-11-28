use tokio::sync::mpsc;
use tokio::time::{Duration, sleep};
use std::collections::HashMap;
use rand::Rng;
use std::time::Instant;
use std::cmp::Ordering;
use serde::{Serialize, Deserialize};
use quinn::{Endpoint, ClientConfig, ServerConfig};
use std::net::SocketAddr;
use std::error::Error;
use anyhow::Result;
use std::sync::Arc;
use rcgen::{Certificate, KeyPair, RsaKeyPair};
use rustls::{ClientConfig, SkipServerVerification};

#[derive(Debug, Clone, PartialEq)]
pub enum State {
    Follower,
    Leader,
    DefactoLeader, // Temporary state when handling election after leader death
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VoteReason {
    HighCPULoad,
    HighMemoryUsage,
    NetworkCongestion,
    HighLatency
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemMetrics {
    pub cpu_load: f64,
    pub memory_usage: f64,
    pub network_bandwidth: f64,
    pub disk_io: f64,
    pub request_latency: f64,
    pub connection_count_for_node: u32,
}

impl Default for SystemMetrics {
    fn default() -> Self {
        Self {
            cpu_load: 50.0,
            memory_usage: 60.0,
            network_bandwidth: 500.0,
            disk_io: 200.0,
            request_latency: 10.0,
            connection_count_for_node: 50,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NodeMessage {
    Heartbeat { 
        leader_id: u64, 
        metrics: SystemMetrics,
        candidates: Vec<Candidate>,
    },
    NegativeVote { 
        voter_id: u64, 
        reason: VoteReason,
        metrics: SystemMetrics,
    },
    ElectionResult { new_leader_id: u64 },
    UpdateMetrics(SystemMetrics),
}


pub struct Node {
    pub id: u64,
    pub state: State,
    pub metrics: SystemMetrics,
    last_heartbeat: Instant,
    heartbeat_timeout: Duration,
    negative_votes_received: HashMap<u64, VoteReason>,
    candidates: Vec<Candidate>,
    current_leader_id: Option<u64>,
    node_channels: HashMap<u64, mpsc::Sender<NodeMessage>>,
    rx: mpsc::Receiver<NodeMessage>,
}

impl Node {
    pub fn new(id: u64, channels: HashMap<u64, mpsc::Sender<NodeMessage>>, rx: mpsc::Receiver<NodeMessage>) -> Self {
        Self {
            id,
            state: State::Follower,
            metrics: SystemMetrics::default(),
            last_heartbeat: Instant::now(),
            heartbeat_timeout: Duration::from_secs(5),
            negative_votes_received: HashMap::new(),
            candidates: Vec::new(),
            current_leader_id: None,
            node_channels: channels,
            rx,
        }
    }

    pub async fn run(&mut self) {
        loop {
            match self.state {
                State::Leader => self.run_leader().await,
                State::Follower => self.run_follower().await,
                State::DefactoLeader => self.handle_election().await,

            }
        }
    }

    async fn run_leader(&mut self) {
        loop {
            self.broadcast_heartbeat().await;

            while let Ok(msg) = self.rx.try_recv() {
                match msg {
                    NodeMessage::NegativeVote { voter_id, reason, metrics } => {
                        println!("Leader received negative vote from Node {} due to {:?}", voter_id, reason);
                        self.negative_votes_received.insert(voter_id, reason.clone());
                        self.update_candidate(voter_id, metrics);
                        
                        println!("Current negative votes: {}", self.negative_votes_received.len());
                        if self.negative_votes_received.len() >= 2 {
                            println!("Received enough negative votes, stepping down");
                            self.state = State::DefactoLeader;
                            return;
                        }
                    }
                    NodeMessage::UpdateMetrics(new_metrics) => {
                        self.metrics = new_metrics;
                        println!("Updated leader metrics: CPU: {:.1}%, Memory: {:.1}%", 
                            self.metrics.cpu_load, self.metrics.memory_usage);
                    }
                    _ => {}
                }
            }

            sleep(Duration::from_secs(1)).await;
        }
    }

    async fn run_follower(&mut self) {
        loop {
            if self.last_heartbeat.elapsed() > self.heartbeat_timeout {
                println!("Node {} detected leader failure", self.id);
                self.state = State::DefactoLeader;
                return;
            }

            while let Ok(msg) = self.rx.try_recv() {
                match msg {
                    NodeMessage::Heartbeat { leader_id, metrics: leader_metrics, candidates } => {
                        println!("Node {} received heartbeat from leader {}", self.id, leader_id);
                        self.last_heartbeat = Instant::now();
                        self.current_leader_id = Some(leader_id);
                        self.candidates = candidates;
                        
                        if let Some(reason) = self.should_cast_negative_vote(&leader_metrics) {
                            self.send_negative_vote(leader_id, reason).await;
                        }
                    }
                    NodeMessage::ElectionResult { new_leader_id } => {
                        if new_leader_id == self.id {
                            self.state = State::Leader;
                            return;
                        } else {
                            self.current_leader_id = Some(new_leader_id);
                        }
                    }
                    _ => {}
                }
            }

            sleep(Duration::from_millis(100)).await;
        }
    }

    async fn handle_election(&mut self) {
        println!("👑 Node {} handling election as DefactoLeader", self.id);
        
        let self_metrics = self.collect_metrics();
        self.update_candidate(self.id, self_metrics.clone());
        
        if let Some(new_leader_id) = self.elect_leader(&self.candidates) {
            println!("👑 Node {} elected as new leader\n   New Leader Metrics:\n   CPU: {:.1}%\n   Memory: {:.1}%\n   Network: {:.1} Mbps\n   Latency: {:.1}ms\n   Connections: {}", 
                new_leader_id,
                self_metrics.cpu_load,
                self_metrics.memory_usage,
                self_metrics.network_bandwidth,
                self_metrics.request_latency,
                self_metrics.connection_count_for_node
            );
            
            // Broadcast result
            self.broadcast_message(NodeMessage::ElectionResult { 
                new_leader_id 
            }).await;

            // Update own state
            self.state = if new_leader_id == self.id {
                State::Leader
            } else {
                State::Follower
            };

            // Clear candidates and negative votes for next round
            self.candidates.clear();
            self.negative_votes_received.clear();
        }
    }

    fn elect_leader(&self, candidates: &[Candidate]) -> Option<u64> {
        candidates.iter()
            .max_by(|a: &&Candidate, b| self.calculate_score(&a.metrics)
                .partial_cmp(&self.calculate_score(&b.metrics))
                .unwrap_or(Ordering::Equal))
            .map(|c| c.id)
    }

    fn collect_metrics(&mut self) -> SystemMetrics {
        SystemMetrics {
            cpu_load: self.measure_cpu_load(),
            memory_usage: self.measure_memory_usage(),
            network_bandwidth: self.measure_network_bandwidth(),
            disk_io: self.measure_disk_io(),
            request_latency: self.measure_request_latency(),
            connection_count_for_node: self.get_connection_count_for_node(),
        }
    }

    fn measure_cpu_load(&self) -> f64 {
        // TODO: Implement actual CPU measurement
        50.0 // Default placeholder value
    }

    fn measure_memory_usage(&self) -> f64 {
        // TODO: Implement actual memory measurement
        60.0 // Default placeholder value
    }

    fn measure_network_bandwidth(&self) -> f64 {
        // TODO: Implement actual network bandwidth measurement
        500.0 // Default placeholder value (Mbps)
    }

    fn measure_disk_io(&self) -> f64 {
        // TODO: Implement actual disk I/O measurement
        200.0 // Default placeholder value (IOPS)
    }

    fn measure_request_latency(&self) -> f64 {
        // TODO: Implement actual latency measurement
        10.0 // Default placeholder value (ms)
    }

    fn get_connection_count_for_node(&self) -> u32 {
        // TODO: Implement actual connection counting
        50 // Default placeholder value
    }

    fn calculate_score(&self, metrics: &SystemMetrics) -> f64 {
        const CPU_WEIGHT: f64 = 0.3;
        const MEMORY_WEIGHT: f64 = 0.2;
        const NETWORK_WEIGHT: f64 = 0.2;
        const LATENCY_WEIGHT: f64 = 0.2;
        const CONNECTIONS_WEIGHT: f64 = 0.1;

        let cpu_score = 1.0 - (metrics.cpu_load / 100.0);
        let memory_score = 1.0 - (metrics.memory_usage / 100.0);
        let network_score = metrics.network_bandwidth / 1000.0;
        let latency_score = 1.0 / metrics.request_latency;
        let connections_score = 1.0 - (metrics.connection_count_for_node as f64 / 1000.0);

        let base_score = cpu_score * CPU_WEIGHT + memory_score * MEMORY_WEIGHT + network_score * NETWORK_WEIGHT + latency_score * LATENCY_WEIGHT + connections_score * CONNECTIONS_WEIGHT;

        // Add ±2% randomization for non-prejudiced selection
        base_score * (0.98 + rand::thread_rng().gen::<f64>() * 0.04)
    }

    fn should_cast_negative_vote(&self, leader_metrics: &SystemMetrics) -> Option<VoteReason> {
        let my_metrics = &self.metrics;

        if my_metrics.cpu_load < leader_metrics.cpu_load * 0.7 {
            println!("🗳️ Node {} casting negative vote due to HighCPULoad\n   Node Metrics vs Leader Metrics:\n   CPU: {:.1}% vs {:.1}%\n   Memory: {:.1}% vs {:.1}%\n   Network: {:.1} vs {:.1} Mbps", 
                self.id, 
                my_metrics.cpu_load, leader_metrics.cpu_load,
                my_metrics.memory_usage, leader_metrics.memory_usage,
                my_metrics.network_bandwidth, leader_metrics.network_bandwidth
            );
            return Some(VoteReason::HighCPULoad);
        }
        if my_metrics.memory_usage < leader_metrics.memory_usage * 0.7 {
            println!("🗳️ Node {} casting negative vote due to HighMemoryUsage\n   Node Metrics vs Leader Metrics:\n   CPU: {:.1}% vs {:.1}%\n   Memory: {:.1}% vs {:.1}%\n   Network: {:.1} vs {:.1} Mbps", 
                self.id, 
                my_metrics.cpu_load, leader_metrics.cpu_load,
                my_metrics.memory_usage, leader_metrics.memory_usage,
                my_metrics.network_bandwidth, leader_metrics.network_bandwidth
            );
            return Some(VoteReason::HighMemoryUsage);
        }
        if my_metrics.network_bandwidth > leader_metrics.network_bandwidth * 1.5 {
            println!("🗳️ Node {} casting negative vote due to NetworkCongestion\n   Node Metrics vs Leader Metrics:\n   CPU: {:.1}% vs {:.1}%\n   Memory: {:.1}% vs {:.1}%\n   Network: {:.1} vs {:.1} Mbps", 
                self.id, 
                my_metrics.cpu_load, leader_metrics.cpu_load,
                my_metrics.memory_usage, leader_metrics.memory_usage,
                my_metrics.network_bandwidth, leader_metrics.network_bandwidth
            );
            return Some(VoteReason::NetworkCongestion);
        }
        if my_metrics.request_latency < leader_metrics.request_latency * 0.5 {
            println!("🗳️ Node {} casting negative vote due to HighLatency\n   Node Metrics vs Leader Metrics:\n   CPU: {:.1}% vs {:.1}%\n   Memory: {:.1}% vs {:.1}%\n   Network: {:.1} vs {:.1} Mbps", 
                self.id, 
                my_metrics.cpu_load, leader_metrics.cpu_load,
                my_metrics.memory_usage, leader_metrics.memory_usage,
                my_metrics.network_bandwidth, leader_metrics.network_bandwidth
            );
            return Some(VoteReason::HighLatency);
        }

        None
    }

    async fn broadcast_heartbeat(&self) {
        println!("Node {} broadcasting heartbeat", self.id);
        self.broadcast_message(NodeMessage::Heartbeat {
            leader_id: self.id,
            metrics: self.metrics.clone(),
            candidates: self.candidates.clone(),
        }).await;
    }

    async fn send_negative_vote(&mut self, leader_id: u64, reason: VoteReason) {
        let current_metrics = self.collect_metrics();
        
        if let Some(leader_tx) = self.node_channels.get(&leader_id) {
            if let Err(e) = leader_tx.send(NodeMessage::NegativeVote { 
                voter_id: self.id,
                reason,
                metrics: current_metrics,
            }).await {
                println!("Failed to send negative vote to leader {}: {}", leader_id, e);
            }
        }
    }

    async fn broadcast_message(&self, msg: NodeMessage) {
        println!("Broadcasting message to {} nodes", self.node_channels.len());
        for (&node_id, tx) in &self.node_channels {
            if node_id != self.id {
                if let Err(e) = tx.send(msg.clone()).await {
                    println!("Failed to send message to node {}: {}", node_id, e);
                } else {
                    println!("Successfully sent message to node {}", node_id);
                }
            }
        }
    }

    fn update_candidate(&mut self, candidate_id: u64, metrics: SystemMetrics) {
        // Find and update existing candidate or add new one
        if let Some(candidate) = self.candidates.iter_mut()
            .find(|c| c.id == candidate_id) 
        {
            candidate.metrics = metrics;
            println!("Updated existing candidate {} metrics", candidate_id);
        } else {
            let score = self.calculate_score(&metrics);
            self.candidates.push(Candidate {
                id: candidate_id,
                metrics,
                score,
            });
            println!("Added new candidate {} to election pool", candidate_id);
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Candidate {
    pub id: u64,
    pub metrics: SystemMetrics,
    pub score: f64,
}

pub struct QuinnNode {
    node: Node,
    server_endpoint: Endpoint,
    client_endpoints: Vec<(SocketAddr, Endpoint)>,
}

impl QuinnNode {
    pub async fn new(
        node_id: u64,
        server_addr: SocketAddr,
        peer_addrs: Vec<SocketAddr>,
    ) -> Result<Self> {
        // Setup server endpoint
        let (server_endpoint, _cert) = make_server_endpoint(server_addr)?;
        
        // Setup client endpoints
        let mut client_endpoints = Vec::new();
        for peer_addr in peer_addrs {
            let client_endpoint = Endpoint::client("0.0.0.0:0".parse().unwrap())?;
            let client_config = configure_client_with_no_verification();
            client_endpoint.set_default_client_config(client_config);
            client_endpoints.push((peer_addr, client_endpoint));
        }

        // Setup channels
        let (tx, rx) = mpsc::channel(100);
        let mut node_channels = HashMap::new();
        for i in 0..3 {
            let (node_tx, _) = mpsc::channel(100);
            node_channels.insert(i as u64, node_tx);
        }

        let node = Node::new(node_id, node_channels, rx);

        Ok(Self {
            node,
            server_endpoint,
            client_endpoints,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        // Spawn server handler
        let server_endpoint = self.server_endpoint.clone();
        let tx = self.node.node_channels.get(&self.node.id).unwrap().clone();
        
        tokio::spawn(async move {
            Self::handle_server(server_endpoint, tx).await;
        });

        // Run the election node
        self.node.run().await;
        Ok(())
    }

    async fn handle_server(endpoint: Endpoint, tx: mpsc::Sender<NodeMessage>) {
        while let Some(conn) = endpoint.accept().await {
            let conn = match conn.await {
                Ok(conn) => conn,
                Err(e) => {
                    println!("Error accepting connection: {}", e);
                    continue;
                }
            };

            let tx = tx.clone();
            tokio::spawn(async move {
                while let Ok((mut send, mut recv)) = conn.accept_bi().await {
                    let msg_bytes = match recv.read_to_end(64 * 1024).await {
                        Ok(bytes) => bytes,
                        Err(e) => {
                            println!("Error reading message: {}", e);
                            continue;
                        }
                    };

                    if let Ok(msg) = bincode::deserialize::<NodeMessage>(&msg_bytes) {
                        let _ = tx.send(msg).await;
                    }
                }
            });
        }
    }

    async fn broadcast_message(&self, msg: NodeMessage) -> Result<()> {
        let msg_bytes = bincode::serialize(&msg)?;

        for (peer_addr, client_endpoint) in &self.client_endpoints {
            if let Ok(conn) = client_endpoint.connect(*peer_addr, "localhost").await {
                if let Ok((mut send, _recv)) = conn.open_bi().await {
                    let _ = send.write_all(&msg_bytes).await;
                    let _ = send.finish().await;
                }
            }
        }
        Ok(())
    }
}

// Add Quinn utility functions
fn configure_client_with_no_verification() -> ClientConfig {
    let crypto = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(SkipServerVerification::new())
        .with_no_client_auth();

    ClientConfig::new(Arc::new(quinn_proto::crypto::rustls::QuicClientConfig::try_from(crypto).unwrap()))
}

pub fn make_server_endpoint(
    bind_addr: SocketAddr,
) -> Result<(Endpoint, rustls::pki_types::CertificateDer<'static>), Box<dyn Error + Send + Sync + 'static>> {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let cert_der = rustls::pki_types::CertificateDer::from(cert.cert);
    let priv_key = rustls::pki_types::PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der());

    let mut server_config = ServerConfig::with_single_cert(
        vec![cert_der.clone()], 
        priv_key.into()
    )?;
    
    let transport_config = Arc::get_mut(&mut server_config.transport).unwrap();
    transport_config.max_concurrent_uni_streams(0_u8.into());

    let endpoint = Endpoint::server(server_config, bind_addr)?;
    Ok((endpoint, cert_der))
}