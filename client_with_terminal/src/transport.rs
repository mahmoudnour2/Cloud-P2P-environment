use crossbeam::channel::{bounded, Receiver, Select, SelectTimeoutError, Sender};
use remote_trait_object::transport::*;
use log::debug;
use quinn::{Endpoint, ClientConfig, ServerConfig, Connection, SendStream, RecvStream, TransportConfig};
use futures::executor;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use std::error::Error;
use std::thread;
use tokio::runtime::Runtime;
use quinn_proto::crypto::rustls::QuicClientConfig;
use crate::quinn_utils::SkipServerVerification;

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
        }).join().map_err(|_| TransportError::Custom)?;

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
                        // Close the connection and drop the used port
                        connection.close(0u32.into(), b"connection error");
                        Err(TransportError::Custom)
                    }
                }
            })
        }).join().map_err(|_| TransportError::Custom)?;

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
    
    // Establish connections
    println!("Establishing connections...");
    let client_connecting = client_endpoint.connect(
        server_address,
        "localhost",
    ).map_err(|e| e.to_string())?;
    
    let client_conn = client_connecting.await.map_err(|e| e.to_string())?;
    

    // Receive the server's IP address

    let server_address = match client_conn.accept_bi().await {
        Ok((_, mut recv)) => {
    
            let mut buffer = Vec::new();
            let max_size = 500 * 1024 * 1024; // 500MB max size, adjust as needed
            buffer = recv.read_to_end(max_size).await
                .map_err(|e| {
                    eprintln!("Error reading data: {:?}", e);
                    e.to_string()
                })?;

            let server_ip = String::from_utf8(buffer).map_err(|e| e.to_string())?;
            println!("Received server IP address: {}", server_ip);
            let server_address: SocketAddr = server_ip.parse::<SocketAddr>().map_err(|e| e.to_string())?;
            Ok(server_address)
        },
        Err(e) => {
            eprintln!("Error accepting stream: {:?}", e);
            Err(e.to_string())
        }
    };

    let server_address = server_address?;


    
    

    // // Establish new connection using the received IP address
    // let new_client_addr: SocketAddr = "0.0.0.0:0".parse::<SocketAddr>().map_err(|e| e.to_string())?;  // Listen on this port

    // let mut client_config = ClientConfig::new(Arc::new(QuicClientConfig::try_from(
    //     rustls::ClientConfig::builder()
    //         .dangerous()
    //         .with_custom_certificate_verifier(SkipServerVerification::new())
    //         .with_no_client_auth(),
    // ).map_err(|e| e.to_string())?));
    // let mut transport_config = TransportConfig::default();
    // transport_config.keep_alive_interval(Some(Duration::from_secs(5)));
    // client_config.transport_config(Arc::new(transport_config));

    // let mut new_client_endpoint = quinn::Endpoint::client(new_client_addr).map_err(|e| e.to_string())?;
    // new_client_endpoint.set_default_client_config(client_config);


    let new_client_connecting = client_endpoint.connect(
        server_address,
        "localhost",
    ).map_err(|e| e.to_string())?;

    let new_client_conn = new_client_connecting.await.map_err(|e| e.to_string())?;
    
    println!("Connections established successfully.");

    Ok(TransportEnds {
        send: QuinnSend {
            connection: new_client_conn.clone(),
        },
        recv: QuinnRecv {
            connection: new_client_conn.clone(),
        },
    })
}