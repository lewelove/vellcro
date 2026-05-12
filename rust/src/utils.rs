use serde_json::Value;
use std::path::{Path, PathBuf};

pub fn expand_path(path_str: &str) -> PathBuf {
    if path_str.starts_with('~')
        && let Some(home) = dirs::home_dir() {
            if path_str == "~" {
                return home;
            }
            if let Some(stripped) = path_str.strip_prefix("~/") {
                return home.join(stripped);
            }
        }
    PathBuf::from(path_str)
}

pub fn sanitize_filename(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

pub fn get_sort_key(filepath: &Path) -> (u8, u32, String) {
    if let Ok(tag) = metaflac::Tag::read_from_path(filepath)
        && let Some(vc) = tag.vorbis_comments()
            && let Some(track_nums) = vc.get("TRACKNUMBER")
                && let Some(num_str) = track_nums.first() {
                    let num_part = num_str.split('/').next().unwrap_or("0");
                    if let Ok(n) = num_part.parse::<u32>() {
                        return (0, n, String::new());
                    }
                }
    let filename = filepath.file_name().unwrap_or_default().to_string_lossy().to_string();
    (1, 0, filename)
}

pub fn join_artists(artist_credit: Option<&Value>) -> String {
    let mut parts = String::new();
    if let Some(credits) = artist_credit.and_then(|c| c.as_array()) {
        for credit in credits {
            if let Some(name) = credit.get("name").and_then(|n| n.as_str()) { parts.push_str(name); }
            else if let Some(artist) = credit.get("artist").and_then(|a| a.get("name")).and_then(|n| n.as_str()) { parts.push_str(artist); }
            if let Some(join) = credit.get("joinphrase").and_then(|j| j.as_str()) { parts.push_str(join); }
        }
    }
    parts
}

pub fn fmt_yyyy_mm(date_str: &str) -> String {
    if date_str.len() >= 7 { return date_str[..7].to_string(); }
    if date_str.len() == 4 { return format!("{date_str}-00"); }
    date_str.to_string()
}

pub fn walk_dir(root: &Path, dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut entries = Vec::new();
    if dir.is_dir() {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                entries.extend(walk_dir(root, &path)?);
            } else if let Ok(rel) = path.strip_prefix(root) {
                entries.push(rel.to_path_buf());
            }
        }
    }
    Ok(entries)
}
