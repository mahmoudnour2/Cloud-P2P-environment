use chacha20poly1305::aead::{Aead, KeyInit, OsRng};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce}; // Or `XChaCha20Poly1305`
use crossbeam::channel::{unbounded, Receiver, Sender};
use std::thread;

mod service_provider;
mod image_steganographer;

use service_provider::ImageEncryptor;
use std::fs::File;
use std::io::{Read, Write};
fn main() {

    let socket = UdpSocket::bind("10.40.32.167:8080")?;
    println!("UDP server listening on 10.40.32.167:8080");

    let mut buffer = [0u8; 1024];
    
    loop {
        // Wait for an incoming message
        let (size, src) = socket.recv_from(&mut buffer)?;

        // Convert the received bytes to a string (assuming UTF-8 encoded text)
        if let Ok(message) = str::from_utf8(&buffer[..size]) {
            println!("Received from {}: {}", src, message);

            // For now, we'll just echo the message back to the sender
            socket.send_to(message.as_bytes(), src)?;
        }
    }
//     let (sender, receiver): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = unbounded();

//     let key = ChaCha20Poly1305::generate_key(&mut OsRng);
//     let encryptor = ImageEncryptor::new(key.clone(), sender);

//     //Hide the image using steganography
//     let steganographer = image_steganographer::ImageSteganographer::new(75, 10);
//     let mut image = steganographer.hide_image("carrier.png","secret.png","output.png").unwrap();

//     // Load the image data
//     let mut image_data = Vec::new();
//     image.read_to_end(&mut image_data).expect("Failed to read image file");

//     thread::spawn(move || {
//         encryptor.encrypt_and_send(image_data);
//     });

//     let received_data = receiver.recv().unwrap();
//     println!("Received encrypted data: {:?}", received_data);

//     // Save the encrypted image
//     let mut encrypted_file = File::create("encrypted_image.enc").expect("Failed to create encrypted file");
//     encrypted_file.write_all(&received_data).expect("Failed to write encrypted data");

//     // Save the key
//     let mut key_file = File::create("encryption_key.key").expect("Failed to create key file");
//     key_file.write_all(key.as_slice()).expect("Failed to write key");
// 
}