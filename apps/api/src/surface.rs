use std::{collections::HashMap, env, fs, path::Path, time::Duration};

use anyhow::{Context, Result, bail};
use geotiff_reader::cog::HttpGeoTiffFile;
use reqwest::blocking::Client;
use serde::Deserialize;
use terrain_core::{GenerationSpec, SurfaceClass, SurfaceField};

const WORLD_COVER_BASE_URL: &str =
    "https://esa-worldcover.s3.eu-central-1.amazonaws.com/v200/2021/map";
const WORLD_COVER_INFO_URL: &str = "https://worldcover2021.esa.int/download";
const WORLD_COVER_ATTRIBUTION: &str = "© ESA WorldCover project / Contains modified Copernicus Sentinel data (2021) processed by ESA WorldCover consortium";
const DEFAULT_OVERPASS_URL: &str = "https://overpass-api.de/api/interpreter";
const OPENSTREETMAP_COPYRIGHT_URL: &str = "https://www.openstreetmap.org/copyright";
const PROMINENT_HIGHWAYS: &str =
    "motorway|motorway_link|trunk|trunk_link|primary|primary_link|secondary|secondary_link";
const FALLBACK_TRAILS: &str = "path|footway|bridleway|track|cycleway";

#[derive(Debug)]
struct SamplePoint {
    output_index: usize,
    longitude: f64,
    latitude: f64,
}

#[derive(Debug, Clone, Copy)]
struct GeoBounds {
    south: f64,
    north: f64,
    west: f64,
    east: f64,
}

#[derive(Debug, Deserialize)]
struct OverpassResponse {
    #[serde(default)]
    elements: Vec<OverpassWay>,
}

#[derive(Debug, Deserialize)]
struct OverpassWay {
    #[serde(default)]
    tags: HashMap<String, String>,
    #[serde(default)]
    geometry: Vec<OverpassPoint>,
}

#[derive(Debug, Deserialize)]
struct OverpassPoint {
    lat: f64,
    lon: f64,
}

#[derive(Debug, Default)]
struct RouteCounts {
    roads: usize,
    trails: usize,
}

pub fn fetch_surface_field(spec: &GenerationSpec, road_cache_dir: &Path) -> Result<SurfaceField> {
    let width = (spec.columns * spec.samples_per_piece + 1) as usize;
    let height = (spec.rows * spec.samples_per_piece + 1) as usize;
    let bounds = bounds_for(spec);
    let mut tiles = HashMap::<String, Vec<SamplePoint>>::new();

    for row in 0..height {
        let v = row as f64 / (height - 1) as f64;
        let latitude = bounds.south + (bounds.north - bounds.south) * v;
        for column in 0..width {
            let u = column as f64 / (width - 1) as f64;
            let longitude = normalize_longitude(bounds.west + (bounds.east - bounds.west) * u);
            tiles
                .entry(world_cover_tile(longitude, latitude))
                .or_default()
                .push(SamplePoint {
                    output_index: row * width + column,
                    longitude,
                    latitude,
                });
        }
    }

    let mut classes = vec![SurfaceClass::Rock; width * height];
    let mut tile_names = tiles.keys().cloned().collect::<Vec<_>>();
    tile_names.sort();
    for tile_name in &tile_names {
        let points = tiles
            .remove(tile_name)
            .context("land-cover tile group disappeared")?;
        sample_tile(tile_name, &points, width, height, &mut classes)?;
    }

    let mut field = SurfaceField::new(
        width,
        height,
        classes,
        format!(
            "ESA WorldCover 2021 v200, 10 m, EPSG:4326, tiles {}; CC BY 4.0; source: {WORLD_COVER_INFO_URL}; {WORLD_COVER_ATTRIBUTION}",
            tile_names.join(", ")
        ),
    )?;
    field.filter_small_patches(spec.width_mm, spec.color_output.minimum_patch_mm);
    if spec.color_output.roads_enabled {
        let counts = paint_roads_or_trails(spec, bounds, road_cache_dir, &mut field)?;
        if counts.roads > 0 {
            field.source.push_str(&format!(
                "; prominent roads: {} OpenStreetMap ways via Overpass API, highway={PROMINENT_HIGHWAYS}; © OpenStreetMap contributors, ODbL; {OPENSTREETMAP_COPYRIGHT_URL}",
                counts.roads
            ));
        } else {
            field.source.push_str(&format!(
                "; no prominent roads found; trail fallback: {} OpenStreetMap ways via Overpass API, highway={FALLBACK_TRAILS}; © OpenStreetMap contributors, ODbL; {OPENSTREETMAP_COPYRIGHT_URL}",
                counts.trails
            ));
        }
    }
    Ok(field)
}

