use crossbeam::channel::{unbounded, Receiver, Sender};
//use std::collections::HashMap;
use std::{env, thread};

mod image_steganographer;
use std::fs::File;
use image_steganographer::ImageSteganographer;
use image_steganographer::SomeImageSteganographer; // Add this line to import the trait
use std::io::{Read, Write};
use std::sync::{Arc, Barrier};
use remote_trait_object::{Context, Config, ServiceToExport, ServiceToImport};
use std::error::Error;
//use remote_trait_object::transport::{TransportSend, TransportRecv};
use transport::create;
use transport::TransportEnds;
mod transport;


fn main() {
    let (sender, receiver): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = unbounded();

    //left empty for now
    let file_path = "";

    // Hide the image using steganography
    //let steganographer = image_steganographer::ImageSteganographer::new(75, 10);
    let carrier_path = format!("{}/carrier.jpg", file_path);
    


    let secret_path = format!("{}/secret.jpg", file_path);
    let output_path1 = format!("{}/stego.png", file_path);
    let output_path2 = format!("{}/output.png", file_path);

    let carrier_path: &str = &format!("{}/carrier.jpg", file_path);
    let secret_path: &str = &format!("{}/secret.jpg", file_path);
    let output_path1: &str = &format!("{}/stego.png", file_path);
    let output_path2: &str = &format!("{}/output.png", file_path);
    //let image = steganographer.encode(&secret_path, &carrier_path, &output_path1).unwrap();
    
    

    //let transport = transport::create();

    //let (msg_sender, msg_receiver): (TransportSend, TransportRecv) = unbounded();

    let transport::TransportEnds {
        recv1,
        send1,
        recv2,
        send2,
    } = transport::create();

    let _context_steganographer = Context::with_initial_service_export(
        Config::default_setup(),
        send1,
        recv1,
        ServiceToExport::new(Box::new(SomeImageSteganographer::new(75,10)) as Box<dyn ImageSteganographer>),
    );

    let (_context_user, image_steganographer): (_, ServiceToImport<dyn ImageSteganographer>) =
        Context::with_initial_service_import(Config::default_setup(), send2, recv2);
    let image_steganographer_proxy: Box<dyn ImageSteganographer> = image_steganographer.into_proxy();

    // Test the encode method
    image_steganographer_proxy.encode(secret_path.clone(), carrier_path.clone(), output_path1.clone()).unwrap();
    println!("Encode method invoked successfully.");

    // Test the decode method
    image_steganographer_proxy.decode(output_path1.clone(), output_path2.clone()).unwrap();
    println!("Decode method invoked successfully.");

    /*
    loop {
        // Block the process and wait until a message is received from middleware
        let message = receiver.recv().expect("Failed to receive message from middleware");
        let sender_clone = sender.clone();
        thread::spawn(move || {
            let sender = sender_clone;
            // Process the message in a new thread
            println!("Received message: {:?}", message);

            // Hide the image using steganography
            let steganographer = image_steganographer::ImageSteganographer::new(75, 10);
            let image = steganographer.encode("carrier.jpg", "secret.jpg", "output.png").unwrap();

            // Turn it into bytes
            let mut image_data = Vec::new();
            image.write_to(&mut image_data, image::ImageOutputFormat::PNG).unwrap();

            // Send the image to the middleware and back to the client
            //let mut sender = sender.clone();
            sender.send(image_data).expect("Failed to send image data to middleware");
        });
    }
    */

    

    
}