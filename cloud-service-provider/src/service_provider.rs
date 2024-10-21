use chacha20poly1305::aead::{Aead, KeyInit, OsRng};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce}; // Or `XChaCha20Poly1305`
use crossbeam::channel::{unbounded, Receiver, Sender};
use std::thread;

pub struct ImageEncryptor {
    key: Key,
    sender: Sender<Vec<u8>>,
}

impl ImageEncryptor {
    pub fn new(key: Key, sender: Sender<Vec<u8>>) -> Self {
        Self { key, sender }
    }

    pub fn encrypt_and_send(&self, image_data: Vec<u8>) {
        let cipher = ChaCha20Poly1305::new(&self.key);
        let nonce = Nonce::from_slice(b"unique nonce"); // 96-bits; unique per message

        let encrypted_data = cipher.encrypt(nonce, image_data.as_ref())
            .expect("encryption failure!");

        self.sender.send(encrypted_data).unwrap();
    }
}

