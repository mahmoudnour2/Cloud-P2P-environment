use tokio::sync::mpsc;
use tokio::time::{Duration, sleep, timeout};
use std::collections::HashMap;
use rand::Rng;
use std::time::Instant;

use std::cmp::Ordering;
use serde::{Serialize, Deserialize};
use quinn::{Endpoint, ClientConfig, ServerConfig,Connection};
use quinn_proto::crypto::rustls::QuicClientConfig;
use std::net::SocketAddr;
use std::error::Error;
use anyhow::Result;
use std::sync::Arc;
use sysinfo::{System};
use crate::quinn_utils::*;
use std::future::Future;

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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemMetrics {
    pub cpu_load: f64,
    pub memory_usage: f64,
}

impl Default for SystemMetrics {
    fn default() -> Self {
        Self {
            cpu_load: 50.0,
            memory_usage: 60.0,
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
    server_endpoint: Endpoint,
    client_endpoints: Vec<(SocketAddr, Endpoint)>,
}

impl Node {
    pub async fn new(id: u64,
        server_addr: SocketAddr,
        peer_addrs: Vec<SocketAddr>,) 
        -> Result<Self> {
        
        //node.metrics = node.collect_metrics(); // Collect actual metrics
        // Setup server endpoint
        println!("Setting up server endpoint on {}", server_addr);
        let (server_endpoint, _cert) = make_server_endpoint(server_addr).map_err(|e| anyhow::anyhow!(e))?;
        
        // Setup client endpoints
        println!("Setting up client endpoints for {} peers", peer_addrs.len());
        let mut client_endpoints = Vec::new();
        for peer_addr in peer_addrs {
            let mut client_endpoint = Endpoint::client("0.0.0.0:0".parse().unwrap())?;
            client_endpoint.set_default_client_config(ClientConfig::new(Arc::new(QuicClientConfig::try_from(
                rustls::ClientConfig::builder()
                    .dangerous()
                    .with_custom_certificate_verifier(SkipServerVerification::new())
                    .with_no_client_auth(),
            )?)));
            client_endpoints.push((peer_addr, client_endpoint));
        }

        let mut node = Node {
            id,
            state: State::Follower,
            metrics: SystemMetrics::default(),
            last_heartbeat: Instant::now(),
            heartbeat_timeout: Duration::from_secs(5),
            negative_votes_received: HashMap::new(),
            candidates: Vec::new(),
            current_leader_id: None,
            server_endpoint,
            client_endpoints,
        };
        node.metrics = node.collect_metrics(); // Collect actual metrics

        Ok(node)
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
            // Update metrics before sending heartbeat
            let new_metrics = self.collect_metrics();
            self.metrics = new_metrics;
            self.broadcast_heartbeat().await;

            // Accept incoming connections and messages with a timeout
            match timeout(Duration::from_secs(1), self.server_endpoint.accept()).await {
                Ok(Some(incoming)) => {
                    if let Ok(conn) = incoming.await {
                        if let Ok((send, mut recv)) = conn.accept_bi().await {
                            if let Ok(msg_bytes) = recv.read_to_end(64 * 1024).await {
                                if let Ok(msg) = bincode::deserialize::<NodeMessage>(&msg_bytes) {
                                    match msg {
                                        NodeMessage::NegativeVote { voter_id, reason, metrics } => {
                                            println!("Leader received negative vote from Node {} due to {:?}", voter_id, reason);
                                            self.negative_votes_received.insert(voter_id, reason.clone());
                                            self.update_candidate(voter_id, metrics);
                                            
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
                            }
                        }
                    }
                }
                Ok(None) => {
                    // No incoming connection within the timeout duration
                }
                Err(_) => {
                    // Timeout occurred
                    println!("Leader timed out waiting for boradcasting heartbeat");
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

            // Accept incoming connections and messages with a timeout
            match timeout(Duration::from_secs(1), self.server_endpoint.accept()).await {
                Ok(Some(incoming)) => {
                    if let Ok(conn) = incoming.await {
                        if let Ok((send, mut recv)) = conn.accept_bi().await {
                            if let Ok(msg_bytes) = recv.read_to_end(64 * 1024).await {
                                if let Ok(msg) = bincode::deserialize::<NodeMessage>(&msg_bytes) {
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
                            }
                        }
                    }
                }
                Ok(None) => {
                    // No incoming connection within the timeout duration
                }
                Err(_) => {
                    // Timeout occurred
                    println!("Node {} timed out waiting for heartbeat", self.id);
                }
            }

            sleep(Duration::from_millis(500)).await;
        }
    }

    async fn handle_election(&mut self) {
        println!("👑 Node {} handling election as DefactoLeader", self.id);
        
        let self_metrics = self.collect_metrics();
        self.update_candidate(self.id, self_metrics.clone());
        
       if let Some(new_leader_id) = self.elect_leader(&self.candidates) {
            println!("👑 Node {} elected as new leader\n   New Leader Metrics:\n   CPU: {:.1}%\n   Memory: {:.1}%\n", 
                new_leader_id,
                self_metrics.cpu_load,
                self_metrics.memory_usage,
    
            );
             
            // Broadcast result
            self.broadcast_message(NodeMessage::ElectionResult { 
                new_leader_id 
            }).await.unwrap();

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
        }
    }
    fn measure_cpu_load(&self) -> f64 {
        let mut system = System::new_all();
        system.refresh_all();
        let cpu_load = system.global_cpu_usage();
        cpu_load as f64
    }

    fn measure_memory_usage(&self) -> f64 {
        let mut system = System::new_all();
        system.refresh_all();
        let total_memory = system.total_memory();
        let used_memory = system.total_memory() - system.available_memory();
        (used_memory as f64 / total_memory as f64) * 100.0
    }


    fn calculate_score(&self, metrics: &SystemMetrics) -> f64 {
        const CPU_WEIGHT: f64 = 0.6;
        const MEMORY_WEIGHT: f64 = 0.4;

   

        let cpu_score = 1.0 - (metrics.cpu_load / 100.0);
        let memory_score = 1.0 - (metrics.memory_usage / 100.0);

       

        let base_score = cpu_score * CPU_WEIGHT + memory_score * MEMORY_WEIGHT;

        // Add ±2% randomization for non-prejudiced selection
        base_score * (0.98 + rand::thread_rng().gen::<f64>() * 0.04)
    }

    fn should_cast_negative_vote(&self, leader_metrics: &SystemMetrics) -> Option<VoteReason> {
        let my_metrics = &self.metrics;

        if my_metrics.cpu_load < leader_metrics.cpu_load * 0.7 {
            println!("🗳️ Node {} casting negative vote due to HighCPULoad\n   Node Metrics vs Leader Metrics:\n   CPU: {:.1}% vs {:.1}%\n   Memory: {:.1}% vs {:.1}%\n", 
                self.id, 
                my_metrics.cpu_load, leader_metrics.cpu_load,
                my_metrics.memory_usage, leader_metrics.memory_usage,
            
            );
            return Some(VoteReason::HighCPULoad);
        }
        if my_metrics.memory_usage < leader_metrics.memory_usage * 0.7 {
            println!("🗳️ Node {} casting negative vote due to HighMemoryUsage\n   Node Metrics vs Leader Metrics:\n   CPU: {:.1}% vs {:.1}%\n   Memory: {:.1}% vs {:.1}%\n", 
                self.id, 
                my_metrics.cpu_load, leader_metrics.cpu_load,
                my_metrics.memory_usage, leader_metrics.memory_usage,
    
            );
            return Some(VoteReason::HighMemoryUsage);
        }
        
        None
    }

    async fn broadcast_heartbeat(&self) {
        println!("Node {} broadcasting heartbeat", self.id);
        self.broadcast_message(NodeMessage::Heartbeat {
            leader_id: self.id,
            metrics: self.metrics.clone(),
            candidates: self.candidates.clone(),
        }).await.unwrap();
    }

    async fn send_negative_vote(&mut self, leader_id: u64, reason: VoteReason) {
        let current_metrics = self.collect_metrics();
        let vote_msg = NodeMessage::NegativeVote { 
            voter_id: self.id,
            reason,
            metrics: current_metrics,
        };
        if let Err(e) = self.broadcast_message(vote_msg).await {
            println!("Failed to send negative vote to leader {}: {}", leader_id, e);
        }
    }

    async fn broadcast_message(&self, msg: NodeMessage) -> Result<()> {
        println!("Node {} broadcasting message", self.id);
        let msg_bytes = bincode::serialize(&msg)?;

        let msg_bytes = bincode::serialize(&msg).expect("Failed to serialize message");

        let mut tasks = vec![];

        for (peer_addr, client_endpoint) in &self.client_endpoints {
            let msg_bytes = msg_bytes.clone();
            let client_endpoint = client_endpoint.clone();
            let peer_addr = *peer_addr;

            let task = tokio::task::spawn(async move {
                println!("Establishing connection to {}", peer_addr);
                
                let result: Result<(), anyhow::Error> = async {
                    let conn = client_endpoint.connect(
                        peer_addr,
                        "localhost",
                    ).map_err(|e| anyhow::anyhow!("{}", e))?
                    .await.map_err(|e| anyhow::anyhow!("{}", e))?;
        
                    if let Ok((mut send, _recv)) = conn.open_bi().await {
                        println!("Sending message to {}", peer_addr);
                        let _ = send.write_all(&msg_bytes); 
                        let _ = send.finish();
                    }
                    Ok(())
                }.await;
                
                if let Err(e) = result {
                    println!("Error sending message to {}: {}", peer_addr, e);
                }
            });

            tasks.push(task);
            
        }

        // Wait for all tasks to complete
        for task in tasks {
            let _ = task.await;
        }
        Ok(())
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

/*
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

        let peer_addrs_clone = peer_addrs.clone();
        // Setup server endpoint
        println!("Setting up server endpoint on {}", server_addr);
        let (server_endpoint, _cert) = make_server_endpoint(server_addr).map_err(|e| anyhow::anyhow!(e))?;
        
        // Setup client endpoints
        println!("Setting up client endpoints for {} peers", peer_addrs.len());
        let mut client_endpoints = Vec::new();
        for peer_addr in peer_addrs {
            let mut client_endpoint = Endpoint::client("0.0.0.0:0".parse().unwrap())?;
            client_endpoint.set_default_client_config(ClientConfig::new(Arc::new(QuicClientConfig::try_from(
                rustls::ClientConfig::builder()
                    .dangerous()
                    .with_custom_certificate_verifier(SkipServerVerification::new())
                    .with_no_client_auth(),
            )?)));
            client_endpoints.push((peer_addr, client_endpoint));
        }

        // Setup channels
        let (tx, rx) = mpsc::channel(100);
        let mut node_channels = HashMap::new();
        
        for (i, peer_addr) in peer_addrs_clone.iter().enumerate() {
            let (node_tx, _) = mpsc::channel(100);
            node_channels.insert(i as u64, node_tx);
        }

        let node = Node::new(node_id, node_channels, rx);
        println!("QuinnNode setup complete");

        Ok(Self {
            node,
            server_endpoint,
            client_endpoints,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        // Spawn server handler
        let server_endpoint = self.server_endpoint.clone();
        let tx = self.node.node_channels.get(&self.node.id);
        if let Some(tx) = tx {
            let tx = tx.clone();
            tokio::spawn(async move {
                Self::handle_server(server_endpoint, tx).await;
            });
        }

        // Run the election node
        self.node.run().await;
        Ok(())
    }

    async fn handle_server(endpoint: Endpoint, tx: mpsc::Sender<NodeMessage>) {
        println!("Server listening on {}", endpoint.local_addr().unwrap());
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
            

            let conn = client_endpoint.connect(
                *peer_addr,
                "localhost",
            ).map_err(|e| anyhow::anyhow!("{}", e))?
            .await.map_err(|e| anyhow::anyhow!("{}", e))?;

            if let Ok((mut send, _recv)) = conn.open_bi().await {
                println!("Sending message to {}", peer_addr);
                let _ = send.write_all(&msg_bytes); 
                let _ = send.finish();
            }
        }
        Ok(())
    }
}
*/