use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use geotiff_reader::cog::HttpGeoTiffFile;
use terrain_core::{GenerationSpec, SurfaceClass, SurfaceField};

const WORLD_COVER_BASE_URL: &str =
    "https://esa-worldcover.s3.eu-central-1.amazonaws.com/v200/2021/map";
const WORLD_COVER_INFO_URL: &str = "https://worldcover2021.esa.int/download";
const WORLD_COVER_ATTRIBUTION: &str = "© ESA WorldCover project / Contains modified Copernicus Sentinel data (2021) processed by ESA WorldCover consortium";

#[derive(Debug)]
struct SamplePoint {
    output_index: usize,
    longitude: f64,
    latitude: f64,
}

pub fn fetch_surface_field(spec: &GenerationSpec) -> Result<SurfaceField> {
    let width = (spec.columns * spec.samples_per_piece + 1) as usize;
    let height = (spec.rows * spec.samples_per_piece + 1) as usize;
    let half_lat = spec.ground_span_km / 2.0 / 110.574;
    let longitude_scale = (111.32 * spec.center_lat.to_radians().cos().abs()).max(20.0);
    let half_lon = spec.ground_span_km / 2.0 / longitude_scale;
    let south = (spec.center_lat - half_lat).max(-85.0);
    let north = (spec.center_lat + half_lat).min(85.0);
    let west = spec.center_lon - half_lon;
    let east = spec.center_lon + half_lon;
    let mut tiles = HashMap::<String, Vec<SamplePoint>>::new();

    for row in 0..height {
        let v = row as f64 / (height - 1) as f64;
        let latitude = south + (north - south) * v;
        for column in 0..width {
            let u = column as f64 / (width - 1) as f64;
            let longitude = normalize_longitude(west + (east - west) * u);
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
    Ok(field)
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
}
