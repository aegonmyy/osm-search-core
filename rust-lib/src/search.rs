//! The search feature: a per-region SQLite index over named OSM features,
//! built from the region PBF, and the queries that run against it.
//!
//! One artifact per region: `<region>.search.db` (FTS5 for names, r-tree
//! for space, a table of admin rings for containment). It is a file, not a
//! service — the SDK opens it, runs the query, closes it. Building the
//! index is a background job (a 700 MB PBF takes minutes); querying is
//! milliseconds.
//!
//! Design contract (from the task handoff, settled with the user):
//! - search is local-only: queries never touch the network;
//! - fetching a region is always an explicit user decision — nothing in
//!   this module downloads anything;
//! - a miss reports what was searched and offers suggestions, and the
//!   wording never claims a result exists in a region that is not indexed.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use rusqlite::Connection;

use crate::pbf::{scan_pbf, MemberKind, PbfVisitor, RelationEntry, ScanOptions, ScanStats};
use crate::region_bbox;

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

/// The search index directory under the cache dir.
pub fn search_dir(cache_dir: &Path) -> PathBuf {
    cache_dir.join("search")
}

/// The index file for a region.
pub fn index_path(cache_dir: &Path, region: &str) -> PathBuf {
    search_dir(cache_dir).join(format!("{region}.search.db"))
}

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS places (
    id INTEGER PRIMARY KEY,
    osm_type TEXT NOT NULL,
    osm_id INTEGER NOT NULL,
    name TEXT NOT NULL,
    class TEXT NOT NULL,
    subclass TEXT NOT NULL,
    importance INTEGER NOT NULL DEFAULT 0,
    population INTEGER,
    lat REAL NOT NULL,
    lon REAL NOT NULL,
    min_lon REAL NOT NULL,
    min_lat REAL NOT NULL,
    max_lon REAL NOT NULL,
    max_lat REAL NOT NULL,
    admin_path TEXT NOT NULL DEFAULT '[]'
);
CREATE VIRTUAL TABLE IF NOT EXISTS places_fts USING fts5(
    text, content='', tokenize='unicode61'
);
CREATE VIRTUAL TABLE IF NOT EXISTS places_rt USING rtree(
    id, min_lon, max_lon, min_lat, max_lat
);
CREATE TABLE IF NOT EXISTS admin_areas (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    admin_level INTEGER NOT NULL,
    ring TEXT NOT NULL,
    min_lon REAL NOT NULL,
    min_lat REAL NOT NULL,
    max_lon REAL NOT NULL,
    max_lat REAL NOT NULL
);
CREATE INDEX IF NOT EXISTS admin_level_idx ON admin_areas(admin_level);
";

/// Open an index read-only, checking it exists.
pub fn open_index(path: &Path) -> Result<Connection> {
    if !path.exists() {
        bail!(
            "no search index at {} — build one with the search_build op",
            path.display()
        );
    }
    let conn = Connection::open(path)
        .with_context(|| format!("opening search index {}", path.display()))?;
    Ok(conn)
}

// ---------------------------------------------------------------------------
// Index build
// ---------------------------------------------------------------------------

/// What the build produced.
#[derive(Debug, Clone, serde::Serialize)]
pub struct IndexStats {
    pub region: String,
    pub db_path: String,
    pub places: u64,
    pub admin_areas: u64,
    pub highways: u64,
    pub houses: u64,
    pub pois: u64,
    pub took_secs: f64,
    pub bytes: u64,
    pub pbf_blobs: u64,
}

/// Build the index for `region` from `pbf` into `db_path`.
///
/// Two passes over the file: the first collects named ways and admin
/// relations (skipping node decoding), the second reads node coordinates
/// for the ways that need them and every named node feature. Admin rings
/// are assembled from relation member ways, simplified, and stored; each
/// place's admin path is resolved by point-in-polygon against them.
pub fn build_index(region: &str, pbf: &Path, db_path: &Path) -> Result<IndexStats> {
    let started = std::time::Instant::now();
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    // Build into a temp sibling, then rename: a half-built index must
    // never masquerade as a usable one, and a rename is atomic.
    let tmp = db_path.with_extension("search.db.building");
    let _ = std::fs::remove_file(&tmp);
    let _ = std::fs::remove_file(db_path.with_extension("search.db.building-wal"));
    let _ = std::fs::remove_file(db_path.with_extension("search.db.building-shm"));

    let collector = pass_one(pbf)?;
    let mut features = pass_two(pbf, &collector)?;
    let admins = assemble_admins(&collector, &features.node_coords);

    // Resolve admin paths (levels ascending) for every feature.
    for f in features.places.iter_mut() {
        f.admin_path = admin_path_of(&admins, f.lon, f.lat);
    }
    for f in features.way_features.iter_mut() {
        f.admin_path = admin_path_of(&admins, f.lon, f.lat);
    }

    let stats = write_index(region, pbf, db_path, &tmp, &features, &admins, started)?;
    Ok(stats)
}

// ---------------------------------------------------------------------------
// Pass one: named ways + admin relations
// ---------------------------------------------------------------------------

/// Ways worth geometry: named highways and house-number ways.
#[derive(Default)]
struct WaysCollect {
    /// way id -> refs (only for ways we keep).
    ways: HashMap<i64, Vec<i64>>,
}

/// An admin relation: boundary/multipolygon with an admin_level.
#[derive(Debug, Clone)]
struct AdminRel {
    #[allow(dead_code)]
    id: i64,
    name: String,
    admin_level: u8,
    outer: Vec<i64>,
    #[allow(dead_code)]
    inner: Vec<i64>,
}

#[derive(Default)]
struct PassOne {
    ways: WaysCollect,
    /// Named-way metadata kept by pass one (name/class/importance) so
    /// pass two can build real place rows from the geometry it collects.
    way_info: HashMap<i64, WayInfo>,
    admins: Vec<AdminRel>,
    /// All node ids the kept ways reference (coords wanted in pass two).
    wanted: HashSet<i64>,
    blobs: u64,
}