fn bounds_for(spec: &GenerationSpec) -> GeoBounds {
    let half_lat = spec.ground_span_km / 2.0 / 110.574;
    let longitude_scale = (111.32 * spec.center_lat.to_radians().cos().abs()).max(20.0);
    let half_lon = spec.ground_span_km / 2.0 / longitude_scale;
    GeoBounds {
        south: (spec.center_lat - half_lat).max(-85.0),
        north: (spec.center_lat + half_lat).min(85.0),
        west: spec.center_lon - half_lon,
        east: spec.center_lon + half_lon,
    }
}

fn paint_roads_or_trails(
    spec: &GenerationSpec,
    bounds: GeoBounds,
    cache_dir: &Path,
    field: &mut SurfaceField,
) -> Result<RouteCounts> {
    let roads = fetch_osm_ways(spec, bounds, cache_dir, "roads", PROMINENT_HIGHWAYS)?;
    let road_count = paint_osm_ways(spec, bounds, field, roads, road_width_scale);
    if road_count > 0 {
        return Ok(RouteCounts {
            roads: road_count,
            trails: 0,
        });
    }
    let trails = fetch_osm_ways(spec, bounds, cache_dir, "trails", FALLBACK_TRAILS)?;
    let trail_count = paint_osm_ways(spec, bounds, field, trails, trail_width_scale);
    Ok(RouteCounts {
        roads: 0,
        trails: trail_count,
    })
}

fn paint_osm_ways(
    spec: &GenerationSpec,
    bounds: GeoBounds,
    field: &mut SurfaceField,
    response: OverpassResponse,
    width_scale: fn(&HashMap<String, String>) -> Option<f32>,
) -> usize {
    let mut painted = 0;
    for way in response.elements {
        if way.geometry.len() < 2 || is_tunnel(&way.tags) {
            continue;
        }
        let Some(scale) = width_scale(&way.tags) else {
            continue;
        };
        let points = way
            .geometry
            .iter()
            .map(|point| {
                let longitude = unwrap_longitude(point.lon, spec.center_lon);
                [
                    ((longitude - bounds.west) / (bounds.east - bounds.west)) as f32,
                    ((point.lat - bounds.south) / (bounds.north - bounds.south)) as f32,
                ]
            })
            .collect::<Vec<_>>();
        field.paint_polyline(
            &points,
            spec.width_mm,
            spec.color_output.road_width_mm * scale,
            SurfaceClass::Road,
        );
        painted += 1;
    }
    painted
}

