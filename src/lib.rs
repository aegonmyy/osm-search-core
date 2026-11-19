//! osm_search_core — standalone search/geocoding module.
//!
//! Consumes the `osm` module's data (PBF path, regions, catalog) at runtime
//! via the Logos RemoteObjects API, builds a local SQLite FTS5+rtree index
//! per region, and answers geocode / reverse-geocode queries offline.
//!
//! This crate is self-contained: no shared code with the `osm` module's
//! cdylib. The search logic, PBF reader, region table, and index builder
//! all live here.

pub mod engine;
pub mod ffi;
pub mod md5_stream;
pub mod pbf;
pub mod region_bbox;
pub mod search;
pub mod set;
pub mod snapshot_font;