fn pass_one(pbf: &Path) -> Result<PassOne> {
    let mut c = PassOne::default();

    struct V<'a> {
        c: &'a mut PassOne,
    }
    impl PbfVisitor for V<'_> {
        fn way(&mut self, w: crate::pbf::WayEntry) {
            let tag = |key: &str| w.tags.iter().find(|(k, _)| *k == key).map(|(_, v)| *v);
            let name = tag("name");
            let is_highway = tag("highway").is_some();
            let is_house = tag("addr:housenumber").is_some();
            if !((is_highway || is_house) && name.is_some() && !w.refs.is_empty()) {
                return;
            }
            let name = name.unwrap_or_default();
            if is_highway {
                let subclass = tag("highway").unwrap_or("").to_string();
                let importance = match subclass.as_str() {
                    "motorway" | "trunk" => 18,
                    "primary" => 14,
                    "secondary" => 11,
                    "tertiary" => 9,
                    _ => 7,
                };
                self.c.way_info.insert(
                    w.id,
                    WayInfo {
                        name: name.to_string(),
                        class: "highway",
                        subclass,
                        importance,
                    },
                );
            } else {
                let hn = tag("addr:housenumber").unwrap_or("");
                self.c.way_info.insert(
                    w.id,
                    WayInfo {
                        name: format!("{hn} {name}"),
                        class: "house",
                        subclass: "house".into(),
                        importance: 2,
                    },
                );
            }
            for r in &w.refs {
                self.c.wanted.insert(*r);
            }
            self.c.ways.ways.insert(w.id, w.refs);
        }
        fn relation(&mut self, r: RelationEntry) {
            let is_admin = r.tags.iter().any(|(k, v)| {
                (*k == "boundary" && (*v == "administrative" || *v == "political"))
                    || (*k == "type" && *v == "boundary")
            }) || (r
                .tags
                .iter()
                .any(|(k, v)| *k == "type" && *v == "multipolygon")
                && r.tags
                    .iter()
                    .any(|(k, v)| *k == "boundary" && *v == "administrative"));
            if !is_admin {
                return;
            }
            let level = r
                .tags
                .iter()
                .find(|(k, _)| *k == "admin_level")
                .and_then(|(_, v)| v.parse::<u8>().ok());
            let Some(level) = level else { return };
            if !(2..=10).contains(&level) {
                return;
            }
            let name = r
                .tags
                .iter()
                .find(|(k, _)| *k == "name")
                .map(|(_, v)| v.to_string())
                .unwrap_or_default();
            if name.is_empty() {
                return;
            }
            let mut outer = Vec::new();
            let mut inner = Vec::new();
            for (kind, id, role) in &r.members {
                if *kind != MemberKind::Way {
                    continue;
                }
                match *role {
                    "inner" => inner.push(*id),
                    _ => outer.push(*id), // "outer" and untagged members
                }
            }
            if outer.is_empty() {
                return;
            }
            // Every member way's nodes are wanted (ring geometry).
            self.c.admins.push(AdminRel {
                id: r.id,
                name,
                admin_level: level,
                outer,
                inner,
            });
        }
        fn on_blob(&mut self, kind: &str, _offset: u64) {
            if kind == "OSMData" {
                self.c.blobs += 1;
            }
        }
    }

    let mut v = V { c: &mut c };
    let stats: ScanStats = scan_pbf(pbf, &ScanOptions { skip_nodes: true }, &mut v)?;
    // Admin member ways also need their refs; add them.
    let admin_ways: Vec<i64> = c
        .admins
        .iter()
        .flat_map(|a| a.outer.iter().copied())
        .collect();
    for wid in admin_ways {
        // The way may not have been kept by pass one (unnamed boundary
        // segments). A second targeted scan is not worth it: instead, keep
        // ALL ways' refs for admin members by scanning once more only if
        // any admin way is missing. In practice boundary relations are
        // built from ways without names, so this is the common case — do
        // a supplementary pass over ways collecting only the missing ids.
        let _ = wid;
    }
    let missing: HashSet<i64> = c
        .admins
        .iter()
        .flat_map(|a| a.outer.iter().copied())
        .filter(|id| !c.ways.ways.contains_key(id))
        .collect();
    if !missing.is_empty() {
        // Collect the missing admin ways (boundary ways carry no name, so
        // pass one skipped them).
        struct W2<'a> {
            ways: &'a mut HashMap<i64, Vec<i64>>,
            wanted: &'a mut HashSet<i64>,
            missing: &'a HashSet<i64>,
        }
        impl PbfVisitor for W2<'_> {
            fn way(&mut self, w: crate::pbf::WayEntry) {
                if self.missing.contains(&w.id) {
                    for r in &w.refs {
                        self.wanted.insert(*r);
                    }
                    self.ways.insert(w.id, w.refs);
                }
            }
        }
        let mut v2 = W2 {
            ways: &mut c.ways.ways,
            wanted: &mut c.wanted,
            missing: &missing,
        };
        scan_pbf(pbf, &ScanOptions { skip_nodes: true }, &mut v2)?;
    }
    let _ = stats;
    Ok(c)
}

// ---------------------------------------------------------------------------
// Pass two: node features + wanted coords
// ---------------------------------------------------------------------------

/// A feature destined for the places table.
#[derive(Debug, Clone, Default)]
struct PlaceRow {
    osm_type: &'static str,
    osm_id: i64,
    name: String,
    class: &'static str,
    subclass: String,
    importance: i64,
    population: Option<i64>,
    lat: f64,
    lon: f64,
    bbox: (f64, f64, f64, f64),
    admin_path: Vec<String>,
}

#[derive(Default)]
struct PassTwo {
    places: Vec<PlaceRow>,
    way_features: Vec<PlaceRow>,
    node_coords: HashMap<i64, (f64, f64)>,
}

fn pass_two(pbf: &Path, c: &PassOne) -> Result<PassTwo> {
    let mut out = PassTwo::default();
    let wanted = &c.wanted;

    struct V<'a> {
        out: &'a mut PassTwo,
        wanted: &'a HashSet<i64>,
        // Way accumulators for centroid/bbox math.
    }
    impl PbfVisitor for V<'_> {
        fn node(&mut self, n: crate::pbf::NodeEntry) {
            if self.wanted.contains(&n.id) {
                self.out.node_coords.insert(n.id, (n.lon, n.lat));
            }
            let name = n.tags.iter().find(|(k, _)| *k == "name").map(|(_, v)| *v);
            let Some(name) = name else { return };
            let tags = |k: &str| n.tags.iter().find(|(tk, _)| *tk == k).map(|(_, v)| *v);

            let mut row: Option<PlaceRow> = None;
            if let Some(place) = tags("place") {
                row = Some(PlaceRow {
                    osm_type: "node",
                    osm_id: n.id,
                    name: name.to_string(),
                    class: "place",
                    subclass: place.to_string(),
                    importance: place_importance(place, tags("population")),
                    population: tags("population").and_then(|p| p.parse().ok()),
                    lat: n.lat,
                    lon: n.lon,
                    bbox: (n.lon, n.lat, n.lon, n.lat),
                    admin_path: Vec::new(),
                });
            } else if tags("amenity").is_some()
                || tags("shop").is_some()
                || tags("leisure").is_some()
                || tags("tourism").is_some()
            {
                let subclass = tags("amenity")
                    .or_else(|| tags("shop"))
                    .or_else(|| tags("leisure"))
                    .or_else(|| tags("tourism"))
                    .unwrap_or("")
                    .to_string();
                row = Some(PlaceRow {
                    osm_type: "node",
                    osm_id: n.id,
                    name: name.to_string(),
                    class: "poi",
                    subclass,
                    importance: 15,
                    population: None,
                    lat: n.lat,
                    lon: n.lon,
                    bbox: (n.lon, n.lat, n.lon, n.lat),
                    admin_path: Vec::new(),
                });
            } else if let Some(hn) = tags("addr:housenumber") {
                row = Some(PlaceRow {
                    osm_type: "node",
                    osm_id: n.id,
                    name: format!("{hn} {name}"),
                    class: "house",
                    subclass: "house".into(),
                    importance: 2,
                    population: None,
                    lat: n.lat,
                    lon: n.lon,
                    bbox: (n.lon, n.lat, n.lon, n.lat),
                    admin_path: Vec::new(),
                });
            }
            if let Some(r) = row {
                self.out.places.push(r);
            }
        }
    }

    let mut v = V {
        out: &mut out,
        wanted,
    };
    scan_pbf(pbf, &ScanOptions::default(), &mut v)?;

    // Way features: representative point + bbox from refs' coords, with
    // the name/class pass one recorded. Admin-only ways have no info and
    // are skipped (their geometry served the admin rings, not the index).
    for (wid, refs) in &c.ways.ways {
        let Some(info) = c.way_info.get(wid) else {
            continue;
        };
        let coords: Vec<(f64, f64)> = refs
            .iter()
            .filter_map(|id| out.node_coords.get(id).copied())
            .collect();
        if coords.is_empty() {
            continue;
        }
        let mut min_lon = f64::MAX;
        let mut min_lat = f64::MAX;
        let mut max_lon = f64::MIN;
        let mut max_lat = f64::MIN;
        let mut sum_lon = 0.0;
        let mut sum_lat = 0.0;
        for (lon, lat) in &coords {
            min_lon = min_lon.min(*lon);
            min_lat = min_lat.min(*lat);
            max_lon = max_lon.max(*lon);
            max_lat = max_lat.max(*lat);
            sum_lon += lon;
            sum_lat += lat;
        }
        let n = coords.len() as f64;
        out.way_features.push(PlaceRow {
            osm_type: "way",
            osm_id: *wid,
            name: info.name.clone(),
            class: info.class,
            subclass: info.subclass.clone(),
            importance: info.importance,
            population: None,
            lat: sum_lat / n,
            lon: sum_lon / n,
            bbox: (min_lon, min_lat, max_lon, max_lat),
            admin_path: Vec::new(),
        });
    }
    Ok(out)
}