fn fetch_osm_ways(
    spec: &GenerationSpec,
    bounds: GeoBounds,
    cache_dir: &Path,
    cache_prefix: &str,
    highway_filter: &str,
) -> Result<OverpassResponse> {
    fs::create_dir_all(cache_dir)
        .with_context(|| format!("create road cache {}", cache_dir.display()))?;
    let cache_path = cache_dir.join(format!(
        "{cache_prefix}-{:.5}-{:.5}-{:.3}.json",
        spec.center_lat, spec.center_lon, spec.ground_span_km,
    ));
    let (bytes, should_cache) = match fs::read(&cache_path) {
        Ok(bytes) => (bytes, false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let query = overpass_query(bounds, highway_filter);
            let base_url =
                env::var("OVERPASS_BASE_URL").unwrap_or_else(|_| DEFAULT_OVERPASS_URL.into());
            let client = Client::builder()
                .user_agent("terrain-puzzle/0.1 (+https://github.com/theatrus/terrain-puzzle)")
                .timeout(Duration::from_secs(45))
                .build()
                .context("build OpenStreetMap road client")?;
            let bytes = client
                .post(&base_url)
                .form(&[("data", query)])
                .send()
                .with_context(|| format!("request {cache_prefix} from OpenStreetMap Overpass"))?
                .error_for_status()
                .context("OpenStreetMap Overpass rejected the road request")?
                .bytes()
                .with_context(|| format!("read OpenStreetMap {cache_prefix} response"))?
                .to_vec();
            (bytes, true)
        }
        Err(error) => {
            return Err(error).with_context(|| format!("read road cache {}", cache_path.display()));
        }
    };
    let response: OverpassResponse = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse OpenStreetMap Overpass {cache_prefix} response"))?;
    if should_cache {
        fs::write(&cache_path, &bytes)
            .with_context(|| format!("cache road data {}", cache_path.display()))?;
    }
    Ok(response)
}

fn overpass_query(bounds: GeoBounds, highway_filter: &str) -> String {
    let bboxes = if bounds.west < -180.0 {
        vec![
            (bounds.south, bounds.west + 360.0, bounds.north, 180.0),
            (bounds.south, -180.0, bounds.north, bounds.east),
        ]
    } else if bounds.east > 180.0 {
        vec![
            (bounds.south, bounds.west, bounds.north, 180.0),
            (bounds.south, -180.0, bounds.north, bounds.east - 360.0),
        ]
    } else {
        vec![(bounds.south, bounds.west, bounds.north, bounds.east)]
    };
    let ways = bboxes
        .iter()
        .map(|(south, west, north, east)| {
            format!(
                "way[\"highway\"~\"^({highway_filter})$\"][\"area\"!=\"yes\"]({south:.7},{west:.7},{north:.7},{east:.7});"
            )
        })
        .collect::<String>();
    format!("[out:json][timeout:30];({ways});out tags geom;")
}

fn road_width_scale(tags: &HashMap<String, String>) -> Option<f32> {
    match tags.get("highway")?.as_str() {
        "motorway" => Some(1.4),
        "trunk" => Some(1.25),
        "primary" => Some(1.0),
        "secondary" => Some(0.8),
        "motorway_link" | "trunk_link" => Some(0.75),
        "primary_link" | "secondary_link" => Some(0.65),
        _ => None,
    }
}

fn trail_width_scale(tags: &HashMap<String, String>) -> Option<f32> {
    match tags.get("highway")?.as_str() {
        "track" => Some(0.7),
        "bridleway" => Some(0.65),
        "cycleway" => Some(0.6),
        "path" | "footway" => Some(0.55),
        _ => None,
    }
}

fn is_tunnel(tags: &HashMap<String, String>) -> bool {
    tags.get("tunnel")
        .is_some_and(|value| value != "no" && value != "false")
}

fn unwrap_longitude(longitude: f64, center: f64) -> f64 {
    center + normalize_longitude(longitude - center)
}

