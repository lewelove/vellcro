use crate::config::AppConfig;
use crate::remote::get_discogs_token;
use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::Utc;
use reqwest::blocking::Client;
use serde_json::{json, Value};
use std::fs;
use xxhash_rust::xxh64::Xxh64;

pub fn run_add(url: &str) -> Result<()> {
    let config = AppConfig::load();
    let collection_folder = config.get_collection_folder()
        .context("Collection folder not configured in ~/.config/vellcro/config.toml")?;

    if !collection_folder.exists() {
        fs::create_dir_all(&collection_folder)?;
    }

    let token = get_discogs_token().context("DISCOGS_API_TOKEN is missing")?;
    let client = Client::builder().user_agent("Vellcro/0.1").build()?;

    let is_master = url.contains("/master/");
    let is_release = url.contains("/release/");

    if !is_master && !is_release {
        anyhow::bail!("URL must be a discogs master or release URL");
    }

    let entity_type = if is_master { "master" } else { "release" };
    let id = url.split('/').last().unwrap_or("").split('-').next().unwrap_or("");
    if id.is_empty() {
        anyhow::bail!("Could not extract ID from URL");
    }

    let api_url = if is_master {
        format!("https://api.discogs.com/masters/{}", id)
    } else {
        format!("https://api.discogs.com/releases/{}", id)
    };

    let auth_header = format!("Discogs token={}", token);
    let resp: Value = client
        .get(&api_url)
        .header("Authorization", &auth_header)
        .send()?
        .json()?;

    let artist_raw = resp.get("artists")
        .and_then(|a| a.as_array())
        .and_then(|a| a.first())
        .and_then(|a| a.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or("Unknown Artist");

    let artist = artist_raw
        .trim_end_matches(|c: char| c.is_numeric() || c == '(' || c == ')' || c.is_whitespace())
        .trim();

    let title = resp.get("title")
        .and_then(|t| t.as_str())
        .unwrap_or("Unknown Album");

    let mut cover_hash_str = String::new();
    if let Some(images) = resp.get("images").and_then(|i| i.as_array()) {
        if let Some(img) = images.first() {
            if let Some(img_url) = img.get("resource_url").and_then(|u| u.as_str()) {
                let img_bytes = client
                    .get(img_url)
                    .header("Authorization", &auth_header)
                    .send()?
                    .bytes()?;
                
                let mut hasher = Xxh64::new(0);
                hasher.update(&img_bytes);
                let hash = hasher.digest();
                
                cover_hash_str = STANDARD
                    .encode(hash.to_be_bytes())
                    .replace('+', ".")
                    .replace('/', "_")
                    .replace('=', "-");

                if let Ok(dyn_img) = image::load_from_memory(&img_bytes) {
                    let cache_dir = config.get_cache_dir();
                    let covers_dir = cache_dir.join("covers");
                    let _ = fs::create_dir_all(&covers_dir);
                    let cover_path = covers_dir.join(format!("{}.png", cover_hash_str));
                    let _ = dyn_img.save_with_format(&cover_path, image::ImageFormat::Png);
                }
            }
        }
    }

    let now = Utc::now();
    let nanos = now.timestamp_nanos_opt().unwrap_or(0);
    let formatted_date = now.format("%B %d %Y").to_string();

    let json_obj = json!({
        "albumartist": artist,
        "album": title,
        "type": entity_type,
        "url": url,
        "cover": cover_hash_str,
        "date_added": [nanos, formatted_date]
    });

    let safe_artist = crate::utils::sanitize_filename(artist);
    let safe_title = crate::utils::sanitize_filename(title);
    let filename = format!("{}-{}.json", safe_artist, safe_title);
    let file_path = collection_folder.join(filename);

    let content = serde_json::to_string_pretty(&json_obj)?;
    fs::write(&file_path, content)?;

    println!("Added {} - {} to collection at {}", artist, title, file_path.display());

    Ok(())
}