/// Kept from pass one: a named way's tags, so pass two can classify it.
#[derive(Debug, Clone)]
pub(crate) struct WayInfo {
    pub name: String,
    pub class: &'static str,
    pub subclass: String,
    pub importance: i64,
}

// ---------------------------------------------------------------------------
// Admin assembly
// ---------------------------------------------------------------------------

/// A stored admin area.
#[derive(Debug, Clone)]
struct AdminArea {
    name: String,
    admin_level: u8,
    /// Simplified outer ring: [(lon, lat)].
    ring: Vec<(f64, f64)>,
    bbox: (f64, f64, f64, f64),
}

fn assemble_admins(c: &PassOne, coords: &HashMap<i64, (f64, f64)>) -> Vec<AdminArea> {
    let mut out = Vec::new();
    for rel in &c.admins {
        // Chain the outer ways into rings.
        let mut segments: Vec<Vec<(f64, f64)>> = Vec::new();
        for wid in &rel.outer {
            if let Some(refs) = c.ways.ways.get(wid) {
                let pts: Vec<(f64, f64)> = refs
                    .iter()
                    .filter_map(|id| coords.get(id).copied())
                    .collect();
                if pts.len() >= 2 {
                    segments.push(pts);
                }
            }
        }
        for ring in chain_rings(segments) {
            let simplified = simplify_ring(&ring, 0.002);
            if simplified.len() < 4 {
                continue;
            }
            let mut min_lon = f64::MAX;
            let mut min_lat = f64::MAX;
            let mut max_lon = f64::MIN;
            let mut max_lat = f64::MIN;
            for (lon, lat) in &simplified {
                min_lon = min_lon.min(*lon);
                min_lat = min_lat.min(*lat);
                max_lon = max_lon.max(*lon);
                max_lat = max_lat.max(*lat);
            }
            out.push(AdminArea {
                name: rel.name.clone(),
                admin_level: rel.admin_level,
                ring: simplified,
                bbox: (min_lon, min_lat, max_lon, max_lat),
            });
        }
    }
    out
}

/// Chain way segments into closed rings by matching endpoints.
fn chain_rings(segments: Vec<Vec<(f64, f64)>>) -> Vec<Vec<(f64, f64)>> {
    let mut remaining: Vec<Vec<(f64, f64)>> = segments;
    let mut rings = Vec::new();
    while !remaining.is_empty() {
        let mut ring = remaining.remove(0);
        let mut closed = {
            let first = ring.first();
            let last = ring.last();
            first.is_some() && last.is_some() && first == last
        };
        let mut guard = 0;
        while !closed && guard < 10_000 {
            guard += 1;
            let ring_end = *ring.last().expect("ring nonempty");
            let ring_start = *ring.first().expect("ring nonempty");
            // Find a segment whose start or end matches the ring end.
            let mut merged = false;
            for i in 0..remaining.len() {
                let seg = &remaining[i];
                let seg_start = *seg.first().expect("seg nonempty");
                let seg_end = *seg.last().expect("seg nonempty");
                let eps = 1e-9;
                let close = |a: (f64, f64), b: (f64, f64)| -> bool {
                    (a.0 - b.0).abs() < eps && (a.1 - b.1).abs() < eps
                };
                if close(seg_start, ring_end) {
                    let seg_body = seg.iter().skip(1).copied().collect::<Vec<_>>();
                    ring.append(&mut seg_body.clone());
                    remaining.remove(i);
                    merged = true;
                    break;
                }
                if close(seg_end, ring_end) {
                    let seg_body = seg.iter().rev().skip(1).copied().collect::<Vec<_>>();
                    ring.append(&mut seg_body.clone());
                    remaining.remove(i);
                    merged = true;
                    break;
                }
                if close(seg_start, ring_start) {
                    // Prepend reversed.
                    let mut head: Vec<(f64, f64)> = seg.iter().rev().skip(1).copied().collect();
                    head.append(&mut ring);
                    ring = head;
                    remaining.remove(i);
                    merged = true;
                    break;
                }
                if close(seg_end, ring_start) {
                    let mut head: Vec<(f64, f64)> = seg.iter().skip(1).copied().collect();
                    head.append(&mut ring);
                    ring = head;
                    remaining.remove(i);
                    merged = true;
                    break;
                }
            }
            if !merged {
                break;
            }
            let first = ring.first();
            let last = ring.last();
            closed = first.is_some() && last.is_some() && first == last;
        }
        if closed && ring.len() >= 4 {
            rings.push(ring);
        }
    }
    rings
}

/// Douglas-Peucker on a ring (tolerance in degrees).
fn simplify_ring(ring: &[(f64, f64)], tol: f64) -> Vec<(f64, f64)> {
    if ring.len() <= 8 {
        return ring.to_vec();
    }
    let mut keep = vec![false; ring.len()];
    keep[0] = true;
    keep[ring.len() - 1] = true;
    simplify_span(ring, 0, ring.len() - 1, tol, &mut keep);
    ring.iter()
        .enumerate()
        .filter(|(i, _)| keep[*i])
        .map(|(_, p)| *p)
        .collect()
}

fn simplify_span(pts: &[(f64, f64)], lo: usize, hi: usize, tol: f64, keep: &mut [bool]) {
    if hi <= lo + 1 {
        return;
    }
    let (x0, y0) = pts[lo];
    let (x1, y1) = pts[hi];
    let mut max_d = 0.0;
    let mut max_i = lo;
    for (i, p) in pts.iter().enumerate().skip(lo + 1).take(hi - lo - 1) {
        // Perpendicular distance to the chord (in degree space, close
        // enough for simplification purposes).
        let (px, py) = *p;
        let dx = x1 - x0;
        let dy = y1 - y0;
        let len2 = dx * dx + dy * dy;
        let d = if len2 == 0.0 {
            ((px - x0).powi(2) + (py - y0).powi(2)).sqrt()
        } else {
            ((dy * (px - x0) - dx * (py - y0)).abs()) / len2.sqrt()
        };
        if d > max_d {
            max_d = d;
            max_i = i;
        }
    }
    if max_d > tol {
        keep[max_i] = true;
        simplify_span(pts, lo, max_i, tol, keep);
        simplify_span(pts, max_i, hi, tol, keep);
    }
}

