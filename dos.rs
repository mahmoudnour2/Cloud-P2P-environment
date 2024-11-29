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

pub fn get_dos_entry_by_ip(client_ip: &str) -> Option<DosEntry> {
    let ip_entries = [
        (
            "192.168.1.100",
            DosEntry::new(
                "client1".to_string(), 
                "192.168.1.100".to_string(), 
                "Beijing_China_Tiananmen-Square-05.jpg,Beijing_China_Tiananmen-Square-Young_Pioneers_of_China-01.jpg,Benoa_Indonesia_Workers-of-a-wharft-01.jpg,Bratislava_Slovakia_National-Museum-01.jpg,Bensberg_Germany_Schloss-Bensberg-Panorama-01.jpg,Angkor_SiemReap_Cambodia_Ankor-Thom-Statue-01.jpg,Beijing_China_Hall-of-Prayer-for-Good-Harvests-01.jpg,Assortment-of-Chinese-Dishes-01.jpg,Bremen_Germany_Statue-of-Roland-01.jpg,Beaufort_Sabah_Houses-at-Jalan-Foo-Patt-Yin-01.jpg,Beaufort_Sabah_Tabung-Haji-01.jpg,Babagon_Sabah_Babagon-Dam-06.jpg,Babagon_Sabah_Babagon-Dam-07a.jpg,Bonn_Germany_Building-Landwirtschaftskammer-Rheinland-01.jpg,Babagon_Sabah_Babagon-Dam-07.jpg,Angkor_SiemReap_Cambodia_Pre_Rup-02.jpg,Angkor_SiemReap_Cambodia_Pre_Rup-01.jpg,Bingyu-Valley_Liaoning_China_Rock-formation-04.jpg,Bingyu-Valley_Liaoning_China_Rock-formation-03.jpg,Bingyu-Valley_Liaoning_China_Rock-formation-02.jpg,Bologna_Italy_Monument-to-the-victims-of_fascistic-terror-01.jpg,Beipu_Taiwan_Cihtian-Temple-01.jpg,Babagon_Sabah_Babagon-Dam-05.jpg,Babagon_Sabah_Babagon-Dam-08.jpg,Babagon_Sabah_Babagon-Dam-10.jpg,Babagon_Sabah_Babagon-Dam-04.jpg,Babagon_Sabah_Babagon-Dam-03.jpg,Bandar-Sahabat_Sabah_Masjid-Bandar-Sahabat-02.jpg,Bandar-Sahabat_Sabah_Masjid-Bandar-Sahabat-01.jpg,Bandar-Sahabat_Sabah_SK-Sahabat-16-02.jpg,Bingkongan_Sabah_SMK-Bengkongan-01.jpg,Badaling_China_Great-Wall-of-China-01.jpg,Babagon_Sabah_Babagon-Dam-02.jpg,Babagon_Sabah_Babagon-Dam-01.jpg,Beaufort_Sabah_Former-Railway-Triangle-01.jpg,Beinan_Taitung_Taiwan_Road-kill-01.jpg,Beaufort_sabah_Masjid-Daerah-Beaufort-01.jpg,Beaufort_Sabah_Former-Railway-Triangle-02a.jpg,Beaufort_Sabah_Former-Railway-Triangle-05.jpg,Beaufort_Sabah_Former-Railway-Triangle-02.jpg,Beaufort_Sabah_Former-Railway-Jetty-01.jpg,BatuPutih_Sabah_AgopBatuTulug-02.jpg,Bruges_Belgium_Concert-Hall-01.jpg,Bruges_Belgium_Kasteel-de-la-Faille-03.jpg,Bruges_Belgium_Kasteel-de-la-Faille-02.jpg,Alishan_Taiwan_Alishan-Forest-Railway-03c.jpg,Alishan_Taiwan_Alishan-Forest-Railway-03a.jpg,Alishan_Taiwan_Xiang-Lin-Elementary-School-02.jpg,Alishan_Taiwan_Xiang-Lin-Elementary-School-01.jpg,Bin_Lang_Taiwan_ISUZU-fire-appliance-01.jpg,Alishan_Taiwan_Alishan-Forest-Railway-03b.jpg,Alishan_Taiwan_Alishan-Forest-Park_Yarder-03.jpg,Alishan_Taiwan_Alishan-Police-Station-01.jpg,Alishan_Taiwan_Alishan-Forest-Railway-04.jpg,Alishan_Taiwan_Alishan-Forest-Park_Yarder-02.jpg,Alishan_Taiwan_Alishan-Forest-Park_Yarder-01.jpg,Alishan_Taiwan_Alishan-Forest-Railway-03.jpg,Alishan_Taiwan_Alishan-Forest-Railway-02.jpg,Anping_Taiwan_Old-houses-of-Anping-01.jpg,Anping_Taiwan_Eternal-Golden-Castle-03.jpg,Anping_Taiwan_Eternal-Golden-Castle-02.jpg,Anping_Taiwan_Eternal-Golden-Castle-01.jpg,Alishan_Taiwan_Alishan-Forest-Railway-01.jpg,Beinan_Taitung_Taiwan_Aboriginal-Stilt-House-01.jpg,Bali-Strait_Indonesia_KMP-Gilimanuk-01.jpg,Bongkud_Sabah_St-Victors-Catholic-Church-03.jpg,Angkor_SiemReap_Cambodia_Statue-of-a-cow-01.jpg,Angkor_SiemReap_Cambodia_Ankor-Wat-01.jpg,Angkor_SiemReap_Cambodia_Ankor-Wat-Relief-01.jpg,Bali-Strait_Indonesia_KMP-Trisila-Bhakti-02.jpg,Bratan_Bali_Pura-Ulun-Danu-Bratan-03.jpg,Bratan_Bali_Indonesia_Balinese-family-after-Puja-01.jpg,Bratan_Bali_Indonesia_Grandfather-and-grandson-after-Puja-01.jpg,Benoa_Bali_Indonesia-Bakso-street-vendor-02.jpg,Benoa_Bali_Indonesia-Bakso-street-vendor-01.jpg,Benoa_Bali_Indonesia-Petamina-tank-truck-01.jpg,Bali-Strait_Indonesia_KMP-Reny-II-03.jpg,Benoa_Bali_Indonesia-Tugboat-Garuda-I-02.jpg,Benoa_Bali_Indonesia_Ships-in-Benoa-Harbour-03.jpg,Beaufort_Sabah_Multi-Purpose-Hall-02.jpg,Benoa_Bali_Indonesia-Timbers-of-a-jetty-01.jpg,Beaufort_Sabah_LongShophouses-01.jpg,Beaufort_Sabah_WismaArjuna-01.jpg,Beaufort_Sabah_LongShophouses-04.jpg,Antwerp_Belgium_Loodswezengebouw-01.jpg,Antwerp_Belgium_Lange-Wapper-01.jpg,Antwerp_Belgium_Saint-Pauls-Church-01.jpg,Antwerp_Belgium_Saint-Pauls-Church-02.jpg,Antwerp_Belgium_Het-Steen-01.jpg,Antwerp_Belgium_Museum-Plantin-Moretus-02.jpg,Besakih_Bali_Indonesia_Pura-Besakih-04a.jpg,Besakih_Bali_Indonesia_Pura-Besakih-05.jpg,17-jewel-lady-watch-inside-view-02.jpg,17-jewel-lady-watch-inside-view-03.jpg,Antwerp_Belgium_Museum-Plantin-Moretus-05.jpg,Antwerp_Belgium_Museum-Plantin-Moretus-04.jpg,Antwerp_Belgium_Museum-Plantin-Moretus-03.jpg,Antwerp_Belgium_Museum-Plantin-Moretus-01.jpg,Antwerp_Belgium_Tower-of-Onze-Lieve-Vrouwekathedraal-01.jpg,Antwerp_Belgium_Brabofountain-01.jpg,17-jewel-lady-watch-inside-view-01.jpg,Bangly-Regency_Bali_Indonesia_Lake-Batur-01.jpg,Bratan_Bali_Indonesia_Road-roller-01.jpg,Benoa_Bali_Indonesia-Mangrove-forest-03.jpg,Benoa_Bali_Indonesia-Mangrove-forest-01.jpg,Besakih_Bali_Indonesia_Pura-Besakih-03.jpg,Benoa_Bali_Indonesia_Ships-in-Benoa-Harbour-02.jpg,Benoa_Bali_Indonesia_Ships-in-Benoa-Harbour-01.jpg,Benoa_Bali_Indonesia-Diveboat-PT-Blue-Dragon-Indonesia-01.jpg,Benoa_Bali_Indonesia-Tugboat-Garuda-I-01.jpg,Benoa_Bali_Indonesia_Unloading-Tuna-in-Benoa-Harbour-04.jpg,Benoa_Bali_Indonesia_Unloading-Tuna-in-Benoa-Harbour-03.jpg,Bali-Strait_Indonesia_LCT-Jambo-VI-01.jpg,Banyuwangi_Indonesia_Onde-onde-01.jpg,Bali-Strait_Indonesia_KMP-Trisila-Bhakti-01.jpg,Bali-Strait_Indonesia_KMP-Sereia-do-mar-02.jpg,Bali-Strait_Indonesia_KMP-Reny-II-02.jpg,Bali-Strait_Indonesia_KMP-Niaga-Ferry-II-01.jpg,Bali-Strait_Indonesia_KMP-Reny-II-01.jpg,Bali-Strait_Indonesia_KMP-Trima-Jaya-9-01.jpg,Bali-Strait_Indonesia_KMP-Dharma-Rucitra-01.jpg,Borobudur-Temple-Park_Indonesia_Reliefs-of-Borobudur-01.jpg,Borobudur-Temple-Park_View_of_the_temple-01.jpg,Bromo-Tengger-Semeru-National-Park_Indonesia_Stairway-to-Bromo-crater-05.jpg,Bromo-Tengger-Semeru-National-Park_Indonesia_Mount-Batok-03.jpg,Borobudur-Temple-Park_Indonesia_Reliefs-of-Borobudur-02.jpg,Borobudur-Temple-Park_Mahouts-of-the-elephant-cage-02.jpg,Borobudur-Temple-Park_Indonesia_Chariot-01.jpg,Angkor_SiemReap_Cambodia_Ankor-Wat-Statue-01.jpg,Borobudur-Temple-Park_Elephant-cage-01.jpg,Borobudur-Temple-Park_Gardener-at-Dagi-Hill-01.jpg,Borobudur-Temple-Park_Indonesia_Statues-of-Borobudu-01.jpg,Benoa_Bali_Indonesia_Meratus-Padang-01.jpg,Beaufort_Sabah_Railbus-2102-01.jpg,Beaufort_Sabah_Railway-Station-01.jpg,Beaufort_Sabah_Town-Mosque-01.jpg,Bilit_Sabah_Ramp-for-loading-palmoil-fruits-02.jpg,Bilit_Sabah_Ramp-for-loading-palmoil-fruits-01.jpg,Borobudur-Temple-Park_Indonesia_Stupas-of-Borobudur-10.jpg,Bromo-Tengger-Semeru-National-Park_Indonesia_Mount-Batok-02a.jpg,Borobudur-Temple-Park_Indonesia_Stupas-of-Borobudur-14.jpg,Borobudur-Temple-Park_Indonesia_Stupas-of-Borobudur-12.jpg,Borobudur-Temple-Park_Indonesia_Stupas-of-Borobudur-11.jpg,Borobudur-Temple-Park_Indonesia_Stupas-of-Borobudur-09.jpg,Borobudur-Temple-Park_Indonesia_Stupas-of-Borobudur-08.jpg,Borobudur-Temple-Park_Indonesia_Stupas-of-Borobudur-07.jpg,Borobudur-Temple-Park_Indonesia_Stupas-of-Borobudur-06.jpg,Banyuwangi_Indonesia_Ferry-Harbour-01.jpg,Badung-Regency_Bali_Indonesia_Acoustic-Scaregrow-in-a-paddy-01.jpg,Besakih_Bali_Indonesia_Pura-Besakih-02.jpg,Besakih_Bali_Indonesia_Roasters-in-Baskets-02.jpg,Besakih_Bali_Indonesia_Roasters-in-Baskets-01.jpg,Balaclava_Mauritius_A-band-of-sparrows-sitting-on-a-prickly-pear-01.jpg,AEG-MIGNON-typewriter-02.jpg,Agfa-Optima-1A-02.jpg,BASF-magnetic-audio-tape-02.jpg,BASF-magnetic-audio-tape-01.jpg,Annealing_equipment-for-ambulant-heat-tratment-01.jpg,AEG-MIGNON-typewriter-01.jpg,Bologna_Italy_San-Giacomo-Maggiore-01.jpg,Benoa_Bali_Indonesia_Lady-Christine-01.jpg,Benoa_Bali_Indonesia_Azamara-Quest-02.jpg,Benoa_Bali_Indonesia_Azamara-Quest-01.jpg,Borobudur-Temple-Park_Indonesia_Lion-guardians-of-Borobudur-01.jpg,Bromo-Tengger-Semeru-National-Park_Indonesia_Mount-Batok-02.jpg,Borobudur-Temple-Park_Indonesia_Stupas-of-Borobudur-05.jpg,Borobudur-Temple-Park_Indonesia_Stupas-of-Borobudur-04.jpg,Borobudur-Temple-Park_Indonesia_Stupas-of-Borobudur-03.jpg,Bromo-Tengger-Semeru-National-Park_Indonesia_Bromo-Crater-02.jpg,Bromo-Tengger-Semeru-National-Park_Indonesia_Stairway-to-Bromo-crater-04.jpg,Bromo-Tengger-Semeru-National-Park_Indonesia_Bromo-Crater-01.jpg,Bromo-Tengger-Semeru-National-Park_Indonesia_Fog-in-the-Bromo-Caldera-01.jpg,Bromo-Tengger-Semeru-National-Park_Indonesia_Horses-02.jpg,Bromo-Tengger-Semeru-National-Park_Indonesia_Stairway-to-Bromo-crater-02.jpg,Bromo-Tengger-Semeru-National-Park_Indonesia_Panoramic-view-of-the-caldera-01.jpg,Bromo-Tengger-Semeru-National-Park_Indonesia_Pura-Tengger-02.jpg,Bromo-Tengger-Semeru-National-Park_Indonesia_Mount-Batok-01.jpg,Bromo-Tengger-Semeru-National-Park_Indonesia_Toyota-Land-Cruiser-in-Caldera-01.jpg,Agfa-Optima-1A-01.jpg,3Com-Etherlink-Network-Interface-Card-02.jpg,Baron_Nganjuk_Java_Indonesia_Baron-Railway-Station-03.jpg,Baron_Nganjuk_Java_Indonesia_Baron-Railway-Station-04.jpg,Baron_Nganjuk_Java_Indonesia_Baron-Railway-Station-02.jpg,Baron_Nganjuk_Java_Indonesia_Baron-Railway-Station-01.jpg,Brantan_Bali_Pura-Ulun-Danu-Bratan-02.jpg,Brantan_Bali_Pura-Ulun-Danu-Bratan-01.jpg,3Com-Etherlink-Network-Interface-Card-05.jpg,3Com-Etherlink-Network-Interface-Card-01.jpg,Besakih_Bali_Indonesia_Pura-Besakih-01.jpg,Barangay-Bulacao_Cebu-City_Philippines_Cockfighting-event-10.jpg,Barangay-Bulacao_Cebu-City_Philippines_Cockfighting-event-03.jpg,Bilit_Sabah_Residential-Dwellings-of-palmoil-estate-workers-01.jpg,Borobudur-Temple-Park_Indonesia_Stupas-of-Borobudur-01.jpg,Benoa_Bali_Indonesia_Unloading-Tuna-in-Benoa-Harbour-02.jpg,Benoa_Bali_Indonesia_Unloading-Tuna-in-Benoa-Harbour-01.jpg,Benoa_Bali_Indonesia_Fish-Trawlers-in-Benoa-Harbour-01.jpg,Bamenohl_Germany_Christine-Koch-Grundschule-01.jpg,Bingkor_Sabah_RumahBesarSedomon-07.jpg,Bingkor_Sabah_RumahBesarSedomon-05.jpg,Bingkor_Sabah_RumahBesarSedomon-01.jpg,BatuPutih_Sabah_AgopBatuTulug-15.jpg,Blaichach_Germany_St-Georg-und-Mauritius-01.jpg,Bawang-Jamal_Sabah_PCS-Bawang-Jamal-01.jpg,Beaufort_Sabah_SAN-Kg-Bingkul-02.jpg,Beaufort_Sabah_SMK-Beaufort-02.jpg,Beaufort_Sabah_Perpustakaan-Negeri-Sabah-Beaufort-01.jpg,Beaufort_Sabah_SMK-Beaufort-II-01.jpg,Beaufort_Sabah_Bridge-over-Sungai-Padas-01.jpg,Amiens_France_Monument-for-Dufresne-du-Cange-01.jpg,Amiens_France_Place-Gambetta-01.jpg,Amiens_France_Rue-du-Don-02.jpg,Bawang-Jamal_Sabah_Rungus-boy-01.jpg,Batangon-Darat_Sabah_Rungus-Woman-01.jpg,Binsulung_Sabah_SK-Binsulung-01.jpg,Binsulung_Sabah_Protestant-Church-Binsulung-02.jpg,Binsulung_Sabah_Multipurpose-Hall-01.jpg,Binsulung_Sabah_Residential-building-01.jpg,Binsulung_Sabah_Protestant-Church-Binsulung-01.jpg,Benkoka_Sabah_Jetty-at-Dataran-Benkoka-Pitas-01.jpg,Benkoka_Sabah_Dataran-Benkoka-Pitas-01.jpg,Amiens_France_Restaurant-Tante-Jeanne-02.jpg,Amiens_France_Quai-Belu-03.jpg,Balakong_Selangor_Malaysia_SapuraKencana-Petroleum-Bhd-Headquarters-01.jpg,Amiens_France_Buildings-in-Rue-Motte-02c.jpg,Amiens_France_Bridges-in-Rue-du-Don-01.jpg,Amiens_France_Buildings-1-and-3-Rue-des-Granges-01.jpg,Amiens_France_Buildings-in-Rue-des-Granges-02.jpg,Amiens_France_Buildings-in-Rue-des-Granges-01.jpg,Amiens_France_Buildings-in-Rue-Motte-02a.jpg,Amiens_France_Quai-Belu-02.jpg,Amiens_France_Horloge-Dewailly-d-Amiens-02a.jpg,Amiens_France_Hotel-de-Ville-03d.jpg,Amiens_France_Hotel-de-Ville-03a.jpg,Amiens_France_Hotel-de-Ville-03c.jpg,Amiens_France_Hotel-de-Ville-03b.jpg,Amiens_France_Buildings-1-11-Place-Notre-Dame-01.jpg,Amiens_France_Hotel-de-Ville-02.jpg,Amiens_France_Belfry-of-Amiens-04.jpg,Amiens_France_Belfry-of-Amiens-03.jpg,Amiens_France_Belfry-of-Amiens-01.jpg,Amiens_France_Firefighters-headquarters-02.jpg,Amiens_France_Jules-et-Jim-01.jpg,Amiens_France_Librairie-du-Labyrinthe-01.jpg,Amiens_France_Hotel-de-Ville-01.jpg,Amiens_France_Bishop-Olivier-Claude-Philippe-Marie-Leborgne-04.jpg,Amiens_France_Bishop-Olivier-Claude-Philippe-Marie-Leborgne-01.jpg,Amiens_France_Bishop-Olivier-Claude-Philippe-Marie-Leborgne-02.jpg,Amiens_France_Building-1-Place-Notre-Dame-04.jpg,Amiens_France_Maison-de-la-Culture-01.jpg,Amiens_France_Buildings-in-Rue-d-Engoulvent-02.jpg,Amiens_France_Firefighters-headquarters-01.jpg,Amiens_France_Horloge-Dewailly-d-Amiens-02.jpg,Amiens_France_Buildings-in-Rue-Motte-01a.jpg,Amiens_France_Quai-Belu-01.jpg,Amiens_France_Buildings-in-Rue-Motte-02.jpg,Amiens_France_Buildings-in-Rue-Motte-01.jpg,Amiens_France_Buildings-in-Rue-d-Engoulvent-01.jpg,Amiens_France_Restaurant-Tante-Jeanne-01.jpg,Amiens_France_Sport-Nautique-d-Amiens-01.jpg,Amiens_France_Building-11-Place-Notre-Dame-01.jpg,Aachen_Germany_Imperial-Cathedral-01.jpg,Aachen_Germany_Domschatz_Sayn-Altar-01.jpg,Aachen_Germany_Imperial-Cathedral-21.jpg,Aachen_Germany_Imperial-Cathedral-19a.jpg,Aachen_Germany_Imperial-Cathedral-19.jpg,Aachen_Germany_Domschatz_Reliquary-for-the-belt-of-Christ-01.jpg,Beaufort_Sabah_Multi-Purpose-Hall-01.jpg,Aachen_Germany_Imperial-Cathedral-15.jpg,Aachen_Germany_Domschatzkammer-Entrance-01.jpg,Aachen_Germany_Imperial-Cathedral-13.jpg,Aachen_Germany_Imperial-Cathedral-08b.jpg,Aachen_Germany_Imperial-Cathedral-08a.jpg,Aachen_Germany_Imperial-Cathedral-10.jpg,Aachen_Germany_Imperial-Cathedral-09.jpg,Aachen_Germany_Imperial-Cathedral-08.jpg,Bruges_Belgium_Horse-fountain-01.jpg,Bruges_Belgium_Carillon-01.jpg,Bruges_Belgium_Horse-fountain-02.jpg,Bruges_Belgium_Belfry-02.jpg,Bruges_Belgium_Belfry-01.jpg,Bruges_Belgium_Sint-Salvatorskathedraal-12.jpg,Bruges_Belgium_Sint-Salvatorskathedraal-13.jpg,Bruges_Belgium_Death-of-the-Virgin_Museum-of-Sint-Salvatorskathedraal-01.jpg,Bruges_Belgium_Sint-Janshospitaal-03.jpg,Bruges_Belgium_Panoramic-view-from-Belfry-01.jpg,Bruges_Belgium_Junction-Sint-Salvadorskerkhof-with-Sint-Salvatorskoorstraat-01.jpg,Bruges_Belgium_Gate-in-Heilige-Geeststraat-01.jpg,Bruges_Belgium_Statue-of-Jan-Breydel-and-Pieter-de-Coninck-on-the-Grote-Markt-02.jpg,Bruges_Belgium_Arsenaalstraat-35-01.jpg,Bruges_Belgium_Godshuis-De-Vos-01.jpg,Bruges_Belgium_De-groote-Hollander-Huidenvettersplein-12-03.jpg,Bruges_Belgium_De-groote-Hollander-Huidenvettersplein-12-02.jpg,Bruges_Belgium_Katelijnestraat-65A-02.jpg,Bruges_Belgium_Katelijnestraat-65A-01.jpg,Bruges_Belgium_Windmill-Bonne-Chiere-01.jpg,Bruges_Belgium_Sint-Salvatorskathedraal-06a.jpg,Bruges_Belgium_Belgian-Nougat-01.jpg,Aachen_Germany_Imperial-Cathedral-06.jpg,Aachen_Germany_Imperial-Cathedral-04.jpg,Aachen_Germany_Saint-Vincent-Fountain-02.jpg,Aachen_Germany_Imperial-Cathedral-05.jpg,Aachen_Germany_Imperial-Cathedral-02.jpg,Aachen_Germany_Imperial-Cathedral-03.jpg,Aachen_Germany_Lamp-post-Hans-von-Reutlingen-Gasse-01.jpg,Aachen_Germany_Saint-Vincent-Fountain-01.jpg,Bruges_Belgium_Historic-Residential-Building-03.jpg,Bruges_Belgium_Bakkersrei-Canal-01.jpg,Bruges_Belgium_Beguinage-02.jpg,Bruges_Belgium_Beguinage-01.jpg,Bruges_Belgium_Statue-of-John-of-Nepomuk-in-Bruges-01.jpg,Bruges_Belgium_Gruuthuse-Hof-02.jpg,Bruges_Belgium_Gentpoortvest-02.jpg,Bruges_Belgium_Groenerei-Canal-with-Peerdenbrug-01.jpg,Bruges_Belgium_Cargoship-Natasha-N-in-Dampoort-Lock-01.jpg,Bruges_Belgium_Cargoship-Natasha-N-entering-Dampoort-Lock-01.jpg,Bruges_Belgium_Windmill-_De-nieuwe-Papegaai-01.jpg,Bruges_Belgium_Ezelpoort-01.jpg,Bruges_Belgium_Smedenpoort-01.jpg,Bruges_Belgium_Gentpoortvest-Watertoren-01.jpg,Bruges_Belgium_Gentpoortvest-01.jpg,Bruges_Belgium_Houses-at-Walplein-01.jpg,Bruges_Belgium_De-Halve-Maan-brewery-01.jpg,Bruges_Belgium_Town-hall-of-Brugge-01.jpg,Bruges_Belgium_Statue-of-Jan-Breydel-and-Pieter-de-Coninck-on-the-Grote-Markt-01.jpg,Bruges_Belgium_Sint-Salvatorskathedraal-07.jpg,Bruges_Belgium_Sint-Salvatorskathedraal-05.jpg,Bruges_Belgium_Sint-Salvatorskathedraal-08.jpg,Bruges_Belgium_Onze-Lieve-Vrouwekerk-02.jpg,Bruges_Belgium_Gruuthuse-Hof-01.jpg,Bruges_Belgium_Onze-Lieve-Vrouwekerk-01.jpg,Bruges_Belgium_Gruuthuse-Museum-01.jpg,Bruges_Belgium_Sint-Salvatorskathedraal-02.jpg,Bruges_Belgium_Steenstraat-and-Belfry-01.jpg,Bruges_Belgium_Sint-Salvatorskathedraal-09.jpg,Bruges_Belgium_Sint-Salvatorskathedraal-11.jpg,Bruges_Belgium_Sint-Salvatorskathedraal-10.jpg,Balung_Tawau_Sabah_Sawit-Kinabalu-Seeds-Sdn-Bhd-02.jpg,Brugges_Belgium_Bakkersrei-Canal-from-Bonifaciusbridge-01.jpg,Acid-corrosion-in-a-flue-gas-duct-02.jpg,Acid-corrosion-in-a-flue-gas-duct-01.jpg,Bruges_Belgium_Ferrari-with-special-car-plate-XXXXXX-01.jpg,Bruges_Belgium_Windmill-Zandwegemolen-01.jpg,Bruges_Belgium_Stone-Chairs-at-Courtyard-of-Gruuthuse-Museuml-01.jpg,Bruges_Belgium_De-groote-Hollander-Huidenvettersplein-12-01.jpg,Bruges_Belgium_Windmill-_Sint-Janshuismolen-01.jpg,Bruges_Belgium_Windmill-Koeleweimolen-01.jpg,Bruges_Belgium_Sint-Janshospitaal-02.jpg,Bruges_Belgium_Historic-Residential-Building-01.jpg,Bruges_Belgium_Historic-Residential-Building-02.jpg,Bruges_Belgium_Sint-Janshospitaal-01.jpg,Balung_Tawau_Sabah_Apas-Balung-Mill-05a.jpg,Balung_Tawau_Sabah_Balung-Estate-01.jpg,Balung_Tawau_Sabah_Sawit-Kinabalu-Seeds-Sdn-Bhd-01.jpg,Balung_Tawau_Sabah_Apas-Balung-Mill-07.jpg,Balung_Tawau_Sabah_Apas-Balung-Mill-06.jpg,Balung_Tawau_Sabah_Apas-Balung-Mill-04.jpg,Balung_Tawau_Sabah_Apas-Balung-Mill-03.jpg,Balung_Tawau_Sabah_Apas-Balung-Mill-01.jpg,Bilit_Sabah_Residential-houses-02.jpg,Bilit_Sabah_Residential-houses-01.jpg,Bilit_Sabah_Residential-houses-03.jpg,Bilit_Sabah_Residential-houses-04.jpg,Bilit_Sabah_Sungai-Kinabatangan-at-Kg-Bilit-01.jpg,Bilit_Sabah_The-last-Frontier-Resort-01.jpg".to_string()
            )
        ),
        (
            "10.0.0.50",
            DosEntry::new(
                "client2".to_string(), 
                "10.0.0.50".to_string(), 
                "Bilit_Sabah_Community-meeting-place-01.jpg,Bilit_Sabah_SK-Bilit-03.jpg,Bilit_Sabah_Kedai-Runcit-01.jpg,Bilit_Sabah_Dewan-Serbaguna-02.jpg,Bilit_Sabah_Dewan-Serbaguna-01.jpg,Bilit_Sabah_Nurul-Hikmah-Mosque-02.jpg,Bilit_Sabah_Nurul-Hikmah-Mosque-01.jpg,Beijing_China_Summer-Palace-02.jpg,Beijing_China_Summer-Palace-01.jpg,Beijing_China_Woman-cleaning-West-Lake-01.jpg,Beijing_China_Beijing-National-Stadium-03.jpg,Beijing_China_Tiananmen-Square-02.jpg,Bensberg_Germany_-Building-Am-Stockbrunnen-03.jpg,Bensberg_Germany_-Building-Corner-_Steinstrasse-Am-Stockbrunnen-02.jpg,Bensberg_Germany_-Building-Corner-_Steinstrasse-Am-Stockbrunnen-01.jpg,Bamenohl_Germany_Catholic-Church-St-Joseph-02.jpg,Bamenohl_Germany_Catholic-Church-St-Joseph-01.jpg,Beijing_China_Forbidden-City-07.jpg,Beijing_China_Forbidden-City-06.jpg,Beijing_China_Forbidden-City-05.jpg,Beijing_China_Forbidden-City-04.jpg,Beijing_China_Forbidden-City-03.jpg,Beijing_China_Forbidden-City-01.jpg,Beijing_China_Tiananmen-Square-03.jpg,Beijing_China_Tiananmen-Square-01.jpg,Badaling_China_Great-Wall-of-China-07.jpg,Badaling_China_Great-Wall-of-China-06.jpg,Badaling_China_Great-Wall-of-China-04.jpg,Badaling_China_Great-Wall-of-China-02.jpg,Bensberg_Germany_Rathaus-und-altes-Schloss-01.jpg,Beaufort_Sabah_SJK-C-Kung-Ming-11.jpg,Beaufort_Sabah_SJK-C-Kung-Ming-10.jpg,Beaufort_Sabah_SJK-C-Kung-Ming-09.jpg,Beaufort_Sabah_SJK-C-Kung-Ming-08.jpg,Beaufort_Sabah_SJK-C-Kung-Ming-07.jpg,Angkor_SiemReap_Cambodia_Banteay_Srei-Relief-01.jpg,Batalha_Portugal_Mosteiro_da_Batalha-01.jpg,Aomen_China_Mailbox-01.jpg,Angkor_SiemReap_Cambodia_Tha-Prom-Temple-01.jpg,Bingyu-Valley_Liaoning_China_Rock-formation-01.jpg,Bingyu-Valley_Liaoning_China_Gate-at-the-barrier-lake-01.jpg,Angkor_SiemReap_Cambodia_Ankor-Thom-Statue-03.jpg,Angkor_SiemReap_Cambodia_Ankor-Thom-Statue-02.jpg,Angkor_SiemReap_Cambodia_Suor-Prat-Towers-02.jpg,Angkor_SiemReap_Cambodia_Suor-Prat-Towers-01.jpg,Angkor_SiemReap_Cambodia_Horse-at-Angkor_Wat-01.jpg,Angkor_SiemReap_Cambodia_Ankor-Wat-Statue-02.jpg,Banaue_Philippines_Batad-Rice-Terraces-03.jpg,Banaue_Philippines_Local-Taxi-01.jpg,Banaue_Philippines_Banaue-Rice-Terraces-01.jpg,Banaue_Philippines_View-of-the-Town-02.jpg,Angkor_SiemReap_Cambodia_World-Heritage-Stele-01.jpg,Beijing_China_Beijing-National-Stadium-02.jpg,Beijing_China_Beijing-National-Stadium-01.jpg,Bingyu-Valley_Liaoning_China_Traditional-Rafts-01.jpg,Banaue_Philippines_Batad-Rice-Terraces-04.jpg,Banaue_Philippines_Handmade-brooms-01.jpg,Banaue_Philippines_Ifugao-Tribesman-01.jpg,Banaue_Philippines_Batad-Rice-Terraces-02.jpg,Ambong_Sabah_Footbridge-03.jpg,Ambong_Sabah_Footbridge-01.jpg,Barrierefreiheit_004_2024_09_13.jpg,Barrierefreiheit_003_2024_09_13.jpg,Barrierefreiheit_001_2015_06_09.jpg,Baustelle_004_2021_08_04.jpg,Baustelle_003_2015_08_31.jpg,009_2015_08_27_Kulturdenkmaeler_Hassloch.jpg,012_2015_07_09_Kulturdenkmaeler_Hassloch.jpg,011_2015_07_09_Kulturdenkmaeler_Hassloch.jpg,Barum_Gotenweg_003_2023_04_02.jpg,Barum_Gotenweg_5_003_2023_04_02.jpg,Barum_Gotenweg_008_2023_04_02.jpg,Barum_Gotenweg_006_2023_04_02.jpg,Barum_Gotenweg_005_2023_04_02.jpg,Barum_Gotenweg_002_2023_04_02.jpg,Barum_Barbarossaweg_2_003_2023_04_02.jpg,Barum_Barbarossaweg_2_002_2023_04_02.jpg,Barum_Gotenweg_1_002_2023_04_02.jpg,Barum_Gotenweg_5_004_2023_04_02.jpg,Barum_Gotenweg_007_2023_04_02.jpg,Barum_Gotenweg_004_2023_04_02.jpg,Barum_Barbarossaweg_2_004_2023_04_02.jpg,Barum_Barbarossaweg_2_005_2023_04_02.jpg,Barum_Barbarossaweg_2_001_2023_04_02.jpg,Baby-Bauernhoftiere_001_2014_08_05.jpg,Baby-Bauernhoftiere_004_2017_06_27.jpg,Baby-Bauernhoftiere_002_2014_03_16.jpg,Abstrakte_Fotografie_003_2021_11_28.jpg,Abstrakte_Fotografie_002_2015_03_14.jpg,Abstrakte_Fotografie_001_2012_10_16.jpg,Bardowick_St._Nikolaihof_19K_001_2022_05_31.jpg,Bardowick_St._Nikolaihof_19G_005_2022_05_31.jpg,Bardowick_St._Nikolaihof_19G_001_2022_05_31.jpg,Bardowick_St._Nikolai_006_2022_05_31.jpg,Bardowick_St._Nikolaihof_002_2022_05_31.jpg,Anstarren_004_2021_04_06.jpg,Anstarren_003_2018_05_15.jpg,Anstarren_002_2017_06_27.jpg,Bardowick_St._Nikolaihof_19I_J_002_2022_05_31.jpg,Bardowick_St._Nikolaihof_19I_J_001_2022_05_31.jpg,Bardowick_St._Nikolaihof_19G_006_2022_05_31.jpg,Bardowick_St._Nikolaihof_19G_004_2022_05_31.jpg,Bardowick_St._Nikolaihof_19G_003_2022_05_31.jpg,Bardowick_St._Nikolaihof_19G_002_2022_05_31.jpg,Bardowick_St._Nikolaihof_19F_004_2022_05_31.jpg,Bardowick_St._Nikolaihof_19F_003_2022_05_31.jpg,Bardowick_St._Nikolaihof_19F_002_2022_05_31.jpg,Bardowick_St._Nikolaihof_19E_002_2022_05_31.jpg,Bardowick_St._Nikolaihof_19E_001_2022_05_31.jpg,Bardowick_St._Nikolai_010_2022_05_31.jpg,Bardowick_St._Nikolai_008_2022_05_31.jpg,Bardowick_St._Nikolai_007_2022_05_31.jpg,Bardowick_St._Nikolai_004_2022_05_31.jpg,Bardowick_St._Nikolai_003_2022_05_31.jpg,Bardowick_St._Nikolai_001_2022_05_31.jpg,001_2022_04_09_Ei.jpg,002_2016_05_31_Regenschirme.jpg,005_2015_08_27_Kulturdenkmaeler_Hassloch.jpg,002_2015_08_27_Kulturdenkmaeler_Hassloch.jpg,001_2012_02_25_Kabel_und_Draehte.jpg,002_2020_02_12_Treppe.jpg,001_2015_06_02_Treppe.jpg,004_2022_04_15_Ei.jpg,002_2022_04_09_Ei.jpg,002_2013_04_13_Kabel_und_Draehte.jpg,2014_11_09_017_Treppe_Flaggenturm.jpg,2014_09_28_Bismarckstein_am_Hartenberg_2.jpg,2014_09_28_Kreuzwegstation_6_am_Weg_zur_Klausenkapelle.jpg,089_2014_09_23_Dreiseithof.jpg,004_2015_04_23_Musikinstrumente.jpg,002_2014_12_17_Musikinstrumente.jpg,004_2018_06_14_Obst.jpg,003_2016_02_08_Brechung.jpg,004_2022_01_02_Brechung.jpg,002_2014_03_18_Brechung.jpg,001_2010_03_21_Brechung.jpg,001_2013_08_29_Dorf.jpg,007_2020_10_10_Pfeifenorgel.jpg,006_2019_09_08_Pfeifenorgel.jpg,005_2014_12_29_Pfeifenorgel.jpg,004_2014_12_29_Pfeifenorgel.jpg,004_2020_10_13_Schweres_Geraet.jpg,003_2020_09_01_Schweres_Geraet.jpg,002_2020_01_23_Schweres_Geraet.jpg,001_2015_09_08_Schweres_Geraet.jpg,041_01_2021_12_06_Kulturdenkmaeler_Forst.jpg,002_2005_08_30_Staedte_in_der_Daemmerung_und_im_Morgengrauen.jpg,004_2020_01_23_Zaeune_und_Grenzmauern.jpg,003_2019_09_27_Zaeune_und_Grenzmauern.jpg,2014_04_19_010_Rheinufer.jpg,004_2021_11_15_Hecken.jpg,002_2016_06_09_Hecken.jpg,001_2015_06_08_Hecken.jpg,002_2015_04_23_Draufsichten_auf_Fahrzeuge.jpg,001_2010_08_13_Draufsichten_auf_Fahrzeuge.jpg,001_2011_10_02_Einsamkeit.jpg,004_2021_09_23_Einsamkeit.jpg,003_2016_05_29_Einsamkeit.jpg,001_2021_03_05_Lichtzeltfotografie.jpg,2021_08_31_006_Hemshof_Friedel.jpg,2021_08_31_005_Hemshof_Friedel.jpg,2021_08_31_004_Hemshof_Friedel.jpg,004_2021_09_17_Lichtzeltfotografie.jpg,002_2021_09_17_Lichtzeltfotografie.jpg,003_2021_09_17_Lichtzeltfotografie.jpg,003_2016_06_05_Wappen.jpg,002_2015_06_02_Wappen.jpg,001_2011_08_28_Wappen.jpg,003_2021_08_04_Spielplatz.jpg,002_2015_10_09_Spielplatz.jpg,001_2009_09_08_Gegenlicht.jpg,004_2019_09_28_Spielplatz.jpg,001_2015_10_09_Spielplatz.jpg,003_2020_05_08_Gegenlicht.jpg,002_2017_06_27_Babyausstattung.jpg,004_2021_06_20_Babyausstattung.jpg,Balloon_on_a_children's_birthday_party.jpg,003_2019_10_27_Voegel_im_Flug.jpg,004_2021_06_07_Nummer_Fuenf.jpg,001_2015_04_23_Nummer_Fuenf.jpg,003_2016_05_23_Wassersport.jpg,003_2020_08_05_Winkel_und_Quadrate.jpg,001_2016_06_11_Winkel_und_Quadrate.jpg,001_2009_05_08_Mobilitaetshilfen_fuer_Behinderte.jpg,2020_10_10_016_Tour_de_Vin.jpg,2020_10_10_012_Tour_de_Vin.jpg,003_2019_09_27_Wind.jpg,003_2015_06_06_Toepferei.jpg,003_2020_08_05_Bruecken.jpg,003_2021_03_05_Handarbeit.jpg,002_2021_03_05_Handarbeit.jpg,003_2021_03_06_Stromerzeugung_und_Transport.jpg,002_2017_11_14_Wind.jpg,002_2013_09_20_Bruecken.jpg,001_2006_09_19_Bruecken.jpg,004_2021_01_14_Regenschirme.jpg,003_2018_05_14_Dunkelheit.jpg,004_2018_05_11_Allee.jpg,003_2016_11_30_Allee.jpg,004_2020_09_01_Abriss.jpg,003_2015_09_08_Abriss.jpg,002_2015_08_31_Abriss.jpg,001_2006_04_15_Abriss.jpg,004_2019_12_24_Toepferei.jpg,002_2014_03_15_Toepferei.jpg,001_2005_10_13_Toepferei.jpg,001_2014_03_17_11_Kai.jpg,2020_11_04_004_Meyers_Konversations_Lexikon.jpg,2020_10_10_017_Tour_de_Vin.jpg,004_2015_06_07_Kurven_und_Spiralen.jpg,003_2015_04_25_Kurven_und_Spiralen.jpg,001_2013_05_27_Kurven_und_Spiralen.jpg,002_1_2015_09_26_Kulturdenkmaeler_Deidesheim.jpg,003_2018_05_03_Das_Blau_der_Natur.jpg,002_2015_06_07_Das_Blau_der_Natur.jpg,001_2008_08_31_Das_Blau_der_Natur.jpg,001_2011_12_03_Bild_mit_hohem_Dynamikbereich.jpg,003_2015_06_06_Schatten.jpg,002_2015_06_08_Schatten.jpg,003_2019_09_26_Regenwasserableitung.jpg,002_2011_08_15_Regenwasserableitung.jpg,001_2006_04_14_Regenwasserableitung.jpg,003_2020_06_16_Do_it_yourself.jpg,002_2012_11_08_Do_it_yourself.jpg,004_2020_06_16_Do_it_yourself.jpg,001_2012_02_27_Do_it_yourself.jpg,004_2020_05_12_Sensor.jpg,002_2020_05_13_Sensor.jpg,003_2020_04_21_Uhren_fuer_den_Innenbereich.jpg,002_2020_04_21_Uhren_fuer_den_Innenbereich.jpg,001_2020_04_21_Uhren_fuer_den_Innenbereich.jpg,001_2015_04_19_Das_Gelb_der_Natur.jpg,002_2011_10_01_Oeffentliche_Parks.jpg,001_2010_11_01_Oeffentliche_Parks.jpg,001_2014_03_19_Obst.jpg,003_2019_12_14_Spiegel.jpg,001_2012_02_03_Spiegel.jpg,004_2019_12_09_Bueromaterial.jpg,002_2019_12_08_Bueromaterial.jpg,004_2018_06_20_Augenpflege.jpg,003_2016_02_23_Augenpflege.jpg,002_2019_10_16_Augenpflege.jpg,001_2019_05_27_Augenpflege.jpg,Belm_-_St._Dionysius_-BT-_02.jpg,Belm_-_Ententeich-Park_-BT-_01.jpg,Belm_-_St._Dionysius_-_Pfarrhaus_-BT-_02.jpg,Belm_-_St._Dionysius_-_Pfarrhaus_-BT-_01.jpg,Belm_-_St._Dionysius_-BT-_01.jpg,004_2019_06_25_Schluessel_und_Schluesselloecher.jpg,003_2019_06_25_Schluessel_und_Schluesselloecher.jpg,002_2019_06_23_Schluessel_und_Schluesselloecher.jpg,001_2019_06_23_Schluessel_und_Schluesselloecher.jpg,004_2017_11_10_Geologie.jpg,001_1996_10_14_Geologie.jpg,004_2014_03_17_Insekten.jpg,003_2013_07_15_Insekten.jpg,001_2010_07_06_Insekten.jpg,2019_03_25_002_Klingen.jpg,004_2019_03_26_Klingen.jpg,004_2019_01_21_Anzuendhuetchen.jpg,003_2019_01_21_Anzuendhuetchen.jpg,001_2015_04_23_Triebwerke_und_Motoren.jpg,002_2016_07_03_Rosa.jpg,001_2014_03_18_Balkone.jpg,Arkaia_-_Girasoles_-BT-_04.jpg,Arkaia_-_Girasoles_-BT-_05.jpg,Arkaia_-_Girasoles_-BT-_01.jpg,Arkaia_-_Girasoles_-BT-_02.jpg,Arkaia_-_Girasoles_-BT-_03.jpg,004_2018_07_18_Telefone.jpg,003_2018_07_18_Telefone.jpg,Belm_-_Vehrte_-_Teufels_Backofen_-BT-_01.jpg,Belm_-_Vehrte_-_Teufels_Teigtrog_-BT-_01.jpg,Ascain_-_Oveja_-BT-_02.jpg,003_2015_05_24_in_Bewegung.jpg,001_2017_11_13_in_Bewegung.jpg,Alaitza_-_Paisaje_con_amapola_y_niebla_-BT-_01.jpg,Bad_Iburg_-_Landesgartenschau_2018_-BT-_02.jpg,Bad_Iburg_-_Landesgartenschau_2018_-BT-_01.jpg,Bad_Iburg_-_Landesgartenschau_2018_-_Baumwipfelpfad_-BT-_03.jpg,Bad_Iburg_-_Landesgartenschau_2018_-_Baumwipfelpfad_-BT-_01.jpg,Bad_Iburg_-_Landesgartenschau_2018_-_Baumwipfelpfad_-BT-_02.jpg,Bad_Essen_-_St.-Nikolai-Kirche_-BT-_04.jpg,Bad_Essen_-_St.-Nikolai-Kirche_-BT-_05.jpg,Bad_Essen_-_St.-Nikolai-Kirche_-BT-_03.jpg,Bad_Essen_-_St.-Nikolai-Kirche_-BT-_02.jpg,Bad_Essen_-_St.-Nikolai-Kirche_-BT-_01.jpg,Bad_Iburg_-_Freeden_-BT-_01.jpg,Bad_Essen_-_Alte_Apotheke_-BT-_01.jpg,Bad_Iburg_-_Schloss_-BT-_01.jpg,Badaia_-_Vistas_con_niebla_-BT-_02.jpg,Berganzo_-_Ruta_del_Agua_-_Canal_-BT-_04.jpg,Berganzo_-_Ruta_del_Agua_-_Canal_-BT-_02.jpg,Berganzo_-_Ruta_del_Agua_-_Canal_-BT-_03.jpg,Berganzo_-_Ruta_del_Agua_-_Canal_-BT-_01.jpg,Berganzo_-_Ruta_del_Agua_-_Inglares_-BT-_04.jpg,Berganzo_-_Ruta_del_Agua_-_Inglares_-BT-_02.jpg,Berganzo_-_Ruta_del_Agua_-_Inglares_-BT-_03.jpg,Berganzo_-_Ruta_del_Agua_-_Inglares_-BT-_01.jpg,004_2016_11_24_Luftfahrzeuge.jpg,Ascain_-_Oveja_-BT-_01.jpg,Bidarte_-_Chapelle_Saint-Joseph_-BT-_01.jpg,Bidarte_-_Chapelle_Saint-Joseph_-BT-_02.jpg,Ascain_-_Zuhalmendi_-_Cruz_-BT-_01.jpg,Ascain_-_Casas_de_pueblo_-BT-_01.jpg,Bera_-_Ayuntamiento_-BT-_01.jpg,Bera_-_Itzea_Etxea_-BT-_01.jpg,001_2007_03_04_Fruehling.jpg,Badaia_-_Barranco_de_los_Goros_-_HDR_-BT-_04.jpg,Badaia_-_Barranco_de_los_Goros_-_HDR_-BT-_01.jpg,Badaia_-_Barranco_de_los_Goros_-_HDR_-BT-_02.jpg,Badaia_-_Barranco_de_los_Goros_-_HDR_-BT-_03.jpg,Arangio_-_Hayas_-BT-_03.jpg,Arangio_-_Hayas_-BT-_02.jpg,Arangio_-_Hayas_-BT-_01.jpg,Badaia_-_Vistas_con_niebla_-BT-_01.jpg,Badaia_-_Cueva_de_los_Goros_-_HDR_-BT-_04.jpg,Badaia_-_Cueva_de_los_Goros_-_HDR_-BT-_01.jpg,Badaia_-_Cueva_de_los_Goros_-_HDR_-BT-_02.jpg,Badaia_-_Cueva_de_los_Goros_-_HDR_-BT-_03.jpg,004_2017_09_06_Blau.jpg,Benasque_-_La_Besurta_02.jpg,Amboto_02.jpg,Benasque_-_Aigualluts_-_Pico_de_la_Mina_02.jpg,Benasque_-_Llanos_del_Hospital_-_Ruinas_01.jpg,Albertia_-_Bosque_01.jpg,Bielsa_-_Barranco_del_Montillo_05.jpg,Bielsa_-_Cruz_de_Guardia_01.jpg,Bielsa_-_Cruz_de_Guardia_03.jpg,Bielsa_-_Cruz_de_Guardia_02.jpg,Bielsa_-_Barranco_del_Montillo_04.jpg,Bielsa_-_Barranco_del_Montillo_03.jpg,Bielsa_-_Barranco_Trigoniero_-_Plana_01.jpg,Bielsa_-_Barranco_Trigoniero_-_Plana_04.jpg,Bielsa_-_Barranco_Trigoniero_-_Cascada_01.jpg,Bielsa_-_Barranco_Trigoniero_-_Plana_03.jpg,Bielsa_-_Barranco_Trigoniero_-_Refugio_03.jpg,Bielsa_-_Barranco_Trigoniero_-_Plana_02.jpg,Bielsa_-_Barranco_Trigoniero_-_Refugio_02.jpg,Bielsa_-_Barranco_Trigoniero_-_Refugio_01.jpg,Bielsa_-_Barranco_Trigoniero_-_Puente_01.jpg,Benasque_-_Llanos_del_Hospital_08.jpg,Benasque_-_Llanos_del_Hospital_06.jpg,Benasque_-_Llanos_del_Hospital_07.jpg,Benasque_-_La_Besurta_01.jpg,Benasque_-_La_Besurta_-_Pista_01.jpg,Benasque_-_Anciles_02.jpg,Benasque_-_Anciles_03.jpg,Benasque_-_Anciles_01.jpg,Benasque_-_Aigualluts_-_Pino_01.jpg,Benasque_-_Llanos_del_Hospital_-_Hotel_02.jpg,Benasque_-_Llanos_del_Hospital_-_Hotel_01.jpg,Benasque_-_Aigualluts_-_Llano_09.jpg,Benasque_-_Aigualluts_-_Llano_10.jpg,Benasque_-_Aigualluts_-_Llano_08.jpg,Benasque_-_Aigualluts_-_Llano_07.jpg,Benasque_-_Aigualluts_-_Llano_04.jpg,Benasque_-_Aigualluts_-_Llano_05.jpg,Benasque_-_Aigualluts_-_Llano_06.jpg,Benasque_-_Aigualluts_-_Llano_03.jpg,Bielsa_-_Camino_Cinca_01.jpg,Bielsa_-_Barranco_del_Montillo_02.jpg,Bielsa_-_Barranco_del_Montillo_01.jpg,Benasque_-_Refugio_de_la_Renclusa_01.jpg".to_string()
            )
        ),
        (
            "10.0.0.50",
            DosEntry::new(
                "client3".to_string(), 
                "10.0.0.50".to_string(), 
                "Benasque_-_La_Renclusa_01.jpg,Benasque_-_Forau_de_Aigualluts_02.jpg,Benasque_-_Forau_de_Aigualluts_01.jpg,Benasque_-_Llanos_del_Hospital_04.jpg,Benasque_-_Llanos_del_Hospital_-_Pino_01.jpg,Benasque_-_Llanos_del_Hospital_05.jpg,Benasque_-_Aigualluts_-_Llano_01.jpg,Benasque_-_Aigualluts_-_Llano_02.jpg,Benasque_-_Aigualluts_-_Cascada_03.jpg,Benasque_-_Aigualluts_-_Cascada_01.jpg,Benasque_-_Aigualluts_-_Cascada_02.jpg,Benasque_-_Aigualluts_-_Pico_de_la_Mina_01.jpg,Bielerkopf_-_Schafe_01.jpg,Bielerkopf_01.jpg,Benasque_-_Llanos_del_Hospital_02.jpg,Benasque_-_Llanos_del_Hospital_03.jpg,Benasque_-_Anciles_-_Patio_02.jpg,Benasque_-_Paisaje_del_valle_01.jpg,Benasque_-_Anciles_-_Verja_y_patio_01.jpg,Benasque_-_Anciles_-_Patio_01.jpg,Benasque_-_Anciles_-_Grifo_01.jpg,Benasque_-_Anciles_-_Labrador_Retriever.jpg,Benasque_-_Linsoles_-_Galgo_01.jpg,Benasque_-_Pico_Cerler_01.jpg,Benasque_-_Ampriu_01.jpg,Benasque_-_Pico_Cerler_03.jpg,Benasque_-_Pico_Cerler_02.jpg,Benasque_04.jpg,Benasque_-_Llanos_del_Hospital_01.jpg,Benasque_03.jpg,Benasque_02.jpg,Benasque_01.jpg,004_2016_11_13_Korrodierte_Objekte.jpg,001_2014_03_14_Korrodierte_Objekte.jpg,004_2015_07_17_Huetten_und_Schuppen.jpg,004_2016_11_29_Menschen_im_Urlaub.jpg,Augstenberg_03.jpg,Augstenberg_-_Schafe_01.jpg,003_2014_03_15_Hoehlen_Bergwerke_und_Dolinen.jpg,001_2009_09_06_Hoehlen_Bergwerke_und_Dolinen.jpg,Augstenberg_-_Gipfelkreuz_01.jpg,Augstenberg_02.jpg,Augstenberg_01.jpg,003_2015_04_23_Musikinstrumente.jpg,Aretxabaleta_-_Iglesia_01.jpg,Aretxabaleta_-_Cebollas.jpg,Arangio_-_Paso_en_una_valla.jpg,Arangio_-_Haya_03.jpg,Arangio_-_Borda_01.jpg,Arangio_-_Haya_01.jpg,Arangio_-_Haya_02.jpg,Amboto_01.jpg,004_2016_08_04_Uhren_an_oeffentlichen_Gebaeuden.jpg,002_2015_04_23_Fluchtpunkt.jpg,Arkaia_-_Remolacha.jpg,Barrio_-_Paisaje_01.jpg,Arlaban_-_Vega_01.jpg,Arlaban_-_Vega_02.jpg,Barrio_-_Iglesia_02.jpg,Barrio_01.jpg,Bachicabo_-_Puerto_de_la_Hoz_01.jpg,Bachicabo_-_Cumbre_02.jpg,Bachicabo_-_Cumbre_01.jpg,Barrio_-_Iglesia_01.jpg,Barbarin_-_Trujales_01.jpg,Barbarin_-_Trujales_02.jpg,Antiguo_Puerto_de_Vitoria_-_Encina_02.jpg,Antiguo_Puerto_de_Vitoria_-_Encina_03.jpg,Antiguo_Puerto_de_Vitoria_-_Encina_01.jpg,Antiguo_Puerto_de_Vitoria_-_Quejigos_01.jpg,004_2015_06_02_Fahrzeuge_Personenverkehr.jpg,Ascain_-_Casa_de_pueblo_02.jpg,Ascain_-_Cementerio_01.jpg,Ascain_-_Chemin_d'Arraioa_01.jpg,Ascain_-_Casa_de_pueblo_01.jpg,Ascain_-_Puente_romano_03.jpg,Ascain_-_Nivelle_01.jpg,Ascain_-_Puente_romano_04.jpg,002_2017_03_30_Haagsche_Hopjes.jpg,001_2017_03_30_Haagsche_Hopjes.jpg,Ascain_-_Puente_romano_02.jpg,Ascain_-_Puente_romano_01.jpg,003_2014_03_17_Netze.jpg,002_2014_03_17_Netze.jpg,001_1998_08_15_Netze.jpg,004_2015_04_23_Traktoren.jpg,Barbarin_02.jpg,Barbarin_01.jpg,Barbarin_-_Fuente_01.jpg,Barbarin_-_Ayuntamiento_01.jpg,Areso_-_Ayuntamiento_01.jpg,Areso_01.jpg,Alto_de_Guirguillano_-_Calzada_Romana_01.jpg,Alto_de_Guirguillano_-_Calzada_Romana_02.jpg,Bidaurreta_-_Fuente_01.jpg,Bidaurreta_01.jpg,004_2016_06_09_Numbers.jpg,002_2014_03_19_Numbers.jpg,001_2013_07_02_Numbers.jpg,Berrobi_-_Tractor_01.jpg,Arrigorrista_-_Narciso_01.jpg,Berrobi_-_Arroyo_01.jpg,Berrobi_-_Plaza_01.jpg,Berrobi_-_Ayuntamiento_01.jpg,Berrobi_01.jpg,001_1997_10_18_Barns.jpg,Berrostegieta_-_Iglesia_02.jpg,Arkaia_-_Paisaje_01.jpg,Arkaia_02.jpg,Arkaia_01.jpg,Badaia_-_Barranco_de_los_Goros_01.jpg,Badaia_-_Santa_Marina_04.jpg,Badaia_-_Ganalto_01.jpg,Badaia_-_Camino_01.jpg,Badaia_-_Barranco_de_los_Goros_-_Camino_01.jpg,Albertia_-_Cumbre_02.jpg,Albertia_-_Cumbre_01.jpg,Albertia_-_Cumbre_03.jpg,Adana_y_Gauna_-_Balsa_de_riego_01.jpg,Adana_y_Gauna_-_Balsa_de_riego_02.jpg,Aldamin_02.jpg,Berrostegieta_-_Iglesia_01.jpg,Bielsa_-_Puerto_Viejo_01.jpg,Bielsa_-_Puerto_Viejo_-_Camino_03.jpg,Bielsa_-_Pico_de_Puerto_Viejo_02.jpg,Bielsa_-_Pico_de_Puerto_Viejo_-_Lacs_de_Barroude_01.jpg,Bielsa_-_Pico_de_Puerto_Viejo_01.jpg,Bielsa_-_Puerto_Viejo_-_Camino_01.jpg,Bielsa_-_Puerto_Viejo_-_Refugio_02.jpg,Bielsa_-_Puerto_Viejo_-_Camino_02.jpg,Bielsa_-_Puerto_Viejo_-_Refugio_01.jpg,Andoin_-_Alimentador_de_ganado_01.jpg,Aracena_-_Cistus_ladanifer_01.jpg,Arkaia_-_Termas_romanas_01.jpg,Bielsa_01.jpg,Bielsa_-_Poste_sendero_PR-HU_137.jpg,Bielsa_-_Puerto_Viejo_-_Crocus_Nudiflorus_01.jpg,Bielsa_-_Mirador_de_Bielsa_01.jpg,Berrostegieta_-_Pinar_02.jpg,Berrostegieta_-_Pinar_01.jpg,Armentia_-_Bidegorri_01.jpg,Aratz_-_Poste_indicador_01.jpg,Aratz_-_Ovejas_Latxa_01.jpg,Berganzo_01.jpg,003_2014_03_16_Unterwasserfotografie.jpg,003_2016_05_27_Feuerwehr_und_Feuerwehrbedarf.jpg,002_2015_04_23_Feuerwehr_und_Feuerwehrbedarf.jpg,Arkaia_-_Iglesia_01.jpg,Ballo_-_Cardo_azul_02.jpg,Ballo_-_marca_gr-282_05.jpg,Ballo_-_marca_gr-282_03.jpg,Ballo_-_marca_gr-282_04.jpg,Ballo_-_Panorama_Aratz_02.jpg,Ballo_-_Poste_indicador_GR25-GR120.jpg,Ballo_-_Cardo_azul_01.jpg,Ballo_-_Panorama_Aratz_01.jpg,Ballo_-_marca_gr-282_01.jpg,Arkaia_-_Fuente_01.jpg,Beginn_Lechweg_01.jpg,004_2012_09_14_Blattadern.jpg,003_2011_04_18_Blattadern.jpg,001_2010_04_26_Blattadern.jpg,004_2016_06_05_Arbeitstiere.jpg,001_2012_05_27_Arbeitstiere.jpg,Bad_Laer_-_Kirchplatz_04.jpg,Bad_Laer_-_Kirchplatz_03.jpg,Bad_Laer_-_Kirchplatz_01.jpg,Bad_Laer_-_Kirchplatz_02.jpg,Aracena_-_Banco_01.jpg,Aracena_05.jpg,2015_06_05_004_Papageien.jpg,Aracena_-_Cabildo_Viejo_01.jpg,Aracena_01.jpg,Aracena_04.jpg,Aracena_-_Paisaje_02.jpg,Aracena_-_Paisaje_01.jpg,Aracena_02.jpg,Aracena_03.jpg,Aracena_-_Fuente_01.jpg,031_2015_12_29_Wiki_Loves_Earth_2016.jpg,Almonaster_La_Real_02.jpg,Almonaster_La_Real_01.jpg,Almonaster_La_Real_Mezquita_04.jpg,Almonaster_La_Real_03.jpg,Almonaster_La_Real_Mezquita_03.jpg,Almonaster_La_Real_Mezquita_01.jpg,Berrostegieta_desde_Eskibel_01.jpg,2012_08_25_004_Langzeitbelichtung.jpg,2009_09_07_003_Langzeitbelichtung.jpg,Apodaka_01.jpg,Apodaka_-_Fuente_01.jpg,Apodaka_02.jpg,Apodaka_03.jpg,Belabia_01.jpg,Belabia_02.jpg,Abetxuko_-_parque_01.jpg,Abetxuko_camino_puente_02.jpg,Abetxuko_camino_puente_01.jpg,Badaia_-_Santa_Marina_02.jpg,Barranco_de_los_Goros_01.jpg,Badaia_-_Pozubarri_01.jpg,Badaia_-_Santa_Marina_01.jpg,Badaia_-_Santa_Marina_03.jpg,Badaia_-_Heces_de_caballo_02.jpg,Badaia_-_Heces_de_caballo_01.jpg,Aldamin_01.jpg,028_2014_08_04_Urlaub_Sulden.jpg,2014_08_09_002_Sankt_Nantwein,_Wolfratshausen.jpg,2015_06_06_003_Briefkasten.jpg,2014_08_05_001_Briefkasten.jpg,2015_07_18_002_Reflexionen.jpg,2015_07_15_001_Reflexionen.jpg,2015_06_26_003_Reflexionen.jpg,2014_04_19_004_Reflexionen.jpg,043_2015_06_02_Castell_de_Bellver.jpg,337_2015_06_08_Creu_de_sa_Cala.jpg,2015_09_08_022_Abriss_Tortenschachtel.jpg,025_2015_06_02_Castell_de_Bellver.jpg,2014_03_14_253_San_Salvador_Mallorca.jpg,003_2015_09_26_Kulturdenkmaeler_Deidesheim.jpg,2014_03_14_256_San_Salvador_Mallorca.jpg,002_2015_09_26_Kulturdenkmaeler_Deidesheim.jpg,2015_07_14_003_Fenster_mit_Blumenkasten.jpg,2013_09_18_002_Fenster_mit_Blumenkasten.jpg,2015_10_11_004_Weathered_window_shutter.jpg,2014_08_08_001_Diagonalen.jpg,2013_12_30_004_Diagonalen.jpg,2011_06_12_003_Diagonalen.jpg,2011_04_03_002_Diagonalen.jpg,2015_08_19_050_Hund_Wallberg.jpg,2014_03_14_343_Fernmeldehandwerker.jpg,2014_03_14_300_Restaurator_Mallorca.jpg,2012_11_01_036_Leopold_Reitz_Weg.jpg,2006_09_06_180_Leuchtturm.jpg,003_04_2015_09_26_Kulturdenkmaeler_Forst.jpg,003_05_2015_09_26_Kulturdenkmaeler_Forst.jpg,023_2015_09_24_Kulturdenkmaeler_Deidesheim.jpg,016_2015_09_24_Kulturdenkmaeler_Deidesheim.jpg,001_2015_09_26_Kulturdenkmaeler_Deidesheim.jpg,006_2015_09_26_Kulturdenkmaeler_Deidesheim.jpg,005_2015_09_26_Kulturdenkmaeler_Deidesheim.jpg,039_2015_09_27_Kulturdenkmaeler_Deidesheim.jpg,004_01_2015_09_22_Kulturdenkmaeler_Ludwigshafen.jpg,004_02_2015_09_22_Kulturdenkmaeler_Ludwigshafen.jpg,004_05_2015_09_22_Kulturdenkmaeler_Ludwigshafen.jpg,004_06_2015_09_22_Kulturdenkmaeler_Ludwigshafen.jpg,004_07_2015_09_22_Kulturdenkmaeler_Ludwigshafen.jpg,004_08_2015_09_22_Kulturdenkmaeler_Ludwigshafen.jpg,2014_03_14_298_Seu_de_Mallorca.jpg,2014_03_14_297_Seu_de_Mallorca.jpg,2014_03_14_283_Seu_de_Mallorca.jpg,2014_03_14_266_San_Salvador_Mallorca.jpg,2014_03_14_248_San_Salvador_Mallorca.jpg,050_2015_06_02_Castell_de_Bellver.jpg,035_2015_06_02_Castell_de_Bellver.jpg,345_2015_06_08_Creu_de_Llombards.jpg,2014_12_29_035_Speyerer_Dom_Grabplatte_Rudolf_von_Habsburg.jpg,2014_12_29_033_Speyerer_Dom_Krypta.jpg,2014_06_22_032_Roemischer_Steinbruch.jpg,2014_06_22_028_Roemischer_Steinbruch.jpg,2014_06_22_031_Roemischer_Steinbruch.jpg,2014_06_22_027_Roemischer_Steinbruch.jpg,2014_08_09_005_Sankt_Nantwein,_Wolfratshausen.jpg,2014_08_09_004_Sankt_Nantwein,_Wolfratshausen.jpg,2014_08_18_006_Ehemaliges_Rathaus,_Marktplatz_9.jpg,2014_08_20_006_Burg_Erfenstein.jpg,2012_07_21_016_Hambacher_Schloss_Ostseite.jpg,2012_07_21_033_Hambacher_Schloss.jpg,2012_06_10_009_Klosterruine_Limburg.jpg,015_2015_05_10_Kulturdenkmaeler_Deidesheim.jpg,022_2015_08_27_Kulturdenkmaeler_Hassloch.jpg,021_2015_08_27_Kulturdenkmaeler_Hassloch.jpg,020_2015_08_27_Kulturdenkmaeler_Hassloch.jpg,019_2015_08_27_Kulturdenkmaeler_Hassloch.jpg,016_2015_07_09_Kulturdenkmaeler_Hassloch.jpg,015_2015_08_27_Kulturdenkmaeler_Hassloch.jpg,010_2015_08_27_Kulturdenkmaeler_Hassloch.jpg,007_2015_08_27_Kulturdenkmaeler_Hassloch.jpg,004_2015_08_27_Kulturdenkmaeler_Hassloch.jpg,001_2015_08_27_Kulturdenkmaeler_Hassloch.jpg,003_2015_08_27_Kulturdenkmaeler_Hassloch.jpg,2015_04_23_086_Raumanzug.jpg,2009_03_14_064_Stretchlimousine.jpg,2014_03_19_397_Eingelegte_Fische.jpg,2012_10_05_005_Weintrauben.jpg,2012_07_26_016_Cocktailtomaten.jpg,2015_06_07_001_Salatteller.jpg,2015_06_28_014_Auge_Sammlungen.jpg,2015_06_26_006_Auge_Sammlungen.jpg,2015_06_26_003_Auge.jpg,2015_06_26_001_unbenannt.jpg,2012_05_18_023_Teufelstisch_(Wiki_Loves_Earth_2015).jpg,022_2015_05_06_Feuchtwiese_im_Bruch_(Wiki_Loves_Earth_2015).jpg,041_2015_05_19_Feuchtgebiet_am_Feuerberg_(Wiki_Loves_Earth_2015).jpg,035_2015_05_07_Feuchtgebiet_am_Feuerberg_(Wiki_Loves_Earth_2015).jpg,088_2015_05_26_Ein_Walnussbaum_(Wiki_Loves_Earth_2015).jpg,017_2015_05_06_Feuchtwiese_im_Bruch_(Wiki_Loves_Earth_2015).jpg,047_2015_05_19_Feuchtgebiet_am_Feuerberg_(Wiki_Loves_Earth_2015).jpg,032_2015_05_07_Feuchtgebiet_am_Feuerberg_(Wiki_Loves_Earth_2015).jpg,042_2015_05_19_Feuchtgebiet_am_Feuerberg_(Wiki_Loves_Earth_2015).jpg,024_2015_05_07_Feuchtwiese_im_Bruch_(Wiki_Loves_Earth_2015).jpg,012_2015_05_06_Winterlinde_(Wiki_Loves_Earth_2015).jpg,065_2015_05_25_Speierling_am_Hohlweg_(Wiki_Loves_Earth_2015).jpg,060_2015_05_10_Platanenallee_(Wiki_Loves_Earth_2015).jpg,025_2015_05_07_Feuchtgebiet_am_Feuerberg_(Wiki_Loves_Earth_2015).jpg,057_2015_05_10_Platanenallee_(Wiki_Loves_Earth_2015).jpg,009_2015_05_06_Winterlinde_(Wiki_Loves_Earth_2015).jpg,2015_05_19_005_Schaltkasten_mit_wildem_Wein_umwachsen.jpg,2015_05_14_001_Wohnhaus_mit_Efeu_bewachsen.jpg,2015_04_08_014_35mm_camera_Fuji_Fujica_Compact_S.jpg,2015_04_07_005_Handbelichtungsmesser_Weston_Master_IV.jpg,2015_04_07_006_Adox_Roll-film_box-camera.jpg,2015_04_08_011_Faltkamera_Zeiss_Ikon_Contessa-Nettel_Cocarette_No._211_U.jpg,2015_04_08_012_Kodak_Instamatic_50.jpg,2015_04_07_004_Handbelichtungsmesser_Weston_Master_IV.jpg,2015_04_08_005_Blitzlicht_Hanimex_TB555.jpg,2015_04_08_009_Analogfilme.jpg,2015_04_07_004_Kopier-Filter_auf_Leuchtpult.jpg,2014_03_18_376_Treppe_Mallorca.jpg,2014_04_19_004_Treppe_Lanxess.jpg,2014_03_20_111_Treppe_Ferienhaus.jpg,2013_08_29_033_Treppe_Fleckenstein.jpg,2014_03_14_073_Wendeltreppe.jpg,2014_03_14_390_Kondensstreifen.jpg,2013_07_16_120_Lochstreifen.jpg,2012_08_05_038_Gurkenranke.jpg,2013_05_27_072_Farnwedel.jpg,2011_08_27_001_Regenbogen.jpg,2011_02_23_006_Weinranke.jpg,2014_03_14_326_Teufelsfigur.jpg,2012_05_27_037_Clown_und_Pferd.jpg,2012_08_05_030_Tomaten.jpg,2014_12_29_002_Ausflug_Speyer.jpg,2006_09_06_111_Urlaub_Ruegen.jpg,115_2014_12_25_Kulturdenkmaeler_Ruppertsberg.jpg,2014_09_14_Katholisches_Schwesternhaus.jpg,2014_12_20_022_Weihnachtsmarkt_Deidesheim.jpg,2014_12_20_026_Weihnachtsmarkt_Deidesheim.jpg,2014_12_20_017_Weihnachtsmarkt_Deidesheim.jpg,2013_12_14_007_Weihnachtsmarkt.jpg,2012_12_01_002_Weihnachtsmarkt.jpg,2014_12_14_006_Weihnachtswaldbasar_Dudenhofen.jpg,2014_12_04_288_Ortler.jpg,2014_12_20_002_Weihnachtsmarkt_Deidesheim.jpg,2014_12_14_015_Weihnachtswaldbasar_Dudenhofen.jpg,2014_09_23_Wegekreuz_an_der_L516.jpg,2014_09_23_Dreiseithof_Obergasse_21.jpg,2014_11_06_006_Blaetter.jpg,2014_11_06_010_Blaetter.jpg,2014_11_09_022_Blaetter.jpg,2012_08_25_Schwedenfeuer.jpg,2009_03_14_Springbrunnen_(W)Einkaufsnacht.jpg,2014_06_22_Klosterruine_Limburg.jpg,2014_09_14_Schulhaus.jpg,2014_09_14_Spolien_2.jpg,2014_09_14_Spolien_1.jpg,2014_09_14_Winzerhof_Forstgasse_2.jpg,2014_09_14_Wegekreuz_Bergweg.jpg,2014_09_23_Wohnhaus_Obergasse_34.jpg,2014_09_23_Teepavillon_an_der_Obergasse_2.jpg,2014_09_23_Hofanlage_Obergasse_14_und_16.jpg,2014_09_23_Keilstein_an_der_Obergasse_15.jpg,2014_09_23_Torfahrt_an_der_Obergasse_17.jpg".to_string()
            )
        ),
    ];

    ip_entries
        .iter()
        .find(|(ip, _)| *ip == client_ip)
        .map(|(_, entry)| entry.clone())
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
