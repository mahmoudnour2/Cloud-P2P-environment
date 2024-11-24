use sqlx::{postgres::PgPoolOptions, Pool, Postgres};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::env;
use std::process::Command;
use dotenv::dotenv;
use time::PrimitiveDateTime;
use sqlx::FromRow; // Import FromRow
use sqlx::Row;


#[derive(Debug, Serialize, Deserialize, FromRow)] // Derive FromRow
pub struct Client {
    client_id: String,
    ip_address: String,
    resources: Option<Value>,
    last_online: Option<PrimitiveDateTime>,
    is_active: Option<bool>,
}


pub struct DoS {
    pool: Pool<Postgres>,
}

impl DoS {
    pub async fn new() -> Result<Self, sqlx::Error> {
        dotenv().ok();
        let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await?;
        Ok(Self { pool })
    }

    fn get_mac_from_ip(ip_address: &str) -> Result<String, Box<dyn std::error::Error>> {
        let output = if cfg!(target_os = "windows") {
            Command::new("cmd")
                .args(["/C", &format!("arp -a {}", ip_address)])
                .output()?
        } else {
            Command::new("sh")
                .arg("-c")
                .arg(&format!(
                    "arp -n {} | grep -o -E '([[:xdigit:]]:){{5}}[[:xdigit:]]'",
                    ip_address
                ))
                .output()?
        };

        let mac = String::from_utf8(output.stdout)?
            .trim()
            .replace(":", "")
            .replace("-", "")
            .to_uppercase();

        if mac.is_empty() {
            return Err("Could not find MAC address for given IP".into());
        }

        Ok(mac)
    }

    fn generate_resource_id(mac_address: &str, file_name: &str) -> String {
        format!("{}_{}", mac_address, file_name)
    }

    pub async fn add_client(
        &self,
        ip_address: &str,
        resources: Option<Value>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        //let client_id = Self::get_mac_from_ip(ip_address)?;
        let client_id = "00:11:22:33:44:55".to_string(); // Hardcoded MAC address


        sqlx::query(
            r#"
            INSERT INTO dos (client_id, ip_address, resources, is_active)
            VALUES ($1, $2, $3, TRUE)
            "#,
        )
        .bind(client_id)
        .bind(ip_address)
        .bind(resources.map(sqlx::types::Json))
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn add_resource(
        &self,
        client_id: &str,
        file_name: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let resource_id = Self::generate_resource_id(client_id, file_name);

        let current_resources: Option<Value> = self.get_client_resources(client_id).await?;
        let mut resources_list = match current_resources {
            Some(Value::String(existing)) => existing,
            _ => String::new(),
        };

        if !resources_list.is_empty() {
            resources_list.push(',');
        }
        resources_list.push_str(&resource_id);

        let new_resources = serde_json::Value::String(resources_list);

        sqlx::query(
            r#"
            UPDATE dos 
            SET resources = $1
            WHERE client_id = $2
            "#,
        )
        .bind(sqlx::types::Json(new_resources))
        .bind(client_id)
        .execute(&self.pool)
        .await?;

        Ok(resource_id)
    }

    pub async fn get_active_clients(&self) -> Result<Vec<Client>, sqlx::Error> {
        sqlx::query_as::<_, Client>(
            r#"
            SELECT client_id, ip_address, resources, last_online, is_active
            FROM dos
            WHERE is_active = true
            "#,
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn update_client_status(
        &self,
        client_id: &str,
        is_active: bool,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            UPDATE dos
            SET is_active = $1, last_online = CURRENT_TIMESTAMP
            WHERE client_id = $2
            "#,
        )
        .bind(is_active)
        .bind(client_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_client_resources(
        &self,
        client_id: &str,
    ) -> Result<Option<Value>, sqlx::Error> {
        let row = sqlx::query(
            r#"
            SELECT resources FROM dos WHERE client_id = $1
            "#,
        )
        .bind(client_id)
        .fetch_optional(&self.pool)
        .await?;
    
        // Extract and map the JSON value correctly
        Ok(row
            .and_then(|r| r.try_get::<Option<sqlx::types::Json<Value>>, _>("resources").ok())
            .flatten()
            .map(|json| json.0))
    }
    
    
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv::dotenv().ok();

    println!("Loaded environment variables.");
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");    println!("DATABASE_URL: {}", database_url);  // This will print the value of DATABASE_URL


    let dos = DoS::new().await?;

    // dos.add_client("192.168.1.14", Some(serde_json::json!(""))).await?;

    // let client_id = "AABBCCDDEEFF"; // Example MAC address
    // Test adding a client with hardcoded MAC address
    dos.add_client("192.168.1.10", Some(serde_json::json!(""))).await?;

    let client_id = "00:11:22:33:44:55";

    dos.add_resource(client_id, "example1.jpg").await?;
    dos.add_resource(client_id, "example2.jpg").await?;

    let active_clients = dos.get_active_clients().await?;
    println!("Active Clients: {:?}", active_clients);

    if let Some(resources) = dos.get_client_resources(client_id).await? {
        println!("Client Resources: {}", resources);
    }

    Ok(())
}