/// Point in polygon (ray casting) on a ring of (lon, lat).
fn point_in_ring(lon: f64, lat: f64, ring: &[(f64, f64)]) -> bool {
    let mut inside = false;
    let n = ring.len();
    if n < 3 {
        return false;
    }
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = ring[i];
        let (xj, yj) = ring[j];
        let crosses = (yi > lat) != (yj > lat);
        if crosses {
            let x_at = (xj - xi) * (lat - yi) / (yj - yi) + xi;
            if lon < x_at {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

/// The admin path for a point: names of containing areas, level ascending.
fn admin_path_of(admins: &[AdminArea], lon: f64, lat: f64) -> Vec<String> {
    let mut hits: Vec<(u8, String)> = Vec::new();
    for a in admins {
        let (min_lon, min_lat, max_lon, max_lat) = a.bbox;
        if lon < min_lon || lon > max_lon || lat < min_lat || lat > max_lat {
            continue;
        }
        if point_in_ring(lon, lat, &a.ring) {
            hits.push((a.admin_level, a.name.clone()));
        }
    }
    hits.sort_by_key(|(l, _)| *l);
    let mut names: Vec<String> = Vec::new();
    for (_, n) in hits {
        if !names.contains(&n) {
            names.push(n);
        }
    }
    names
}

// ---------------------------------------------------------------------------
// Importance
// ---------------------------------------------------------------------------

fn place_importance(place: &str, population: Option<&str>) -> i64 {
    let pop: i64 = population.and_then(|p| p.parse().ok()).unwrap_or(0);
    let base = match place {
        "city" => 60,
        "town" => 45,
        "municipality" | "borough" | "suburb" | "quarter" => 35,
        "village" | "hamlet" | "island" => 25,
        "islet" | "isolated_dwelling" | "locality" => 15,
        _ => 20,
    };
    base + (pop / 10_000).min(40)
}

// ---------------------------------------------------------------------------
// Write
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn write_index(
    region: &str,
    pbf: &Path,
    db_path: &Path,
    tmp: &Path,
    features: &PassTwo,
    admins: &[AdminArea],
    started: std::time::Instant,
) -> Result<IndexStats> {
    let md5 = md5_of(pbf)?;
    let conn = Connection::open(tmp)?;
    conn.execute_batch(SCHEMA)?;
    conn.pragma_update(None, "journal_mode", "DELETE")?;
    let tx = conn.unchecked_transaction()?;

    {
        let mut insert_place = conn.prepare(
            "INSERT INTO places (id, osm_type, osm_id, name, class, subclass, importance,
             population, lat, lon, min_lon, min_lat, max_lon, max_lat, admin_path)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )?;
        let mut insert_fts = conn.prepare("INSERT INTO places_fts (rowid, text) VALUES (?, ?)")?;
        let mut insert_rt = conn.prepare(
            "INSERT INTO places_rt (id, min_lon, max_lon, min_lat, max_lat) VALUES (?, ?, ?, ?, ?)",
        )?;
        let mut id: i64 = 0;
        let mut put = |row: &PlaceRow| -> Result<()> {
            id += 1;
            let admin_json = serde_json::to_string(&row.admin_path).unwrap_or_else(|_| "[]".into());
            insert_place.execute(rusqlite::params![
                id,
                row.osm_type,
                row.osm_id,
                row.name,
                row.class,
                row.subclass,
                row.importance,
                row.population,
                row.lat,
                row.lon,
                row.bbox.0,
                row.bbox.1,
                row.bbox.2,
                row.bbox.3,
                admin_json,
            ])?;
            let mut text = row.name.clone();
            for n in &row.admin_path {
                text.push(' ');
                text.push_str(n);
            }
            insert_fts.execute(rusqlite::params![id, text])?;
            insert_rt.execute(rusqlite::params![
                id, row.bbox.0, row.bbox.2, row.bbox.1, row.bbox.3
            ])?;
            Ok(())
        };
        for row in &features.places {
            put(row)?;
        }
        for row in &features.way_features {
            put(row)?;
        }
    }

    {
        let mut insert_admin = conn.prepare(
            "INSERT INTO admin_areas (id, name, admin_level, ring, min_lon, min_lat, max_lon, max_lat)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )?;
        for (i, a) in admins.iter().enumerate() {
            let ring_json = serde_json::to_string(&a.ring)?;
            insert_admin.execute(rusqlite::params![
                i as i64 + 1,
                a.name,
                a.admin_level,
                ring_json,
                a.bbox.0,
                a.bbox.1,
                a.bbox.2,
                a.bbox.3,
            ])?;
        }
    }

    let places = features.places.len() + features.way_features.len();
    let highways = features
        .way_features
        .iter()
        .filter(|f| f.class == "highway")
        .count();
    let houses = features
        .places
        .iter()
        .chain(features.way_features.iter())
        .filter(|f| f.class == "house")
        .count();
    let pois = features.places.iter().filter(|f| f.class == "poi").count();

    conn.prepare("INSERT OR REPLACE INTO meta (key, value) VALUES ('region', ?)")?
        .execute([region])?;
    conn.prepare("INSERT OR REPLACE INTO meta (key, value) VALUES ('pbf_md5', ?)")?
        .execute([hex::encode(md5)])?;
    conn.prepare("INSERT OR REPLACE INTO meta (key, value) VALUES ('built_at', ?)")?
        .execute([format!("{}", now_unix())])?;
    conn.prepare("INSERT OR REPLACE INTO meta (key, value) VALUES ('schema', '1')")?
        .execute([])?;
    conn.prepare("INSERT OR REPLACE INTO meta (key, value) VALUES ('places', ?)")?
        .execute([format!("{places}")])?;
    tx.commit()?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    drop(conn);
    std::fs::rename(tmp, db_path)?;

    Ok(IndexStats {
        region: region.to_string(),
        db_path: db_path.display().to_string(),
        places: places as u64,
        admin_areas: admins.len() as u64,
        highways: highways as u64,
        houses: houses as u64,
        pois: pois as u64,
        took_secs: started.elapsed().as_secs_f64(),
        bytes: std::fs::metadata(db_path).map(|m| m.len()).unwrap_or(0),
        pbf_blobs: 0,
    })
}

fn md5_of(path: &Path) -> Result<[u8; 16]> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut hasher = crate::md5_stream::Md5Stream::new();
    let mut buf = [0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize())
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Queries
// ---------------------------------------------------------------------------

/// One geocode hit.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PlaceHit {
    pub osm_type: String,
    pub osm_id: i64,
    pub name: String,
    pub class: String,
    pub subclass: String,
    pub importance: i64,
    pub lat: f64,
    pub lon: f64,
    pub bbox: (f64, f64, f64, f64),
    pub admin_path: Vec<String>,
    pub region: String,
}

/// Search one index for `query` (already tokenized to prefix terms),
/// returning up to `limit` hits ranked by importance then text rank.
pub fn geocode_in(
    conn: &Connection,
    region: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<PlaceHit>> {
    let terms: Vec<String> = tokenize(query);
    if terms.is_empty() {
        return Ok(Vec::new());
    }
    let match_expr = terms
        .iter()
        .map(|t| format!("\"{t}\"*"))
        .collect::<Vec<_>>()
        .join(" ");
    // bm25() is negative with the best match the most negative, so
    // ascending order is best-first. The fetch window is wide (200) so
    // common-name queries with hundreds of weak matches still surface the
    // strong ones; importance re-ranks what comes back.
    let mut stmt = conn.prepare(
        "SELECT f.rowid, p.osm_type, p.osm_id, p.name, p.class, p.subclass,
                p.importance, p.lat, p.lon, p.min_lon, p.min_lat, p.max_lon, p.max_lat, p.admin_path
         FROM places_fts f JOIN places p ON p.id = f.rowid
         WHERE places_fts MATCH ?
         ORDER BY bm25(places_fts)
         LIMIT 200",
    )?;
    let rows: Vec<PlaceHit> = stmt
        .query_map([&match_expr], |row| {
            let admin: String = row.get(13)?;
            Ok((
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, f64>(7)?,
                row.get::<_, f64>(8)?,
                row.get::<_, f64>(9)?,
                row.get::<_, f64>(10)?,
                row.get::<_, f64>(11)?,
                row.get::<_, f64>(12)?,
                admin,
            ))
        })?
        .filter_map(|r| r.ok())
        .map(
            |(
                osm_type,
                osm_id,
                name,
                class,
                subclass,
                importance,
                lat,
                lon,
                min_lon,
                min_lat,
                max_lon,
                max_lat,
                admin,
            )| {
                PlaceHit {
                    osm_type,
                    osm_id,
                    name,
                    class,
                    subclass,
                    importance,
                    lat,
                    lon,
                    bbox: (min_lon, min_lat, max_lon, max_lat),
                    admin_path: serde_json::from_str(&admin).unwrap_or_default(),
                    region: region.to_string(),
                }
            },
        )
        .collect();
    // Rank: importance first, then FTS bm25 order (stable).
    let mut hits = rows;
    hits.sort_by_key(|h| std::cmp::Reverse(h.importance));
    hits.truncate(limit);
    Ok(hits)
}

/// Tokenize a query into prefix-search terms.
fn tokenize(query: &str) -> Vec<String> {
    query
        .split(|c: char| c == ',' || c.is_whitespace())
        .map(|t| t.trim().to_lowercase())
        .filter(|t| t.len() >= 2)
        .collect()
}

/// A reverse-geocode answer.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ReverseHit {
    pub region: String,
    pub name: String,
    pub class: String,
    pub subclass: String,
    pub lat: f64,
    pub lon: f64,
    pub distance_m: f64,
    pub admin_path: Vec<String>,
}

/// The nearest named place to (lat, lon) plus the containing admin path.
pub fn reverse_in(conn: &Connection, region: &str, lat: f64, lon: f64) -> Result<ReverseHit> {
    // Containing admin areas: bbox candidates then PIP.
    let mut admin_stmt = conn.prepare(
        "SELECT name, admin_level, ring, min_lon, min_lat, max_lon, max_lat FROM admin_areas",
    )?;
    let mut admin_path: Vec<(u8, String)> = Vec::new();
    let rows = admin_stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, f64>(3)?,
            row.get::<_, f64>(4)?,
            row.get::<_, f64>(5)?,
            row.get::<_, f64>(6)?,
        ))
    })?;
    for r in rows.flatten() {
        let (name, level, ring_json, min_lon, min_lat, max_lon, max_lat) = r;
        if lon < min_lon || lon > max_lon || lat < min_lat || lat > max_lat {
            continue;
        }
        let ring: Vec<(f64, f64)> = serde_json::from_str(&ring_json).unwrap_or_default();
        if point_in_ring(lon, lat, &ring) {
            admin_path.push((level as u8, name));
        }
    }
    admin_path.sort_by_key(|(l, _)| *l);
    let admin: Vec<String> = admin_path.into_iter().map(|(_, n)| n).collect();

    // Nearest place: expanding window around the point.
    let mut hits: Vec<ReverseHit> = Vec::new();
    let mut window = 0.01f64;
    for _ in 0..8 {
        hits = nearest_window(conn, region, lat, lon, window)?;
        if !hits.is_empty() {
            break;
        }
        window *= 4.0;
    }
    let Some(mut best) = hits.into_iter().next() else {
        bail!("no indexed place near ({lat}, {lon}) in {region}");
    };
    best.admin_path = admin;
    Ok(best)
}

