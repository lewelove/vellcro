pub mod keys;

use anyhow::{Context, Result};
use globset::{Glob, GlobSetBuilder};
use indexmap::IndexMap;
use lava_torrent::torrent::v1::Torrent;
use serde_json::{json, Value};
use std::fmt::Write as FmtWrite;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::{AppConfig, ManifestConfig, ManifestKeyConfig};
use crate::discid::get_ctdb_id;
use crate::remote::fetch_remote_metadata;
use crate::utils::join_artists;

const FAKE_HASH: &str = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

pub struct TrackData {
    pub tracknumber: u32,
    pub discnumber: u32,
    pub title: String,
    pub artist: String,
    pub musicbrainz_trackid: String,
    pub musicbrainz_releasetrackid: String,
    pub musicbrainz_artistid: String,
}

pub enum SourceType {
    Torrent {
        nix_path: String,
        hash: String,
    },
    Disk {
        nix_path: String,
    },
}

struct ManifestParams<'a> {
    source_type: SourceType,
    cover_hash: &'a str,
    valid_paths: &'a [String],
    remote_tracks: &'a [TrackData],
    album_ctx: &'a keys::AlbumContext<'a>,
    manifest_cfg: &'a ManifestConfig,
    active_flags: &'a [String],
    album_artist: &'a str,
    title: &'a str,
}

pub fn run(
    mb_url: &str,
    flags: Option<&str>,
    torrent: Option<&str>,
    disk: Option<&str>,
    tracks_filter: &str,
) -> Result<()> {
    let config = AppConfig::load();
    let manifest_cfg = config.manifest.unwrap_or_default();

    let raw_data = fetch_remote_metadata(mb_url).context("Failed to fetch metadata from URL")?;
    let cover_hash = hash_cover_file()?;

    let (source_type, valid_paths, base_dir_for_ctdb) = if let Some(t_path) = torrent {
        let (torrent_struct, torrent_hash, rel_torrent) = process_torrent(t_path)?;
        let paths = match_torrent_files(&torrent_struct, tracks_filter)?;
        let nix_path = if rel_torrent.is_absolute() {
            format!("\"{}\"", rel_torrent.to_string_lossy())
        } else {
            format!("./{}", rel_torrent.to_string_lossy())
        };
        (SourceType::Torrent { nix_path, hash: torrent_hash }, paths, PathBuf::from("."))
    } else {
        let d_path_str = disk.unwrap_or(".");
        let d_path = Path::new(d_path_str).canonicalize()?;
        let current_dir = std::env::current_dir()?;

        let rel_disk = d_path
            .strip_prefix(&current_dir)
            .map_or_else(|_| d_path.clone(), Path::to_path_buf);

        let disk_nix_path = if rel_disk.is_absolute() {
            format!("\"{}\"", rel_disk.to_string_lossy())
        } else {
            let s = rel_disk.to_string_lossy();
            if s.is_empty() {
                "./.".to_string()
            } else {
                format!("./{s}")
            }
        };

        let paths = match_disk_files(&d_path, tracks_filter)?;

        (SourceType::Disk { nix_path: disk_nix_path }, paths, d_path)
    };

    let is_rg = raw_data.get("_is_rg").and_then(serde_json::Value::as_bool).unwrap_or(false);
    let mb = raw_data.get("musicbrainz").context("Missing musicbrainz data")?;
    let rg = mb.get("release_group").context("Missing release_group data")?;
    let rel = mb.get("release");
    let dg_fallback = json!({});
    let dg = raw_data.get("discogs").unwrap_or(&dg_fallback);

    let album_artist = join_artists(rg.get("artist-credit"));
    let title = rg.get("title").and_then(|t| t.as_str()).unwrap_or("Unknown Title");

    let remote_tracks = extract_remote_tracks(rel, dg, is_rg);

    let ctdb = if !is_rg { get_ctdb_id(&base_dir_for_ctdb) } else { None };
    
    let album_ctx = keys::AlbumContext {
        rg,
        rel,
        dg,
        is_rg,
        ctdb,
        album_artist: &album_artist,
    };

    let active_flags: Vec<String> = flags.map_or_else(
        || vec!["metadata".to_string()],
        |f_str| {
            f_str
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        },
    );

    let out = generate_nix_manifest(&ManifestParams {
        source_type,
        cover_hash: &cover_hash,
        valid_paths: &valid_paths,
        remote_tracks: &remote_tracks,
        album_ctx: &album_ctx,
        manifest_cfg: &manifest_cfg,
        active_flags: &active_flags,
        album_artist: &album_artist,
        title,
    });

    println!("{out}");
    Ok(())
}

fn process_torrent(torrent_path_str: &str) -> Result<(Torrent, String, PathBuf)> {
    let torrent_path = Path::new(torrent_path_str).canonicalize()?;
    let current_dir = std::env::current_dir()?;

    let output = Command::new("nix")
        .args(["hash", "file", torrent_path.to_str().unwrap()])
        .output()?;

    if !output.status.success() {
        anyhow::bail!("Failed to hash torrent file with nix");
    }
    let torrent_hash = String::from_utf8(output.stdout)?.trim().to_string();
    let torrent = Torrent::read_from_file(&torrent_path).context("Failed to parse torrent")?;

    let rel_torrent = torrent_path
        .strip_prefix(&current_dir)
        .map_or_else(|_| torrent_path.clone(), Path::to_path_buf);

    Ok((torrent, torrent_hash, rel_torrent))
}

