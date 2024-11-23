mod dos;
use dos::{read_dos_file, write_dos_file, DosEntry, setup_drive_hub, clean_entries};
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let hub = setup_drive_hub().await?;
    let filename = "DoS.tsv";
    
    // Read current entries
    let mut dos_entries = read_dos_file(&hub, filename).await?;
    
    // Clean existing entries
    clean_entries(&mut dos_entries);
    
    // Add a new entry
    let new_entry = DosEntry::new(
        "new_client1".to_string(),
        "192.168.0.2".to_string(),
        "resource_2".to_string(),
    );
    let new_entry1 = DosEntry::new(
        "new_client2".to_string(),
        "192.168.0.4".to_string(),
        "resource_r".to_string(),
    );
    dos_entries.push(new_entry);
    dos_entries.push(new_entry1);

    // Write the updated entries back to Google Drive
    write_dos_file(&hub, filename, &dos_entries, "DoS").await?;

    Ok(())
}