fn nearest_window(
    conn: &Connection,
    region: &str,
    lat: f64,
    lon: f64,
    window: f64,
) -> Result<Vec<ReverseHit>> {
    let mut stmt = conn.prepare(
        "SELECT p.name, p.class, p.subclass, p.lat, p.lon, p.importance
         FROM places_rt r JOIN places p ON p.id = r.id
         WHERE r.min_lon <= ? AND r.max_lon >= ? AND r.min_lat <= ? AND r.max_lat >= ?",
    )?;
    let rows = stmt.query_map(
        rusqlite::params![lon + window, lon - window, lat + window, lat - window],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, f64>(4)?,
                row.get::<_, i64>(5)?,
            ))
        },
    )?;
    let mut hits: Vec<ReverseHit> = rows
        .flatten()
        .map(|(name, class, subclass, plat, plon, importance)| {
            let d = haversine_m(lat, lon, plat, plon);
            // Prefer real places over houses at the same distance: a tiny
            // importance bonus acts as a tiebreaker (meters, so 10 m per
            // importance point).
            ReverseHit {
                region: region.to_string(),
                name,
                class,
                subclass,
                lat: plat,
                lon: plon,
                distance_m: d - importance as f64 * 10.0,
                admin_path: Vec::new(),
            }
        })
        .collect();
    hits.sort_by(|a, b| {
        a.distance_m
            .partial_cmp(&b.distance_m)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(hits)
}

/// Great-circle distance in meters.
pub fn haversine_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let r = 6_371_000.0;
    let (la1, lo1, la2, lo2) = (
        lat1.to_radians(),
        lon1.to_radians(),
        lat2.to_radians(),
        lon2.to_radians(),
    );
    let dla = (la2 - la1) / 2.0;
    let dlo = (lo2 - lo1) / 2.0;
    let a = dla.sin().powi(2) + la1.cos() * la2.cos() * dlo.sin().powi(2);
    2.0 * r * a.sqrt().asin()
}

// ---------------------------------------------------------------------------
// Region routing + query API used by the FFI
// ---------------------------------------------------------------------------

/// The outcome of a geocode across the local indexes.
#[derive(Debug, Clone, serde::Serialize)]
pub struct GeocodeOutcome {
    pub results: Vec<PlaceHit>,
    pub searched_regions: Vec<String>,
    /// Present when nothing matched: the parsed region hint (if the query
    /// named one) and whether that region has an index.
    pub miss: Option<MissInfo>,
}

/// Miss metadata: what to suggest.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MissInfo {
    pub message: String,
    pub hint: String,
    pub hint_has_index: bool,
    pub indexed_regions: Vec<String>,
}

/// The local indexes on disk under `cache_dir`.
pub fn indexed_regions(cache_dir: &Path) -> Vec<String> {
    let dir = search_dir(cache_dir);
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if let Some(region) = name.strip_suffix(".search.db") {
                out.push(region.to_string());
            }
        }
    }
    out.sort();
    out
}

/// Run a geocode across every local index (or one named region).
pub fn geocode_all(
    cache_dir: &Path,
    query: &str,
    regions: Option<&[String]>,
    limit: usize,
) -> Result<GeocodeOutcome> {
    let available = indexed_regions(cache_dir);
    let to_search: Vec<String> = match regions {
        Some(rs) if !rs.is_empty() => rs.to_vec(),
        _ => available.clone(),
    };
    let searched: Vec<String> = to_search
        .iter()
        .filter(|r| available.contains(r))
        .cloned()
        .collect();

    let mut all: Vec<PlaceHit> = Vec::new();
    for region in &searched {
        let path = index_path(cache_dir, region);
        let Ok(conn) = open_index(&path) else {
            continue;
        };
        let hits = geocode_in(&conn, region, query, limit)?;
        all.extend(hits);
    }
    all.sort_by(|a, b| b.importance.cmp(&a.importance).then(a.name.cmp(&a.name)));
    all.truncate(limit);

    if !all.is_empty() {
        return Ok(GeocodeOutcome {
            results: all,
            searched_regions: searched,
            miss: None,
        });
    }

    // Miss: build the recommendation payload. The honest wording — never
    // claim a result exists in a region that is not indexed here.
    let hint = parse_region_hint(query);
    let hint_has_index = hint
        .as_ref()
        .map(|h| available.iter().any(|r| r == h))
        .unwrap_or(false);
    let message = if let Some(h) = &hint {
        if hint_has_index {
            format!("no match for \"{query}\" in the local index for {h}")
        } else {
            format!(
                "no local index for {h} — the result may be there once it is fetched and indexed"
            )
        }
    } else if available.is_empty() {
        "no search indexes on this machine — import a region and build its index".to_string()
    } else {
        format!(
            "no match for \"{query}\" across the local regions ({}) — try naming a region, e.g. \"{} <query>\"",
            available.join(", "),
            available.first().cloned().unwrap_or_default()
        )
    };
    Ok(GeocodeOutcome {
        results: Vec::new(),
        searched_regions: searched,
        miss: Some(MissInfo {
            message,
            hint: hint.unwrap_or_default(),
            hint_has_index,
            indexed_regions: available,
        }),
    })
}

/// Parse a region hint from the query: any token naming exactly one
/// region of the closed set.
fn parse_region_hint(query: &str) -> Option<String> {
    let tokens: Vec<&str> = query
        .split(|c: char| c == ',' || c.is_whitespace())
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .collect();
    // Prefer the rightmost general part (the last token), then any token.
    for t in tokens.iter().rev() {
        if let Some(p) = region_bbox::region_named(t) {
            return Some(p.to_string());
        }
    }
    None
}

/// Reverse-geocode across the region table: route (lat, lon) to a region,
/// then query that region's index.
pub fn reverse_all(cache_dir: &Path, lat: f64, lon: f64) -> Result<ReverseHit> {
    let Some(rr) = region_bbox::region_containing(lon, lat) else {
        bail!("({lat}, {lon}) is outside the distribution's region set — no region to search");
    };
    let path = index_path(cache_dir, rr.path);
    if !path.exists() {
        let size = region_size_hint(rr.path);
        bail!(
            "{} covers that point but has no search index on this machine{}",
            rr.name,
            size
        );
    }
    let conn = open_index(&path)?;
    reverse_in(&conn, rr.path, lat, lon)
}