fn build_globset(tracks_filter: &str) -> Result<globset::GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for part in tracks_filter.split(',') {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        let pattern = if !trimmed.contains('/') && !trimmed.contains('*') && !trimmed.contains('?') {
            format!("**/*.{}", trimmed.trim_start_matches('.'))
        } else {
            trimmed.to_string()
        };
        builder.add(Glob::new(&pattern)?);
    }
    Ok(builder.build()?)
}

fn match_torrent_files(torrent: &Torrent, tracks_filter: &str) -> Result<Vec<String>> {
    let globset = build_globset(tracks_filter)?;
    let mut valid_paths = Vec::new();
    if let Some(files) = &torrent.files {
        for f in files {
            if globset.is_match(f.path.to_string_lossy().as_ref()) {
                valid_paths.push(format!("{}/{}", torrent.name, f.path.to_string_lossy()));
            }
        }
    } else {
        let name_str = &torrent.name;
        if globset.is_match(name_str) {
            valid_paths.push(name_str.clone());
        }
    }
    valid_paths.sort_by(|a, b| alphanumeric_sort::compare_path(Path::new(a), Path::new(b)));
    Ok(valid_paths)
}

fn match_disk_files(disk_path: &Path, tracks_filter: &str) -> Result<Vec<String>> {
    let globset = build_globset(tracks_filter)?;
    let mut valid_paths = Vec::new();
    if disk_path.is_file() {
        let name = disk_path.file_name().unwrap_or_default().to_string_lossy();
        if globset.is_match(name.as_ref()) {
            valid_paths.push(name.to_string());
        }
    } else {
        for f in crate::utils::walk_dir(disk_path, disk_path)? {
            let s = f.to_string_lossy();
            if globset.is_match(s.as_ref()) {
                valid_paths.push(s.to_string());
            }
        }
    }
    valid_paths.sort_by(|a, b| alphanumeric_sort::compare_path(Path::new(a), Path::new(b)));
    Ok(valid_paths)
}

fn hash_cover_file() -> Result<String> {
    let cover_file_path = Path::new("cover.png");
    let cover_hash = if cover_file_path.exists() {
        let out = Command::new("nix")
            .args(["hash", "file", "cover.png"])
            .output()?;
        if out.status.success() {
            String::from_utf8(out.stdout)?.trim().to_string()
        } else {
            FAKE_HASH.to_string()
        }
    } else {
        FAKE_HASH.to_string()
    };
    Ok(cover_hash)
}

