use anyhow::{Context, Result};
use globset::{Glob, GlobSetBuilder};
use lava_torrent::torrent::v1::Torrent;
use serde_json::{Value, json, Map};
use std::fmt::Write as FmtWrite;
use std::path::{Path, PathBuf};
use std::process::Command;
use crate::remote::fetch_remote_metadata;
use crate::utils::{join_artists, fmt_yyyy_mm};
use crate::discid::get_ctdb_id;

struct TrackData {
    tracknumber: u32,
    discnumber: u32,
    title: String,
    artist: String,
    musicbrainz_trackid: String,
    musicbrainz_releasetrackid: String,
    musicbrainz_artistid: String,
}

pub fn run(mb_url: &str, use_metadata: bool, use_mbid: bool, use_url: bool, torrent_path_str: &str, tracks_filter: &str) -> Result<()> {
    let raw_data = fetch_remote_metadata(mb_url).context("Failed to fetch metadata from URL")?;
    
    let torrent_path = Path::new(torrent_path_str).canonicalize()?;
    let current_dir = std::env::current_dir()?;

    let output = Command::new("nix")
        .args(["hash", "file", torrent_path.to_str().unwrap()])
        .output()?;

    if !output.status.success() {
        anyhow::bail!("Failed to hash torrent file with nix");
    }
    let torrent_hash = String::from_utf8(output.stdout)?.trim().to_string();

    let cover_file_path = Path::new("cover.png");
    let cover_hash = if cover_file_path.exists() {
        let out = Command::new("nix")
            .args(["hash", "file", "cover.png"])
            .output()?;
        if out.status.success() {
            String::from_utf8(out.stdout)?.trim().to_string()
        } else {
            "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string()
        }
    } else {
        "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string()
    };

    let torrent = Torrent::read_from_file(&torrent_path).context("Failed to parse torrent")?;

    let mut builder = GlobSetBuilder::new();
    for part in tracks_filter.split(',') {
        let trimmed = part.trim();
        if trimmed.is_empty() { continue; }
        let pattern = if !trimmed.contains('/') && !trimmed.contains('*') && !trimmed.contains('?') {
            format!("**/*.{}", trimmed.trim_start_matches('.'))
        } else {
            trimmed.to_string()
        };
        builder.add(Glob::new(&pattern)?);
    }
    let globset = builder.build()?;

    let mut valid_paths = Vec::new();
    if let Some(files) = &torrent.files {
        for f in files {
            let path_buf = f.path.clone();
            if globset.is_match(path_buf.to_string_lossy().as_ref()) {
                valid_paths.push(path_buf);
            }
        }
    } else {
        let name_str = &torrent.name;
        if globset.is_match(name_str) {
            valid_paths.push(Path::new(name_str).to_path_buf());
        }
    }
    valid_paths.sort_by(|a, b| alphanumeric_sort::compare_path(a, b));

    let rel_torrent = torrent_path
        .strip_prefix(&current_dir)
        .map_or_else(|_| torrent_path.clone(), Path::to_path_buf);

    let torrent_nix_path = if rel_torrent.is_absolute() {
        format!("\"{}\"", rel_torrent.to_string_lossy())
    } else {
        format!("./{}", rel_torrent.to_string_lossy())
    };

    let is_rg = raw_data.get("_is_rg").and_then(|v| v.as_bool()).unwrap_or(false);
    let mb = raw_data.get("musicbrainz").unwrap();
    let rg = mb.get("release_group").unwrap();
    let rel = mb.get("release");
    let dg_fallback = json!({});
    let dg = raw_data.get("discogs").unwrap_or(&dg_fallback);

    let album_artist = join_artists(rg.get("artist-credit"));
    let title = rg.get("title").and_then(|t| t.as_str()).unwrap_or("Unknown Title");

    let mut album_meta = Map::new();
    if use_metadata {
        album_meta.insert("albumartist".to_string(), json!(album_artist));
        album_meta.insert("album".to_string(), json!(title));
        let date_str = rg.get("first-release-date").and_then(|d| d.as_str()).unwrap_or("");
        album_meta.insert("date".to_string(), json!(if date_str.len() >= 4 { &date_str[..4] } else { "" }));
        if let Some(master) = dg.get("master") {
            if let Some(g) = master.get("genres") { album_meta.insert("genre".to_string(), g.clone()); }
            if let Some(s) = master.get("styles") { album_meta.insert("styles".to_string(), s.clone()); }
        }
        album_meta.insert("original_yyyy_mm".to_string(), json!(fmt_yyyy_mm(date_str)));
        if !is_rg {
            if let Some(release) = rel {
                if let Some(c) = release.get("country").and_then(|c| c.as_str()) {
                    album_meta.insert("country".to_string(), json!(c));
                }
                if let Some(label_list) = release.get("label-info").and_then(|l| l.as_array()) {
                    if let Some(node) = label_list.first() {
                        if let Some(n) = node.get("label").and_then(|l| l.get("name")).and_then(|n| n.as_str()) {
                            album_meta.insert("label".to_string(), json!(n));
                        }
                        if let Some(c) = node.get("catalog-number").and_then(|c| c.as_str()) {
                            album_meta.insert("catalognumber".to_string(), json!(c));
                        }
                    }
                }
                let rel_date = release.get("date").and_then(|d| d.as_str()).unwrap_or("");
                album_meta.insert("release_yyyy_mm".to_string(), json!(fmt_yyyy_mm(rel_date)));
            }
        }
    }

    let mut album_mbid = Map::new();
    let mut album_url_map = Map::new();
    if use_mbid || use_url {
        if !is_rg {
            if let Some(id) = rel.and_then(|r| r.get("id")).and_then(|i| i.as_str()) {
                if use_mbid { album_mbid.insert("musicbrainz_albumid".to_string(), json!(id)); }
                if use_url { album_url_map.insert("musicbrainz_release".to_string(), json!(format!("https://musicbrainz.org/release/{}", id))); }
            }
        }
        if let Some(id) = rg.get("id").and_then(|i| i.as_str()) {
            if use_mbid { album_mbid.insert("musicbrainz_releasegroupid".to_string(), json!(id)); }
            if use_url { album_url_map.insert("musicbrainz_releasegroup".to_string(), json!(format!("https://musicbrainz.org/release-group/{}", id))); }
        }
        if let Some(id) = rg.get("artist-credit").and_then(|a| a.as_array()).and_then(|a| a.first()).and_then(|c| c.get("artist")).and_then(|a| a.get("id")).and_then(|i| i.as_str()) {
            if use_mbid { album_mbid.insert("musicbrainz_albumartistid".to_string(), json!(id)); }
        }
        if is_rg {
            if let Some(id) = dg.get("master").and_then(|m| m.get("id")) {
                if use_url { album_url_map.insert("discogs".to_string(), json!(format!("https://discogs.com/master/{}", id))); }
            }
        } else {
            if let Some(id) = dg.get("release").and_then(|r| r.get("id")) {
                if use_url { album_url_map.insert("discogs".to_string(), json!(format!("https://discogs.com/release/{}", id))); }
            }
        }
        if !is_rg {
            if let Some(ctdb) = get_ctdb_id(&PathBuf::from(".")) {
                if use_url { album_url_map.insert("ctdbtocid".to_string(), json!(format!("http://db.cuetools.net/?tocid={}", ctdb))); }
            }
        }
    }

    let mut remote_tracks = Vec::new();
    if !is_rg {
        if let Some(release) = rel {
            if let Some(media) = release.get("media").and_then(|m| m.as_array()) {
                for medium in media {
                    let disc_num = medium.get("position").and_then(|p| p.as_u64()).unwrap_or(1) as u32;
                    let track_list = medium.get("tracks").or_else(|| medium.get("track")).and_then(|t| t.as_array());
                    if let Some(tracks) = track_list {
                        for track in tracks {
                            let t_num = track.get("number").and_then(|n| n.as_str()).and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
                            let t_title = track.get("title").and_then(|t| t.as_str())
                                .or_else(|| track.get("recording").and_then(|r| r.get("title")).and_then(|t| t.as_str())).unwrap_or("Untitled");
                            let t_artist = join_artists(track.get("artist-credit").or_else(|| track.get("recording").and_then(|r| r.get("artist-credit"))));
                            let mbid_t = track.get("recording").and_then(|r| r.get("id")).and_then(|i| i.as_str()).unwrap_or("");
                            let mbid_r = track.get("id").and_then(|i| i.as_str()).unwrap_or("");
                            let mbid_a = track.get("artist-credit").or_else(|| track.get("recording").and_then(|r| r.get("artist-credit")))
                                .and_then(|a| a.as_array()).and_then(|a| a.first()).and_then(|c| c.get("artist")).and_then(|a| a.get("id")).and_then(|i| i.as_str()).unwrap_or("");
                            remote_tracks.push(TrackData {
                                tracknumber: t_num,
                                discnumber: disc_num,
                                title: t_title.to_string(),
                                artist: t_artist,
                                musicbrainz_trackid: mbid_t.to_string(),
                                musicbrainz_releasetrackid: mbid_r.to_string(),
                                musicbrainz_artistid: mbid_a.to_string(),
                            });
                        }
                    }
                }
            }
        }
    } else if let Some(master) = dg.get("master") {
        if let Some(tracklist) = master.get("tracklist").and_then(|t| t.as_array()) {
            let tracks: Vec<&Value> = tracklist.iter().filter(|t| t.get("type_").and_then(|ty| ty.as_str()) == Some("track")).collect();
            for (i, track) in tracks.iter().enumerate() {
                remote_tracks.push(TrackData {
                    tracknumber: (i + 1) as u32,
                    discnumber: 1,
                    title: track.get("title").and_then(|t| t.as_str()).unwrap_or("Untitled").to_string(),
                    artist: String::new(),
                    musicbrainz_trackid: String::new(),
                    musicbrainz_releasetrackid: String::new(),
                    musicbrainz_artistid: String::new(),
                });
            }
        }
    }

    let pname_base = if album_artist.is_empty() { title.to_lowercase() } else { format!("{}-{}", album_artist.to_lowercase(), title.to_lowercase()) };
    let sanitized_pname = pname_base.chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect::<String>().split('-').filter(|s| !s.is_empty()).collect::<Vec<_>>().join("-");

    let mut out = String::new();
    let _ = writeln!(out, "{{ vellum }}:\n");
    let _ = writeln!(out, "vellum.mkAlbum {{\n");
    let _ = writeln!(out, "  pname = \"{sanitized_pname}\";\n");
    let _ = writeln!(out, "  sourceDisk = {{");
    let _ = writeln!(out, "    hash = \"sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=\";");
    let _ = writeln!(out, "  }};\n");
    let _ = writeln!(out, "  sourceTorrent = {{");
    let _ = writeln!(out, "    file = {torrent_nix_path};");
    let _ = writeln!(out, "    hash = \"{torrent_hash}\";");
    let _ = writeln!(out, "  }};\n");
    let _ = writeln!(out, "  cover = {{");
    let _ = writeln!(out, "    file = ./cover.png;");
    let _ = writeln!(out, "    hash = \"{cover_hash}\";");
    let _ = writeln!(out, "  }};\n");
    let _ = writeln!(out, "  album = {{");
    let _ = writeln!(out, "    metadata = {{");
    let _ = write!(out, "{}", to_nix_attributes(&Value::Object(album_meta), "      "));
    let _ = writeln!(out, "    }};");
    let _ = writeln!(out, "    mbid = {{");
    let _ = write!(out, "{}", to_nix_attributes(&Value::Object(album_mbid), "      "));
    let _ = writeln!(out, "    }};");
    let _ = writeln!(out, "    url = {{");
    let _ = write!(out, "{}", to_nix_attributes(&Value::Object(album_url_map), "      "));
    let _ = writeln!(out, "    }};\n  }};\n");
    let _ = writeln!(out, "  tracks = [");
    let total_count = std::cmp::max(valid_paths.len(), remote_tracks.len());
    for i in 0..total_count {
        let file_path = valid_paths.get(i).map_or_else(String::new, |path_buf| {
            let inner_path_str = path_buf.to_string_lossy();
            if torrent.files.is_some() { format!("{}/{}", torrent.name, inner_path_str) } else { inner_path_str.to_string() }
        });
        let mut track_meta = Map::new();
        let mut track_mbid = Map::new();
        if let Some(t_data) = remote_tracks.get(i) {
            if use_metadata {
                track_meta.insert("tracknumber".to_string(), json!(t_data.tracknumber));
                if t_data.discnumber > 1 || remote_tracks.iter().any(|t| t.discnumber > 1) { track_meta.insert("discnumber".to_string(), json!(t_data.discnumber)); }
                track_meta.insert("title".to_string(), json!(t_data.title));
                if !t_data.artist.is_empty() && t_data.artist != album_artist { track_meta.insert("artist".to_string(), json!(t_data.artist)); }
            }
            if use_mbid {
                if !t_data.musicbrainz_trackid.is_empty() { track_mbid.insert("musicbrainz_trackid".to_string(), json!(t_data.musicbrainz_trackid)); }
                if !t_data.musicbrainz_releasetrackid.is_empty() { track_mbid.insert("musicbrainz_releasetrackid".to_string(), json!(t_data.musicbrainz_releasetrackid)); }
                if !t_data.musicbrainz_artistid.is_empty() { track_mbid.insert("musicbrainz_artistid".to_string(), json!(t_data.musicbrainz_artistid)); }
            }
        }
        let _ = writeln!(out, "    {{");
        let _ = writeln!(out, "      file = \"{file_path}\";");
        let _ = writeln!(out, "      metadata = {{");
        let _ = write!(out, "{}", to_nix_attributes(&Value::Object(track_meta), "        "));
        let _ = writeln!(out, "      }};");
        let _ = writeln!(out, "      mbid = {{");
        let _ = write!(out, "{}", to_nix_attributes(&Value::Object(track_mbid), "        "));
        let _ = writeln!(out, "      }};");
        let _ = write!(out, "    }}");
        if i < total_count - 1 { let _ = writeln!(out, ","); } else { let _ = writeln!(out); }
    }
    let _ = writeln!(out, "  ];\n}}");
    println!("{out}");
    Ok(())
}

fn to_nix_attributes(val: &Value, indent: &str) -> String {
    let mut res = String::new();
    if let Some(tab) = val.as_object() {
        for (k, v) in tab {
            let _ = writeln!(res, "{indent}{k} = {};", to_nix_value(v));
        }
    }
    res
}

fn to_nix_value(val: &Value) -> String {
    match val {
        Value::String(s) => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Array(arr) => {
            let mut items = vec![];
            for v in arr {
                items.push(to_nix_value(v));
            }
            format!("[ {} ]", items.join(" "))
        }
        _ => "\"\"".to_string(),
    }
}
