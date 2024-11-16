use rustls::pki_types::CertificateDer;
use tokio::sync::mpsc;
use tokio::time::{Duration, sleep, timeout};
use std::collections::HashMap;
use rand::Rng;
use std::time::Instant;
use std::cmp::Ordering;
use serde::{Serialize, Deserialize};
use quinn::{ClientConfig, Connection, Endpoint, ServerConfig, TransportConfig};
use quinn_proto::crypto::rustls::QuicClientConfig;
use std::net::SocketAddr;
use std::error::Error;
use anyhow::Result;
use std::sync::Arc;
use sysinfo::{System};
use crate::{quinn_utils::*, CURRENT_LEADER_ID};
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

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
    pub server_endpoint: Endpoint,
    pub _cert: CertificateDer<'static>, 
    pub client_endpoints: Vec<(SocketAddr, Endpoint)>,
}

impl Node {
    pub async fn new(
        id: u64,
        server_addr: SocketAddr,
        peer_addrs: Vec<SocketAddr>,
    ) -> Result<Self> {
       // println!("Setting up server endpoint on {}", server_addr);
        let (server_endpoint, _cert) = make_server_endpoint(server_addr).map_err(|e| anyhow::anyhow!(e))?;
        
       //println!("Setting up client endpoints for {} peers", peer_addrs.len());
        let mut client_endpoints = Vec::new();
        for peer_addr in peer_addrs {
           // println!("Setting up client endpoint for peer {}", peer_addr);
            let mut client_endpoint = Endpoint::client("0.0.0.0:0".parse().unwrap())?;
            //println!("Client endpoint created for peer {}", peer_addr);
            
            let mut client_config = ClientConfig::new(Arc::new(QuicClientConfig::try_from(
                rustls::ClientConfig::builder()
                    .dangerous()
                    .with_custom_certificate_verifier(SkipServerVerification::new())
                    .with_no_client_auth(),
            )?));
            
            let mut transport_config = TransportConfig::default();
            transport_config.keep_alive_interval(Some(Duration::from_secs(10)));
            client_config.transport_config(Arc::new(transport_config));
            client_endpoint.set_default_client_config(client_config);
            //println!("Client config set for peer {}", peer_addr);
            client_endpoints.push((peer_addr, client_endpoint));
        }

        let mut node = Node {
            id,
            state: State::Follower,
            metrics: SystemMetrics::default(),
            last_heartbeat: Instant::now(),
            heartbeat_timeout: Duration::from_millis(5000 + rand::thread_rng().gen_range(0..1000)),
            negative_votes_received: HashMap::new(),
            candidates: Vec::new(),
            current_leader_id: None,
            server_endpoint,
            _cert,
            client_endpoints,
        };
        node.metrics = node.collect_metrics();

        Ok(node)
    }

    pub async fn run(&mut self) {
        loop {
            match self.state {
                State::Leader => self.run_leader().await,
                State::Follower => self.run_follower().await,
                State::DefactoLeader => self.handle_election().await,
            }
            let ctrl_c_timeout = Duration::from_secs(1);
            match timeout(ctrl_c_timeout, tokio::signal::ctrl_c()).await {
                Ok(Ok(())) => {
                    println!("Node {} received Ctrl+C, exiting...", self.id);
                    break;
                }
                Ok(Err(e)) => {
                    println!("Error waiting for Ctrl+C: {}", e);
                }
                Err(_) => {
                    // Timeout occurred, continue the loop
                }
            }
        }
    }
    
