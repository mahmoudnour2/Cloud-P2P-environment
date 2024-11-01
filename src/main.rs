//! This example demonstrates how to make a QUIC connection that ignores the server certificate.
//!
//! Checkout the `README.md` for guidance.

use std::{
    fs,
    io::{self, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
    thread,
};

use anyhow::{anyhow, Result};
use clap::Parser;
use quinn_proto::crypto::rustls::QuicClientConfig;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime, PrivatePkcs8KeyDer};
use tracing::{error, info};
use url::Url;
use quinn::{Endpoint, ClientConfig, ServerConfig};
use std::error::Error;
use std::net::ToSocketAddrs;
use std::fs::File;

mod client;
mod server;

use client::*;
use server::*;
fn main() {
    
    let server_thread = thread::spawn(|| {
        tracing::subscriber::set_global_default(
        tracing_subscriber::FmtSubscriber::builder()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .finish(),
    )
    .unwrap();
    let opt = server::Opt::parse();
    let code = {
        if let Err(e) = server::run(opt) {
            eprintln!("ERROR: {e}");
            1
        } else {
            0
        }
    };
    ::std::process::exit(code);
    });
    
    
    let client_thread = thread::spawn(|| {
        tracing::subscriber::set_global_default(
        tracing_subscriber::FmtSubscriber::builder()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .finish(),
    )
    .unwrap();
    let opt = client::Opt::parse();
    // Check if the bind address is localhost
    if opt.bind.ip().is_loopback() {
        eprintln!("ERROR: Binding to localhost is not allowed.");
        std::process::exit(1);
    }
    let code = {
        if let Err(e) = client::run(opt) {
            eprintln!("ERROR: {e}");
            1
        } else {
            0
        }
    };
    ::std::process::exit(code);
    });

    server_thread.join().unwrap();
    client_thread.join().unwrap();
}

