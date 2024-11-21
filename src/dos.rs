use hyper_util::client::legacy::Client;
use hyper_rustls::HttpsConnectorBuilder;
use yup_oauth2::{InstalledFlowAuthenticator, InstalledFlowReturnMethod};
use google_drive3::DriveHub;
use hyper_util::rt::TokioExecutor;
use rustls::crypto::ring;
use std::error::Error;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_rustls::HttpsConnector;
use csv::{ReaderBuilder, WriterBuilder};
use google_drive3::api::File as DriveFile;
use std::io::Cursor;
use http_body_util::BodyExt;

// Struct to represent the DoS.tsv file's data
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DosEntry {
    pub Client_ID: String,
    pub Client_IP: String,
    pub resources: String,
}

pub async fn read_dos_file<C>(
    hub: &google_drive3::DriveHub<C>,
    filename: &str
) -> Result<Vec<DosEntry>, Box<dyn Error>>
where
    C: hyper_util::client::legacy::connect::Connect + Clone + Send + Sync + 'static,
{
    let result = hub.files().list()
        .add_scope("https://www.googleapis.com/auth/drive.readonly")
        .supports_all_drives(true)
        .include_items_from_all_drives(true)
        .q(&format!("name = '{}' and trashed = false", filename))
        .spaces("drive")
        .doit()
        .await;

    match result {
        Ok((_, files_list)) => {
            if let Some(files) = files_list.files {
                if let Some(file) = files.first() {
                    if let Some(file_id) = &file.id {
                        println!("Found file with ID: {}", file_id);

                        // Download file content
                        match hub.files()
                            .get(file_id)
                            .add_scope("https://www.googleapis.com/auth/drive.readonly")
                            .param("alt", "media")
                            .doit()
                            .await 
                        {
                            Ok((response, _)) => {
                                let body_vec = response.into_body()
                                    .collect()
                                    .await?
                                    .to_bytes()
                                    .to_vec();

                                // Convert body to string
                                let content = String::from_utf8(body_vec)?;

                                // Parse TSV using csv crate
                                let mut reader = ReaderBuilder::new()
                                    .delimiter(b'\t')
                                    .has_headers(true) // Explicitly handle headers
                                    .from_reader(content.as_bytes());

                                let mut dos_entries = Vec::new();
                                for result in reader.deserialize() {
                                    let record: DosEntry = result?;
                                    dos_entries.push(record);
                                }

                                Ok(dos_entries)
                            },
                            Err(e) => {
                                eprintln!("Error downloading file: {:?}", e);
                                Err(format!("Failed to download file: {:?}", e).into())
                            }
                        }
                    } else {
                        Err("File ID not found".into())
                    }
                } else {
                    Err("File not found".into())
                }
            } else {
                Err("No files found".into())
            }
        }
        Err(err) => {
            eprintln!("Error listing files: {:?}", err);
            Err(err.into())
        }
    }
}

// Function to write entries back to the file (with additions or deletions)
pub async fn write_dos_file<C>(
    hub: &google_drive3::DriveHub<C>,
    filename: &str,
    entries: &[DosEntry],
    target_name: &str
) -> Result<(), Box<dyn Error>>
where
    C: hyper_util::client::legacy::connect::Connect + Clone + Send + Sync + 'static,
{
    let parent_id = find_target_id(hub, target_name).await?;
    println!("Found target '{}' with ID: {}", target_name, parent_id);

    // First, delete the existing file before uploading the new one
    delete_existing_files(hub, filename, &parent_id).await?;

    // Serialize the entries to TSV format
    let buffer = serialize_entries(entries)?;

    // Upload the new file to Google Drive
    upload_file(hub, filename, &parent_id, buffer).await
}

// Helper function to serialize entries into TSV format (excluding the header)
pub fn serialize_entries(entries: &[DosEntry]) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut buffer = Vec::new();
    let mut writer = WriterBuilder::new()
        .delimiter(b'\t')
        .has_headers(false) // Ensure headers are not written again
        .from_writer(&mut buffer);

    // Write only the entries without the header
    for entry in entries {
        writer.serialize(entry)?;
    }
    writer.flush()?; // Ensure all data is written to the buffer
    drop(writer); // Drop writer to release the borrow on buffer
    Ok(buffer)
}

