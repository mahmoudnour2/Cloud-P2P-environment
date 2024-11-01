use crossbeam::channel::{bounded, Receiver, Select, SelectTimeoutError, Sender};
use remote_trait_object::transport::*;
use log::debug;
use quinn::{Endpoint, ClientConfig, ServerConfig, Connection, SendStream, RecvStream};
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
        self.runtime.block_on(async {
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
        self.runtime.block_on(async {
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
    pub send1: QuinnSend,
    pub recv1: QuinnRecv,
    pub send2: QuinnSend,
    pub recv2: QuinnRecv,
}

// Create function now establishes Quinn connections
pub async fn create(server_endpoint: Endpoint, client_endpoint: Endpoint) -> Result<TransportEnds, Box<dyn Error>> {
    let runtime = Arc::new(Runtime::new()?);
    
    // Establish connections
    let server_conn = server_endpoint.accept().await.unwrap().await?;
    let client_conn = client_endpoint.connect(
        server_endpoint.local_addr()?,
        "localhost",
    )?.await?;

    Ok(TransportEnds {
        send1: QuinnSend {
            connection: server_conn.clone(),
            runtime: runtime.clone(),
        },
        recv1: QuinnRecv {
            connection: server_conn,
            runtime: runtime.clone(),
        },
        send2: QuinnSend {
            connection: client_conn.clone(),
            runtime: runtime.clone(),
        },
        recv2: QuinnRecv {
            connection: client_conn,
            runtime,
        },
    })
}
