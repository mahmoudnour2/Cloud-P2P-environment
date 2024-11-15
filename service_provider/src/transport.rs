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
use std::hash::{Hash, Hasher};

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
        let connection = self.connection.clone();
        
        // let result = thread::spawn(move || {
        //     futures::executor::block_on(async {
        //         let (mut send, _recv) = connection.open_bi().await
        //             .map_err(|_| TransportError::Custom)?;
                
        //         send.write_all(&data).await
        //             .map_err(|_| TransportError::Custom)?;
                
        //         send.finish()
        //             .map_err(|_| TransportError::Custom)
        //     })
        // }).join().expect("Thread panicked");
        
        
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
#[derive(Debug, Clone)]
pub struct QuinnRecv {
    connection: Connection,
    //runtime: Arc<Runtime>,
}

impl TransportRecv for QuinnRecv {
    fn recv(&self, timeout: Option<std::time::Duration>) -> Result<Vec<u8>, TransportError> {
        
        let connection = self.connection.clone();
        // let result:Result<Vec<u8>, TransportError> = thread::spawn(move || {
        //     futures::executor::block_on(async {
        //         let (_, mut recv) = connection.accept_bi().await
        //             .map_err(|_| TransportError::Custom)?;
                
        //         let mut buffer = Vec::new();
        //         let max_size = 30*1024 * 1024; // 30MB max size, adjust as needed
        //         buffer = recv.read_to_end(max_size).await
        //             .map_err(|_| TransportError::Custom)?;
                
        //         Ok(buffer)
        //     })
        // }).join().expect("Thread panicked");

        futures::executor::block_on(async {
            let (_, mut recv) = self.connection.accept_bi().await
                .map_err(|_| TransportError::Custom)?;
            
                let mut buffer = Vec::new();
                let max_size = 30*1024 * 1024; // 30MB max size, adjust as needed
                buffer = recv.read_to_end(max_size).await
                    .map_err(|_| TransportError::Custom)?;
                
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
    pub fn close(&self) {
        self.send.connection.close(0u32.into(), &[]);
    }


}


// Create function now establishes Quinn connections
pub async fn create(server_conn: Connection) -> Result<TransportEnds, String> {
    
    // Establish connections
    println!("Establishing connections...");
    //let server_conn = server_endpoint.accept().await.unwrap().await.map_err(|e| e.to_string())?;
    println!("Connections established successfully.");

    Ok(TransportEnds {
        send: QuinnSend {
            connection: server_conn.clone(),
        },
        recv: QuinnRecv {
            connection: server_conn.clone(),
        },
    })
}