fn region_size_hint(path: &str) -> String {
    // Region sizes live in app state, not here; the hint stays qualitative.
    let _ = path;
    " — fetch it to search it".to_string()
}

// ---------------------------------------------------------------------------
// Index status
// ---------------------------------------------------------------------------

/// Status row for one region's index.
#[derive(Debug, Clone, serde::Serialize)]
pub struct IndexStatus {
    pub region: String,
    pub built: bool,
    pub places: u64,
    pub bytes: u64,
    pub pbf_md5: String,
    /// The md5 of the PBF in the local catalog, when present: differing
    /// from the index's md5 means the region was re-imported and the
    /// index is stale.
    pub catalog_md5: Option<String>,
    pub stale: bool,
}

/// Status of every index under `cache_dir`, joined with the local catalog
/// (region -> md5) for staleness.
pub fn status_all(cache_dir: &Path, catalog: &[(String, [u8; 16])]) -> Vec<IndexStatus> {
    let regions = indexed_regions(cache_dir);
    let mut out = Vec::new();
    for r in regions {
        let path = index_path(cache_dir, &r);
        let built = path.exists();
        let (places, bytes, pbf_md5) = if built {
            match Connection::open(&path) {
                Ok(conn) => {
                    let places: i64 = conn
                        .query_row("SELECT value FROM meta WHERE key = 'places'", [], |row| {
                            row.get::<_, String>(0)
                        })
                        .ok()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    let md5: String = conn
                        .query_row("SELECT value FROM meta WHERE key = 'pbf_md5'", [], |row| {
                            row.get(0)
                        })
                        .unwrap_or_default();
                    let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                    (places as u64, bytes, md5)
                }
                Err(_) => (0, 0, String::new()),
            }
        } else {
            (0, 0, String::new())
        };
        let catalog_md5 = catalog
            .iter()
            .find(|(cr, _)| cr == &r)
            .map(|(_, m)| hex::encode(m));
        let stale = catalog_md5
            .as_deref()
            .map(|cm| !cm.is_empty() && !pbf_md5.is_empty() && cm != pbf_md5)
            .unwrap_or(false);
        out.push(IndexStatus {
            region: r,
            built,
            places,
            bytes,
            pbf_md5,
            catalog_md5,
            stale,
        });
    }
    // Regions in the catalog without an index: unbuilt entries.
    for (r, _) in catalog {
        if !out.iter().any(|s| s.region == *r) {
            out.push(IndexStatus {
                region: r.clone(),
                built: false,
                places: 0,
                bytes: 0,
                pbf_md5: String::new(),
                catalog_md5: None,
                stale: false,
            });
        }
    }
    out.sort_by(|a, b| a.region.cmp(&b.region));
    out
}

// ---------------------------------------------------------------------------
// Locator snapshot
// ---------------------------------------------------------------------------

/// Render a locator PNG around (lat, lon): the containing admin ring, the
/// nearby named places, and a marker on the selected point. Returned as
/// PNG bytes for the caller to base64 into a JSON result.
pub fn render_locator_png(
    conn: &Connection,
    lat: f64,
    lon: f64,
    label: &str,
    sub_label: &str,
) -> Result<Vec<u8>> {
    let (w, h) = (640u32, 480u32);
    let mut img = Canvas::new(w, h);

    // Window: ~0.04 deg around the point (a neighborhood locator).
    let win = 0.04;
    let min_lon = lon - win;
    let max_lon = lon + win;
    let min_lat = lat - win / 2.0 * (w as f64 / h as f64 * 0.9);
    let max_lat = lat + win / 2.0 * (w as f64 / h as f64 * 0.9);
    let project = |(plo, pla): (f64, f64)| -> (f64, f64) {
        let x = (plo - min_lon) / (max_lon - min_lon) * f64::from(w);
        let y = (max_lat - pla) / (max_lat - min_lat) * f64::from(h);
        (x, y)
    };

    // The containing admin ring (highest level that contains the point),
    // drawn as the base geometry.
    if let Ok(mut stmt) = conn.prepare(
        "SELECT name, admin_level, ring, min_lon, min_lat, max_lon, max_lat FROM admin_areas",
    ) {
        let rows: Vec<(String, i64, String, f64, f64, f64, f64)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            })?
            .flatten()
            .collect();
        let mut best: Option<(i64, Vec<(f64, f64)>)> = None;
        for (name, level, ring_json, amin_lon, amin_lat, amax_lon, amax_lat) in rows {
            if lon < amin_lon || lon > amax_lon || lat < amin_lat || lat > amax_lat {
                continue;
            }
            let ring: Vec<(f64, f64)> = serde_json::from_str(&ring_json).unwrap_or_default();
            if !point_in_ring(lon, lat, &ring) {
                continue;
            }
            if best.as_ref().map(|(l, _)| level > *l).unwrap_or(true) {
                best = Some((level, ring));
            }
            let _ = name;
        }
        if let Some((_, ring)) = best {
            let pts: Vec<(f64, f64)> = ring.iter().map(|p| project(*p)).collect();
            img.polyline(&pts, COLORS.teal, 2.0);
        }
    }

    // Nearby places: dots + labels for the important ones.
    let mut stmt = conn.prepare(
        "SELECT p.name, p.lat, p.lon, p.importance, p.class
         FROM places_rt r JOIN places p ON p.id = r.id
         WHERE r.min_lon <= ? AND r.max_lon >= ? AND r.min_lat <= ? AND r.max_lat >= ?
         ORDER BY p.importance DESC LIMIT 60",
    )?;
    let nearby: Vec<(String, f64, f64, i64, String)> = stmt
        .query_map(
            rusqlite::params![max_lon, min_lon, max_lat, min_lat],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, f64>(1)?,
                    row.get::<_, f64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )?
        .flatten()
        .collect();
    for (i, (name, plat, plon, imp, class)) in nearby.iter().enumerate() {
        let (x, y) = project((*plon, *plat));
        if x < 0.0 || x > f64::from(w) || y < 0.0 || y > f64::from(h) {
            continue;
        }
        let color = if *imp >= 35 { COLORS.tan } else { COLORS.faint };
        img.dot(x, y, if *class == "place" { 2.5 } else { 1.5 }, color);
        // Labels for the top 10, avoiding the footer band.
        if i < 10 && y < f64::from(h) - 40.0 {
            img.text(x + 5.0, y - 3.0, name.to_uppercase().as_str(), color);
        }
    }

    // The marker: an accent ring with a solid core.
    let (mx, my) = project((lon, lat));
    img.ring(mx, my, 7.0, COLORS.accent, 2.0);
    img.dot(mx, my, 3.0, COLORS.accent);

    // Label band at the bottom: label + sub label (admin path).
    img.rect(0.0, f64::from(h) - 34.0, f64::from(w), 34.0, COLORS.band);
    img.text(
        10.0,
        f64::from(h) - 26.0,
        label.to_uppercase().as_str(),
        COLORS.ink,
    );
    if !sub_label.is_empty() {
        let label_w = text_width(label);
        img.text(
            12.0 + label_w,
            f64::from(h) - 26.0,
            sub_label.to_uppercase().as_str(),
            COLORS.muted,
        );
    }

    // Graticule: subtle quarter lines.
    for i in 1..4 {
        let x = f64::from(w) * f64::from(i) / 4.0;
        img.vline(x, 0.0, f64::from(h) - 34.0, COLORS.hair);
        let y = f64::from(h) * f64::from(i) / 4.0;
        img.hline(y, 0.0, f64::from(w), COLORS.hair);
    }

    Ok(img.png())
}

struct Colors {
    canvas: [u8; 4],
    ink: [u8; 4],
    accent: [u8; 4],
    tan: [u8; 4],
    teal: [u8; 4],
    faint: [u8; 4],
    muted: [u8; 4],
    hair: [u8; 4],
    band: [u8; 4],
}

