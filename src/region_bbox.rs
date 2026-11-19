//! Region bounding boxes for the search feature: the 72-region set
//! joined with Geofabrik's published geometry (index-v1.json, fetched
//! 2026-11-15; each bbox computed from the extract polygon and rounded
//! to 3 decimals). One source of truth for coordinate routing: which
//! region contains a point, and which region a query token names.
//!
//! The table is embedded (not fetched at runtime) because routing a
//! reverse-geocode must work offline with zero network, and 72 rows
//! cost under 2 KB. Regenerate with the script in scripts/ if the
//! region set ever changes.

use crate::set::REGIONS;

/// A region row: the registry path, its display name, and the bbox
/// (min_lon, min_lat, max_lon, max_lat).
#[derive(Debug, Clone, Copy)]
pub struct RegionBBox {
    pub path: &'static str,
    pub name: &'static str,
    pub min_lon: f64,
    pub min_lat: f64,
    pub max_lon: f64,
    pub max_lat: f64,
}

/// The closed region set with bboxes, ordered exactly as [`REGIONS`].
pub static REGION_BBOXES: &[RegionBBox] = &[
    RegionBBox {
        path: "germany",
        name: "Germany",
        min_lon: 5.864,
        min_lat: 47.265,
        max_lon: 15.051,
        max_lat: 55.148,
    },
    RegionBBox {
        path: "france",
        name: "France",
        min_lon: -6.937,
        min_lat: 41.239,
        max_lon: 9.9,
        max_lat: 51.429,
    },
    RegionBBox {
        path: "great-britain",
        name: "United Kingdom",
        min_lon: -14.991,
        min_lat: 49.523,
        max_lon: 2.514,
        max_lat: 61.136,
    },
    RegionBBox {
        path: "italy",
        name: "Italy",
        min_lon: 6.603,
        min_lat: 35.076,
        max_lon: 19.125,
        max_lat: 47.1,
    },
    RegionBBox {
        path: "spain",
        name: "Spain",
        min_lon: -9.779,
        min_lat: 35.264,
        max_lon: 5.099,
        max_lat: 44.149,
    },
    RegionBBox {
        path: "poland",
        name: "Poland",
        min_lon: 13.99,
        min_lat: 48.986,
        max_lon: 24.161,
        max_lat: 55.228,
    },
    RegionBBox {
        path: "netherlands",
        name: "Netherlands",
        min_lon: 2.936,
        min_lat: 50.748,
        max_lon: 7.23,
        max_lat: 54.018,
    },
    RegionBBox {
        path: "belgium",
        name: "Belgium",
        min_lon: 2.341,
        min_lat: 49.495,
        max_lon: 6.41,
        max_lat: 51.598,
    },
    RegionBBox {
        path: "switzerland",
        name: "Switzerland",
        min_lon: 5.954,
        min_lat: 45.816,
        max_lon: 10.496,
        max_lat: 47.811,
    },
    RegionBBox {
        path: "austria",
        name: "Austria",
        min_lon: 9.527,
        min_lat: 46.37,
        max_lon: 17.164,
        max_lat: 49.024,
    },
    RegionBBox {
        path: "czech-republic",
        name: "Czech Republic",
        min_lon: 12.085,
        min_lat: 48.543,
        max_lon: 18.863,
        max_lat: 51.064,
    },
    RegionBBox {
        path: "sweden",
        name: "Sweden",
        min_lon: 10.541,
        min_lat: 55.027,
        max_lon: 24.225,
        max_lat: 69.066,
    },
    RegionBBox {
        path: "norway",
        name: "Norway",
        min_lon: -11.368,
        min_lat: 57.553,
        max_lon: 35.527,
        max_lat: 81.052,
    },
    RegionBBox {
        path: "denmark",
        name: "Denmark",
        min_lon: 7.701,
        min_lat: 54.441,
        max_lon: 15.654,
        max_lat: 58.062,
    },
    RegionBBox {
        path: "finland",
        name: "Finland",
        min_lon: 19.024,
        min_lat: 59.288,
        max_lon: 31.616,
        max_lat: 70.1,
    },
    RegionBBox {
        path: "portugal",
        name: "Portugal",
        min_lon: -31.6,
        min_lat: 29.7,
        max_lon: -6.18,
        max_lat: 42.164,
    },
    RegionBBox {
        path: "greece",
        name: "Greece",
        min_lon: 18.971,
        min_lat: 34.591,
        max_lon: 29.657,
        max_lat: 41.75,
    },
    RegionBBox {
        path: "ireland",
        name: "Ireland",
        min_lon: -14.402,
        min_lat: 49.6,
        max_lon: -5.059,
        max_lat: 56.866,
    },
    RegionBBox {
        path: "hungary",
        name: "Hungary",
        min_lon: 16.108,
        min_lat: 45.732,
        max_lon: 22.906,
        max_lat: 48.589,
    },
    RegionBBox {
        path: "romania",
        name: "Romania",
        min_lon: 20.242,
        min_lat: 43.612,
        max_lon: 30.279,
        max_lat: 48.269,
    },
    RegionBBox {
        path: "bulgaria",
        name: "Bulgaria",
        min_lon: 22.349,
        min_lat: 41.227,
        max_lon: 29.188,
        max_lat: 44.218,
    },
    RegionBBox {
        path: "ukraine",
        name: "Ukraine",
        min_lon: 22.133,
        min_lat: 44.009,
        max_lon: 40.238,
        max_lat: 52.386,
    },
    RegionBBox {
        path: "belarus",
        name: "Belarus",
        min_lon: 23.175,
        min_lat: 51.234,
        max_lon: 32.769,
        max_lat: 56.174,
    },
    RegionBBox {
        path: "turkey",
        name: "Turkey",
        min_lon: 25.524,
        min_lat: 35.717,
        max_lon: 44.86,
        max_lat: 43.075,
    },
    RegionBBox {
        path: "canada",
        name: "Canada",
        min_lon: -141.776,
        min_lat: 41.66,
        max_lon: -44.177,
        max_lat: 85.04,
    },
    RegionBBox {
        path: "mexico",
        name: "Mexico",
        min_lon: -132.495,
        min_lat: 9.457,
        max_lon: -85.491,
        max_lat: 32.721,
    },
    RegionBBox {
        path: "japan",
        name: "Japan",
        min_lon: 122.561,
        min_lat: 20.082,
        max_lon: 154.471,
        max_lat: 45.815,
    },
    RegionBBox {
        path: "south-korea",
        name: "South Korea",
        min_lon: 124.319,
        min_lat: 32.361,
        max_lon: 132.339,
        max_lat: 38.65,
    },
    RegionBBox {
        path: "indonesia",
        name: "Indonesia",
        min_lon: 92.765,
        min_lat: -11.607,
        max_lon: 141.022,
        max_lat: 6.999,
    },
    RegionBBox {
        path: "thailand",
        name: "Thailand",
        min_lon: 97.136,
        min_lat: 5.598,
        max_lon: 105.639,
        max_lat: 20.469,
    },
    RegionBBox {
        path: "vietnam",
        name: "Vietnam",
        min_lon: 102.096,
        min_lat: 7.382,
        max_lon: 114.642,
        max_lat: 23.402,
    },
    RegionBBox {
        path: "malaysia",
        name: "Malaysia",
        min_lon: 99.256,
        min_lat: 0.831,
        max_lon: 119.668,
        max_lat: 7.737,
    },
    RegionBBox {
        path: "philippines",
        name: "Philippines",
        min_lon: 112.166,
        min_lat: 4.383,
        max_lon: 127.074,
        max_lat: 21.53,
    },
    RegionBBox {
        path: "pakistan",
        name: "Pakistan",
        min_lon: 60.822,
        min_lat: 23.376,
        max_lon: 77.224,
        max_lat: 37.107,
    },
    RegionBBox {
        path: "bangladesh",
        name: "Bangladesh",
        min_lon: 87.998,
        min_lat: 18.584,
        max_lon: 92.697,
        max_lat: 26.647,
    },
    RegionBBox {
        path: "iran",
        name: "Iran",
        min_lon: 44.023,
        min_lat: 24.039,
        max_lon: 63.354,
        max_lat: 39.79,
    },
    RegionBBox {
        path: "australia",
        name: "Australia",
        min_lon: 68.133,
        min_lat: -57.071,
        max_lon: 169.002,
        max_lat: -8.81,
    },
    RegionBBox {
        path: "brazil",
        name: "Brazil",
        min_lon: -74.023,
        min_lat: -35.466,
        max_lon: -27.672,
        max_lat: 5.523,
    },
    RegionBBox {
        path: "argentina",
        name: "Argentina",
        min_lon: -73.615,
        min_lat: -55.683,
        max_lon: -53.635,
        max_lat: -21.726,
    },
    RegionBBox {
        path: "colombia",
        name: "Colombia",
        min_lon: -83.231,
        min_lat: -4.257,
        max_lon: -66.815,
        max_lat: 16.419,
    },
    RegionBBox {
        path: "peru",
        name: "Peru",
        min_lon: -86.025,
        min_lat: -20.306,
        max_lon: -68.554,
        max_lat: 0.06,
    },
    RegionBBox {
        path: "chile",
        name: "Chile",
        min_lon: -113.223,
        min_lat: -58.452,
        max_lon: -65.514,
        max_lat: -17.457,
    },
    RegionBBox {
        path: "south-africa",
        name: "South Africa",
        min_lon: 15.996,
        min_lat: -47.585,
        max_lon: 39.243,
        max_lat: -22.117,
    },
    RegionBBox {
        path: "egypt",
        name: "Egypt",
        min_lon: 24.626,
        min_lat: 21.984,
        max_lon: 37.163,
        max_lat: 32.525,
    },
    RegionBBox {
        path: "nigeria",
        name: "Nigeria",
        min_lon: 2.665,
        min_lat: 2.674,
        max_lon: 14.681,
        max_lat: 13.899,
    },
    RegionBBox {
        path: "kenya",
        name: "Kenya",
        min_lon: 33.868,
        min_lat: -4.816,
        max_lon: 41.977,
        max_lat: 4.63,
    },
    RegionBBox {
        path: "morocco",
        name: "Morocco",
        min_lon: -18.183,
        min_lat: 20.404,
        max_lon: -0.994,
        max_lat: 36.061,
    },
    RegionBBox {
        path: "ethiopia",
        name: "Ethiopia",
        min_lon: 32.967,
        min_lat: 3.389,
        max_lon: 48.005,
        max_lat: 14.916,
    },
    RegionBBox {
        path: "us/california",
        name: "US: California",
        min_lon: -125.894,
        min_lat: 32.482,
        max_lon: -114.129,
        max_lat: 42.016,
    },
    RegionBBox {
        path: "us/texas",
        name: "US: Texas",
        min_lon: -106.649,
        min_lat: 25.696,
        max_lon: -93.019,
        max_lat: 36.521,
    },
    RegionBBox {
        path: "us/florida",
        name: "US: Florida",
        min_lon: -88.469,
        min_lat: 24.2,
        max_lon: -79.437,
        max_lat: 31.003,
    },
    RegionBBox {
        path: "us/new-york",
        name: "US: New York",
        min_lon: -79.765,
        min_lat: 40.439,
        max_lon: -71.661,
        max_lat: 45.018,
    },
    RegionBBox {
        path: "us/washington",
        name: "US: Washington",
        min_lon: -126.742,
        min_lat: 45.539,
        max_lon: -116.912,
        max_lat: 49.007,
    },
    RegionBBox {
        path: "us/illinois",
        name: "US: Illinois",
        min_lon: -91.516,
        min_lat: 36.967,
        max_lon: -87.493,
        max_lat: 42.509,
    },
    RegionBBox {
        path: "us/georgia",
        name: "US: Georgia",
        min_lon: -85.607,
        min_lat: 30.354,
        max_lon: -80.743,
        max_lat: 35.004,
    },
    RegionBBox {
        path: "us/pennsylvania",
        name: "US: Pennsylvania",
        min_lon: -80.523,
        min_lat: 39.664,
        max_lon: -74.688,
        max_lat: 42.516,
    },
    RegionBBox {
        path: "india/central-zone",
        name: "India: Central Zone",
        min_lon: 74.015,
        min_lat: 17.771,
        max_lon: 84.647,
        max_lat: 31.491,
    },
    RegionBBox {
        path: "india/eastern-zone",
        name: "India: Eastern Zone",
        min_lon: 81.367,
        min_lat: 17.781,
        max_lon: 89.918,
        max_lat: 27.528,
    },
    RegionBBox {
        path: "india/north-eastern-zone",
        name: "India: North Eastern Zone",
        min_lon: 88.009,
        min_lat: 21.927,
        max_lon: 97.42,
        max_lat: 29.429,
    },
    RegionBBox {
        path: "india/northern-zone",
        name: "India: Northern Zone",
        min_lon: 69.439,
        min_lat: 23.047,
        max_lon: 79.489,
        max_lat: 35.731,
    },
    RegionBBox {
        path: "india/southern-zone",
        name: "India: Southern Zone",
        min_lon: 70.208,
        min_lat: 5.944,
        max_lon: 95.514,
        max_lat: 19.923,
    },
    RegionBBox {
        path: "india/western-zone",
        name: "India: Western Zone",
        min_lon: 67.675,
        min_lat: 14.389,
        max_lon: 80.914,
        max_lat: 24.718,
    },
    RegionBBox {
        path: "china/guangdong",
        name: "China: Guangdong",
        min_lon: 109.356,
        min_lat: 20.059,
        max_lon: 117.665,
        max_lat: 25.524,
    },
    RegionBBox {
        path: "china/jiangsu",
        name: "China: Jiangsu",
        min_lon: 116.354,
        min_lat: 30.759,
        max_lon: 122.945,
        max_lat: 35.555,
    },
    RegionBBox {
        path: "china/shandong",
        name: "China: Shandong",
        min_lon: 114.813,
        min_lat: 34.376,
        max_lon: 123.563,
        max_lat: 38.44,
    },
    RegionBBox {
        path: "china/zhejiang",
        name: "China: Zhejiang",
        min_lon: 118.021,
        min_lat: 26.692,
        max_lon: 123.838,
        max_lat: 31.191,
    },
    RegionBBox {
        path: "china/sichuan",
        name: "China: Sichuan",
        min_lon: 97.313,
        min_lat: 26.046,
        max_lon: 108.551,
        max_lat: 34.332,
    },
    RegionBBox {
        path: "china/henan",
        name: "China: Henan",
        min_lon: 110.346,
        min_lat: 31.378,
        max_lon: 116.647,
        max_lat: 36.368,
    },
    RegionBBox {
        path: "russia/central-fed-district",
        name: "Russia: Central FD",
        min_lon: 30.7,
        min_lat: 49.504,
        max_lon: 47.67,
        max_lat: 59.653,
    },
    RegionBBox {
        path: "russia/northwestern-fed-district",
        name: "Russia: Northwestern FD",
        min_lon: 19.408,
        min_lat: 54.313,
        max_lon: 72.625,
        max_lat: 82.546,
    },
    RegionBBox {
        path: "russia/volga-fed-district",
        name: "Russia: Volga FD",
        min_lon: 41.721,
        min_lat: 49.751,
        max_lon: 61.734,
        max_lat: 61.72,
    },
    RegionBBox {
        path: "russia/siberian-fed-district",
        name: "Russia: Siberian FD",
        min_lon: 70.275,
        min_lat: 49.021,
        max_lon: 147.713,
        max_lat: 81.887,
    },
];

