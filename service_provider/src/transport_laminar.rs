use crossbeam::channel::{bounded, Receiver, Select, SelectTimeoutError, Sender};
use remote_trait_object::transport::*;
use log::debug;
use quinn::{Endpoint, ClientConfig, ServerConfig, Connection, SendStream, RecvStream};
use futures::executor;
use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::error::Error;
use std::thread;
use tokio::runtime::Runtime;
use std::hash::{Hash, Hasher};
use std::net::IpAddr;
use crate::quinn_utils::*;
use local_ip_address::local_ip;
use laminar::{Socket, Packet};

// Custom transport error types
#[derive(Debug)]
pub enum QuinnTransportError {
    ConnectionError(quinn::ConnectionError),
    WriteError(quinn::WriteError),
    ReadError(quinn::ReadError),
    Custom(String),
}

impl From<quinn::ConnectionError> for QuinnTransportError {
    fn from(err: quinn::ConnectionError) -> Self {
        QuinnTransportError::ConnectionError(err)
    }
}

// Modified IntraSend to use Quinn
#[derive(Debug,Clone)]
pub struct QuinnSend {
    connection: Connection,
    //runtime: Arc<Runtime>,
}


impl TransportSend for QuinnSend {
    fn send(
        &self,
        data: &[u8],
        timeout: Option<std::time::Duration>,
    ) -> Result<(), TransportError> {
        let data = data.to_vec();
        /*
        let connection = self.connection.clone();
        let result = thread::spawn(move || {
            futures::executor::block_on(async {
                let (mut send, _recv) = connection.open_bi().await
                    .map_err(|_| TransportError::Custom)?;
                
                send.write_all(&data).await
                    .map_err(|_| TransportError::Custom)?;
                
                send.finish()
                    .map_err(|_| TransportError::Custom)
            })
        }).join().expect("Thread panicked");
        */
        
        let data = data.to_vec();
        let connection = self.connection.clone();
        let result = thread::spawn(move || {
            let rt = Runtime::new().expect("Failed to create runtime");
            rt.block_on(async move {
                match connection.open_bi().await {
                    Ok((mut send, _recv)) => {
                
                        send.write_all(&data).await
                            .map_err(|e| {
                                eprintln!("Error writing data: {:?}", e);
                                TransportError::Custom
                            })?;
                        
                        send.finish()
                            .map_err(|e| {
                                eprintln!("Error finishing stream: {:?}", e);
                                TransportError::Custom
                            })
                    },
                    Err(e) => {
                        eprintln!("Error opening stream: {:?}", e);
                        Err(TransportError::Custom)
                    }
                }
            })
        }).join().expect("Thread panicked");

        result
    }

    fn create_terminator(&self) -> Box<dyn Terminate> {
        Box::new(QuinnTerminator(self.connection.clone()))
    }
}

// Modified IntraRecv to use Quinn
#[derive(Debug, Clone)]
pub struct QuinnRecv {
    connection: Connection,
    //runtime: Arc<Runtime>,
}

impl TransportRecv for QuinnRecv {
    fn recv(&self, timeout: Option<std::time::Duration>) -> Result<Vec<u8>, TransportError> {
        
        /*
        let connection = self.connection.clone();
        
        let result:Result<Vec<u8>, TransportError> = thread::spawn(move || {
            futures::executor::block_on(async {
                let (_, mut recv) = connection.accept_bi().await
                    .map_err(|_| TransportError::Custom)?;
                
                let mut buffer = Vec::new();
                let max_size = 30*1024 * 1024; // 30MB max size, adjust as needed
                buffer = recv.read_to_end(max_size).await
                    .map_err(|_| TransportError::Custom)?;
                
                Ok(buffer)
            })
        }).join().expect("Thread panicked");
        */

        let connection = self.connection.clone();
        let result = thread::spawn(move || {
            let rt = Runtime::new().expect("Failed to create runtime");
            rt.block_on(async move {
                match connection.accept_bi().await {
                    Ok((_, mut recv)) => {
                
                        let mut buffer = Vec::new();
                        let max_size = 500 * 1024 * 1024; // 500MB max size, adjust as needed
                        buffer = recv.read_to_end(max_size).await
                            .map_err(|e| {
                                eprintln!("Error reading data: {:?}", e);
                                TransportError::Custom
                            })?;

                        Ok(buffer)
                    },
                    Err(e) => {
                        eprintln!("Error accepting stream: {:?}", e);
                        Err(TransportError::Custom)
                    }
                }
            })
        }).join().expect("Thread panicked");

        result
    }