// Helper function to upload file to Google Drive
pub async fn upload_file<C>(
    hub: &google_drive3::DriveHub<C>,
    filename: &str,
    parent_id: &str,
    buffer: Vec<u8>
) -> Result<(), Box<dyn Error>>
where
    C: hyper_util::client::legacy::connect::Connect + Clone + Send + Sync + 'static,
{
    let file_metadata = DriveFile {
        name: Some(filename.to_string()),
        mime_type: Some("text/tab-separated-values".to_string()),
        parents: Some(vec![parent_id.to_string()]),
        ..Default::default()
    };

    let cursor = Cursor::new(buffer);

    match hub.files()
        .create(file_metadata)
        .add_scope("https://www.googleapis.com/auth/drive")
        .supports_all_drives(true)
        .upload_resumable(cursor, "text/tab-separated-values".parse()?)
        .await
    {
        Ok(_) => {
            println!("File '{}' uploaded successfully.", filename);
            Ok(())
        }
        Err(e) => {
            eprintln!("Error uploading file '{}': {:?}", filename, e);
            Err(Box::new(e))
        }
    }
}

// Function to delete an entry from the list
pub fn delete_entry(entries: &mut Vec<DosEntry>, client_id: &str) {
    if let Some(pos) = entries.iter().position(|e| e.Client_ID == client_id) {
        entries.remove(pos);
    }
}

// Function to add an entry to the list
pub fn add_entry(entries: &mut Vec<DosEntry>, new_entry: DosEntry) {
    entries.push(new_entry);
}


// Helper function to find the target folder or drive
pub async fn find_target_id<C>(
    hub: &google_drive3::DriveHub<C>,
    target_name: &str
) -> Result<String, Box<dyn Error>>
where
    C: hyper_util::client::legacy::connect::Connect + Clone + Send + Sync + 'static,
{
    // Try finding a shared drive first
    if let Ok((_, result)) = hub.drives().list()
        .add_scope("https://www.googleapis.com/auth/drive")
        .q(&format!("name = '{}'", target_name))
        .doit()
        .await
    {
        if let Some(drive) = result.drives.and_then(|drives| drives.first().cloned()) {
            if let Some(id) = &drive.id {
                return Ok(id.clone());
            } else {
                return Err(format!("Shared drive '{}' found but has no ID.", target_name).into());
            }
        }
    }
    
    // If no shared drive, try finding a folder
    if let Ok((_, result)) = hub.files().list()
        .add_scope("https://www.googleapis.com/auth/drive")
        .supports_all_drives(true)
        .include_items_from_all_drives(true)
        .q(&format!("name = '{}' and trashed = false and mimeType = 'application/vnd.google-apps.folder'", target_name))
        .doit()
        .await
    {
        if let Some(folder) = result.files.and_then(|files| files.first().cloned()) {
            return folder.id.clone().ok_or_else(|| {
                format!("Folder '{}' found but has no ID.", target_name)
            }.into());
        }
    }

    Err(format!("No shared drive or folder found with name '{}'.", target_name).into())
}

// Helper function to check for and delete existing files
pub async fn delete_existing_files<C>(
    hub: &google_drive3::DriveHub<C>,
    filename: &str,
    parent_id: &str
) -> Result<(), Box<dyn Error>>
where
    C: hyper_util::client::legacy::connect::Connect + Clone + Send + Sync + 'static,
{
    if let Ok((_, result)) = hub.files().list()
        .add_scope("https://www.googleapis.com/auth/drive")
        .supports_all_drives(true)
        .include_items_from_all_drives(true)
        .q(&format!("name = '{}' and trashed = false and '{}' in parents", filename, parent_id))
        .doit()
        .await
    {
        if let Some(files) = result.files {
            for file in files {
                if let Some(file_id) = file.id {
                    // Attempt to delete the file
                    println!("Deleting existing file with ID: {}", file_id);
                    let delete_result = hub.files()
                        .delete(&file_id)
                        .add_scope("https://www.googleapis.com/auth/drive")
                        .supports_all_drives(true)
                        .doit()
                        .await;

                    if let Err(e) = delete_result {
                        if let Some(drive_error) = e.source().and_then(|source| source.downcast_ref::<google_drive3::Error>()) {
                            if drive_error.to_string().contains("notFound") {
                                println!("File not found during deletion (likely already deleted): {}", file_id);
                                continue;
                            }
                        }
                        return Err(format!("Failed to delete file '{}': {:?}", file_id, e).into());
                    }
                }
            }
        }
    }
    Ok(())
}


