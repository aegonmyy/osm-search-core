//! `osm_search_core` — standalone OSM geocoding/search core module
//! (Rust staticlib, `interface: cdylib`).
//!
//! Reads an OSM region `.osm.pbf`, builds a per-region SQLite FTS5+rtree index
//! in a caller-supplied cache dir, and answers geocode / reverse-geocode queries
//! offline. A minimal background job system runs index builds; the app polls
//! `job_status`.
//!
//! Self-contained: no shared code with the `osm` module's cdylib. The search
//! logic, PBF reader, 72-region bbox table, MD5 streamer and index builder all
//! live here. The core declares NO module dependencies — the `osm_search` app
//! resolves the catalog / cache_dir / PBF paths via the `osm` module and passes
//! them as `invoke_op` arguments (the `open` op seeds the engine, `search_build`
//! takes a `pbf` path).
//!
//! The IPC contract is `osm_search_core.lidl` — a single generic `invoke_op(name,
//! args)` the app plugin forwards search ops through, mirroring how `osm_renderer`
//! drives the `osm_renderer_core`. The builder generates the trait +
//! `install` scaffold (`generated/provider_gen.rs`) from the .lidl. Every op
//! returns a JSON string the QML parses.

// The builder injects the generated scaffold here (no OUT_DIR; CARGO_MANIFEST_DIR).
include!(concat!(env!("CARGO_MANIFEST_DIR"), "/generated/provider_gen.rs"));

mod engine;
mod md5_stream;
mod pbf;
mod region_bbox;
mod search;
mod set;
mod snapshot_font;

use std::sync::OnceLock;

use serde_json::{json, Value};

use crate::engine::{CatalogRegion, Engine};

/// Module-held engine: seeded once by the `open` op, read by every other op.
static ENGINE: OnceLock<Engine> = OnceLock::new();

#[derive(Default)]
struct OsmSearchCoreImpl;

impl OsmSearchCoreModule for OsmSearchCoreImpl {
    /// Generic op dispatch. `name` is one of open / geocode / reverse_geocode /
    /// search_build / search_index_status / search_snapshot / job_status;
    /// `args` is a JSON object string. Returns a JSON result string.
    fn invoke_op(&mut self, name: String, args: String) -> String {
        let args: Value = serde_json::from_str(&args)
            .unwrap_or_else(|_| Value::Object(Default::default()));
        dispatch(&name, &args)
    }
}

#[no_mangle]
pub extern "Rust" fn logos_module_install() {
    install::<OsmSearchCoreImpl>();
}

/// Dispatch a search op. Mirrors the osm core's `{"ok":…,"result":…}` /
/// `{"ok":false,"error":…}` contract.
fn dispatch(op: &str, args: &Value) -> String {
    // `open` seeds the engine BEFORE the ENGINE.get() guard below — it is the
    // one op that runs with no engine yet. The app calls it once with the
    // cache_dir + catalog it resolved via the osm module.
    if op == "open" {
        let cache_dir = args
            .get("cache_dir")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let mut catalog = Vec::new();
        if let Some(arr) = args.get("catalog").and_then(Value::as_array) {
            for c in arr {
                let region = c
                    .get("region")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let md5_hex = c.get("md5").and_then(Value::as_str).unwrap_or("");
                let mut md5 = [0u8; 16];
                if let Ok(b) = hex::decode(md5_hex) {
                    if b.len() == 16 {
                        md5.copy_from_slice(&b);
                    }
                }
                if !region.is_empty() {
                    catalog.push(CatalogRegion { region, md5 });
                }
            }
        }
        let engine = Engine::new(cache_dir.into(), catalog);
        let _ = ENGINE.set(engine);
        return json!({ "ok": true }).to_string();
    }

    let Some(eng) = ENGINE.get() else {
        return err("osm_search not open: call open first");
    };

    match op {
        "geocode" => {
            let Some(query) = str_arg_v(args, "query") else {
                return err("geocode requires query");
            };
            let regions = args
                .get("regions")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(String::from)
                        .collect::<Vec<_>>()
                });
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
            match search::geocode_all(&eng.cache_dir, &query, regions.as_deref(), limit) {
                Ok(outcome) => json!({ "ok": true, "outcome": outcome }).to_string(),
                Err(e) => err(format!("geocode: {e:#}")),
            }
        }
        "reverse_geocode" => {
            let Some(lat) = args.get("lat").and_then(Value::as_f64) else {
                return err("reverse_geocode requires lat");
            };
            let Some(lon) = args.get("lon").and_then(Value::as_f64) else {
                return err("reverse_geocode requires lon");
            };
            match search::reverse_all(&eng.cache_dir, lat, lon) {
                Ok(r) => json!({ "ok": true, "result": r }).to_string(),
                Err(e) => err(format!("reverse_geocode: {e:#}")),
            }
        }
        "search_build" => {
            let Some(region) = str_arg_v(args, "region") else {
                return err("search_build requires region");
            };
            let pbf = match str_arg_v(args, "pbf") {
                Some(p) => p,
                None => match eng.catalog_pbf(&region) {
                    Some(_) => {
                        return err(format!(
                            "region {region} needs an explicit pbf path; pass pbf=..."
                        ))
                    }
                    None => return err("search_build requires region+pbf"),
                },
            };
            if !std::path::Path::new(&pbf).exists() {
                return err(format!("pbf file not found: {pbf}"));
            }
            if eng.running_build(&region) {
                return err(format!("an index build for {region} is already running"));
            }
            let id = eng.spawn_build(region, pbf);
            json!({ "ok": true, "job_id": id, "op": "search_build" }).to_string()
        }
        "search_index_status" => {
            let catalog: Vec<(String, [u8; 16])> = eng
                .catalog
                .iter()
                .map(|c| (c.region.clone(), c.md5))
                .collect();
            let statuses = search::status_all(&eng.cache_dir, &catalog);
            json!({ "ok": true, "indexes": statuses }).to_string()
        }
        "search_snapshot" => {
            // Snapshot needs an open Connection; the app can build one via
            // search_build then call this with region+lat+lon. Not yet wired.
            err("search_snapshot: not yet implemented in osm_search_core (needs Connection)")
        }
        "job_status" => {
            let Some(id) = str_arg_v(args, "job_id") else {
                return err("job_status requires job_id");
            };
            match eng.job(&id) {
                Some(j) => json!({ "ok": true, "job": j }).to_string(),
                None => err(format!("no such job: {id}")),
            }
        }
        _ => err(format!("unknown op: {op}")),
    }
}

fn str_arg_v(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(String::from)
}

fn err<S: AsRef<str>>(s: S) -> String {
    json!({ "ok": false, "error": s.as_ref() }).to_string()
}
