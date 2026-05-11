use crate::utils::fmt_yyyy_mm;
use serde_json::{json, Value};

use super::TrackData;

pub struct AlbumContext<'a> {
    pub rg: &'a Value,
    pub rel: Option<&'a Value>,
    pub dg: &'a Value,
    pub is_rg: bool,
    pub ctdb: Option<String>,
    pub album_artist: &'a str,
}

pub struct TrackContext<'a> {
    pub track: &'a TrackData,
    pub has_multiple_discs: bool,
    pub album_artist: &'a str,
}

pub fn resolve_album_key(key: &str, ctx: &AlbumContext) -> Option<Value> {
    match key {
        "albumartist" => Some(albumartist(ctx)),
        "album" => album(ctx),
        "date" => Some(date(ctx)),
        "genre" => genre(ctx),
        "styles" => styles(ctx),
        "original_yyyy_mm" => Some(original_yyyy_mm(ctx)),
        "country" => country(ctx),
        "label" => label(ctx),
        "catalognumber" => catalognumber(ctx),
        "release_yyyy_mm" => release_yyyy_mm(ctx),
        "musicbrainz_albumid" => musicbrainz_albumid(ctx),
        "musicbrainz_releasegroupid" => musicbrainz_releasegroupid(ctx),
        "musicbrainz_albumartistid" => musicbrainz_albumartistid(ctx),
        "musicbrainz_release" => musicbrainz_release(ctx),
        "musicbrainz_releasegroup" => musicbrainz_releasegroup(ctx),
        "discogs" => discogs(ctx),
        "ctdbtocid" => ctdbtocid(ctx),
        _ => None,
    }
}

pub fn resolve_track_key(key: &str, ctx: &TrackContext) -> Option<Value> {
    match key {
        "tracknumber" => Some(tracknumber(ctx)),
        "discnumber" => discnumber(ctx),
        "title" => Some(title(ctx)),
        "artist" => artist(ctx),
        "musicbrainz_trackid" => musicbrainz_trackid(ctx),
        "musicbrainz_releasetrackid" => musicbrainz_releasetrackid(ctx),
        "musicbrainz_artistid" => musicbrainz_artistid(ctx),
        _ => None,
    }
}

fn albumartist(ctx: &AlbumContext) -> Value {
    json!(ctx.album_artist)
}

fn album(ctx: &AlbumContext) -> Option<Value> {
    ctx.rg
        .get("title")
        .and_then(|t| t.as_str())
        .map(|t| json!(t))
}

fn date(ctx: &AlbumContext) -> Value {
    let date_str = ctx
        .rg
        .get("first-release-date")
        .and_then(|d| d.as_str())
        .unwrap_or("");
    json!(if date_str.len() >= 4 {
        &date_str[..4]
    } else {
        ""
    })
}

fn genre(ctx: &AlbumContext) -> Option<Value> {
    ctx.dg.get("master").and_then(|m| m.get("genres")).cloned()
}

fn styles(ctx: &AlbumContext) -> Option<Value> {
    ctx.dg.get("master").and_then(|m| m.get("styles")).cloned()
}

fn original_yyyy_mm(ctx: &AlbumContext) -> Value {
    let date_str = ctx
        .rg
        .get("first-release-date")
        .and_then(|d| d.as_str())
        .unwrap_or("");
    json!(fmt_yyyy_mm(date_str))
}

fn country(ctx: &AlbumContext) -> Option<Value> {
    if !ctx.is_rg {
        ctx.rel
            .and_then(|r| r.get("country"))
            .and_then(|c| c.as_str())
            .map(|c| json!(c))
    } else {
        None
    }
}

fn label(ctx: &AlbumContext) -> Option<Value> {
    if !ctx.is_rg {
        ctx.rel
            .and_then(|r| r.get("label-info"))
            .and_then(|l| l.as_array())
            .and_then(|a| a.first())
            .and_then(|n| n.get("label"))
            .and_then(|l| l.get("name"))
            .and_then(|n| n.as_str())
            .map(|n| json!(n))
    } else {
        None
    }
}

