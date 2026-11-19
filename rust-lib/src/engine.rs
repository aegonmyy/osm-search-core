//! The search engine: holds the runtime context the FFI ops need
//! (catalog + cache dir + job registry), independent of the osm module.
//!
//! The app passes `open { catalog, cache_dir }` once, then calls geocode /
//! reverse_geocode / search_build / search_index_status / search_snapshot.
//! `search_build` runs in a background thread (its own minimal job system);
//! the app polls `job_status` for progress.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::Serialize;

use crate::search;

/// One region in the catalog the app opened with.
#[derive(Clone, Debug)]
pub struct CatalogRegion {
    pub region: String,
    pub md5: [u8; 16],
}

/// A running or finished background index build.
#[derive(Clone, Debug, Serialize)]
pub struct Job {
    pub id: String,
    pub op: String,
    pub label: String,
    pub state: String,        // "running" | "finished" | "error"
    pub progress: f32,        // 0.0..1.0
    pub error: Option<String>,
}

pub struct Engine {
    pub cache_dir: PathBuf,
    pub catalog: Vec<CatalogRegion>,
    pub jobs: Arc<Mutex<HashMap<String, Job>>>,
}

impl Engine {
    pub fn new(cache_dir: PathBuf, catalog: Vec<CatalogRegion>) -> Self {
        Self { cache_dir, catalog, jobs: Arc::new(Mutex::new(HashMap::new())) }
    }

    /// Resolve the imported PBF for a region from the local catalog.
    /// Returns None when the region is not imported/fetched.
    pub fn catalog_pbf(&self, region: &str) -> Option<String> {
        // The app resolves the pbf path itself (it owns the osm bridge);
        // we just surface whether the region is in the catalog. The actual
        // pbf path is passed by the app at build time.
        self.catalog.iter().find(|c| c.region == region).map(|_| String::new())
    }

    pub fn catalog_md5(&self, region: &str) -> Option<[u8; 16]> {
        self.catalog.iter().find(|c| c.region == region).map(|c| c.md5)
    }

    pub fn index_path(&self, region: &str) -> PathBuf {
        search::index_path(&self.cache_dir, region)
    }

    /// Is a build for this region already running?
    pub fn running_build(&self, region: &str) -> bool {
        let jobs = self.jobs.lock().unwrap();
        jobs.values().any(|j| j.state == "running" && j.op == "search_build" && j.label.contains(region))
    }

    /// Spawn a background index build.
    pub fn spawn_build(&self, region: String, pbf: String) -> String {
        let id = format!("sb-{}", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis());
        let label = format!("search_build {region}");
        let job = Job {
            id: id.clone(), op: "search_build".into(), label: label.clone(),
            state: "running".into(), progress: 0.0, error: None,
        };
        self.jobs.lock().unwrap().insert(id.clone(), job);

        let jobs = self.jobs.clone();
        let cache_dir = self.cache_dir.clone();
        let job_id = id.clone();
        std::thread::spawn(move || {
            let db = search::index_path(&cache_dir, &region);
            let result = search::build_index(&region, std::path::Path::new(&pbf), &db);
            let mut j = jobs.lock().unwrap();
            let job = j.get_mut(&job_id).expect("job gone");
            match result {
                Ok(_) => { job.state = "finished".into(); job.progress = 1.0; }
                Err(e) => { job.state = "error".into(); job.error = Some(format!("{e:#}")); }
            }
        });
        id
    }

    pub fn job(&self, id: &str) -> Option<Job> {
        self.jobs.lock().unwrap().get(id).cloned()
    }
}
