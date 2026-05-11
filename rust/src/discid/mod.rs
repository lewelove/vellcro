use crate::utils;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::fs;
use std::path::{Path, PathBuf};
use sha1::{Digest, Sha1};
use std::fmt::Write;

pub fn get_total_samples(filepath: &Path) -> Result<u64, String> {
    let tag = metaflac::Tag::read_from_path(filepath).map_err(|e| e.to_string())?;
    tag.get_streaminfo()
        .map(|info| info.total_samples)
        .ok_or_else(|| "No StreamInfo block found".to_string())
}

pub fn get_ctdb_id(folder_path: &Path) -> Option<String> {
    let mut files: Vec<PathBuf> = fs::read_dir(folder_path)
        .into_iter()
        .flatten()
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("flac"))
        .collect();

    if files.is_empty() {
        return None;
    }

    files.sort_by(|a, b| {
        let key_a = utils::get_sort_key(a);
        let key_b = utils::get_sort_key(b);
        
        match key_a.0.cmp(&key_b.0) {
            std::cmp::Ordering::Equal => {
                match key_a.1.cmp(&key_b.1) {
                    std::cmp::Ordering::Equal => alphanumeric_sort::compare_path(a, b),
                    other => other,
                }
            },
            other => other,
        }
    });

    let mut offsets = Vec::new();
    let mut current_offset = 150;

    for f in &files {
        if let Ok(samples) = get_total_samples(f) {
            let sectors = samples / 588;
            offsets.push(current_offset);
            current_offset += sectors;
        }
    }

    let leadout = current_offset;
    let pregap = offsets[0];

    let mut x = String::new();
    for offset in offsets.iter().skip(1) {
        let _ = write!(x, "{:08X}", offset - pregap);
    }
    let _ = write!(x, "{:08X}", leadout - pregap);

    while x.len() < 800 {
        x.push('0');
    }

    let mut hasher = Sha1::new();
    hasher.update(x.as_bytes());
    let sha1_bytes = hasher.finalize();

    let ctdb = STANDARD
        .encode(sha1_bytes)
        .replace('+', ".")
        .replace('/', "_")
        .replace('=', "-");

    Some(ctdb)
}

pub fn run() {
    if let Some(ctdb) = get_ctdb_id(&PathBuf::from(".")) {
        println!("https://db.cuetools.net/?tocid={ctdb}");
    } else {
        std::process::exit(1);
    }
}