    fn create_terminator(&self) -> Box<dyn Terminate> {
        Box::new(QuinnTerminator(self.connection.clone()))
    }
}

// Modified Terminator for Quinn
pub struct QuinnTerminator(Connection);

impl Terminate for QuinnTerminator {
    fn terminate(&self) {
        self.0.close(0u32.into(), b"terminated");
    }
}

// Modified TransportEnds for Quinn
#[derive(Debug,Clone)]
pub struct TransportEnds {
    pub send: QuinnSend,
    pub recv: QuinnRecv,
}
impl PartialEq for TransportEnds {
    fn eq(&self, other: &Self) -> bool {
        self.send.connection.stable_id() == other.send.connection.stable_id() &&
        self.recv.connection.stable_id() == other.recv.connection.stable_id()
    }
}

impl Eq for TransportEnds {}

impl Hash for TransportEnds {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.send.connection.stable_id().hash(state);
        self.recv.connection.stable_id().hash(state);
    }
}



impl TransportEnds {

    pub fn is_active(&self) -> bool {

        // Check if the connection is still active by attempting a simple operation with a timeout
        let result = futures::executor::block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(5), self.send.connection.open_uni()).await
        });

        match result {
            Ok(Ok(_)) => true,
            _ => false,
        }

    }
    pub fn get_connection_id(&self) -> String{
        format!("{}", self.send.connection.stable_id())
    }
    pub fn get_remote_address(&self) -> String{
        format!("{}", self.send.connection.remote_address())
    }


}


// Create function now establishes Quinn connections
pub async fn create(server_conn: Connection) -> Result<TransportEnds, String> {
    
    // Establish connections
    println!("Establishing connections...");
    //let server_conn = server_endpoint.accept().await.unwrap().await.map_err(|e| e.to_string())?;
    let local_ip: IpAddr = local_ip().unwrap();
    let local_addr = SocketAddr::new(local_ip, 0);

    let socket = UdpSocket::bind(local_addr).map_err(|e| e.to_string())?;
    let actual_addr = socket.local_addr().map_err(|e| e.to_string())?;
    println!("Assigned port: {}", actual_addr.port());
    std::mem::drop(socket);

    let addr = format!("{}", actual_addr);
    println!("Sending address: {}", addr);
    match server_conn.open_bi().await {
        Ok((mut send, _recv)) => {
    
            send.write_all(addr.as_bytes()).await
                .map_err(|e| {
                    eprintln!("Error writing data: {:?}", e);
                    e.to_string()
                })?;
            
            send.finish()
                .map_err(|e| {
                    eprintln!("Error finishing stream: {:?}", e);
                    e.to_string()
                })?;
        },
        Err(e) => {
            eprintln!("Error opening stream: {:?}", e);
            return Err(e.to_string());
        }
    };

    let (endpoint, _cert) = make_server_endpoint(actual_addr).unwrap();
    let new_conn = endpoint.accept().await.unwrap().await.map_err(|e| e.to_string())?;

    println!("Connections established successfully.");

    let transport_ends = TransportEnds {
        send: QuinnSend {
            connection: new_conn.clone(),
        },
        recv: QuinnRecv {
            connection: new_conn.clone(),
        },
    };

    // Close the conections
    server_conn.close(0u32.into(), b"endpoint closed");


    Ok(transport_ends)

}