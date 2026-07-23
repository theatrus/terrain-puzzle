use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use image::RgbImage;
use reqwest::blocking::Client;
use terrain_core::{GenerationSpec, HeightField};

use crate::cache;

const TILE_SIZE: f64 = 256.0;
const EARTH_CIRCUMFERENCE_M: f64 = 40_075_016.686;
const TILE_BASE_URL: &str = "https://s3.amazonaws.com/elevation-tiles-prod/terrarium";
const ATTRIBUTION_URL: &str = "https://github.com/tilezen/joerd/blob/master/docs/attribution.md";

pub fn fetch_height_field(spec: &GenerationSpec, cache_dir: &Path) -> Result<HeightField> {
    let samples = spec.effective_samples_per_piece();
    let sample_width = (spec.columns * samples + 1) as usize;
    let sample_height = (spec.rows * samples + 1) as usize;
    fetch_height_field_at_size(spec, cache_dir, sample_width, sample_height)
}

pub fn fetch_preview_height_field(
    spec: &GenerationSpec,
    cache_dir: &Path,
    size: usize,
) -> Result<HeightField> {
    let size = size.clamp(32, 128);
    fetch_height_field_at_size(spec, cache_dir, size, size)
}

fn fetch_height_field_at_size(
    spec: &GenerationSpec,
    cache_dir: &Path,
    sample_width: usize,
    sample_height: usize,
) -> Result<HeightField> {
    let zoom = choose_zoom(spec, sample_width.max(sample_height));
    let client = Client::builder()
        .user_agent("terrain-puzzle/0.1 (+local terrain mesh generator)")
        .timeout(Duration::from_secs(20))
        .build()?;
    let mut tiles: HashMap<(u32, u32), RgbImage> = HashMap::new();
    let half_lat = spec.ground_span_km / 2.0 / 110.574;
    let longitude_scale = (111.32 * spec.center_lat.to_radians().cos().abs()).max(20.0);
    let half_lon = spec.ground_span_km / 2.0 / longitude_scale;
    let south = (spec.center_lat - half_lat).max(-85.0);
    let north = (spec.center_lat + half_lat).min(85.0);
    let west = spec.center_lon - half_lon;
    let east = spec.center_lon + half_lon;
    let mut values_m = Vec::with_capacity(sample_width * sample_height);

    for row in 0..sample_height {
        let v = row as f64 / (sample_height - 1) as f64;
        let latitude = south + (north - south) * v;
        for column in 0..sample_width {
            let u = column as f64 / (sample_width - 1) as f64;
            let longitude = normalize_longitude(west + (east - west) * u);
            values_m.push(sample_elevation(
                &client, cache_dir, &mut tiles, zoom, longitude, latitude,
            )?);
        }
    }

    HeightField::new(
        sample_width,
        sample_height,
        values_m,
        format!("Mapzen Terrain Tiles on AWS, Terrarium z{zoom}; attribution: {ATTRIBUTION_URL}"),
    )
}

fn choose_zoom(spec: &GenerationSpec, samples: usize) -> u8 {
    let target_resolution_m =
        spec.ground_span_km * 1_000.0 / (samples.saturating_sub(1).max(1)) as f64;
    let latitude_scale = spec.center_lat.to_radians().cos().abs().max(0.1);
    let desired = (EARTH_CIRCUMFERENCE_M * latitude_scale
        / (TILE_SIZE * target_resolution_m.max(1.0)))
    .log2()
    .ceil() as i32;
    desired.clamp(5, 14) as u8
}

fn sample_elevation(
    client: &Client,
    cache_dir: &Path,
    tiles: &mut HashMap<(u32, u32), RgbImage>,
    zoom: u8,
    longitude: f64,
    latitude: f64,
) -> Result<f32> {
    let tile_count = 1_u32 << zoom;
    let x = (longitude + 180.0) / 360.0 * tile_count as f64;
    let latitude_radians = latitude.clamp(-85.051_128_78, 85.051_128_78).to_radians();
    let y = (1.0
        - (latitude_radians.tan() + 1.0 / latitude_radians.cos()).ln() / std::f64::consts::PI)
        / 2.0
        * tile_count as f64;
    let tile_x = x.floor() as u32 % tile_count;
    let tile_y = (y.floor() as u32).min(tile_count - 1);
    let pixel_x = ((x.fract() * TILE_SIZE).floor() as u32).min(255);
    let pixel_y = ((y.fract() * TILE_SIZE).floor() as u32).min(255);

    if let std::collections::hash_map::Entry::Vacant(entry) = tiles.entry((tile_x, tile_y)) {
        let tile = load_tile(client, cache_dir, zoom, tile_x, tile_y)?;
        entry.insert(tile);
    }
    let pixel = tiles
        .get(&(tile_x, tile_y))
        .context("elevation tile cache lost a tile")?
        .get_pixel(pixel_x, pixel_y);
    Ok(pixel[0] as f32 * 256.0 + pixel[1] as f32 + pixel[2] as f32 / 256.0 - 32_768.0)
}

fn load_tile(client: &Client, cache_dir: &Path, zoom: u8, x: u32, y: u32) -> Result<RgbImage> {
    let path = cache_path(cache_dir, zoom, x, y);
    let bytes = if path.is_file() {
        fs::read(&path).with_context(|| format!("read cached tile {}", path.display()))?
    } else {
        let url = format!("{TILE_BASE_URL}/{zoom}/{x}/{y}.png");
        let response = client
            .get(&url)
            .send()
            .with_context(|| format!("download elevation tile {zoom}/{x}/{y}"))?;
        if !response.status().is_success() {
            bail!(
                "elevation tile {zoom}/{x}/{y} returned {}",
                response.status()
            );
        }
        let bytes = response.bytes()?.to_vec();
        cache::store(&path, &bytes)
            .with_context(|| format!("cache elevation tile {}", path.display()))?;
        bytes
    };

    let image = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
        .with_context(|| format!("decode elevation tile {zoom}/{x}/{y}"))?
        .to_rgb8();
    if image.width() != 256 || image.height() != 256 {
        bail!(
            "elevation tile {zoom}/{x}/{y} has unexpected size {}x{}",
            image.width(),
            image.height()
        );
    }
    Ok(image)
}

fn cache_path(cache_dir: &Path, zoom: u8, x: u32, y: u32) -> PathBuf {
    cache_dir
        .join(zoom.to_string())
        .join(x.to_string())
        .join(format!("{y}.png"))
}

fn normalize_longitude(longitude: f64) -> f64 {
    (longitude + 180.0).rem_euclid(360.0) - 180.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_stays_in_supported_range() {
        let spec = GenerationSpec::default();
        assert!((5..=14).contains(&choose_zoom(&spec, 85)));
    }

    #[test]
    fn longitude_wraps() {
        assert!((normalize_longitude(181.0) + 179.0).abs() < f64::EPSILON);
        assert!((normalize_longitude(-181.0) - 179.0).abs() < f64::EPSILON);
    }
}
