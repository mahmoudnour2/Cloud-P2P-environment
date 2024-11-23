// dos.rs
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
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_RETRIES: u32 = 3;
const RETRY_DELAY_MS: u64 = 1000;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DosEntry {
    pub Client_ID: String,
    pub Client_IP: String,
    pub resources: String,
    #[serde(default)]
    pub last_updated: u64,  // Unix timestamp
}

impl DosEntry {
    pub fn new(client_id: String, client_ip: String, resources: String) -> Self {
        Self {
            Client_ID: client_id,
            Client_IP: client_ip,
            resources,
            last_updated: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        }
    }
}

pub async fn read_dos_file<C>(
    hub: &DriveHub<C>,
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
        .await?;

    let files = result.1.files.ok_or("No files found")?;
    let file = files.first().ok_or("File not found")?;
    let file_id = file.id.as_ref().ok_or("File ID not found")?;

    let response = hub.files()
        .get(file_id)
        .add_scope("https://www.googleapis.com/auth/drive.readonly")
        .param("alt", "media")
        .doit()
        .await?;

    let body_vec = response.0.into_body()
        .collect()
        .await?
        .to_bytes()
        .to_vec();

    let content = String::from_utf8(body_vec)?;
    let mut reader = ReaderBuilder::new()
        .delimiter(b'\t')
        .has_headers(true)
        .flexible(true)  // Allow flexible number of fields
        .from_reader(content.as_bytes());

    let mut dos_entries = Vec::new();
    for result in reader.deserialize() {
        match result {
            Ok(record) => {
                let entry: DosEntry = record;
                dos_entries.push(entry);
            }
            Err(e) => {
                eprintln!("Warning: Skipped malformed record: {}", e);
                continue;
            }
        }
    }

    Ok(dos_entries)
}

pub fn serialize_entries(entries: &[DosEntry]) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut buffer = Vec::new();
    {
        let mut writer = WriterBuilder::new()
            .delimiter(b'\t')
            .has_headers(true)
            .from_writer(&mut buffer);

        // Write entries without repeating headers
        for entry in entries {
            writer.serialize(entry)?;
        }
        
        writer.flush()?;
    }
    Ok(buffer)
}

// Add this validation function before writing
pub fn validate_entries(entries: &[DosEntry]) -> Result<(), Box<dyn Error>> {
    for entry in entries {
        if entry.Client_ID.trim().is_empty() {
            return Err("Client ID cannot be empty".into());
        }
        if entry.Client_IP.trim().is_empty() {
            return Err("Client IP cannot be empty".into());
        }
        // Basic IP format validation
        if !entry.Client_IP.split('.').all(|octet| {
            if let Ok(num) = octet.parse::<u8>() {
                true
            } else {
                false
            }
        }) {
            return Err(format!("Invalid IP address format: {}", entry.Client_IP).into());
        }
    }
    Ok(())
}

// Update the write_dos_file function to include validation
pub async fn write_dos_file<C>(
    hub: &DriveHub<C>,
    filename: &str,
    entries: &[DosEntry],
    target_name: &str
) -> Result<(), Box<dyn Error>>
where
    C: hyper_util::client::legacy::connect::Connect + Clone + Send + Sync + 'static,
{
    // Validate entries before proceeding
    validate_entries(entries)?;

    let parent_id = find_target_id(hub, target_name).await?;
    println!("Found target '{}' with ID: {}", target_name, parent_id);

    // Update timestamps
    let mut updated_entries = entries.to_vec();
    let current_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_secs();
    
    for entry in &mut updated_entries {
        entry.last_updated = current_time;
    }

    // Sort entries for consistency
    updated_entries.sort_by(|a, b| a.Client_ID.cmp(&b.Client_ID));

    // First try to create backup of existing file
    create_backup(hub, filename, &parent_id).await?;

    // Then delete existing file
    match delete_existing_files(hub, filename, &parent_id).await {
        Ok(_) => println!("Successfully deleted existing file"),
        Err(e) => eprintln!("Warning: Could not delete existing file: {}", e),
    }

    // Serialize entries to TSV with proper formatting
    let buffer = serialize_entries(&updated_entries)?;

    // Upload new file with retries
    let mut attempts = 0;
    while attempts < MAX_RETRIES {
        match upload_file(hub, filename, &parent_id, buffer.clone()).await {
            Ok(_) => return Ok(()),
            Err(e) => {
                attempts += 1;
                if attempts == MAX_RETRIES {
                    return Err(e);
                }
                eprintln!("Upload attempt {} failed: {}. Retrying...", attempts, e);
                tokio::time::sleep(tokio::time::Duration::from_millis(RETRY_DELAY_MS)).await;
            }
        }
    }

    Err("Failed to upload file after all retries".into())
}

