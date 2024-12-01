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
use std::net::IpAddr;
use std::str::FromStr;

const MAX_RETRIES: u32 = 3;
const RETRY_DELAY_MS: u64 = 1000;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DosEntry {
    pub Client_ID: String,
    pub Client_IP: String,
    pub resources: String,
    #[serde(default = "default_timestamp")]
    pub last_updated: u64, // Unix timestamp
}

fn default_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl DosEntry {
    pub fn new(client_id: String, client_ip: String, resources: String) -> Self {
        Self {
            Client_ID: client_id,
            Client_IP: client_ip,
            resources,
            last_updated: default_timestamp(),
        }
    }
}

pub async fn read_dos_file<C>(
    hub: &DriveHub<C>,
    filename: &str,
) -> Result<Vec<DosEntry>, Box<dyn Error>>
where
    C: hyper_util::client::legacy::connect::Connect + Clone + Send + Sync + 'static,
{
    match hub
        .files()
        .list()
        .add_scope("https://www.googleapis.com/auth/drive.readonly")
        .supports_all_drives(true)
        .include_items_from_all_drives(true)
        .q(&format!("name = '{}' and trashed = false", filename))
        .spaces("drive")
        .doit()
        .await
    {
        Ok(result) => {
            let files = result.1.files.ok_or("No files found")?;
            let file = files.first().ok_or("File not found")?;
            let file_id = file.id.as_ref().ok_or("File ID not found")?;

            let response = hub
                .files()
                .get(file_id)
                .add_scope("https://www.googleapis.com/auth/drive.readonly")
                .param("alt", "media")
                .doit()
                .await?;

            let body_vec = response.0.into_body().collect().await?.to_bytes().to_vec();

            let content = String::from_utf8(body_vec)?;
            let mut reader = ReaderBuilder::new()
                .delimiter(b'\t')
                .has_headers(true)
                .flexible(true)
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
        Err(e) => {
            eprintln!("Failed to read DoS file '{}': {}", filename, e);
            Ok(Vec::new()) // Return an empty vector to allow continuation
        }
    }
}

pub async fn add_dos_entry<C>(
    hub: &DriveHub<C>,
    client_ip: &str,
    client_id: &str,
    resources: &str,
    drive_id: &str,
    tsv_filename: &str,
) -> Result<(), Box<dyn std::error::Error>>
where
    C: hyper_util::client::legacy::connect::Connect + Clone + Send + Sync + 'static,
{
    // Validate IP address
    IpAddr::from_str(client_ip)?;

    let mut entries = read_dos_file(hub, tsv_filename).await?;

    // Check for duplicate entries based on Client_ID or Client_IP
    if entries.iter().any(|entry| entry.Client_ID == client_id || entry.Client_IP == client_ip) {
        eprintln!(
            "Duplicate entry detected. Client_ID: '{}' or Client_IP: '{}' already exists.",
            client_id, client_ip
        );
        return Ok(()); // Log error, but don't terminate
    }

    let new_entry = DosEntry::new(client_id.to_string(), client_ip.to_string(), resources.to_string());
    entries.push(new_entry);

    // Sort entries to maintain consistency
    entries.sort_by(|a, b| a.Client_ID.cmp(&b.Client_ID));

    write_dos_file(hub, tsv_filename, &entries, drive_id).await?;
    println!(
        "Successfully added new entry. Client_ID: '{}', Client_IP: '{}', Resources: '{}'.",
        client_id, client_ip, resources
    );

    Ok(())
}

pub async fn delete_dos_entry<C>(
    hub: &DriveHub<C>,
    client_ip: &str,
    drive_id: &str,
    tsv_filename: &str,
) -> Result<(), Box<dyn std::error::Error>>
where
    C: hyper_util::client::legacy::connect::Connect + Clone + Send + Sync + 'static,
{
    // Validate IP address
    IpAddr::from_str(client_ip)?;

    let mut entries = read_dos_file(hub, tsv_filename).await?;

    let initial_len = entries.len();
    entries.retain(|entry| entry.Client_IP != client_ip);
    let final_len = entries.len();

    if initial_len == final_len {
        eprintln!("No entry found with Client_IP: {}. Nothing was deleted.", client_ip);
    } else {
        println!("Entry with Client_IP: {} has been successfully deleted.", client_ip);
    }

    write_dos_file(hub, tsv_filename, &entries, drive_id).await?;

    Ok(())
}

pub fn serialize_entries(entries: &[DosEntry]) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut buffer = Vec::new();
    {
        let mut writer = WriterBuilder::new()
            .delimiter(b'\t')
            .has_headers(true)
            .from_writer(&mut buffer);

        for entry in entries {
            writer.serialize(entry)?;
        }
        
        writer.flush()?;
    }
    Ok(buffer)
}

pub async fn write_dos_file<C>(
    hub: &DriveHub<C>,
    filename: &str,
    entries: &[DosEntry],
    target_name: &str
) -> Result<(), Box<dyn Error>>
where
    C: hyper_util::client::legacy::connect::Connect + Clone + Send + Sync + 'static,
{
    // Find parent folder or drive
    let parent_id = find_target_id(hub, target_name).await?;

    // Check if file already exists
    let existing_file = hub.files().list()
        .add_scope("https://www.googleapis.com/auth/drive")
        .supports_all_drives(true)
        .include_items_from_all_drives(true)
        .q(&format!("name = '{}' and trashed = false and '{}' in parents", filename, parent_id))
        .doit()
        .await?;

    // Serialize entries to TSV with proper formatting
    let buffer = serialize_entries(entries)?;

    if let Some(files) = existing_file.1.files {
        if let Some(file) = files.first() {
            if let Some(file_id) = &file.id {
                // Update existing file instead of creating a new one
                hub.files()
                    .update(DriveFile::default(), file_id)
                    .add_scope("https://www.googleapis.com/auth/drive")
                    .supports_all_drives(true)
                    .upload(Cursor::new(buffer), "text/tab-separated-values".parse()?)
                    .await?;

                println!("File '{}' updated successfully.", filename);
                return Ok(());
            }
        }
    }

    // If no existing file, create a new one
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

    println!("File '{}' created successfully.", filename);
    Ok(())
}
pub fn clean_entries(entries: &mut Vec<DosEntry>) {
    entries.dedup_by(|a, b| a.Client_ID == b.Client_ID);  // Remove duplicates
    entries.retain(|entry| {  // Remove invalid entries
        !entry.Client_ID.trim().is_empty() && 
        !entry.Client_IP.trim().is_empty() &&
        entry.last_updated > 0
    });
}

pub async fn get_all_entries<C>(
    hub: &DriveHub<C>,
    tsv_filename: &str,
) -> Result<Vec<DosEntry>, Box<dyn Error>>
where
    C: hyper_util::client::legacy::connect::Connect + Clone + Send + Sync + 'static,
{
    // Call read_dos_file to retrieve entries
    let entries = read_dos_file(hub, tsv_filename).await?;

    if entries.is_empty() {
        println!("No entries found in the file: {}", tsv_filename);
    } else {
        println!(
            "Retrieved {} entries from the file: {}",
            entries.len(),
            tsv_filename
        );
    }

    Ok(entries)
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
