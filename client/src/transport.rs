use crossbeam::channel::{bounded, Receiver, Select, SelectTimeoutError, Sender};
use remote_trait_object::transport::*;
use log::debug;
use quinn::{Endpoint, ClientConfig, ServerConfig, Connection, SendStream, RecvStream};
use futures::executor;
use std::net::SocketAddr;
use std::sync::Arc;
use std::error::Error;
use std::thread;
use tokio::runtime::Runtime;
use std::time::Instant;

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
impl QuinnSend {
    pub fn connection(&self) -> &Connection {
        &self.connection
    }
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
        
        let result =
        futures::executor::block_on(async {
            let (mut send, _recv) = self.connection.open_bi().await
                .map_err(|e| {
                    eprintln!("Error opening bidirectional stream: {:?}", e);
                    TransportError::Custom
                })?;
            
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
        });
        

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

        let result =
        futures::executor::block_on(async {
            let (_, mut recv) = self.connection.accept_bi().await
                .map_err(|_| TransportError::Custom)?;
            
                let mut buffer = Vec::new();
                let max_size = 500*1024 * 1024; // 30MB max size, adjust as needed
                buffer = recv.read_to_end(max_size).await
                    .map_err(|_| TransportError::Custom)?;
                
                Ok(buffer)
        });

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
#[derive(Debug, Clone)]
pub struct TransportEnds {
    pub send: QuinnSend,
    pub recv: QuinnRecv,
}
// Create function now establishes Quinn connections
pub async fn create(client_endpoint: Endpoint, server_address: SocketAddr) -> Result<TransportEnds, String> {
    //let runtime = Arc::new(Runtime::new().map_err(|e| e.to_string())?);
    
    // Establish connections
    println!("Establishing connections...");
    let start = Instant::now();
    let client_conn = client_endpoint.connect(
        server_address,
        "localhost",
    ).map_err(|e| e.to_string())?
    .await.map_err(|e| e.to_string())?;
    let duration = start.elapsed();
    println!("Connection established successfully in {:.2?} seconds.", duration);
    // println!("Connections established successfully.");

    Ok(TransportEnds {
        send: QuinnSend {
            connection: client_conn.clone(),
        },
        recv: QuinnRecv {
            connection: client_conn.clone(),
        },
    })
}