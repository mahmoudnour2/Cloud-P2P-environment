use std::fs::File;
use std::io::{self, Read, Write};
use std::net::UdpSocket;

fn main() -> io::Result<()> {
    let socket = UdpSocket::bind("127.0.0.1:0")?; // Bind to any available port
    socket.connect("127.0.0.1:8080")?;

    // Read the image file
    let mut file = File::open("input_image.jpg")?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)?;

    // Send the image data to the server
    socket.send(&buffer)?;

    // Receive the echoed image data back from the server
    let mut recv_buffer = [0u8; 65507]; // Same size as server buffer
    let size = socket.recv(&mut recv_buffer)?;

    // Save the received data as a new image file
    let mut output = File::create("output_image.jpg")?;
    output.write_all(&recv_buffer[..size])?;

    println!("Image received and saved as output_image.jpg");

    Ok(())
}