// Add a new function to clean existing data
pub fn clean_entries(entries: &mut Vec<DosEntry>) {
    entries.dedup_by(|a, b| a.Client_ID == b.Client_ID);  // Remove duplicates
    entries.retain(|entry| {  // Remove invalid entries
        !entry.Client_ID.trim().is_empty() && 
        !entry.Client_IP.trim().is_empty() &&
        entry.last_updated > 0
    });
}
async fn create_backup<C>(
    hub: &DriveHub<C>,
    filename: &str,
    parent_id: &str,
) -> Result<(), Box<dyn Error>>
where
    C: hyper_util::client::legacy::connect::Connect + Clone + Send + Sync + 'static,
{
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_secs();
    
    let backup_name = format!("{}.backup_{}", filename, timestamp);

    // Find the existing file
    if let Ok((_, file_list)) = hub.files().list()
        .q(&format!("name = '{}' and '{}' in parents", filename, parent_id))
        .add_scope("https://www.googleapis.com/auth/drive")
        .supports_all_drives(true)
        .doit()
        .await
    {
        if let Some(files) = file_list.files {
            if let Some(file) = files.first() {
                if let Some(file_id) = &file.id {
                    let file_metadata = google_drive3::api::File {
                        name: Some(backup_name),
                        parents: Some(vec![parent_id.to_string()]),
                        ..Default::default()
                    };

                    hub.files()
                        .copy(file_metadata, file_id.as_str())
                        .add_scope("https://www.googleapis.com/auth/drive")
                        .supports_all_drives(true)
                        .doit()
                        .await?;
                }
            }
        }
    }

    Ok(())
}

async fn upload_file<C>(
    hub: &DriveHub<C>,
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

    hub.files()
        .create(file_metadata)
        .add_scope("https://www.googleapis.com/auth/drive")
        .supports_all_drives(true)
        .upload_resumable(cursor, "text/tab-separated-values".parse()?)
        .await?;

    println!("File '{}' uploaded successfully.", filename);
    Ok(())
}

pub async fn find_target_id<C>(
    hub: &DriveHub<C>,
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
            if let Some(id) = drive.id {
                return Ok(id);
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
            if let Some(id) = folder.id {
                return Ok(id);
            }
        }
    }

    Err(format!("No shared drive or folder found with name '{}'.", target_name).into())
}

async fn delete_existing_files<C>(
    hub: &DriveHub<C>,
    filename: &str,
    parent_id: &str
) -> Result<(), Box<dyn Error>>
where
    C: hyper_util::client::legacy::connect::Connect + Clone + Send + Sync + 'static,
{
    let result = hub.files().list()
        .add_scope("https://www.googleapis.com/auth/drive")
        .supports_all_drives(true)
        .include_items_from_all_drives(true)
        .q(&format!("name = '{}' and trashed = false and '{}' in parents", filename, parent_id))
        .doit()
        .await?;

    if let Some(files) = result.1.files {
        for file in files {
            if let Some(file_id) = file.id {
                match hub.files()
                    .delete(&file_id)
                    .add_scope("https://www.googleapis.com/auth/drive")
                    .supports_all_drives(true)
                    .doit()
                    .await
                {
                    Ok(_) => println!("Successfully deleted file: {}", file_id),
                    Err(e) => eprintln!("Failed to delete file {}: {}", file_id, e),
                }
            }
        }
    }

    Ok(())
}

pub async fn setup_drive_hub() -> Result<DriveHub<HttpsConnector<HttpConnector>>, Box<dyn Error>> {
    ring::default_provider()
        .install_default()
        .expect("Failed to install default CryptoProvider");

    let secret = yup_oauth2::read_application_secret("client_secret.json").await?;

    let auth = InstalledFlowAuthenticator::builder(secret, InstalledFlowReturnMethod::HTTPRedirect)
        .persist_tokens_to_disk("token.json")
        .build()
        .await?;

    let https_connector = HttpsConnectorBuilder::new()
        .with_native_roots()?
        .https_only()
        .enable_http1()
        .enable_http2()
        .build();

    let hyper_client = Client::builder(TokioExecutor::new())
        .build(https_connector);

    Ok(DriveHub::new(hyper_client, auth))
}