fn extract_remote_tracks(rel: Option<&Value>, dg: &Value, is_rg: bool) -> Vec<TrackData> {
    let mut remote_tracks = Vec::new();
    if !is_rg {
        if let Some(release) = rel
            && let Some(media) = release.get("media").and_then(|m| m.as_array()) {
                for medium in media {
                    let disc_num = medium.get("position").and_then(serde_json::Value::as_u64).unwrap_or(1) as u32;
                    let track_list = medium
                        .get("tracks")
                        .or_else(|| medium.get("track"))
                        .and_then(|t| t.as_array());
                    if let Some(tracks) = track_list {
                        for track in tracks {
                            let t_num = track
                                .get("number")
                                .and_then(|n| n.as_str())
                                .and_then(|s| s.parse::<u32>().ok())
                                .unwrap_or(0);
                            let t_title = track
                                .get("title")
                                .and_then(|t| t.as_str())
                                .or_else(|| {
                                    track.get("recording").and_then(|r| r.get("title")).and_then(|t| t.as_str())
                                })
                                .unwrap_or("Untitled");
                            let t_artist = join_artists(track.get("artist-credit").or_else(|| {
                                track.get("recording").and_then(|r| r.get("artist-credit"))
                            }));
                            let mbid_t = track
                                .get("recording")
                                .and_then(|r| r.get("id"))
                                .and_then(|i| i.as_str())
                                .unwrap_or("");
                            let mbid_r = track.get("id").and_then(|i| i.as_str()).unwrap_or("");
                            let mbid_a = track
                                .get("artist-credit")
                                .or_else(|| {
                                    track.get("recording").and_then(|r| r.get("artist-credit"))
                                })
                                .and_then(|a| a.as_array())
                                .and_then(|a| a.first())
                                .and_then(|c| c.get("artist"))
                                .and_then(|a| a.get("id"))
                                .and_then(|i| i.as_str())
                                .unwrap_or("");
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
    } else if let Some(master) = dg.get("master")
        && let Some(tracklist) = master.get("tracklist").and_then(|t| t.as_array()) {
            let tracks: Vec<&Value> = tracklist
                .iter()
                .filter(|t| t.get("type_").and_then(|ty| ty.as_str()) == Some("track"))
                .collect();
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
    remote_tracks
}

fn has_keys_for_level_and_flag(map: Option<&IndexMap<String, ManifestKeyConfig>>, target_level: &str, target_flag: &str) -> bool {
    let Some(map) = map else { return false; };
    map.values().any(|cfg| {
        let lvl_match = cfg.level == target_level || cfg.level == format!("{target_level}s");
        let flag_match = cfg.flag == target_flag;
        lvl_match && flag_match
    })
}

fn generate_nix_manifest(params: &ManifestParams) -> String {
    let pname_base = if params.album_artist.is_empty() {
        params.title.to_lowercase()
    } else {
        format!("{}-{}", params.album_artist.to_lowercase(), params.title.to_lowercase())
    };
    let sanitized_pname = pname_base
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");

    let mut out = String::new();
    let _ = writeln!(out, "{{ vellum }}:\n");
    let _ = writeln!(out, "vellum.mkAlbum {{\n");
    let _ = writeln!(out, "  pname = \"{sanitized_pname}\";\n");

    match &params.source_type {
        SourceType::Disk { nix_path } => {
            let _ = writeln!(out, "  sourceDisk = {{");
            let _ = writeln!(out, "    file = {nix_path};");
            let _ = writeln!(out, "    hash = \"{FAKE_HASH}\";");
            let _ = writeln!(out, "  }};\n");
        }
        SourceType::Torrent { nix_path, hash } => {
            let _ = writeln!(out, "  sourceDisk = {{");
            let _ = writeln!(out, "    hash = \"{FAKE_HASH}\";");
            let _ = writeln!(out, "  }};\n");
            let _ = writeln!(out, "  sourceTorrent = {{");
            let _ = writeln!(out, "    file = {nix_path};");
            let _ = writeln!(out, "    hash = \"{hash}\";");
            let _ = writeln!(out, "  }};\n");
        }
    }

    let _ = writeln!(out, "  cover = {{");
    let _ = writeln!(out, "    file = ./cover.png;");
    let _ = writeln!(out, "    hash = \"{}\";", params.cover_hash);
    let _ = writeln!(out, "  }};\n");

    let _ = writeln!(out, "  album = {{");

    for flag in params.active_flags {
        if has_keys_for_level_and_flag(params.manifest_cfg.keys.as_ref(), "album", flag) {
            let _ = writeln!(out, "    {flag} = {{");
            render_section(params.manifest_cfg.keys.as_ref(), "album", flag, Some(params.album_ctx), None, "      ", &mut out);
            let _ = writeln!(out, "    }};");
        }
    }

    let _ = writeln!(out, "  }};\n");
    let _ = writeln!(out, "  tracks = [");

    let total_count = std::cmp::max(params.valid_paths.len(), params.remote_tracks.len());
    let has_multiple_discs = params.remote_tracks.iter().any(|t| t.discnumber > 1);

    for i in 0..total_count {
        let file_path = params.valid_paths.get(i).cloned().unwrap_or_default();

        let _ = writeln!(out, "    {{");
        let _ = writeln!(out, "      file = \"{file_path}\";");

        if let Some(t_data) = params.remote_tracks.get(i) {
            let t_ctx = keys::TrackContext {
                track: t_data,
                has_multiple_discs,
                album_artist: params.album_artist,
            };

            for flag in params.active_flags {
                if has_keys_for_level_and_flag(params.manifest_cfg.keys.as_ref(), "track", flag) {
                    let _ = writeln!(out, "      {flag} = {{");
                    render_section(params.manifest_cfg.keys.as_ref(), "track", flag, None, Some(&t_ctx), "        ", &mut out);
                    let _ = writeln!(out, "      }};");
                }
            }
        } else {
            for flag in params.active_flags {
                if has_keys_for_level_and_flag(params.manifest_cfg.keys.as_ref(), "track", flag) {
                    let _ = writeln!(out, "      {flag} = {{}};");
                }
            }
        }

        let _ = writeln!(out, "    }}");
    }
    let _ = writeln!(out, "  ];\n}}");
    out
}

fn render_section(
    map: Option<&IndexMap<String, ManifestKeyConfig>>,
    target_level: &str,
    target_flag: &str,
    album_ctx: Option<&keys::AlbumContext>,
    track_ctx: Option<&keys::TrackContext>,
    indent: &str,
    out: &mut String,
) {
    let Some(map) = map else { return; };
    for (key, cfg) in map {
        let lvl_match = cfg.level == target_level || cfg.level == format!("{target_level}s");
        let flag_match = cfg.flag == target_flag;

        if lvl_match && flag_match {
            let val = album_ctx.map_or_else(
                || track_ctx.and_then(|ctx| keys::resolve_track_key(key, ctx)),
                |ctx| keys::resolve_album_key(key, ctx)
            );

            if let Some(v) = val {
                if cfg.newline {
                    let _ = writeln!(out);
                }
                let _ = writeln!(out, "{indent}{key} = {};", to_nix_value(&v));
            }
        }
    }
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