fn sample_tile(
    tile_name: &str,
    points: &[SamplePoint],
    target_width: usize,
    target_height: usize,
    output: &mut [SurfaceClass],
) -> Result<()> {
    let url = format!("{WORLD_COVER_BASE_URL}/ESA_WorldCover_10m_2021_v200_{tile_name}_Map.tif");
    let remote = HttpGeoTiffFile::open(&url)
        .with_context(|| format!("open ESA WorldCover tile {tile_name}"))?;
    let geotiff = remote.inner();
    if geotiff.epsg() != Some(4326) {
        bail!(
            "ESA WorldCover tile {tile_name} uses unexpected CRS {:?}",
            geotiff.epsg()
        );
    }

    let base_pixels = points
        .iter()
        .map(|point| {
            geotiff
                .geo_to_pixel(point.longitude, point.latitude)
                .with_context(|| format!("map a coordinate into tile {tile_name}"))
        })
        .collect::<Result<Vec<_>>>()?;
    let base_col_min = base_pixels
        .iter()
        .map(|(column, _)| column.floor().max(0.0) as usize)
        .min()
        .unwrap_or(0);
    let base_col_max = base_pixels
        .iter()
        .map(|(column, _)| column.ceil().max(0.0) as usize)
        .max()
        .unwrap_or(base_col_min);
    let base_row_min = base_pixels
        .iter()
        .map(|(_, row)| row.floor().max(0.0) as usize)
        .min()
        .unwrap_or(0);
    let base_row_max = base_pixels
        .iter()
        .map(|(_, row)| row.ceil().max(0.0) as usize)
        .max()
        .unwrap_or(base_row_min);
    let base_window_width = base_col_max.saturating_sub(base_col_min) + 1;
    let base_window_height = base_row_max.saturating_sub(base_row_min) + 1;
    let overview =
        (0..geotiff.overview_count())
            .filter_map(|index| {
                let ifd = geotiff.overview_ifd(index).ok()?;
                let scale_x = ifd.width() as f64 / geotiff.width() as f64;
                let scale_y = ifd.height() as f64 / geotiff.height() as f64;
                let window_width = (base_window_width as f64 * scale_x).ceil() as usize;
                let window_height = (base_window_height as f64 * scale_y).ceil() as usize;
                (window_width <= target_width * 2 && window_height <= target_height * 2)
                    .then_some((index, ifd.width(), ifd.height()))
            })
            .max_by_key(|(_, width, height)| u64::from(*width) * u64::from(*height));
    let (raster_width, raster_height) = overview
        .map(|(_, width, height)| (width, height))
        .unwrap_or((geotiff.width(), geotiff.height()));
    let scale_x = raster_width as f64 / geotiff.width() as f64;
    let scale_y = raster_height as f64 / geotiff.height() as f64;
    let pixels = base_pixels
        .into_iter()
        .map(|(column, row)| (column * scale_x, row * scale_y))
        .collect::<Vec<_>>();
    let col_min = pixels
        .iter()
        .map(|(column, _)| column.floor().max(0.0) as usize)
        .min()
        .unwrap_or(0)
        .min(raster_width.saturating_sub(1) as usize);
    let col_max = pixels
        .iter()
        .map(|(column, _)| column.ceil().max(0.0) as usize)
        .max()
        .unwrap_or(col_min)
        .min(raster_width.saturating_sub(1) as usize);
    let row_min = pixels
        .iter()
        .map(|(_, row)| row.floor().max(0.0) as usize)
        .min()
        .unwrap_or(0)
        .min(raster_height.saturating_sub(1) as usize);
    let row_max = pixels
        .iter()
        .map(|(_, row)| row.ceil().max(0.0) as usize)
        .max()
        .unwrap_or(row_min)
        .min(raster_height.saturating_sub(1) as usize);
    let rows = row_max - row_min + 1;
    let columns = col_max - col_min + 1;
    let window = match overview {
        Some((index, _, _)) => {
            geotiff.read_overview_band_window::<u8>(index, 0, row_min, col_min, rows, columns)
        }
        None => geotiff.read_band_window::<u8>(0, row_min, col_min, rows, columns),
    }
    .with_context(|| format!("read ESA WorldCover tile {tile_name}"))?;

    for (point, (column, row)) in points.iter().zip(pixels) {
        let column = (column.round() as isize).clamp(col_min as isize, col_max as isize) as usize;
        let row = (row.round() as isize).clamp(row_min as isize, row_max as isize) as usize;
        let value = window[[row - row_min, column - col_min]];
        if value == 0 {
            bail!(
                "ESA WorldCover has no data at {}, {}",
                point.latitude,
                point.longitude
            );
        }
        output[point.output_index] = classify_world_cover(value);
    }
    Ok(())
}

