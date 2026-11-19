//! C ABI the Logos module host loads. Thin: parses JSON args, calls the
//! engine, returns JSON. The `osm_search_core` lib is a cdylib so this is
//! the only extern "C" surface the Logos host needs.
//!
//!   invoke_op_json(op, args_json) -> json_result (heap, freed by host)
//!   set_engine(engine_ptr)        -> void  (host hands the engine in)
//!
//! The host (C++ module_plugin) owns the Engine and passes a raw pointer;
//! we keep it in a static for the duration of a call.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::OnceLock;

use serde_json::{json, Value};

use crate::engine::{CatalogRegion, Engine};
use crate::search;

static ENGINE: OnceLock<Engine> = OnceLock::new();

/// Host sets the engine once after `open`.
#[no_mangle]
pub extern "C" fn osm_search_set_engine(ptr: usize) {
    if ptr == 0 { return; }
    let engine: Box<Engine> = unsafe { Box::from_raw(ptr as *mut Engine) };
    let _ = ENGINE.set(*engine);
}

/// Build an engine from `open { cache_dir, catalog: [{region, md5_hex}] }`.
/// Returns a raw pointer the host hands back to `osm_search_set_engine`.
#[no_mangle]
pub extern "C" fn osm_search_make_engine(args_json: *const c_char) -> usize {
    let args = match parse_args(args_json) {
        Some(a) => a,
        None => return 0,
    };
    let cache_dir = args.get("cache_dir").and_then(Value::as_str).unwrap_or("").to_string();
    let mut catalog = Vec::new();
    if let Some(arr) = args.get("catalog").and_then(Value::as_array) {
        for c in arr {
            let region = c.get("region").and_then(Value::as_str).unwrap_or("").to_string();
            let md5_hex = c.get("md5").and_then(Value::as_str).unwrap_or("");
            let mut md5 = [0u8; 16];
            if let Ok(b) = hex::decode(md5_hex) { if b.len() == 16 { md5.copy_from_slice(&b); } }
            if !region.is_empty() {
                catalog.push(CatalogRegion { region, md5 });
            }
        }
    }
    let engine = Engine::new(cache_dir.into(), catalog);
    Box::into_raw(Box::new(engine)) as usize
}

#[no_mangle]
pub extern "C" fn osm_search_invoke_op_json(op: *const c_char, args_json: *const c_char) -> *mut c_char {
    let op = match str_arg(op) { Some(s) => s, None => return c_err("missing op") };
    let args = parse_args(args_json).unwrap_or_else(|| Value::Object(Default::default()));
    let result = dispatch(&op, &args);
    let s = CString::new(result).unwrap_or_else(|_| CString::new("{}").unwrap());
    s.into_raw()
}

fn dispatch(op: &str, args: &Value) -> String {
    let Some(eng) = ENGINE.get() else {
        return json!({ "ok": false, "error": "osm_search not open: call open first" }).to_string();
    };

    match op {
        "geocode" => {
            let Some(query) = str_arg_v(args, "query") else {
                return err("geocode requires query");
            };
            let regions = args.get("regions").and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect::<Vec<_>>());
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
            match search::geocode_all(&eng.cache_dir, &query, regions.as_deref(), limit) {
                Ok(outcome) => json!({ "ok": true, "outcome": outcome }).to_string(),
                Err(e) => err(format!("geocode: {e:#}")),
            }
        }
        "reverse_geocode" => {
            let Some(lat) = args.get("lat").and_then(Value::as_f64) else { return err("reverse_geocode requires lat"); };
            let Some(lon) = args.get("lon").and_then(Value::as_f64) else { return err("reverse_geocode requires lon"); };
            match search::reverse_all(&eng.cache_dir, lat, lon) {
                Ok(r) => json!({ "ok": true, "result": r }).to_string(),
                Err(e) => err(format!("reverse_geocode: {e:#}")),
            }
        }
        "search_build" => {
            let Some(region) = str_arg_v(args, "region") else { return err("search_build requires region"); };
            let pbf = match str_arg_v(args, "pbf") {
                Some(p) => p,
                None => match eng.catalog_pbf(&region) {
                    Some(_) => return err(format!("region {region} needs an explicit pbf path; pass pbf=...")),
                    None => return err("search_build requires region+pbf"),
                }
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
            let catalog: Vec<(String, [u8; 16])> = eng.catalog.iter()
                .map(|c| (c.region.clone(), c.md5)).collect();
            let statuses = search::status_all(&eng.cache_dir, &catalog);
            json!({ "ok": true, "indexes": statuses }).to_string()
        }
        "search_snapshot" => {
            // Snapshot needs an open Connection; the app can build one
            // via search_build then call this with region+lat+lon.
            // For now, return not-yet-implemented.
            err("search_snapshot: not yet implemented in osm_search_core (needs Connection)")
        }
        "job_status" => {
            let Some(id) = str_arg_v(args, "job_id") else { return err("job_status requires job_id"); };
            match eng.job(&id) {
                Some(j) => json!({ "ok": true, "job": j }).to_string(),
                None => err(format!("no such job: {id}")),
            }
        }
        _ => err(format!("unknown op: {op}")),
    }
}

fn parse_args(p: *const c_char) -> Option<Value> {
    if p.is_null() { return None; }
    let s = unsafe { CStr::from_ptr(p) }.to_str().ok()?;
    serde_json::from_str(s).ok()
}
fn str_arg(p: *const c_char) -> Option<String> {
    if p.is_null() { return None; }
    unsafe { CStr::from_ptr(p) }.to_str().ok().map(String::from)
}
fn str_arg_v(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(String::from)
}
fn err<S: AsRef<str>>(s: S) -> String {
    json!({ "ok": false, "error": s.as_ref() }).to_string()
}
fn c_err(msg: &str) -> *mut c_char {
    let s = CString::new(err(msg)).unwrap_or_else(|_| CString::new("{}").unwrap());
    s.into_raw()
}