    async fn run_leader(&mut self) {
        loop {
            let new_metrics = self.collect_metrics();
            self.metrics = new_metrics;
            self.broadcast_heartbeat().await;
            
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
                    //println!("Leader timed out waiting for boradcasting heartbeat");
                    ();
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
            //println!("Listening!!!");
            
            match timeout(Duration::from_millis(100), self.server_endpoint.accept()).await {
                Ok(Some(incoming)) => {
                    if let Ok(conn) = incoming.await {
                        println!("{}", conn.remote_address());
                        if let Ok((send, mut recv)) = conn.accept_bi().await {
                           //println!("Connection Accepted");
                            
                            if let Ok(msg_bytes) = recv.read_to_end(64 * 1024).await {
                               // println!("Message Received");
                                //println!("Message length: {}", msg_bytes.len());
                                
                                if let Ok(msg) = bincode::deserialize::<NodeMessage>(&msg_bytes) {
                                   // println!("Begin Message Decoding");
                                    match msg {
                                        NodeMessage::Heartbeat { leader_id, metrics: leader_metrics, candidates } => {
                                            println!("Node {} received heartbeat from leader {}", self.id, leader_id);
                                            self.last_heartbeat = Instant::now();
                                            self.current_leader_id = Some(leader_id);
                                            CURRENT_LEADER_ID.store(leader_id, AtomicOrdering::SeqCst);
                                            self.candidates = candidates;
                                            
                                            if let Some(reason) = self.should_cast_negative_vote(&leader_metrics) {
                                                self.send_negative_vote(leader_id, reason).await;
                                            }
                                        }
                                        NodeMessage::ElectionResult { new_leader_id } => {
                                            println!("Node {} received election result: new leader is {}", self.id, new_leader_id);
                                            CURRENT_LEADER_ID.store(new_leader_id, AtomicOrdering::SeqCst);
                                            if new_leader_id == self.id {
                                                self.state = State::Leader;
                                                return;
                                            } else {
                                                self.current_leader_id = Some(new_leader_id);
                                            }
                                        }
                                        _ => {}
                                    }
                                } else {
                                    println!("MESSAGE NEVER DECODED");
                                }
                            } else {
                                println!("MESSAGE NEVER RECEIVED");
                            }
                        } else {
                            println!("CONNECTION NEVER ACCEPTED");
                        }
                    }
                }
                Ok(None) => {
                    // No incoming connection within the timeout duration
                }
                Err(_) => {
                    //println!("Timeout occurred while waiting for incoming connections");
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
            println!("👑 Node {} elected as new leader\n New Leader Metrics:\n CPU: {:.1}%\n Memory: {:.1}%\n", 
                new_leader_id,
                self_metrics.cpu_load,
                self_metrics.memory_usage,
            );
            
            self.broadcast_message(NodeMessage::ElectionResult { 
                new_leader_id 
            }).await.unwrap();

            self.state = if new_leader_id == self.id {
                CURRENT_LEADER_ID.store(new_leader_id, AtomicOrdering::SeqCst);
                State::Leader
            } else {
                State::Follower
            };

            self.candidates.clear();
            self.negative_votes_received.clear();
            self.last_heartbeat = Instant::now();
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

        if my_metrics.cpu_load < leader_metrics.cpu_load * 0.5 {
            println!("🗳️ Node {} casting negative vote due to HighCPULoad\n Node Metrics vs Leader Metrics:\n CPU: {:.1}% vs {:.1}%\n Memory: {:.1}% vs {:.1}%\n", 
                self.id, 
                my_metrics.cpu_load, leader_metrics.cpu_load,
                my_metrics.memory_usage, leader_metrics.memory_usage,
            );
            return Some(VoteReason::HighCPULoad);
        }
        if my_metrics.memory_usage < leader_metrics.memory_usage * 0.5 {
            println!("🗳️ Node {} casting negative vote due to HighMemoryUsage\n Node Metrics vs Leader Metrics:\n CPU: {:.1}% vs {:.1}%\n Memory: {:.1}% vs {:.1}%\n", 
                self.id, 
                my_metrics.cpu_load, leader_metrics.cpu_load,
                my_metrics.memory_usage, leader_metrics.memory_usage,
            );
            return Some(VoteReason::HighMemoryUsage);
        }
        
        None
    }

    pub async fn broadcast_heartbeat(&self) {
        //println!("Node {} broadcasting heartbeat", self.id);
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
        //println!("Node {} broadcasting message", self.id);
        let msg_bytes = bincode::serialize(&msg)?;
        let msg_bytes = bincode::serialize(&msg).expect("Failed to serialize message");
        let msg_bytes = &msg_bytes;
        let mut tasks = vec![];

        for (peer_addr, client_endpoint) in &self.client_endpoints {
            let msg_bytes = msg_bytes.clone();
            let client_endpoint = client_endpoint.clone();
            let peer_addr = *peer_addr;

            let task = tokio::task::spawn(async move {
                //println!("Establishing connection to {}", peer_addr);
                
                let result: Result<(), anyhow::Error> = async {
                    let conn = client_endpoint.connect(
                        peer_addr,
                        "localhost",
                    )?
                    .await?;
                    //println!("[client] connected: addr={}", conn.remote_address());

                    if let Ok((mut send, _recv)) = conn.open_bi().await {
                        //println!("Sending message to {}", peer_addr);
                        let _ = send.write_all(&msg_bytes).await; 
                        //println!("wrote mesagge");
                        let _ = send.finish();
                        //println!("finished mesaggess");
                        sleep(Duration::from_millis(50)).await;
                    }
                    Ok(())
                }.await;

                //println!("Message sent to {}", peer_addr);

                if let Err(e) = result {
                    println!("Error sending message to {}: {}", peer_addr, e);
                }
            });

            let random_timeout = Duration::from_millis(200 + rand::thread_rng().gen_range(0..100));
            let task = tokio::time::timeout(random_timeout, task);
            let task = async {
                match task.await {
                    Ok(_) => (),
                    Err(_) => (),
                }
            };

            tasks.push(task);
        }

        for task in tasks {
            let _ = task.await;
        }
        Ok(())
    }

    fn update_candidate(&mut self, candidate_id: u64, metrics: SystemMetrics) {
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