fn classify_world_cover(value: u8) -> SurfaceClass {
    match value {
        10 => SurfaceClass::Forest,
        70 => SurfaceClass::Snow,
        80 => SurfaceClass::Water,
        _ => SurfaceClass::Rock,
    }
}

fn world_cover_tile(longitude: f64, latitude: f64) -> String {
    let south = (latitude / 3.0).floor() as i32 * 3;
    let west = (longitude / 3.0).floor() as i32 * 3;
    format!(
        "{}{:02}{}{:03}",
        if south < 0 { 'S' } else { 'N' },
        south.unsigned_abs(),
        if west < 0 { 'W' } else { 'E' },
        west.unsigned_abs(),
    )
}

fn normalize_longitude(longitude: f64) -> f64 {
    (longitude + 180.0).rem_euclid(360.0) - 180.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_world_cover_tile_names() {
        assert_eq!(world_cover_tile(-121.7603, 46.8523), "N45W123");
        assert_eq!(world_cover_tile(138.7274, 35.3606), "N33E138");
        assert_eq!(world_cover_tile(-1.0, -1.0), "S03W003");
    }

    #[test]
    fn maps_world_cover_classes_to_print_colors() {
        assert_eq!(classify_world_cover(10), SurfaceClass::Forest);
        assert_eq!(classify_world_cover(70), SurfaceClass::Snow);
        assert_eq!(classify_world_cover(80), SurfaceClass::Water);
        assert_eq!(classify_world_cover(60), SurfaceClass::Rock);
        assert_eq!(classify_world_cover(30), SurfaceClass::Rock);
    }

    #[test]
    fn builds_prominent_road_query_with_geometry() {
        let query = overpass_query(
            GeoBounds {
                south: 47.0,
                north: 48.0,
                west: -123.0,
                east: -122.0,
            },
            PROMINENT_HIGHWAYS,
        );
        assert!(query.contains("motorway"));
        assert!(query.contains("secondary_link"));
        assert!(query.contains("[\"area\"!=\"yes\"]"));
        assert!(query.contains("(47.0000000,-123.0000000,48.0000000,-122.0000000)"));
        assert!(query.ends_with("out tags geom;"));
    }

    #[test]
    fn assigns_wider_lines_to_higher_road_classes() {
        let tags = |class: &str| HashMap::from([("highway".into(), class.into())]);
        assert!(road_width_scale(&tags("motorway")) > road_width_scale(&tags("primary")));
        assert!(road_width_scale(&tags("primary")) > road_width_scale(&tags("secondary")));
        assert_eq!(road_width_scale(&tags("residential")), None);
    }

    #[test]
    fn builds_trail_fallback_query_and_widths() {
        let query = overpass_query(
            GeoBounds {
                south: 46.8,
                north: 46.9,
                west: -121.9,
                east: -121.7,
            },
            FALLBACK_TRAILS,
        );
        assert!(query.contains("path|footway|bridleway|track|cycleway"));
        let tags = |class: &str| HashMap::from([("highway".into(), class.into())]);
        assert!(trail_width_scale(&tags("track")) > trail_width_scale(&tags("path")));
        assert_eq!(trail_width_scale(&tags("primary")), None);
    }

    #[test]
    fn unwraps_longitudes_around_the_date_line() {
        assert!((unwrap_longitude(-179.9, 179.9) - 180.1).abs() < 0.001);
        assert!((unwrap_longitude(179.9, -179.9) + 180.1).abs() < 0.001);
    }
}