fn catalognumber(ctx: &AlbumContext) -> Option<Value> {
    if !ctx.is_rg {
        ctx.rel
            .and_then(|r| r.get("label-info"))
            .and_then(|l| l.as_array())
            .and_then(|a| a.first())
            .and_then(|n| n.get("catalog-number"))
            .and_then(|c| c.as_str())
            .map(|c| json!(c))
    } else {
        None
    }
}

fn release_yyyy_mm(ctx: &AlbumContext) -> Option<Value> {
    if !ctx.is_rg {
        let rel_date = ctx
            .rel
            .and_then(|r| r.get("date"))
            .and_then(|d| d.as_str())
            .unwrap_or("");
        Some(json!(fmt_yyyy_mm(rel_date)))
    } else {
        None
    }
}

fn musicbrainz_albumid(ctx: &AlbumContext) -> Option<Value> {
    if !ctx.is_rg {
        ctx.rel
            .and_then(|r| r.get("id"))
            .and_then(|i| i.as_str())
            .map(|i| json!(i))
    } else {
        None
    }
}

fn musicbrainz_releasegroupid(ctx: &AlbumContext) -> Option<Value> {
    ctx.rg
        .get("id")
        .and_then(|i| i.as_str())
        .map(|i| json!(i))
}

fn musicbrainz_albumartistid(ctx: &AlbumContext) -> Option<Value> {
    ctx.rg
        .get("artist-credit")
        .and_then(|a| a.as_array())
        .and_then(|a| a.first())
        .and_then(|c| c.get("artist"))
        .and_then(|a| a.get("id"))
        .and_then(|i| i.as_str())
        .map(|i| json!(i))
}

fn musicbrainz_release(ctx: &AlbumContext) -> Option<Value> {
    if !ctx.is_rg {
        ctx.rel
            .and_then(|r| r.get("id"))
            .and_then(|i| i.as_str())
            .map(|i| json!(format!("https://musicbrainz.org/release/{}", i)))
    } else {
        None
    }
}

fn musicbrainz_releasegroup(ctx: &AlbumContext) -> Option<Value> {
    ctx.rg
        .get("id")
        .and_then(|i| i.as_str())
        .map(|i| json!(format!("https://musicbrainz.org/release-group/{}", i)))
}

fn discogs(ctx: &AlbumContext) -> Option<Value> {
    if ctx.is_rg {
        ctx.dg
            .get("master")
            .and_then(|m| m.get("id"))
            .map(|id| json!(format!("https://discogs.com/master/{}", id)))
    } else {
        ctx.dg
            .get("release")
            .and_then(|r| r.get("id"))
            .map(|id| json!(format!("https://discogs.com/release/{}", id)))
    }
}

fn ctdbtocid(ctx: &AlbumContext) -> Option<Value> {
    ctx.ctdb
        .as_ref()
        .map(|ctdb| json!(format!("http://db.cuetools.net/?tocid={}", ctdb)))
}

fn tracknumber(ctx: &TrackContext) -> Value {
    json!(ctx.track.tracknumber)
}

fn discnumber(ctx: &TrackContext) -> Option<Value> {
    if ctx.has_multiple_discs {
        Some(json!(ctx.track.discnumber))
    } else {
        None
    }
}

fn title(ctx: &TrackContext) -> Value {
    json!(ctx.track.title)
}

fn artist(ctx: &TrackContext) -> Option<Value> {
    if !ctx.track.artist.is_empty() && ctx.track.artist != ctx.album_artist {
        Some(json!(ctx.track.artist))
    } else {
        None
    }
}

fn musicbrainz_trackid(ctx: &TrackContext) -> Option<Value> {
    if !ctx.track.musicbrainz_trackid.is_empty() {
        Some(json!(ctx.track.musicbrainz_trackid))
    } else {
        None
    }
}

fn musicbrainz_releasetrackid(ctx: &TrackContext) -> Option<Value> {
    if !ctx.track.musicbrainz_releasetrackid.is_empty() {
        Some(json!(ctx.track.musicbrainz_releasetrackid))
    } else {
        None
    }
}

fn musicbrainz_artistid(ctx: &TrackContext) -> Option<Value> {
    if !ctx.track.musicbrainz_artistid.is_empty() {
        Some(json!(ctx.track.musicbrainz_artistid))
    } else {
        None
    }
}