/// The bbox row for a registry path.
pub fn by_path(path: &str) -> Option<&'static RegionBBox> {
    REGION_BBOXES.iter().find(|r| r.path == path)
}

/// The region whose bbox contains the point, if any. Regions are a
/// non-overlapping partition of the world, so at most one matches; the
/// first match wins regardless of order.
pub fn region_containing(lon: f64, lat: f64) -> Option<&'static RegionBBox> {
    REGION_BBOXES
        .iter()
        .find(|r| lon >= r.min_lon && lon <= r.max_lon && lat >= r.min_lat && lat <= r.max_lat)
}

/// A lowercase token (or full "a/b" path) names exactly one region: return
/// its registry path. Tokens that would match several regions (none exist
/// today, but the set has decomposed countries) or none return `None`, and
/// the caller searches every local index instead.
pub fn region_named(token: &str) -> Option<&'static str> {
    let t = token.trim().to_lowercase();
    if t.is_empty() {
        return None;
    }
    let mut hit: Option<&'static str> = None;
    for r in REGION_BBOXES {
        // Compare against the full path, its last segment, the display
        // name, and the country segment of subregions ("us" alone is
        // ambiguous for the decomposed countries, so a parent match that
        // names several regions resolves to None by the unique-hit rule).
        let path = r.path;
        let last = path.rsplit('/').next().unwrap_or(path);
        let named = r.name.to_lowercase();
        let parent = path.split_once('/').map(|(p, _)| p);
        let matched = path == t || last == t || named == t || parent == Some(t.as_str());
        if matched {
            if hit.is_some() {
                return None; // ambiguous: not a single region
            }
            hit = Some(r.path);
        }
    }
    hit
}

