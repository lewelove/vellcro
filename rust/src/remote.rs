use regex::Regex;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

pub fn get_discogs_token() -> Option<String> {
    let mut home_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home_dir.push(".secrets/discogs.env");
    let _ = dotenvy::from_filename(home_dir);

    let token = std::env::var("DISCOGS_API_TOKEN").unwrap_or_default();
    if token.is_empty() {
        None
    } else {
        Some(token)
    }
}

pub fn fetch_remote_metadata(url: &str) -> Option<Value> {
    let rg_regex = Regex::new(r"(release|release-group)/([a-f0-9\-]+)").unwrap();
    let caps = rg_regex.captures(url)?;
    
    let mode = caps.get(1)?.as_str();
    let entity_id = caps.get(2)?.as_str();
    let is_rg = mode == "release-group";

    let config = crate::config::AppConfig::load();
    let cache_dir = config.get_cache_dir();
    let _ = fs::create_dir_all(&cache_dir);

    let prefix = if is_rg { "rg" } else { "rel" };
    let cache_file = cache_dir.join(format!("{prefix}-{entity_id}.json"));

    if cache_file.exists()
        && let Ok(content) = fs::read_to_string(&cache_file)
            && let Ok(mut data) = serde_json::from_str::<Value>(&content) {
                data["_is_rg"] = json!(is_rg);
                return Some(data);
            }

    let client = reqwest::blocking::Client::builder()
        .user_agent("Vellcro/0.1")
        .build()
        .unwrap();

    let mut data = json!({ "_is_rg": is_rg });

    if is_rg {
        let rg_url = format!("https://musicbrainz.org/ws/2/release-group/{entity_id}?inc=tags+url-rels+artist-credits&fmt=json");
        let rg: Value = client.get(&rg_url).send().ok()?.json().ok()?;
        
        data["discogs"] = get_discogs_data(&client, &rg);
        data["musicbrainz"] = json!({ "release_group": rg });
    } else {
        let rel_url = format!("https://musicbrainz.org/ws/2/release/{entity_id}?inc=labels+release-groups+url-rels+recordings+artist-credits+media&fmt=json");
        let release: Value = client.get(&rel_url).send().ok()?.json().ok()?;
        
        let rg_id = release.get("release-group").and_then(|rg| rg.get("id")).and_then(|id| id.as_str()).unwrap_or("");
        let rg = if !rg_id.is_empty() {
            let rg_url = format!("https://musicbrainz.org/ws/2/release-group/{rg_id}?inc=tags+url-rels+artist-credits&fmt=json");
            client.get(&rg_url).send().ok()?.json().ok()
        } else {
            None
        };

        data["discogs"] = get_discogs_data(&client, &release);
        data["musicbrainz"] = json!({ 
            "release": release, 
            "release_group": rg.unwrap_or_else(|| json!({})) 
        });
    }

    if let Ok(json_str) = serde_json::to_string_pretty(&data) {
        let _ = fs::write(&cache_file, json_str);
    }

    Some(data)
}

fn get_discogs_data(client: &reqwest::blocking::Client, mb_obj: &Value) -> Value {
    let Some(token) = get_discogs_token() else { return json!({ "error": "Missing token" }) };

    let mut discogs_url = String::new();
    if let Some(relations) = mb_obj.get("relations").and_then(|r| r.as_array()) {
        for rel in relations {
            if let Some(url_str) = rel.get("url").and_then(|u| u.get("resource")).and_then(|s| s.as_str())
                && (url_str.contains("discogs.com/release/") || url_str.contains("discogs.com/master/")) {
                    discogs_url = url_str.to_string();
                    break;
                }
        }
    }

    if discogs_url.is_empty() {
        return json!({ "error": "No relation" });
    }

    let auth_header = format!("Discogs token={token}");
    let mut result = json!({});
    let rel_re = Regex::new(r"release/(\d+)").unwrap();
    let mas_re = Regex::new(r"master/(\d+)").unwrap();

    if let Some(caps) = rel_re.captures(&discogs_url) {
        let id = caps.get(1).unwrap().as_str();
        if let Ok(resp) = client.get(format!("https://api.discogs.com/releases/{id}")).header("Authorization", &auth_header).send()
            && let Ok(rel_data) = resp.json::<Value>() {
                result["release"] = rel_data.clone();
                if let Some(m_id) = rel_data.get("master_id").and_then(serde_json::Value::as_i64) {
                    thread::sleep(Duration::from_secs(1));
                    if let Ok(m_resp) = client.get(format!("https://api.discogs.com/masters/{m_id}")).header("Authorization", &auth_header).send()
                        && let Ok(m_data) = m_resp.json::<Value>() { result["master"] = m_data; }
                }
            }
    } else if let Some(caps) = mas_re.captures(&discogs_url) {
        let id = caps.get(1).unwrap().as_str();
        if let Ok(resp) = client.get(format!("https://api.discogs.com/masters/{id}")).header("Authorization", &auth_header).send()
            && let Ok(m_data) = resp.json::<Value>() { result["master"] = m_data; }
    }

    result
}
