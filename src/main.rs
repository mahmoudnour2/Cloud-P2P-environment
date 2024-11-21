mod dos;  // This tells Rust to look for the dos.rs file

use dos::{read_dos_file, write_dos_file, DosEntry, setup_drive_hub ,add_entry, delete_entry};
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let hub = setup_drive_hub().await?;
    let filename = "DoS.tsv";  // Replace with your actual file name
    let dos_entries = read_dos_file(&hub, filename).await?;
    
    // Add a new entry
    let new_entry = DosEntry {
        Client_ID: "new_client1".to_string(),
        Client_IP: "192.168.0.2".to_string(),
        resources: "resource_2".to_string(),
    };
    let mut updated_entries = dos_entries.clone();
    add_entry(&mut updated_entries, new_entry);

    // Delete an entry by Client_ID
    delete_entry(&mut updated_entries, "1234");

    // Write the updated entries back to Google Drive
    write_dos_file(&hub, filename, &updated_entries, "DoS").await?;

    Ok(())
}