// Example usage remains the same as in previous version
pub async fn setup_drive_hub() -> Result<DriveHub<HttpsConnector<HttpConnector>>, Box<dyn Error>>
{
    ring::default_provider()
        .install_default()
        .expect("Failed to install default CryptoProvider");

    // Load OAuth2 credentials from client_secret.json
    let secret = yup_oauth2::read_application_secret("client_secret.json").await?;

    // Set up the authenticator with Installed Flow
    let auth = InstalledFlowAuthenticator::builder(secret, InstalledFlowReturnMethod::HTTPRedirect)
        .persist_tokens_to_disk("token.json") // Optional: Store tokens for reuse
        .build()
        .await?;

    // Create an HTTPS connector
    let https_connector = HttpsConnectorBuilder::new()
        .with_native_roots()? // Use system's native certificate roots
        .https_only()         // Only allow HTTPS connections
        .enable_http1()       // Enable HTTP/1.1
        .enable_http2()       // Enable HTTP/2
        .build();

    // Build the Hyper client
    let hyper_client = Client::builder(TokioExecutor::new())
        .build(https_connector);

    // Create the Google Drive Hub
    let hub = DriveHub::new(hyper_client, auth);

    Ok(hub)
}


pub async fn list_files_in_shared_drive<C>(
    hub: &DriveHub<C>,
) -> Result<(), Box<dyn Error>>
where
    C: hyper_util::client::legacy::connect::Connect + Clone + Send + Sync + 'static,
{
    let result = hub.drives().list().doit().await;

    match result {
        Ok((_, drive_list)) => {
            if let Some(drives) = drive_list.drives {
                let mut shared_drive_id = None;
                for drive in drives {
                    if let Some(name) = drive.name {
                        if name == "Distributed Project" {
                            shared_drive_id = Some(drive.id.unwrap_or_default());
                            println!("Found shared drive 'Distributed Project' with ID: {}", shared_drive_id.clone().unwrap());
                            break;
                        }
                    }
                }
                if shared_drive_id.is_none() {
                    println!("Shared drive 'Distributed Project' not found.");
                    return Ok(());
                }

                // Now list files and folders within the 'Distributed Project' shared drive
                let mut next_page_token: Option<String> = None;
                println!("Listing files and folders in 'Distributed Project':");

                loop {
                    let result = hub.files().list()
                        .add_scope("https://www.googleapis.com/auth/drive.readonly") // Use read-only scope
                        .supports_all_drives(true)                                  // Include shared drives
                        .include_items_from_all_drives(true)                        // Include shared items
                        .corpora("drive")                                           // Use 'drive' as the corpora
                        .drive_id(shared_drive_id.clone().unwrap().as_str())       // Pass the shared drive ID
                        .param("fields", "nextPageToken, files(id, name)")          // Use .param() to specify fields
                        .page_token(next_page_token.unwrap_or_default().as_str())
                        .doit()
                        .await;

                    match result {
                        Ok((_, files_list)) => {
                            if let Some(files) = files_list.files {
                                if files.is_empty() {
                                    println!("No files or folders found in the shared drive.");
                                } else {
                                    for file in files {
                                        println!(
                                            "Item: {} (ID: {})",
                                            file.name.unwrap_or("Unnamed".to_string()),
                                            file.id.unwrap_or("Unknown ID".to_string())
                                        );
                                    }
                                }
                            } else {
                                println!("No files or folders found in the shared drive.");
                            }
                            next_page_token = files_list.next_page_token;
                            if next_page_token.is_none() {
                                break; // Exit loop if no more pages
                            }
                        }
                        Err(err) => {
                            eprintln!("Error listing files and folders: {:?}", err);
                            break;
                        }
                    }
                }
            }
        }
        Err(err) => {
            eprintln!("Error listing shared drives: {:?}", err);
        }
    }

    Ok(())
}