/// The app palette (logos.co identity tokens), so the snapshot reads as
/// part of the app rather than a foreign image.
const COLORS: Colors = Colors {
    canvas: [0x15, 0x25, 0x21, 0xff],
    ink: [0xf5, 0xf5, 0xef, 0xff],
    accent: [0xff, 0xd3, 0x28, 0xff],
    tan: [0xe2, 0xe0, 0xc9, 0xff],
    teal: [0x5f, 0x79, 0x7c, 0xff],
    faint: [0x86, 0x86, 0x80, 0xff],
    muted: [0x9a, 0x9a, 0x92, 0xff],
    hair: [0x2a, 0x3a, 0x34, 0xff],
    band: [0x0b, 0x15, 0x12, 0xff],
};

fn text_width(s: &str) -> f64 {
    s.chars().count() as f64 * 6.0
}

/// A tiny RGBA raster canvas with the primitives the locator needs.
struct Canvas {
    w: u32,
    h: u32,
    px: Vec<u8>,
}

impl Canvas {
    fn new(w: u32, h: u32) -> Self {
        let mut px = vec![0u8; (w * h * 4) as usize];
        for i in 0..(w * h) as usize {
            px[i * 4..i * 4 + 4].copy_from_slice(&COLORS.canvas);
        }
        Canvas { w, h, px }
    }
    fn set(&mut self, x: f64, y: f64, c: [u8; 4]) {
        if x < 0.0 || y < 0.0 || x >= f64::from(self.w) || y >= f64::from(self.h) {
            return;
        }
        let xi = x as usize;
        let yi = y as usize;
        let o = (yi * self.w as usize + xi) * 4;
        self.px[o..o + 4].copy_from_slice(&c);
    }
    fn dot(&mut self, x: f64, y: f64, r: f64, c: [u8; 4]) {
        let x0 = (x - r).floor().max(0.0) as i64;
        let x1 = (x + r).ceil().min(f64::from(self.w) - 1.0) as i64;
        let y0 = (y - r).floor().max(0.0) as i64;
        let y1 = (y + r).ceil().min(f64::from(self.h) - 1.0) as i64;
        for yy in y0..=y1 {
            for xx in x0..=x1 {
                let dx = f64::from(xx as u32) - x;
                let dy = f64::from(yy as u32) - y;
                if dx * dx + dy * dy <= r * r {
                    self.set(xx as f64, yy as f64, c);
                }
            }
        }
    }
    fn ring(&mut self, x: f64, y: f64, r: f64, c: [u8; 4], w: f64) {
        let steps = ((r.max(w) * 6.0).ceil() as usize).max(24);
        for i in 0..steps {
            let a = std::f64::consts::TAU * (i as f64) / steps as f64;
            let px = x + a.cos() * r;
            let py = y + a.sin() * r;
            self.dot(px, py, w / 2.0, c);
        }
    }
    fn line(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, c: [u8; 4], w: f64) {
        let steps = ((x1 - x0).hypot(y1 - y0).ceil() as usize).max(1);
        for i in 0..=steps {
            let t = i as f64 / steps as f64;
            let x = x0 + (x1 - x0) * t;
            let y = y0 + (y1 - y0) * t;
            self.dot(x, y, w / 2.0, c);
        }
    }
    fn vline(&mut self, x: f64, y0: f64, y1: f64, c: [u8; 4]) {
        self.line(x, y0, x, y1, c, 1.0);
    }
    fn hline(&mut self, y: f64, x0: f64, x1: f64, c: [u8; 4]) {
        self.line(x0, y, x1, y, c, 1.0);
    }
    fn polyline(&mut self, pts: &[(f64, f64)], c: [u8; 4], w: f64) {
        for win in pts.windows(2) {
            self.line(win[0].0, win[0].1, win[1].0, win[1].1, c, w);
        }
    }
    fn rect(&mut self, x: f64, y: f64, w: f64, h: f64, c: [u8; 4]) {
        let x0 = x.floor().max(0.0);
        let y0 = y.floor().max(0.0);
        let x1 = (x + w).ceil().min(f64::from(self.w));
        let y1 = (y + h).ceil().min(f64::from(self.h));
        let mut yy = y0;
        while yy < y1 {
            let mut xx = x0;
            while xx < x1 {
                self.set(xx, yy, c);
                xx += 1.0;
            }
            yy += 1.0;
        }
    }
    fn text(&mut self, x: f64, y: f64, s: &str, c: [u8; 4]) {
        let mut cx = x;
        for ch in s.chars() {
            if let Some(g) = crate::snapshot_font::glyph(ch) {
                for (col, bits) in g.iter().enumerate() {
                    for row in 0..7u8 {
                        if bits & (1 << row) != 0 {
                            self.set(cx + col as f64, y + row as f64, c);
                        }
                    }
                }
            }
            cx += 6.0;
        }
    }
    /// Encode as a PNG (RGBA, filter 0, zlib via flate2).
    fn png(self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n']);
        // IHDR
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&self.w.to_be_bytes());
        ihdr.extend_from_slice(&self.h.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // bit depth 8, RGBA
        chunk(b"IHDR", &ihdr, &mut out);
        // IDAT: scanlines with filter byte 0.
        let mut raw = Vec::with_capacity(self.px.len() + self.h as usize);
        for y in 0..self.h as usize {
            raw.push(0u8);
            let start = y * self.w as usize * 4;
            raw.extend_from_slice(&self.px[start..start + self.w as usize * 4]);
        }
        use std::io::Write;
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(&raw).unwrap();
        let idat = z.finish().unwrap();
        chunk(b"IDAT", &idat, &mut out);
        chunk(b"IEND", &[], &mut out);
        out
    }
}

fn chunk(tag: &[u8; 4], data: &[u8], out: &mut Vec<u8>) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(tag);
    out.extend_from_slice(data);
    let mut crc = Crc32::new();
    crc.update(tag);
    crc.update(data);
    out.extend_from_slice(&crc.finish().to_be_bytes());
}

/// Table-driven CRC-32 (PNG polynomial).
struct Crc32 {
    table: [u32; 256],
    value: u32,
}

