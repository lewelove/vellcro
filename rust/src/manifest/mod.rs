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

pub struct TrackData {
    pub tracknumber: u32,
    pub discnumber: u32,
    pub title: String,
    pub artist: String,
    pub musicbrainz_trackid: String,
    pub musicbrainz_releasetrackid: String,
    pub musicbrainz_artistid: String,
}

#[derive(Clone, Copy)]
struct ManifestParams<'a> {
    torrent: &'a Torrent,
    torrent_hash: &'a str,
    torrent_nix_path: &'a str,
    cover_hash: &'a str,
    valid_paths: &'a [PathBuf],
    remote_tracks: &'a [TrackData],
    album_ctx: &'a keys::AlbumContext<'a>,
    manifest_cfg: &'a ManifestConfig,
    use_metadata: bool,
    use_mbid: bool,
    use_url: bool,
    album_artist: &'a str,
    title: &'a str,
}

pub fn run(
    mb_url: &str,
    use_metadata: bool,
    use_mbid: bool,
    use_url: bool,
    torrent_path_str: &str,
    tracks_filter: &str,
) -> Result<()> {
    let config = AppConfig::load();
    let manifest_cfg = config.manifest.unwrap_or_default();

    let raw_data = fetch_remote_metadata(mb_url).context("Failed to fetch metadata from URL")?;
    let (torrent, torrent_hash, rel_torrent) = process_torrent(torrent_path_str)?;
    let valid_paths = match_files(&torrent, tracks_filter)?;
    let cover_hash = hash_cover_file()?;

    let is_rg = raw_data.get("_is_rg").and_then(serde_json::Value::as_bool).unwrap_or(false);
    let mb = raw_data.get("musicbrainz").context("Missing musicbrainz data")?;
    let rg = mb.get("release_group").context("Missing release_group data")?;
    let rel = mb.get("release");
    let dg_fallback = json!({});
    let dg = raw_data.get("discogs").unwrap_or(&dg_fallback);

    let album_artist = join_artists(rg.get("artist-credit"));
    let title = rg.get("title").and_then(|t| t.as_str()).unwrap_or("Unknown Title");

    let remote_tracks = extract_remote_tracks(rel, dg, is_rg);

    let ctdb = if !is_rg { get_ctdb_id(&PathBuf::from(".")) } else { None };
    
    let album_ctx = keys::AlbumContext {
        rg,
        rel,
        dg,
        is_rg,
        ctdb,
        album_artist: &album_artist,
    };

    let torrent_nix_path = if rel_torrent.is_absolute() {
        format!("\"{}\"", rel_torrent.to_string_lossy())
    } else {
        format!("./{}", rel_torrent.to_string_lossy())
    };

    let out = generate_nix_manifest(ManifestParams {
        torrent: &torrent,
        torrent_hash: &torrent_hash,
        torrent_nix_path: &torrent_nix_path,
        cover_hash: &cover_hash,
        valid_paths: &valid_paths,
        remote_tracks: &remote_tracks,
        album_ctx: &album_ctx,
        manifest_cfg: &manifest_cfg,
        use_metadata,
        use_mbid,
        use_url,
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

fn match_files(torrent: &Torrent, tracks_filter: &str) -> Result<Vec<PathBuf>> {
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
            "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string()
        }
    } else {
        "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string()
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

fn has_keys_for_level(map: Option<&IndexMap<String, ManifestKeyConfig>>, target_level: &str) -> bool {
    let Some(map) = map else { return false; };
    map.values().any(|cfg| cfg.level == target_level || cfg.level == format!("{target_level}s"))
}

fn generate_nix_manifest(params: ManifestParams) -> String {
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

    let has_album_meta = params.use_metadata && has_keys_for_level(params.manifest_cfg.metadata.as_ref(), "album");
    let has_album_mbid = params.use_mbid && has_keys_for_level(params.manifest_cfg.mbid.as_ref(), "album");
    let has_album_url = params.use_url && has_keys_for_level(params.manifest_cfg.url.as_ref(), "album");

    let has_track_meta = params.use_metadata && has_keys_for_level(params.manifest_cfg.metadata.as_ref(), "track");
    let has_track_mbid = params.use_mbid && has_keys_for_level(params.manifest_cfg.mbid.as_ref(), "track");
    let has_track_url = params.use_url && has_keys_for_level(params.manifest_cfg.url.as_ref(), "track");

    let mut out = String::new();
    let _ = writeln!(out, "{{ vellum }}:\n");
    let _ = writeln!(out, "vellum.mkAlbum {{\n");
    let _ = writeln!(out, "  pname = \"{sanitized_pname}\";\n");
    let _ = writeln!(out, "  sourceDisk = {{");
    let _ = writeln!(out, "    hash = \"sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=\";");
    let _ = writeln!(out, "  }};\n");
    let _ = writeln!(out, "  sourceTorrent = {{");
    let _ = writeln!(out, "    file = {};", params.torrent_nix_path);
    let _ = writeln!(out, "    hash = \"{}\";", params.torrent_hash);
    let _ = writeln!(out, "  }};\n");
    let _ = writeln!(out, "  cover = {{");
    let _ = writeln!(out, "    file = ./cover.png;");
    let _ = writeln!(out, "    hash = \"{}\";", params.cover_hash);
    let _ = writeln!(out, "  }};\n");

    let _ = writeln!(out, "  album = {{");

    if has_album_meta {
        let _ = writeln!(out, "    metadata = {{");
        render_section(params.manifest_cfg.metadata.as_ref(), "album", Some(params.album_ctx), None, "      ", &mut out);
        let _ = writeln!(out, "    }};");
    }
    if has_album_mbid {
        let _ = writeln!(out, "    mbid = {{");
        render_section(params.manifest_cfg.mbid.as_ref(), "album", Some(params.album_ctx), None, "      ", &mut out);
        let _ = writeln!(out, "    }};");
    }
    if has_album_url {
        let _ = writeln!(out, "    url = {{");
        render_section(params.manifest_cfg.url.as_ref(), "album", Some(params.album_ctx), None, "      ", &mut out);
        let _ = writeln!(out, "    }};");
    }

    let _ = writeln!(out, "  }};\n");
    let _ = writeln!(out, "  tracks = [");

    let total_count = std::cmp::max(params.valid_paths.len(), params.remote_tracks.len());
    let has_multiple_discs = params.remote_tracks.iter().any(|t| t.discnumber > 1);

    for i in 0..total_count {
        let file_path = params.valid_paths.get(i).map_or_else(String::new, |path_buf| {
            let inner_path_str = path_buf.to_string_lossy();
            if params.torrent.files.is_some() {
                format!("{}/{}", params.torrent.name, inner_path_str)
            } else {
                inner_path_str.to_string()
            }
        });

        let _ = writeln!(out, "    {{");
        let _ = writeln!(out, "      file = \"{file_path}\";");

        if let Some(t_data) = params.remote_tracks.get(i) {
            let t_ctx = keys::TrackContext {
                track: t_data,
                has_multiple_discs,
                album_artist: params.album_artist,
            };

            if has_track_meta {
                let _ = writeln!(out, "      metadata = {{");
                render_section(params.manifest_cfg.metadata.as_ref(), "track", None, Some(&t_ctx), "        ", &mut out);
                let _ = writeln!(out, "      }};");
            }
            if has_track_mbid {
                let _ = writeln!(out, "      mbid = {{");
                render_section(params.manifest_cfg.mbid.as_ref(), "track", None, Some(&t_ctx), "        ", &mut out);
                let _ = writeln!(out, "      }};");
            }
            if has_track_url {
                let _ = writeln!(out, "      url = {{");
                render_section(params.manifest_cfg.url.as_ref(), "track", None, Some(&t_ctx), "        ", &mut out);
                let _ = writeln!(out, "      }};");
            }
        } else {
            if has_track_meta { let _ = writeln!(out, "      metadata = {{}};"); }
            if has_track_mbid { let _ = writeln!(out, "      mbid = {{}};"); }
            if has_track_url { let _ = writeln!(out, "      url = {{}};"); }
        }

        let _ = writeln!(out, "    }}");
    }
    let _ = writeln!(out, "  ];\n}}");
    out
}

fn render_section(
    map: Option<&IndexMap<String, ManifestKeyConfig>>,
    target_level: &str,
    album_ctx: Option<&keys::AlbumContext>,
    track_ctx: Option<&keys::TrackContext>,
    indent: &str,
    out: &mut String,
) {
    let Some(map) = map else { return; };
    for (key, cfg) in map {
        if cfg.level == target_level || cfg.level == format!("{target_level}s") {
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
