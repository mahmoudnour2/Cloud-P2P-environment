use crossbeam::channel::{bounded, Receiver, Select, SelectTimeoutError, Sender};
use remote_trait_object::transport::*;
use log::debug;
use quinn::{Endpoint, ClientConfig, ServerConfig, Connection, SendStream, RecvStream};
use futures::executor;
use std::net::SocketAddr;
use std::sync::Arc;
use std::error::Error;
use tokio::runtime::Runtime;

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
#[derive(Debug)]
pub struct QuinnSend {
    connection: Connection,
    runtime: Arc<Runtime>,
}


impl TransportSend for QuinnSend {
    fn send(
        &self,
        data: &[u8],
        timeout: Option<std::time::Duration>,
    ) -> Result<(), TransportError> {
        let data = data.to_vec();
        futures::executor::block_on(async {
            let (mut send, _recv) = self.connection.open_bi().await
                .map_err(|e| TransportError::Custom)?;
            
            send.write_all(&data).await
                .map_err(|e| TransportError::Custom)?;
            
            send.finish()
                .map_err(|e| TransportError::Custom)
        })
    }

    fn create_terminator(&self) -> Box<dyn Terminate> {
        Box::new(QuinnTerminator(self.connection.clone()))
    }
}

// Modified IntraRecv to use Quinn
#[derive(Debug)]
pub struct QuinnRecv {
    connection: Connection,
    runtime: Arc<Runtime>,
}

impl TransportRecv for QuinnRecv {
    fn recv(&self, timeout: Option<std::time::Duration>) -> Result<Vec<u8>, TransportError> {
        
        futures::executor::block_on(async {
            let (_, mut recv) = self.connection.accept_bi().await
                .map_err(|e| TransportError::Custom)?;
            
                let mut buffer = Vec::new();
                let max_size = 30*1024 * 1024; // 30MB max size, adjust as needed
                buffer = recv.read_to_end(max_size).await
                    .map_err(|e| TransportError::Custom)?;
                
                Ok(buffer)
        })
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
pub struct TransportEnds {
    pub send: QuinnSend,
    pub recv: QuinnRecv,
}

// Create function now establishes Quinn connections
pub async fn create(client_endpoint: Endpoint, server_address: SocketAddr) -> Result<TransportEnds, String> {
    let runtime = Arc::new(Runtime::new().map_err(|e| e.to_string())?);
    
    // Establish connections
    println!("Establishing connections...");
    let client_conn = client_endpoint.connect(
        server_address,
        "localhost",
    ).map_err(|e| e.to_string())?
    .await.map_err(|e| e.to_string())?;
    println!("Connections established successfully.");

    Ok(TransportEnds {
        send: QuinnSend {
            connection: client_conn.clone(),
            runtime: runtime.clone(),
        },
        recv: QuinnRecv {
            connection: client_conn.clone(),
            runtime: runtime.clone(),
        },
    })
}