/// The search fallbacks for a region a query names but this machine has no
/// index for: the same cascade the UI's miss panel offers.
pub fn recommend_region(path: &str) -> bool {
    REGIONS.iter().any(|r| r.path == path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_matches_region_set() {
        assert_eq!(REGION_BBOXES.len(), REGIONS.len());
        for (a, b) in REGION_BBOXES.iter().zip(REGIONS.iter()) {
            assert_eq!(a.path, b.path, "bbox row out of order with REGIONS");
        }
    }

    #[test]
    fn bboxes_are_sane() {
        for r in REGION_BBOXES {
            assert!(r.min_lon < r.max_lon, "{} lon bbox inverted", r.path);
            assert!(r.min_lat < r.max_lat, "{} lat bbox inverted", r.path);
            assert!(
                r.min_lon >= -180.0 && r.max_lon <= 180.0,
                "{} lon out of range",
                r.path
            );
            assert!(
                r.min_lat >= -90.0 && r.max_lat <= 90.0,
                "{} lat out of range",
                r.path
            );
        }
    }

    #[test]
    fn routing() {
        // Lagos, Nigeria.
        assert_eq!(
            region_containing(3.407, 6.455).map(|r| r.path),
            Some("nigeria")
        );
        // California.
        assert_eq!(
            region_containing(-120.0, 37.0).map(|r| r.path),
            Some("us/california")
        );
        // South Atlantic: no region. (The mid-Atlantic north of the
        // equator is inside Portugal's bbox, which stretches to the
        // Azores at lon -31.6.)
        assert!(region_containing(-20.0, -30.0).is_none());
    }

    #[test]
    fn naming() {
        assert_eq!(region_named("nigeria"), Some("nigeria"));
        assert_eq!(region_named("Nigeria"), Some("nigeria"));
        assert_eq!(region_named("california"), Some("us/california"));
        assert_eq!(region_named("us/california"), Some("us/california"));
        assert_eq!(region_named("germany"), Some("germany"));
        // "us" alone names many decomposed regions: no single hint.
        assert_eq!(region_named("us"), None);
        // Nothing in the set is named "atlantis".
        assert_eq!(region_named("atlantis"), None);
    }
}