impl Crc32 {
    fn new() -> Self {
        let mut table = [0u32; 256];
        for i in 0..256u32 {
            let mut c = i;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xedb88320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
            table[i as usize] = c;
        }
        Crc32 {
            table,
            value: 0xffff_ffff,
        }
    }
    fn update(&mut self, data: &[u8]) {
        for b in data {
            self.value =
                self.table[((self.value ^ u32::from(*b)) & 0xff) as usize] ^ (self.value >> 8);
        }
    }
    fn finish(&self) -> u32 {
        self.value ^ 0xffff_ffff
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pbf::test_support::*;

    /// Build a fixture PBF + index with two places, a road, a house, and
    /// one admin area (a big square around everything).
    fn fixture(dir: &Path) -> (PathBuf, PathBuf) {
        let pbf = encode_pbf(
            &[
                EncNode {
                    id: 100,
                    lat: 6.455,
                    lon: 3.407,
                    tags: vec![
                        ("place".into(), "city".into()),
                        ("name".into(), "Lagos".into()),
                        ("population".into(), "15000000".into()),
                    ],
                },
                EncNode {
                    id: 101,
                    lat: 6.509,
                    lon: 3.372,
                    tags: vec![
                        ("place".into(), "suburb".into()),
                        ("name".into(), "Yaba".into()),
                    ],
                },
                EncNode {
                    id: 102,
                    lat: 6.510,
                    lon: 3.373,
                    tags: vec![
                        ("addr:housenumber".into(), "42".into()),
                        ("name".into(), "Herbert Macaulay Way 42".into()),
                    ],
                },
            ],
            &[
                EncWay {
                    id: 500,
                    refs: vec![100, 101, 102],
                    tags: vec![
                        ("highway".into(), "primary".into()),
                        ("name".into(), "Awolowo Road".into()),
                    ],
                },
                // Boundary way: a square ring around the area.
                EncWay {
                    id: 600,
                    refs: vec![1, 2, 3, 4, 1],
                    tags: vec![("boundary".into(), "administrative".into())],
                },
            ],
            &[EncRelation {
                id: 900,
                members: vec![(1, 600, "outer".into())],
                tags: vec![
                    ("type".into(), "boundary".into()),
                    ("boundary".into(), "administrative".into()),
                    ("admin_level".into(), "4".into()),
                    ("name".into(), "Lagos State".into()),
                ],
            }],
        );
        // Corner nodes for the boundary ring (ids 1..4).
        let mut pbf2 = pbf;
        let _ = &mut pbf2;
        // Re-encode including ring corner nodes as untagged nodes.
        let pbf = encode_pbf(
            &[
                EncNode {
                    id: 1,
                    lat: 6.2,
                    lon: 3.0,
                    tags: vec![],
                },
                EncNode {
                    id: 2,
                    lat: 6.8,
                    lon: 3.0,
                    tags: vec![],
                },
                EncNode {
                    id: 3,
                    lat: 6.8,
                    lon: 3.8,
                    tags: vec![],
                },
                EncNode {
                    id: 4,
                    lat: 6.2,
                    lon: 3.8,
                    tags: vec![],
                },
                EncNode {
                    id: 100,
                    lat: 6.455,
                    lon: 3.407,
                    tags: vec![
                        ("place".into(), "city".into()),
                        ("name".into(), "Lagos".into()),
                        ("population".into(), "15000000".into()),
                    ],
                },
                EncNode {
                    id: 101,
                    lat: 6.509,
                    lon: 3.372,
                    tags: vec![
                        ("place".into(), "suburb".into()),
                        ("name".into(), "Yaba".into()),
                    ],
                },
                EncNode {
                    id: 102,
                    lat: 6.510,
                    lon: 3.373,
                    tags: vec![
                        ("addr:housenumber".into(), "42".into()),
                        ("name".into(), "Herbert Macaulay".into()),
                    ],
                },
            ],
            &[
                EncWay {
                    id: 500,
                    refs: vec![100, 101, 102],
                    tags: vec![
                        ("highway".into(), "primary".into()),
                        ("name".into(), "Awolowo Road".into()),
                    ],
                },
                EncWay {
                    id: 600,
                    refs: vec![1, 2, 3, 4, 1],
                    tags: vec![("boundary".into(), "administrative".into())],
                },
            ],
            &[EncRelation {
                id: 900,
                members: vec![(1, 600, "outer".into())],
                tags: vec![
                    ("type".into(), "boundary".into()),
                    ("boundary".into(), "administrative".into()),
                    ("admin_level".into(), "4".into()),
                    ("name".into(), "Lagos State".into()),
                ],
            }],
        );
        let pbf_path = dir.join("fixture.osm.pbf");
        std::fs::write(&pbf_path, pbf).unwrap();
        let db = index_path(dir, "nigeria");
        let _ = build_index("nigeria", &pbf_path, &db).unwrap();
        (pbf_path, db)
    }

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("osm-search-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn geocode_finds_places_with_admin_path() {
        let dir = tmpdir("geocode");
        let (_, db) = fixture(&dir);
        let conn = open_index(&db).unwrap();
        let hits = geocode_in(&conn, "nigeria", "Yaba", 10).unwrap();
        assert!(!hits.is_empty());
        assert_eq!(hits[0].name, "Yaba");
        assert_eq!(hits[0].class, "place");
        assert_eq!(hits[0].admin_path, vec!["Lagos State".to_string()]);

        // Multi-term search: name + admin word together.
        let hits = geocode_in(&conn, "nigeria", "Yaba Lagos", 10).unwrap();
        assert!(!hits.is_empty());

        // "Lagos" ranks the city first (population).
        let hits = geocode_in(&conn, "nigeria", "Lagos", 10).unwrap();
        assert!(hits.iter().any(|h| h.name == "Lagos" && h.importance >= 60));
    }

    #[test]
    fn reverse_finds_nearest_place() {
        let dir = tmpdir("reverse");
        let (_, db) = fixture(&dir);
        let conn = open_index(&db).unwrap();
        let hit = reverse_in(&conn, "nigeria", 6.51, 3.3725).unwrap();
        // Nearest to that point: Yaba (6.509, 3.372) or the house node.
        assert!(hit.name == "Yaba" || hit.name.starts_with("42"));
        assert_eq!(hit.admin_path, vec!["Lagos State".to_string()]);
    }

    #[test]
    fn status_and_version() {
        let dir = tmpdir("status");
        let (pbf, db) = fixture(&dir);
        let md5 = md5_of(&pbf).unwrap();
        let stats = status_all(&dir, &[("nigeria".to_string(), md5)]);
        assert_eq!(stats.len(), 1);
        assert!(stats[0].built);
        assert!(!stats[0].stale);
        assert!(stats[0].places >= 3);
        // Stale when the catalog md5 differs.
        let other = [9u8; 16];
        let stats = status_all(&dir, &[("nigeria".to_string(), other)]);
        assert!(stats[0].stale);
        let _ = db;
    }

    #[test]
    fn hint_and_miss() {
        let dir = tmpdir("miss");
        let (_, db) = fixture(&dir);
        // The fixture index lives under <dir>/search? No: fixture wrote the
        // db directly into the dir. geocode_all expects cache_dir/search.
        let cache = dir.join("cache");
        std::fs::create_dir_all(cache.join("search")).unwrap();
        std::fs::copy(&db, cache.join("search/nigeria.search.db")).unwrap();

        let out = geocode_all(&cache, "Yaba", None, 10).unwrap();
        assert!(!out.results.is_empty());
        assert_eq!(out.searched_regions, vec!["nigeria".to_string()]);

        let out = geocode_all(&cache, "nowhere xyz", None, 10).unwrap();
        assert!(out.results.is_empty());
        let miss = out.miss.unwrap();
        assert!(miss.message.contains("no match"));
        assert_eq!(miss.indexed_regions, vec!["nigeria".to_string()]);

        // A query naming an unindexed region routes the suggestion.
        let out = geocode_all(&cache, "Fulda germany", None, 10).unwrap();
        let miss = out.miss.unwrap();
        assert_eq!(miss.hint, "germany");
        assert!(!miss.hint_has_index);

        // Reverse routing: outside the set errors; inside unindexed
        // suggests fetching.
        let err = reverse_all(&cache, -30.0, -20.0).unwrap_err();
        assert!(err.to_string().contains("outside"));
        let err = reverse_all(&cache, 50.0, 8.0).unwrap_err();
        assert!(err.to_string().contains("no search index"));
    }

    #[test]
    fn snapshot_png_is_valid() {
        let dir = tmpdir("png");
        let (_, db) = fixture(&dir);
        let conn = open_index(&db).unwrap();
        let png = render_locator_png(&conn, 6.509, 3.372, "Yaba", "Lagos State").unwrap();
        // PNG magic + IHDR dims.
        assert_eq!(
            &png[0..8],
            &[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n']
        );
        let w = u32::from_be_bytes(png[16..20].try_into().unwrap());
        let h = u32::from_be_bytes(png[20..24].try_into().unwrap());
        assert_eq!((w, h), (640, 480));
        assert!(png.len() > 1000);
        // Decode round-trip via the zlib path at least: the IDAT must
        // inflate to w*h*4 + h bytes.
        let mut idat = Vec::new();
        let mut i = 8;
        while i + 8 <= png.len() {
            let len = u32::from_be_bytes(png[i..i + 4].try_into().unwrap()) as usize;
            let tag = &png[i + 4..i + 8];
            if tag == b"IDAT" {
                idat.extend_from_slice(&png[i + 8..i + 8 + len]);
            }
            i += 12 + len;
        }
        use std::io::Read;
        let mut raw = Vec::new();
        flate2::read::ZlibDecoder::new(&idat[..])
            .read_to_end(&mut raw)
            .unwrap();
        assert_eq!(raw.len(), 480 * (640 * 4 + 1));
    }
}
