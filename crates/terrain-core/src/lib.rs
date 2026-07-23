use std::{
    collections::{HashMap, VecDeque},
    fs::{self, File},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};
use geo::{Area, Buffer, Coord, LineString, Polygon};
use serde::{Deserialize, Serialize};
use spade::{ConstrainedDelaunayTriangulation, Point2, Triangulation};
use ttf_parser::{Face, OutlineBuilder};
use zip::{ZipWriter, write::SimpleFileOptions};

const TRAY_FONT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/fonts/AtkinsonHyperlegible-Regular.ttf"
));

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GenerationSpec {
    pub center_lat: f64,
    pub center_lon: f64,
    pub ground_span_km: f64,
    pub width_mm: f32,
    pub rows: u32,
    pub columns: u32,
    pub base_mm: f32,
    pub relief_mm: f32,
    pub clearance_mm: f32,
    pub samples_per_piece: u32,
    pub overlay_samples_per_piece: u32,
    pub solid_model: bool,
    pub place_name: String,
    pub tray: TraySpec,
    pub buildings: BuildingSpec,
    pub color_output: ColorOutputSpec,
}

impl Default for GenerationSpec {
    fn default() -> Self {
        Self {
            center_lat: 46.8523,
            center_lon: -121.7603,
            ground_span_km: 18.0,
            width_mm: 180.0,
            rows: 3,
            columns: 3,
            base_mm: 2.4,
            relief_mm: 14.0,
            clearance_mm: 0.14,
            samples_per_piece: 64,
            overlay_samples_per_piece: 112,
            solid_model: false,
            place_name: "Mount Rainier".into(),
            tray: TraySpec::default(),
            buildings: BuildingSpec::default(),
            color_output: ColorOutputSpec::default(),
        }
    }
}

impl GenerationSpec {
    pub fn validate(&self) -> Result<()> {
        if !(-85.0..=85.0).contains(&self.center_lat) {
            bail!("center latitude must be between -85 and 85 degrees");
        }
        if !(-180.0..=180.0).contains(&self.center_lon) {
            bail!("center longitude must be between -180 and 180 degrees");
        }
        if !(0.5..=250.0).contains(&self.ground_span_km) {
            bail!("ground span must be between 0.5 and 250 km");
        }
        if !(60.0..=500.0).contains(&self.width_mm) {
            bail!("model width must be between 60 and 500 mm");
        }
        if !(2..=16).contains(&self.rows) || !(2..=16).contains(&self.columns) {
            bail!("piece rows and columns must each be between 2 and 16");
        }
        if !(1.0..=12.0).contains(&self.base_mm) {
            bail!("base depth must be between 1 and 12 mm");
        }
        if !(1.0..=80.0).contains(&self.relief_mm) {
            bail!("relief must be between 1 and 80 mm");
        }
        if !(0.0..=0.8).contains(&self.clearance_mm) {
            bail!("clearance must be between 0 and 0.8 mm");
        }
        if !(16..=160).contains(&self.samples_per_piece) {
            bail!("samples per piece must be between 16 and 160");
        }
        if !(32..=192).contains(&self.overlay_samples_per_piece) {
            bail!("overlay samples per piece must be between 32 and 192");
        }
        if self.place_name.trim().is_empty() || self.place_name.chars().count() > 48 {
            bail!("place label must contain between 1 and 48 characters");
        }
        if self.place_name.chars().any(char::is_control) {
            bail!("place label cannot contain control characters");
        }
        self.tray.validate()?;
        self.buildings.validate()?;
        self.color_output.validate()?;
        Ok(())
    }

    pub fn height_mm(&self) -> f32 {
        self.width_mm * self.rows as f32 / self.columns as f32
    }

    pub fn effective_samples_per_piece(&self) -> u32 {
        if self.color_output.enabled || self.buildings.enabled {
            self.samples_per_piece.max(self.overlay_samples_per_piece)
        } else {
            self.samples_per_piece
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BuildingSpec {
    pub enabled: bool,
    pub z_scale: f32,
}

impl Default for BuildingSpec {
    fn default() -> Self {
        Self {
            enabled: false,
            z_scale: 5.0,
        }
    }
}

impl BuildingSpec {
    fn validate(&self) -> Result<()> {
        if !(0.5..=30.0).contains(&self.z_scale) {
            bail!("building Z scale must be between 0.5 and 30");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TraySpec {
    pub enabled: bool,
    pub tray_color: String,
    pub contour_color: String,
    pub label_color: String,
    pub clearance_mm: f32,
    pub rim_width_mm: f32,
    pub floor_mm: f32,
    pub rim_height_mm: f32,
    pub contour_count: u32,
}

impl Default for TraySpec {
    fn default() -> Self {
        Self {
            enabled: false,
            tray_color: "#252822".into(),
            contour_color: "#E7E4D8".into(),
            label_color: "#F4F3EC".into(),
            clearance_mm: 0.6,
            rim_width_mm: 8.0,
            floor_mm: 1.6,
            rim_height_mm: 3.2,
            contour_count: 18,
        }
    }
}

impl TraySpec {
    fn validate(&self) -> Result<()> {
        for (name, color) in [
            ("tray", &self.tray_color),
            ("contour", &self.contour_color),
            ("tray label", &self.label_color),
        ] {
            if !valid_hex_color(color) {
                bail!("{name} color must use #RRGGBB");
            }
        }
        if !(0.2..=2.0).contains(&self.clearance_mm) {
            bail!("tray clearance must be between 0.2 and 2 mm");
        }
        if !(5.0..=16.0).contains(&self.rim_width_mm) {
            bail!("tray rim width must be between 5 and 16 mm");
        }
        if !(1.0..=4.0).contains(&self.floor_mm) {
            bail!("tray floor must be between 1 and 4 mm");
        }
        if !(2.0..=8.0).contains(&self.rim_height_mm) {
            bail!("tray rim height must be between 2 and 8 mm");
        }
        if !(5..=60).contains(&self.contour_count) {
            bail!("tray contour count must be between 5 and 60");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorOutputSpec {
    pub enabled: bool,
    pub forest_color: String,
    pub rock_color: String,
    pub snow_color: String,
    pub water_color: String,
    pub road_color: String,
    pub roads_enabled: bool,
    pub road_width_mm: f32,
    pub road_height_mm: f32,
    pub minimum_patch_mm: f32,
}

impl Default for ColorOutputSpec {
    fn default() -> Self {
        Self {
            enabled: false,
            forest_color: "#28543A".into(),
            rock_color: "#7C7468".into(),
            snow_color: "#F4F3EC".into(),
            water_color: "#2F76B5".into(),
            road_color: "#D8A33C".into(),
            roads_enabled: true,
            road_width_mm: 1.0,
            road_height_mm: 0.2,
            minimum_patch_mm: 1.2,
        }
    }
}

impl ColorOutputSpec {
    fn validate(&self) -> Result<()> {
        for (name, color) in [
            ("forest", &self.forest_color),
            ("rock", &self.rock_color),
            ("snow", &self.snow_color),
            ("water", &self.water_color),
            ("road", &self.road_color),
        ] {
            if !valid_hex_color(color) {
                bail!("{name} color must use #RRGGBB");
            }
        }
        if !(0.6..=5.0).contains(&self.road_width_mm) {
            bail!("road line width must be between 0.6 and 5 mm");
        }
        if !(0.08..=0.4).contains(&self.road_height_mm) {
            bail!("road layer height must be between 0.08 and 0.4 mm");
        }
        if !(0.4..=8.0).contains(&self.minimum_patch_mm) {
            bail!("minimum color patch must be between 0.4 and 8 mm");
        }
        Ok(())
    }
}

fn valid_hex_color(color: &str) -> bool {
    color.len() == 7
        && color.starts_with('#')
        && color[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceClass {
    Rock,
    Forest,
    Snow,
    Water,
    Road,
}

impl SurfaceClass {
    fn material_index(self) -> u32 {
        match self {
            Self::Rock => 0,
            Self::Forest => 1,
            Self::Snow => 2,
            Self::Water => 3,
            Self::Road => 4,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub name: String,
    pub media_type: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectManifest {
    pub generator: String,
    pub terrain_source: String,
    pub surface_source: Option<String>,
    pub spec: GenerationSpec,
    pub artifacts: Vec<Artifact>,
}

#[derive(Debug, Clone)]
pub struct HeightField {
    pub width: usize,
    pub height: usize,
    pub values_m: Vec<f32>,
    pub source: String,
}

#[derive(Debug, Clone)]
pub struct SurfaceField {
    pub width: usize,
    pub height: usize,
    pub classes: Vec<SurfaceClass>,
    pub building_heights_m: Vec<f32>,
    pub source: String,
}

impl SurfaceField {
    pub fn new(
        width: usize,
        height: usize,
        classes: Vec<SurfaceClass>,
        source: impl Into<String>,
    ) -> Result<Self> {
        if width < 2 || height < 2 {
            bail!("surface field must be at least 2 by 2");
        }
        if classes.len() != width * height {
            bail!("surface field dimensions do not match its values");
        }
        Ok(Self {
            width,
            height,
            classes,
            building_heights_m: vec![0.0; width * height],
            source: source.into(),
        })
    }

    pub fn filter_small_patches(&mut self, print_width_mm: f32, minimum_patch_mm: f32) {
        let cells_across =
            minimum_patch_mm / print_width_mm.max(f32::EPSILON) * (self.width - 1) as f32;
        let minimum_cells = (std::f32::consts::PI * (cells_across * 0.5).powi(2))
            .ceil()
            .max(2.0) as usize;
        for _ in 0..2 {
            self.filter_components_smaller_than(minimum_cells);
        }
    }

    pub fn paint_polyline(
        &mut self,
        points: &[[f32; 2]],
        print_width_mm: f32,
        line_width_mm: f32,
        class: SurfaceClass,
    ) {
        if points.len() < 2 {
            return;
        }
        let cells_per_mm = (self.width - 1) as f32 / print_width_mm.max(f32::EPSILON);
        let radius = (line_width_mm * 0.5 * cells_per_mm).max(0.75);
        for segment in points.windows(2) {
            let start = [
                segment[0][0] * (self.width - 1) as f32,
                segment[0][1] * (self.height - 1) as f32,
            ];
            let end = [
                segment[1][0] * (self.width - 1) as f32,
                segment[1][1] * (self.height - 1) as f32,
            ];
            let min_x = (start[0].min(end[0]) - radius).floor().max(0.0) as usize;
            let max_x = (start[0].max(end[0]) + radius)
                .ceil()
                .min((self.width - 1) as f32) as usize;
            let min_y = (start[1].min(end[1]) - radius).floor().max(0.0) as usize;
            let max_y = (start[1].max(end[1]) + radius)
                .ceil()
                .min((self.height - 1) as f32) as usize;
            let delta = [end[0] - start[0], end[1] - start[1]];
            let length_squared = delta[0] * delta[0] + delta[1] * delta[1];
            for y in min_y..=max_y {
                for x in min_x..=max_x {
                    let offset = [x as f32 - start[0], y as f32 - start[1]];
                    let t = if length_squared <= f32::EPSILON {
                        0.0
                    } else {
                        ((offset[0] * delta[0] + offset[1] * delta[1]) / length_squared)
                            .clamp(0.0, 1.0)
                    };
                    let nearest = [start[0] + delta[0] * t, start[1] + delta[1] * t];
                    let distance_squared =
                        (x as f32 - nearest[0]).powi(2) + (y as f32 - nearest[1]).powi(2);
                    if distance_squared <= radius * radius {
                        self.classes[y * self.width + x] = class;
                    }
                }
            }
        }
    }

    pub fn paint_building(&mut self, points: &[[f32; 2]], height_m: f32) {
        if points.len() < 3 || !height_m.is_finite() || height_m <= 0.0 {
            return;
        }
        let pixels = points
            .iter()
            .map(|point| {
                [
                    point[0] * (self.width - 1) as f32,
                    point[1] * (self.height - 1) as f32,
                ]
            })
            .collect::<Vec<_>>();
        let polygon_min_x = pixels
            .iter()
            .map(|point| point[0])
            .fold(f32::INFINITY, f32::min);
        let polygon_max_x = pixels
            .iter()
            .map(|point| point[0])
            .fold(f32::NEG_INFINITY, f32::max);
        let polygon_min_y = pixels
            .iter()
            .map(|point| point[1])
            .fold(f32::INFINITY, f32::min);
        let polygon_max_y = pixels
            .iter()
            .map(|point| point[1])
            .fold(f32::NEG_INFINITY, f32::max);
        if polygon_max_x < 0.0
            || polygon_min_x > (self.width - 1) as f32
            || polygon_max_y < 0.0
            || polygon_min_y > (self.height - 1) as f32
        {
            return;
        }
        let min_x = polygon_min_x.floor().max(0.0) as usize;
        let max_x = polygon_max_x.ceil().min((self.width - 1) as f32) as usize;
        let min_y = polygon_min_y.floor().max(0.0) as usize;
        let max_y = polygon_max_y.ceil().min((self.height - 1) as f32) as usize;
        let mut painted = false;
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                if point_in_polygon([x as f32, y as f32], &pixels) {
                    let height = &mut self.building_heights_m[y * self.width + x];
                    *height = height.max(height_m);
                    painted = true;
                }
            }
        }
        if !painted {
            let center = pixels.iter().fold([0.0, 0.0], |sum, point| {
                [sum[0] + point[0], sum[1] + point[1]]
            });
            let x = (center[0] / pixels.len() as f32)
                .round()
                .clamp(0.0, (self.width - 1) as f32) as usize;
            let y = (center[1] / pixels.len() as f32)
                .round()
                .clamp(0.0, (self.height - 1) as f32) as usize;
            let height = &mut self.building_heights_m[y * self.width + x];
            *height = height.max(height_m);
        }
    }

    fn filter_components_smaller_than(&mut self, minimum_cells: usize) {
        let original = self.classes.clone();
        let mut visited = vec![false; original.len()];
        for start in 0..original.len() {
            if visited[start] {
                continue;
            }
            let class = original[start];
            let mut queue = VecDeque::from([start]);
            let mut component = Vec::new();
            let mut neighbours = [0_usize; 5];
            visited[start] = true;
            while let Some(index) = queue.pop_front() {
                component.push(index);
                let x = index % self.width;
                let y = index / self.width;
                for neighbour in [
                    x.checked_sub(1).map(|value| y * self.width + value),
                    (x + 1 < self.width).then_some(y * self.width + x + 1),
                    y.checked_sub(1).map(|value| value * self.width + x),
                    (y + 1 < self.height).then_some((y + 1) * self.width + x),
                ]
                .into_iter()
                .flatten()
                {
                    let neighbour_class = original[neighbour];
                    if neighbour_class == class {
                        if !visited[neighbour] {
                            visited[neighbour] = true;
                            queue.push_back(neighbour);
                        }
                    } else {
                        neighbours[neighbour_class.material_index() as usize] += 1;
                    }
                }
            }
            if component.len() < minimum_cells {
                let replacement = neighbours
                    .into_iter()
                    .enumerate()
                    .max_by_key(|(index, count)| (*count, usize::MAX - *index))
                    .map(|(index, _)| match index {
                        1 => SurfaceClass::Forest,
                        2 => SurfaceClass::Snow,
                        3 => SurfaceClass::Water,
                        4 => SurfaceClass::Road,
                        _ => SurfaceClass::Rock,
                    })
                    .unwrap_or(SurfaceClass::Rock);
                for index in component {
                    self.classes[index] = replacement;
                }
            }
        }
    }

    fn at(&self, u: f32, v: f32) -> SurfaceClass {
        let x = (u.clamp(0.0, 1.0) * (self.width - 1) as f32).round() as usize;
        let y = (v.clamp(0.0, 1.0) * (self.height - 1) as f32).round() as usize;
        self.classes[y * self.width + x]
    }

    fn building_height_at(&self, u: f32, v: f32) -> f32 {
        let x = (u.clamp(0.0, 1.0) * (self.width - 1) as f32).round() as usize;
        let y = (v.clamp(0.0, 1.0) * (self.height - 1) as f32).round() as usize;
        self.building_heights_m[y * self.width + x]
    }

    fn coverage(&self) -> [f32; 5] {
        let mut counts = [0_usize; 5];
        for class in &self.classes {
            counts[class.material_index() as usize] += 1;
        }
        let total = self.classes.len() as f32;
        counts.map(|count| count as f32 * 100.0 / total)
    }
}

impl HeightField {
    pub fn new(
        width: usize,
        height: usize,
        values_m: Vec<f32>,
        source: impl Into<String>,
    ) -> Result<Self> {
        if width < 2 || height < 2 {
            bail!("height field must be at least 2 by 2");
        }
        if values_m.len() != width * height {
            bail!("height field dimensions do not match its values");
        }
        if values_m.iter().any(|value| !value.is_finite()) {
            bail!("height field contains a non-finite value");
        }
        Ok(Self {
            width,
            height,
            values_m,
            source: source.into(),
        })
    }

    fn normalized_at(&self, u: f32, v: f32, minimum: f32, range: f32) -> f32 {
        let x = u.clamp(0.0, 1.0) * (self.width - 1) as f32;
        let y = v.clamp(0.0, 1.0) * (self.height - 1) as f32;
        let x0 = x.floor() as usize;
        let y0 = y.floor() as usize;
        let x1 = (x0 + 1).min(self.width - 1);
        let y1 = (y0 + 1).min(self.height - 1);
        let tx = x - x0 as f32;
        let ty = y - y0 as f32;
        let sample =
            |sample_x: usize, sample_y: usize| self.values_m[sample_y * self.width + sample_x];
        let bottom = sample(x0, y0) * (1.0 - tx) + sample(x1, y0) * tx;
        let top = sample(x0, y1) * (1.0 - tx) + sample(x1, y1) * tx;
        ((bottom * (1.0 - ty) + top * ty - minimum) / range).clamp(0.0, 1.0)
    }

    fn range(&self) -> (f32, f32) {
        let minimum = self.values_m.iter().copied().fold(f32::INFINITY, f32::min);
        let maximum = self
            .values_m
            .iter()
            .copied()
            .fold(f32::NEG_INFINITY, f32::max);
        (minimum, (maximum - minimum).max(1.0))
    }
}

#[derive(Debug, Clone)]
struct Mesh {
    name: String,
    vertices: Vec<[f32; 3]>,
    triangles: Vec<[u32; 3]>,
    materials: Vec<SurfaceClass>,
}

#[derive(Default)]
struct MeshBuilder {
    vertices: Vec<[f32; 3]>,
    triangles: Vec<[u32; 3]>,
    materials: Vec<SurfaceClass>,
    indices: HashMap<(i64, i64, i64), u32>,
}

impl MeshBuilder {
    fn vertex(&mut self, point: [f32; 3]) -> u32 {
        let key = (
            (point[0] * 100_000.0).round() as i64,
            (point[1] * 100_000.0).round() as i64,
            (point[2] * 100_000.0).round() as i64,
        );
        *self.indices.entry(key).or_insert_with(|| {
            let index = self.vertices.len() as u32;
            self.vertices.push(point);
            index
        })
    }

    fn triangle(&mut self, a: [f32; 3], b: [f32; 3], c: [f32; 3], material: SurfaceClass) {
        let triangle = [self.vertex(a), self.vertex(b), self.vertex(c)];
        self.triangles.push(triangle);
        self.materials.push(material);
    }

    fn quad(&mut self, a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3], material: SurfaceClass) {
        self.triangle(a, b, c, material);
        self.triangle(a, c, d, material);
    }

    fn finish(self, name: impl Into<String>) -> Mesh {
        Mesh {
            name: name.into(),
            vertices: self.vertices,
            triangles: self.triangles,
            materials: self.materials,
        }
    }
}

fn build_tray(spec: &GenerationSpec, height_field: Option<&HeightField>) -> Result<Mesh> {
    let tray = &spec.tray;
    let inner_width = spec.width_mm + tray.clearance_mm * 2.0;
    let inner_height = spec.height_mm() + tray.clearance_mm * 2.0;
    let outer_width = inner_width + tray.rim_width_mm * 2.0;
    let outer_height = inner_height + tray.rim_width_mm * 2.0;
    let inner_x0 = tray.rim_width_mm;
    let inner_y0 = tray.rim_width_mm;
    let inner_x1 = inner_x0 + inner_width;
    let inner_y1 = inner_y0 + inner_height;
    let floor_z = tray.floor_mm;
    let rim_z = tray.floor_mm + tray.rim_height_mm;
    let mut x_coordinates = regular_coordinates(0.0, outer_width, 0.35);
    let mut y_coordinates = regular_coordinates(0.0, outer_height, 0.35);
    insert_coordinate(&mut x_coordinates, inner_x0);
    insert_coordinate(&mut x_coordinates, inner_x1);
    insert_coordinate(&mut y_coordinates, inner_y0);
    insert_coordinate(&mut y_coordinates, inner_y1);
    let inner_x = x_coordinates
        .iter()
        .copied()
        .filter(|x| *x >= inner_x0 && *x <= inner_x1)
        .collect::<Vec<_>>();
    let inner_y = y_coordinates
        .iter()
        .copied()
        .filter(|y| *y >= inner_y0 && *y <= inner_y1)
        .collect::<Vec<_>>();
    let left_rim_x = x_coordinates
        .iter()
        .copied()
        .filter(|x| *x <= inner_x0)
        .collect::<Vec<_>>();
    let right_rim_x = x_coordinates
        .iter()
        .copied()
        .filter(|x| *x >= inner_x1)
        .collect::<Vec<_>>();
    let front_rim_y = y_coordinates
        .iter()
        .copied()
        .filter(|y| *y <= inner_y0)
        .collect::<Vec<_>>();
    let back_rim_y = y_coordinates
        .iter()
        .copied()
        .filter(|y| *y >= inner_y1)
        .collect::<Vec<_>>();
    let label = tray_label(spec, outer_width, tray.rim_width_mm)?;
    let z_coordinates = [0.0, rim_z];

    let height_range = height_field.map(HeightField::range);
    let mut mesh = MeshBuilder::default();

    for y in inner_y.windows(2) {
        for x in inner_x.windows(2) {
            let u0 = (x[0] - inner_x0) / inner_width;
            let u1 = (x[1] - inner_x0) / inner_width;
            let v0 = (y[0] - inner_y0) / inner_height;
            let v1 = (y[1] - inner_y0) / inner_height;
            let bands = [
                contour_band(spec, height_field, height_range, u0, v0),
                contour_band(spec, height_field, height_range, u1, v0),
                contour_band(spec, height_field, height_range, u1, v1),
                contour_band(spec, height_field, height_range, u0, v1),
            ];
            let material = if bands.iter().any(|band| *band != bands[0]) {
                SurfaceClass::Forest
            } else {
                SurfaceClass::Rock
            };
            mesh.quad(
                [x[0], y[0], floor_z],
                [x[1], y[0], floor_z],
                [x[1], y[1], floor_z],
                [x[0], y[1], floor_z],
                material,
            );
        }
    }

    for x in x_coordinates.windows(2) {
        for y in front_rim_y.windows(2) {
            mesh.quad(
                [x[0], y[0], rim_z],
                [x[1], y[0], rim_z],
                [x[1], y[1], rim_z],
                [x[0], y[1], rim_z],
                SurfaceClass::Rock,
            );
        }
        for y in back_rim_y.windows(2) {
            mesh.quad(
                [x[0], y[0], rim_z],
                [x[1], y[0], rim_z],
                [x[1], y[1], rim_z],
                [x[0], y[1], rim_z],
                SurfaceClass::Rock,
            );
        }
    }
    for y in inner_y.windows(2) {
        for x in left_rim_x.windows(2) {
            mesh.quad(
                [x[0], y[0], rim_z],
                [x[1], y[0], rim_z],
                [x[1], y[1], rim_z],
                [x[0], y[1], rim_z],
                SurfaceClass::Rock,
            );
        }
        for x in right_rim_x.windows(2) {
            mesh.quad(
                [x[0], y[0], rim_z],
                [x[1], y[0], rim_z],
                [x[1], y[1], rim_z],
                [x[0], y[1], rim_z],
                SurfaceClass::Rock,
            );
        }
    }

    for x in inner_x.windows(2) {
        mesh.quad(
            [x[0], inner_y0, floor_z],
            [x[1], inner_y0, floor_z],
            [x[1], inner_y0, rim_z],
            [x[0], inner_y0, rim_z],
            SurfaceClass::Rock,
        );
        mesh.quad(
            [x[1], inner_y1, floor_z],
            [x[0], inner_y1, floor_z],
            [x[0], inner_y1, rim_z],
            [x[1], inner_y1, rim_z],
            SurfaceClass::Rock,
        );
    }
    for y in inner_y.windows(2) {
        mesh.quad(
            [inner_x0, y[1], floor_z],
            [inner_x0, y[0], floor_z],
            [inner_x0, y[0], rim_z],
            [inner_x0, y[1], rim_z],
            SurfaceClass::Rock,
        );
        mesh.quad(
            [inner_x1, y[0], floor_z],
            [inner_x1, y[1], floor_z],
            [inner_x1, y[1], rim_z],
            [inner_x1, y[0], rim_z],
            SurfaceClass::Rock,
        );
    }

    for z in z_coordinates.windows(2) {
        for x in x_coordinates.windows(2) {
            mesh.quad(
                [x[0], 0.0, z[0]],
                [x[1], 0.0, z[0]],
                [x[1], 0.0, z[1]],
                [x[0], 0.0, z[1]],
                SurfaceClass::Rock,
            );
            mesh.quad(
                [x[1], outer_height, z[0]],
                [x[0], outer_height, z[0]],
                [x[0], outer_height, z[1]],
                [x[1], outer_height, z[1]],
                SurfaceClass::Rock,
            );
        }
        for y in y_coordinates.windows(2) {
            mesh.quad(
                [0.0, y[1], z[0]],
                [0.0, y[0], z[0]],
                [0.0, y[0], z[1]],
                [0.0, y[1], z[1]],
                SurfaceClass::Rock,
            );
            mesh.quad(
                [outer_width, y[0], z[0]],
                [outer_width, y[1], z[0]],
                [outer_width, y[1], z[1]],
                [outer_width, y[0], z[1]],
                SurfaceClass::Rock,
            );
        }
    }

    let center = [outer_width * 0.5, outer_height * 0.5, 0.0];
    let mut boundary = Vec::new();
    boundary.extend(x_coordinates.iter().map(|x| [*x, 0.0, 0.0]));
    boundary.extend(y_coordinates.iter().skip(1).map(|y| [outer_width, *y, 0.0]));
    boundary.extend(
        x_coordinates
            .iter()
            .rev()
            .skip(1)
            .map(|x| [*x, outer_height, 0.0]),
    );
    boundary.extend(
        y_coordinates
            .iter()
            .rev()
            .skip(1)
            .take(y_coordinates.len().saturating_sub(2))
            .map(|y| [0.0, *y, 0.0]),
    );
    for index in 0..boundary.len() {
        let current = boundary[index];
        let next = boundary[(index + 1) % boundary.len()];
        mesh.triangle(center, next, current, SurfaceClass::Rock);
    }
    label.add_embossed_shapes(&mut mesh, rim_z)?;

    Ok(mesh.finish("terrain-tray"))
}

fn contour_band(
    spec: &GenerationSpec,
    height_field: Option<&HeightField>,
    height_range: Option<(f32, f32)>,
    u: f32,
    v: f32,
) -> u32 {
    (normalized_height(
        height_field,
        height_range,
        u,
        v,
        spec.center_lat,
        spec.center_lon,
    ) * spec.tray.contour_count as f32)
        .floor() as u32
}

fn regular_coordinates(start: f32, end: f32, maximum_step: f32) -> Vec<f32> {
    let segments = (((end - start) / maximum_step).ceil().max(1.0) as usize).min(1_024);
    (0..=segments)
        .map(|index| start + (end - start) * index as f32 / segments as f32)
        .collect()
}

fn insert_coordinate(coordinates: &mut Vec<f32>, value: f32) {
    coordinates.push(value);
    coordinates.sort_by(f32::total_cmp);
    coordinates.dedup_by(|a, b| (*a - *b).abs() < 0.000_01);
}

fn tray_font() -> Result<Face<'static>> {
    Face::parse(TRAY_FONT, 0).map_err(|error| anyhow!("parse bundled tray font: {error:?}"))
}

#[derive(Default)]
struct GlyphOutline {
    contours: Vec<Vec<[f32; 2]>>,
    current: Vec<[f32; 2]>,
}

impl GlyphOutline {
    fn push_point(&mut self, point: [f32; 2]) {
        if self
            .current
            .last()
            .is_none_or(|last| distance_squared(*last, point) > 0.000_001)
        {
            self.current.push(point);
        }
    }

    fn finish_contour(&mut self) {
        if self.current.len() > 2 {
            if distance_squared(self.current[0], *self.current.last().unwrap()) < 0.000_001 {
                self.current.pop();
            }
            if self.current.len() > 2 {
                self.contours.push(std::mem::take(&mut self.current));
                return;
            }
        }
        self.current.clear();
    }
}

impl OutlineBuilder for GlyphOutline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.finish_contour();
        self.push_point([x, y]);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.push_point([x, y]);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let start = *self.current.last().unwrap_or(&[x, y]);
        flatten_quadratic(start, [x1, y1], [x, y], 0, &mut self.current);
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let start = *self.current.last().unwrap_or(&[x, y]);
        flatten_cubic(start, [x1, y1], [x2, y2], [x, y], 0, &mut self.current);
    }

    fn close(&mut self) {
        self.finish_contour();
    }
}

fn flatten_quadratic(
    start: [f32; 2],
    control: [f32; 2],
    end: [f32; 2],
    depth: u8,
    output: &mut Vec<[f32; 2]>,
) {
    if depth >= 10 || point_line_distance(control, start, end) <= 2.0 {
        output.push(end);
        return;
    }
    let start_control = midpoint(start, control);
    let control_end = midpoint(control, end);
    let middle = midpoint(start_control, control_end);
    flatten_quadratic(start, start_control, middle, depth + 1, output);
    flatten_quadratic(middle, control_end, end, depth + 1, output);
}

fn flatten_cubic(
    start: [f32; 2],
    control_a: [f32; 2],
    control_b: [f32; 2],
    end: [f32; 2],
    depth: u8,
    output: &mut Vec<[f32; 2]>,
) {
    let flatness =
        point_line_distance(control_a, start, end).max(point_line_distance(control_b, start, end));
    if depth >= 10 || flatness <= 2.0 {
        output.push(end);
        return;
    }
    let start_a = midpoint(start, control_a);
    let a_b = midpoint(control_a, control_b);
    let b_end = midpoint(control_b, end);
    let first_middle = midpoint(start_a, a_b);
    let second_middle = midpoint(a_b, b_end);
    let middle = midpoint(first_middle, second_middle);
    flatten_cubic(start, start_a, first_middle, middle, depth + 1, output);
    flatten_cubic(middle, second_middle, b_end, end, depth + 1, output);
}

fn midpoint(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5]
}

fn distance_squared(a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)
}

fn point_line_distance(point: [f32; 2], start: [f32; 2], end: [f32; 2]) -> f32 {
    let length_squared = distance_squared(start, end);
    if length_squared <= f32::EPSILON {
        return distance_squared(point, start).sqrt();
    }
    let cross =
        (end[0] - start[0]) * (start[1] - point[1]) - (start[0] - point[0]) * (end[1] - start[1]);
    cross.abs() / length_squared.sqrt()
}

fn point_in_contours(point: [f32; 2], contours: &[Vec<[f32; 2]>]) -> bool {
    contours
        .iter()
        .filter(|contour| point_in_polygon(point, contour))
        .count()
        % 2
        == 1
}

fn add_extruded_contours(
    mesh: &mut MeshBuilder,
    contours: &[Vec<[f32; 2]>],
    bottom_z: f32,
    top_z: f32,
    material: SurfaceClass,
) -> Result<()> {
    let mut points = Vec::new();
    let mut constraints = Vec::new();
    for contour in contours.iter().filter(|contour| contour.len() > 2) {
        let start = points.len();
        points.extend(
            contour
                .iter()
                .map(|point| Point2::new(f64::from(point[0]), f64::from(point[1]))),
        );
        constraints.extend(
            (0..contour.len()).map(|index| [start + index, start + (index + 1) % contour.len()]),
        );
    }
    if points.len() < 3 {
        return Ok(());
    }

    let triangulation =
        ConstrainedDelaunayTriangulation::<Point2<f64>>::bulk_load_cdt(points, constraints)
            .context("triangulate vector tray label")?;
    for face in triangulation.inner_faces() {
        let positions = face.vertices().map(|vertex| vertex.position());
        let centroid = [
            ((positions[0].x + positions[1].x + positions[2].x) / 3.0) as f32,
            ((positions[0].y + positions[1].y + positions[2].y) / 3.0) as f32,
        ];
        if !point_in_contours(centroid, contours) {
            continue;
        }
        let mut triangle = positions.map(|point| [point.x as f32, point.y as f32]);
        let area = (triangle[1][0] - triangle[0][0]) * (triangle[2][1] - triangle[0][1])
            - (triangle[1][1] - triangle[0][1]) * (triangle[2][0] - triangle[0][0]);
        if area < 0.0 {
            triangle.swap(1, 2);
        }
        mesh.triangle(
            [triangle[0][0], triangle[0][1], top_z],
            [triangle[1][0], triangle[1][1], top_z],
            [triangle[2][0], triangle[2][1], top_z],
            material,
        );
        mesh.triangle(
            [triangle[2][0], triangle[2][1], bottom_z],
            [triangle[1][0], triangle[1][1], bottom_z],
            [triangle[0][0], triangle[0][1], bottom_z],
            material,
        );
    }

    for contour in contours.iter().filter(|contour| contour.len() > 2) {
        for index in 0..contour.len() {
            let a = contour[index];
            let b = contour[(index + 1) % contour.len()];
            let edge = [b[0] - a[0], b[1] - a[1]];
            let edge_length = (edge[0].powi(2) + edge[1].powi(2)).sqrt();
            if edge_length <= f32::EPSILON {
                continue;
            }
            let middle = midpoint(a, b);
            let probe = [
                middle[0] - edge[1] / edge_length * 0.002,
                middle[1] + edge[0] / edge_length * 0.002,
            ];
            if point_in_contours(probe, contours) {
                mesh.quad(
                    [a[0], a[1], bottom_z],
                    [b[0], b[1], bottom_z],
                    [b[0], b[1], top_z],
                    [a[0], a[1], top_z],
                    material,
                );
            } else {
                mesh.quad(
                    [b[0], b[1], bottom_z],
                    [a[0], a[1], bottom_z],
                    [a[0], a[1], top_z],
                    [b[0], b[1], top_z],
                    material,
                );
            }
        }
    }
    Ok(())
}

struct TrayLabel {
    text: String,
    origin_x: f32,
    baseline_y: f32,
    scale: f32,
}

impl TrayLabel {
    fn add_embossed_shapes(&self, mesh: &mut MeshBuilder, rim_z: f32) -> Result<()> {
        let face = tray_font()?;
        let mut pen_x = 0.0;
        for character in self.text.chars() {
            let glyph_id = face
                .glyph_index(character)
                .ok_or_else(|| anyhow!("tray font has no glyph for {character:?}"))?;
            let advance = face
                .glyph_hor_advance(glyph_id)
                .ok_or_else(|| anyhow!("tray font has no advance for {character:?}"))?
                as f32;
            let mut outline = GlyphOutline::default();
            if face.outline_glyph(glyph_id, &mut outline).is_some() {
                outline.finish_contour();
                let contours = outline
                    .contours
                    .into_iter()
                    .map(|contour| {
                        contour
                            .into_iter()
                            .map(|point| {
                                [
                                    self.origin_x + (pen_x + point[0]) * self.scale,
                                    self.baseline_y + point[1] * self.scale,
                                ]
                            })
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>();
                add_extruded_contours(
                    mesh,
                    &contours,
                    rim_z - 0.02,
                    rim_z + 0.56,
                    SurfaceClass::Snow,
                )?;
            }
            pen_x += advance;
        }
        Ok(())
    }
}

fn tray_label(spec: &GenerationSpec, width: f32, lip_depth: f32) -> Result<TrayLabel> {
    let mut place = spec
        .place_name
        .chars()
        .flat_map(char::to_uppercase)
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, ' ' | '-' | '\'' | '.') {
                character
            } else {
                ' '
            }
        })
        .collect::<String>();
    place = place.split_whitespace().collect::<Vec<_>>().join(" ");
    place.truncate(place.floor_char_boundary(26));
    let latitude = coordinate_label(spec.center_lat, 'N', 'S');
    let longitude = coordinate_label(spec.center_lon, 'E', 'W');
    let text = format!("{place}  {latitude} {longitude}");
    let face = tray_font()?;
    let logical_width = text
        .chars()
        .filter_map(|character| face.glyph_index(character))
        .filter_map(|glyph_id| face.glyph_hor_advance(glyph_id))
        .map(f32::from)
        .sum::<f32>();
    let cap_height = f32::from(face.capital_height().unwrap_or(face.ascender()));
    let scale =
        ((width - 4.0) / logical_width.max(1.0)).min((lip_depth - 1.6) / cap_height.max(1.0));
    let text_width = logical_width * scale;
    let text_height = cap_height * scale;
    Ok(TrayLabel {
        text,
        origin_x: (width - text_width) * 0.5,
        baseline_y: (lip_depth - text_height) * 0.5,
        scale,
    })
}

fn coordinate_label(value: f64, positive: char, negative: char) -> String {
    format!(
        "{:.4}{}",
        value.abs(),
        if value >= 0.0 { positive } else { negative }
    )
}

pub fn generate_project(spec: &GenerationSpec, output_dir: &Path) -> Result<ProjectManifest> {
    generate_project_inner(spec, None, None, output_dir)
}

pub fn generate_project_with_height_field(
    spec: &GenerationSpec,
    height_field: &HeightField,
    output_dir: &Path,
) -> Result<ProjectManifest> {
    generate_project_inner(spec, Some(height_field), None, output_dir)
}

pub fn generate_project_with_fields(
    spec: &GenerationSpec,
    height_field: &HeightField,
    surface_field: Option<&SurfaceField>,
    output_dir: &Path,
) -> Result<ProjectManifest> {
    generate_project_inner(spec, Some(height_field), surface_field, output_dir)
}

fn generate_project_inner(
    spec: &GenerationSpec,
    height_field: Option<&HeightField>,
    surface_field: Option<&SurfaceField>,
    output_dir: &Path,
) -> Result<ProjectManifest> {
    spec.validate()?;
    if spec.color_output.enabled && surface_field.is_none() {
        bail!("color output requires ESA WorldCover surface data");
    }
    if spec.buildings.enabled && surface_field.is_none() {
        bail!("building output requires OpenStreetMap building data");
    }
    fs::create_dir_all(output_dir)
        .with_context(|| format!("create output directory {}", output_dir.display()))?;

    let object_count = if spec.solid_model {
        1
    } else {
        (spec.rows * spec.columns) as usize
    };

    let mut artifacts = Vec::new();
    let project_path = output_dir.join(if spec.solid_model {
        "terrain-solid.3mf"
    } else {
        "terrain-puzzle.3mf"
    });
    let mut project_writer = ThreeMfWriter::new(spec, &project_path)?;
    for index in 0..object_count {
        let row = if spec.solid_model {
            0
        } else {
            index as u32 / spec.columns
        };
        let column = if spec.solid_model {
            0
        } else {
            index as u32 % spec.columns
        };
        let mesh = build_piece(spec, height_field, surface_field, row, column)?;
        let name = if spec.solid_model {
            "terrain-solid.stl".into()
        } else {
            format!("piece-{}-{}.stl", row + 1, column + 1)
        };
        let path = output_dir.join(&name);
        write_binary_stl(&mesh, &path)?;
        artifacts.push(file_artifact(&path, "model/stl")?);
        project_writer.write_mesh(&mesh)?;
    }
    project_writer.finish()?;
    artifacts.push(file_artifact(&project_path, "model/3mf")?);

    if spec.tray.enabled {
        let tray_mesh = build_tray(spec, height_field)?;
        let tray_stl_path = output_dir.join("terrain-tray.stl");
        write_binary_stl(&tray_mesh, &tray_stl_path)?;
        artifacts.push(file_artifact(&tray_stl_path, "model/stl")?);

        let mut tray_spec = spec.clone();
        tray_spec.solid_model = true;
        tray_spec.color_output.enabled = true;
        tray_spec.color_output.rock_color = spec.tray.tray_color.clone();
        tray_spec.color_output.forest_color = spec.tray.contour_color.clone();
        tray_spec.color_output.snow_color = spec.tray.label_color.clone();
        tray_spec.color_output.water_color = spec.tray.tray_color.clone();
        tray_spec.color_output.road_color = spec.tray.tray_color.clone();
        let tray_3mf_path = output_dir.join("terrain-tray.3mf");
        let mut tray_writer = ThreeMfWriter::new(&tray_spec, &tray_3mf_path)?;
        tray_writer.write_mesh(&tray_mesh)?;
        tray_writer.finish()?;
        artifacts.push(file_artifact(&tray_3mf_path, "model/3mf")?);
    }

    let preview_path = output_dir.join("preview.json");
    let preview_size = (spec.rows.max(spec.columns) * spec.effective_samples_per_piece() + 1)
        .clamp(96, 192) as usize;
    let preview = build_preview(spec, height_field, surface_field, preview_size);
    fs::write(&preview_path, serde_json::to_vec(&preview)?)
        .with_context(|| format!("write {}", preview_path.display()))?;
    artifacts.push(file_artifact(&preview_path, "application/json")?);

    let manifest = ProjectManifest {
        generator: format!("terrain-puzzle/{}", env!("CARGO_PKG_VERSION")),
        terrain_source: height_field
            .map(|field| field.source.clone())
            .unwrap_or_else(|| "deterministic-preview-surface".into()),
        surface_source: surface_field.map(|field| field.source.clone()),
        spec: spec.clone(),
        artifacts,
    };
    let manifest_path = output_dir.join("manifest.json");
    fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)
        .with_context(|| format!("write {}", manifest_path.display()))?;

    let mut complete = manifest;
    complete
        .artifacts
        .push(file_artifact(&manifest_path, "application/json")?);
    Ok(complete)
}

fn build_piece(
    spec: &GenerationSpec,
    height_field: Option<&HeightField>,
    surface_field: Option<&SurfaceField>,
    row: u32,
    column: u32,
) -> Result<Mesh> {
    let base_samples = if spec.solid_model {
        (spec.samples_per_piece * 2).clamp(96, 256) as usize
    } else {
        spec.samples_per_piece as usize
    };
    let samples = base_samples.max(spec.effective_samples_per_piece() as usize);
    let piece_width = if spec.solid_model {
        spec.width_mm
    } else {
        spec.width_mm / spec.columns as f32
    };
    let piece_height = if spec.solid_model {
        spec.height_mm()
    } else {
        spec.height_mm() / spec.rows as f32
    };
    let origin_x = if spec.solid_model {
        0.0
    } else {
        column as f32 * piece_width
    };
    let origin_y = if spec.solid_model {
        0.0
    } else {
        row as f32 * piece_height
    };
    let assembled_width = spec.width_mm;
    let assembled_height = spec.height_mm();
    let height_range = height_field.map(HeightField::range);
    let outline = if spec.solid_model {
        solid_outline(spec, samples)
    } else {
        piece_outline(spec, row, column, false)?
    }
    .into_iter()
    .map(|[x, y]| [x - origin_x, y - origin_y])
    .collect::<Vec<_>>();
    let mut points = outline
        .iter()
        .map(|point| Point2::new(point[0] as f64, point[1] as f64))
        .collect::<Vec<_>>();
    let constraints = (0..outline.len())
        .map(|index| [index, (index + 1) % outline.len()])
        .collect::<Vec<_>>();

    let minimum_x = outline
        .iter()
        .map(|point| point[0])
        .fold(f32::INFINITY, f32::min);
    let maximum_x = outline
        .iter()
        .map(|point| point[0])
        .fold(f32::NEG_INFINITY, f32::max);
    let minimum_y = outline
        .iter()
        .map(|point| point[1])
        .fold(f32::INFINITY, f32::min);
    let maximum_y = outline
        .iter()
        .map(|point| point[1])
        .fold(f32::NEG_INFINITY, f32::max);
    let spacing = piece_width.min(piece_height) / samples as f32;
    let grid_columns = ((maximum_x - minimum_x) / spacing).ceil() as usize;
    let grid_rows = ((maximum_y - minimum_y) / spacing).ceil() as usize;
    for grid_y in 0..grid_rows {
        let y = minimum_y + (grid_y as f32 + 0.5) * spacing;
        for grid_x in 0..grid_columns {
            let x = minimum_x + (grid_x as f32 + 0.5) * spacing;
            if point_in_polygon([x, y], &outline) {
                points.push(Point2::new(x as f64, y as f64));
            }
        }
    }

    let triangulation =
        ConstrainedDelaunayTriangulation::<Point2<f64>>::bulk_load_cdt(points, constraints)
            .context("triangulate terrain outline")?;
    let top_count = triangulation.num_vertices();
    let mut vertices = Vec::with_capacity(top_count * 2);
    for layer in 0..2 {
        for vertex in triangulation.vertices() {
            let position = vertex.position();
            let assembled_x = position.x as f32 + origin_x;
            let assembled_y = position.y as f32 + origin_y;
            let z = if layer == 0 {
                spec.base_mm
                    + spec.relief_mm
                        * normalized_height(
                            height_field,
                            height_range,
                            assembled_x / assembled_width,
                            assembled_y / assembled_height,
                            spec.center_lat,
                            spec.center_lon,
                        )
                    + building_height_mm(
                        spec,
                        surface_field,
                        assembled_x / assembled_width,
                        assembled_y / assembled_height,
                    )
                    + road_height_mm(
                        spec,
                        surface_field,
                        assembled_x / assembled_width,
                        assembled_y / assembled_height,
                    )
            } else {
                0.0
            };
            vertices.push([position.x as f32, position.y as f32, z]);
        }
    }

    let mut top_triangles = Vec::with_capacity(triangulation.num_inner_faces());
    let mut top_materials = Vec::with_capacity(triangulation.num_inner_faces());
    for face in triangulation.inner_faces() {
        let face_vertices = face.vertices();
        let positions = face_vertices.map(|vertex| vertex.position());
        let centroid = [
            ((positions[0].x + positions[1].x + positions[2].x) / 3.0) as f32,
            ((positions[0].y + positions[1].y + positions[2].y) / 3.0) as f32,
        ];
        if !point_in_polygon(centroid, &outline) {
            continue;
        }
        let mut top = face_vertices.map(|vertex| vertex.fix().index() as u32);
        let area = (positions[1].x - positions[0].x) * (positions[2].y - positions[0].y)
            - (positions[1].y - positions[0].y) * (positions[2].x - positions[0].x);
        if area < 0.0 {
            top.swap(1, 2);
        }
        top_triangles.push(top);
        top_materials.push(
            surface_field
                .map(|field| {
                    field.at(
                        (centroid[0] + origin_x) / assembled_width,
                        (centroid[1] + origin_y) / assembled_height,
                    )
                })
                .unwrap_or(SurfaceClass::Rock),
        );
    }

    let mut edge_uses = HashMap::<(u32, u32), (u32, [u32; 2])>::new();
    for triangle in &top_triangles {
        for directed in [
            [triangle[0], triangle[1]],
            [triangle[1], triangle[2]],
            [triangle[2], triangle[0]],
        ] {
            let key = if directed[0] < directed[1] {
                (directed[0], directed[1])
            } else {
                (directed[1], directed[0])
            };
            let entry = edge_uses.entry(key).or_insert((0, directed));
            entry.0 += 1;
        }
    }

    let mut triangles = Vec::with_capacity(top_triangles.len() * 2 + edge_uses.len() * 2);
    let mut materials = Vec::with_capacity(triangles.capacity());
    for (top, material) in top_triangles.into_iter().zip(top_materials) {
        triangles.push(top);
        materials.push(material);
        triangles.push([
            top[0] + top_count as u32,
            top[2] + top_count as u32,
            top[1] + top_count as u32,
        ]);
        materials.push(SurfaceClass::Rock);
    }
    for (_, [from, to]) in edge_uses.into_values().filter(|(uses, _)| *uses == 1) {
        triangles.push([from, to + top_count as u32, to]);
        materials.push(SurfaceClass::Rock);
        triangles.push([from, from + top_count as u32, to + top_count as u32]);
        materials.push(SurfaceClass::Rock);
    }

    Ok(Mesh {
        name: if spec.solid_model {
            "Solid Terrain".into()
        } else {
            format!("Piece {}-{}", row + 1, column + 1)
        },
        vertices,
        triangles,
        materials,
    })
}

fn solid_outline(spec: &GenerationSpec, edge_samples: usize) -> Vec<[f32; 2]> {
    let corners = [
        [0.0, 0.0],
        [spec.width_mm, 0.0],
        [spec.width_mm, spec.height_mm()],
        [0.0, spec.height_mm()],
    ];
    let mut outline = Vec::with_capacity(edge_samples * 4);
    for edge in 0..4 {
        let start = corners[edge];
        let end = corners[(edge + 1) % corners.len()];
        for index in 0..edge_samples {
            let t = index as f32 / edge_samples as f32;
            outline.push([
                start[0] + (end[0] - start[0]) * t,
                start[1] + (end[1] - start[1]) * t,
            ]);
        }
    }
    outline
}

fn piece_outline(
    spec: &GenerationSpec,
    row: u32,
    column: u32,
    exact_shared_edge: bool,
) -> Result<Vec<[f32; 2]>> {
    let bottom_left = puzzle_grid_point(spec, row, column);
    let bottom_right = puzzle_grid_point(spec, row, column + 1);
    let top_right = puzzle_grid_point(spec, row + 1, column + 1);
    let top_left = puzzle_grid_point(spec, row + 1, column);
    let nominal_piece_size =
        (spec.width_mm / spec.columns as f32).min(spec.height_mm() / spec.rows as f32);
    let base_depth = nominal_piece_size * 0.17;
    let edge_samples = spec.samples_per_piece.clamp(64, 128) as usize;
    let mut outline = Vec::with_capacity(edge_samples * 4);

    for index in 0..edge_samples {
        let t = index as f32 / edge_samples as f32;
        outline.push(puzzle_edge_point(
            bottom_left,
            bottom_right,
            shared_edge_pattern(0, row, column),
            edge_sign(0, column, row, spec.rows),
            t,
            base_depth,
        ));
    }
    for index in 0..edge_samples {
        let t = index as f32 / edge_samples as f32;
        outline.push(puzzle_edge_point(
            bottom_right,
            top_right,
            shared_edge_pattern(1, column + 1, row),
            edge_sign(1, row, column + 1, spec.columns),
            t,
            base_depth,
        ));
    }
    for index in 0..edge_samples {
        let t = 1.0 - index as f32 / edge_samples as f32;
        outline.push(puzzle_edge_point(
            top_left,
            top_right,
            shared_edge_pattern(0, row + 1, column),
            edge_sign(0, column, row + 1, spec.rows),
            t,
            base_depth,
        ));
    }
    for index in 0..edge_samples {
        let t = 1.0 - index as f32 / edge_samples as f32;
        outline.push(puzzle_edge_point(
            bottom_left,
            top_left,
            shared_edge_pattern(1, column, row),
            edge_sign(1, row, column, spec.columns),
            t,
            base_depth,
        ));
    }

    if !exact_shared_edge && spec.clearance_mm > 0.0 {
        outline = inset_outline(&outline, spec.clearance_mm * 0.5)?;
    }
    Ok(outline)
}

#[derive(Debug, Clone, Copy)]
struct EdgePattern {
    center: f32,
    radius_along: f32,
    depth_scale: f32,
    skew: f32,
}

fn puzzle_grid_point(spec: &GenerationSpec, row: u32, column: u32) -> [f32; 2] {
    let piece_width = spec.width_mm / spec.columns as f32;
    let piece_height = spec.height_mm() / spec.rows as f32;
    let seed = ((row as u64) << 32) | column as u64;
    let x = if column == 0 {
        0.0
    } else if column == spec.columns {
        spec.width_mm
    } else {
        column as f32 * piece_width + (edge_noise(seed, 0) - 0.5) * piece_width * 0.18
    };
    let y = if row == 0 {
        0.0
    } else if row == spec.rows {
        spec.height_mm()
    } else {
        row as f32 * piece_height + (edge_noise(seed, 1) - 0.5) * piece_height * 0.18
    };
    [x, y]
}

fn shared_edge_pattern(orientation: u64, line: u32, segment: u32) -> EdgePattern {
    let seed = orientation.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (line as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9)
        ^ (segment as u64).wrapping_mul(0x94D0_49BB_1331_11EB);
    EdgePattern {
        center: 0.43 + edge_noise(seed, 2) * 0.14,
        radius_along: 0.11 + edge_noise(seed, 3) * 0.035,
        depth_scale: 0.88 + edge_noise(seed, 4) * 0.24,
        skew: (edge_noise(seed, 5) - 0.5) * 0.05,
    }
}

fn edge_noise(seed: u64, lane: u64) -> f32 {
    let mut value = seed ^ lane.wrapping_mul(0xD6E8_FEB8_6659_FD93);
    value ^= value >> 30;
    value = value.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^= value >> 31;
    ((value >> 40) as u32) as f32 / 16_777_215.0
}

fn edge_sign(orientation: u64, segment: u32, line: u32, line_count: u32) -> f32 {
    if line == 0 || line == line_count {
        0.0
    } else {
        let seed = orientation.wrapping_mul(0xA24B_AED4_963E_E407)
            ^ (line as u64).wrapping_mul(0x9FB2_1C65_1E98_DF25)
            ^ (segment as u64).wrapping_mul(0xC13F_A9A9_02A6_328F);
        if edge_noise(seed, 7) < 0.5 { -1.0 } else { 1.0 }
    }
}

fn puzzle_edge_point(
    start: [f32; 2],
    end: [f32; 2],
    pattern: EdgePattern,
    sign: f32,
    t: f32,
    base_depth: f32,
) -> [f32; 2] {
    let delta = [end[0] - start[0], end[1] - start[1]];
    let length = delta[0].hypot(delta[1]).max(f32::EPSILON);
    let tangent = [delta[0] / length, delta[1] / length];
    let normal = [-tangent[1], tangent[0]];
    let [along, offset] = if sign == 0.0 {
        [t, 0.0]
    } else {
        jigsaw_edge(t, pattern)
    };
    let depth = base_depth * pattern.depth_scale;
    [
        start[0] + delta[0] * along + normal[0] * sign * depth * offset,
        start[1] + delta[1] * along + normal[1] * sign * depth * offset,
    ]
}

fn jigsaw_edge(t: f32, pattern: EdgePattern) -> [f32; 2] {
    let radius = pattern.radius_along;
    let neck = radius * 0.46;
    let shoulder_start = pattern.center - radius - 0.085;
    let shoulder_end = pattern.center + radius + 0.085;
    let neck_left = [pattern.center - neck, 0.18];
    let neck_right = [pattern.center + neck, 0.18];
    let head_left = [pattern.center - radius, 0.58];
    let head_right = [pattern.center + radius, 0.58];
    let quarter_circle = 0.552_284_8;
    let point = if t < 0.26 {
        [t / 0.26 * shoulder_start, 0.0]
    } else if t < 0.34 {
        cubic_bezier(
            [shoulder_start, 0.0],
            [shoulder_start + 0.045, -0.01],
            [neck_left[0] - 0.025, 0.04],
            neck_left,
            (t - 0.26) / 0.08,
        )
    } else if t < 0.42 {
        cubic_bezier(
            neck_left,
            [neck_left[0] + 0.012, 0.34],
            [head_left[0], 0.45],
            head_left,
            (t - 0.34) / 0.08,
        )
    } else if t < 0.5 {
        cubic_bezier(
            head_left,
            [
                head_left[0],
                head_left[1] + (1.0 - head_left[1]) * quarter_circle,
            ],
            [pattern.center - radius * quarter_circle, 1.0],
            [pattern.center, 1.0],
            (t - 0.42) / 0.08,
        )
    } else if t < 0.58 {
        cubic_bezier(
            [pattern.center, 1.0],
            [pattern.center + radius * quarter_circle, 1.0],
            [
                head_right[0],
                head_right[1] + (1.0 - head_right[1]) * quarter_circle,
            ],
            head_right,
            (t - 0.5) / 0.08,
        )
    } else if t < 0.66 {
        cubic_bezier(
            head_right,
            [head_right[0], 0.45],
            [neck_right[0] - 0.012, 0.34],
            neck_right,
            (t - 0.58) / 0.08,
        )
    } else if t < 0.74 {
        cubic_bezier(
            neck_right,
            [neck_right[0] + 0.025, 0.04],
            [shoulder_end - 0.045, -0.01],
            [shoulder_end, 0.0],
            (t - 0.66) / 0.08,
        )
    } else {
        [shoulder_end + (t - 0.74) / 0.26 * (1.0 - shoulder_end), 0.0]
    };
    [point[0] + pattern.skew * point[1], point[1]]
}

fn inset_outline(outline: &[[f32; 2]], distance: f32) -> Result<Vec<[f32; 2]>> {
    let mut coordinates = outline
        .iter()
        .map(|point| Coord {
            x: point[0] as f64,
            y: point[1] as f64,
        })
        .collect::<Vec<_>>();
    coordinates.push(coordinates[0]);

    let inset = Polygon::new(LineString::new(coordinates), vec![]).buffer(-(distance as f64));
    let polygon = inset
        .0
        .into_iter()
        .max_by(|first, second| first.unsigned_area().total_cmp(&second.unsigned_area()))
        .context("clearance removed the puzzle-piece outline")?;
    if !polygon.interiors().is_empty() {
        bail!("clearance produced holes in the puzzle-piece outline");
    }

    let mut result = Vec::<[f32; 2]>::new();
    for point in &polygon.exterior().0 {
        let candidate = [point.x as f32, point.y as f32];
        let is_duplicate = result.last().is_some_and(|previous| {
            (previous[0] - candidate[0]).hypot(previous[1] - candidate[1]) < 0.000_01
        });
        if !is_duplicate {
            result.push(candidate);
        }
    }
    if result.len() > 1
        && (result[0][0] - result[result.len() - 1][0])
            .hypot(result[0][1] - result[result.len() - 1][1])
            < 0.000_01
    {
        result.pop();
    }
    Ok(result)
}

fn cubic_bezier(
    start: [f32; 2],
    control_a: [f32; 2],
    control_b: [f32; 2],
    end: [f32; 2],
    t: f32,
) -> [f32; 2] {
    let inverse = 1.0 - t;
    let weights = [
        inverse.powi(3),
        3.0 * inverse.powi(2) * t,
        3.0 * inverse * t.powi(2),
        t.powi(3),
    ];
    [
        start[0] * weights[0]
            + control_a[0] * weights[1]
            + control_b[0] * weights[2]
            + end[0] * weights[3],
        start[1] * weights[0]
            + control_a[1] * weights[1]
            + control_b[1] * weights[2]
            + end[1] * weights[3],
    ]
}

fn point_in_polygon(point: [f32; 2], polygon: &[[f32; 2]]) -> bool {
    let mut inside = false;
    let mut previous = polygon.len() - 1;
    for current in 0..polygon.len() {
        let a = polygon[current];
        let b = polygon[previous];
        let crosses = (a[1] > point[1]) != (b[1] > point[1])
            && point[0] < (b[0] - a[0]) * (point[1] - a[1]) / (b[1] - a[1]) + a[0];
        if crosses {
            inside = !inside;
        }
        previous = current;
    }
    inside
}

fn terrain_height(u: f32, v: f32, lat: f64, lon: f64) -> f32 {
    let u = u.clamp(0.0, 1.0);
    let v = v.clamp(0.0, 1.0);
    let seed_a = (lat as f32).to_radians().sin() * 1.7;
    let seed_b = (lon as f32).to_radians().cos() * 1.3;
    let ridge = ((u * 9.2 + seed_a) * 1.2).sin() * 0.19 + ((v * 7.1 - seed_b) * 1.4).cos() * 0.14;
    let folds = ((u * 3.8 + v * 5.6 + seed_b) * std::f32::consts::PI)
        .sin()
        .abs()
        * 0.17;
    let dx = u - (0.54 + seed_b * 0.05);
    let dy = v - (0.48 + seed_a * 0.05);
    let peak = (-((dx * dx * 5.5) + (dy * dy * 7.0))).exp() * 0.63;
    (0.12 + ridge + folds + peak).clamp(0.03, 1.0)
}

fn normalized_height(
    height_field: Option<&HeightField>,
    range: Option<(f32, f32)>,
    u: f32,
    v: f32,
    lat: f64,
    lon: f64,
) -> f32 {
    match (height_field, range) {
        (Some(field), Some((minimum, span))) => field.normalized_at(u, v, minimum, span),
        _ => terrain_height(u, v, lat, lon),
    }
}

fn building_height_mm(
    spec: &GenerationSpec,
    surface_field: Option<&SurfaceField>,
    u: f32,
    v: f32,
) -> f32 {
    if !spec.buildings.enabled {
        return 0.0;
    }
    let height_m = surface_field
        .map(|field| field.building_height_at(u, v))
        .unwrap_or(0.0);
    height_m * spec.width_mm / (spec.ground_span_km as f32 * 1_000.0) * spec.buildings.z_scale
}

fn road_height_mm(
    spec: &GenerationSpec,
    surface_field: Option<&SurfaceField>,
    u: f32,
    v: f32,
) -> f32 {
    if !spec.color_output.enabled || !spec.color_output.roads_enabled {
        return 0.0;
    }
    surface_field
        .filter(|field| field.at(u, v) == SurfaceClass::Road)
        .map(|_| spec.color_output.road_height_mm)
        .unwrap_or(0.0)
}

fn build_preview(
    spec: &GenerationSpec,
    height_field: Option<&HeightField>,
    surface_field: Option<&SurfaceField>,
    size: usize,
) -> serde_json::Value {
    let mut heights = Vec::with_capacity(size * size);
    let mut surface_classes = surface_field.map(|_| Vec::with_capacity(size * size));
    let range = height_field.map(HeightField::range);
    for y in 0..size {
        for x in 0..size {
            let u = x as f32 / (size - 1) as f32;
            let v = y as f32 / (size - 1) as f32;
            let terrain =
                normalized_height(height_field, range, u, v, spec.center_lat, spec.center_lon);
            let building =
                building_height_mm(spec, surface_field, u, v) / spec.relief_mm.max(f32::EPSILON);
            let road = road_height_mm(spec, surface_field, u, v) / spec.relief_mm.max(f32::EPSILON);
            heights.push(terrain + building + road);
            if let (Some(field), Some(classes)) = (surface_field, surface_classes.as_mut()) {
                classes.push(field.at(u, v).material_index());
            }
        }
    }
    let mut preview = serde_json::json!({
        "width": size,
        "height": size,
        "values": heights,
        "rows": spec.rows,
        "columns": spec.columns,
        "solid_model": spec.solid_model,
    });
    if let (Some(field), Some(classes)) = (surface_field, surface_classes) {
        let coverage = field.coverage();
        preview["surface_classes"] = serde_json::json!(classes);
        preview["surface_palette"] = serde_json::json!({
            "rock": spec.color_output.rock_color,
            "forest": spec.color_output.forest_color,
            "snow": spec.color_output.snow_color,
            "water": spec.color_output.water_color,
            "road": spec.color_output.road_color,
        });
        preview["surface_coverage"] = serde_json::json!({
            "rock": coverage[0],
            "forest": coverage[1],
            "snow": coverage[2],
            "water": coverage[3],
            "road": coverage[4],
        });
        preview["surface_source"] = serde_json::json!(field.source);
    }
    preview
}

fn write_binary_stl(mesh: &Mesh, path: &Path) -> Result<()> {
    let mut writer = BufWriter::new(
        File::create(path).with_context(|| format!("create STL {}", path.display()))?,
    );
    let mut header = [0_u8; 80];
    let label = format!("Terrain Puzzle — {}", mesh.name);
    let bytes = label.as_bytes();
    header[..bytes.len().min(80)].copy_from_slice(&bytes[..bytes.len().min(80)]);
    writer.write_all(&header)?;
    writer.write_all(&(mesh.triangles.len() as u32).to_le_bytes())?;

    for triangle in &mesh.triangles {
        let a = mesh.vertices[triangle[0] as usize];
        let b = mesh.vertices[triangle[1] as usize];
        let c = mesh.vertices[triangle[2] as usize];
        let normal = face_normal(a, b, c);
        for value in normal.into_iter().chain(a).chain(b).chain(c) {
            writer.write_all(&value.to_le_bytes())?;
        }
        writer.write_all(&0_u16.to_le_bytes())?;
    }
    writer.flush()?;
    Ok(())
}

fn face_normal(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> [f32; 3] {
    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let cross = [
        ab[1] * ac[2] - ab[2] * ac[1],
        ab[2] * ac[0] - ab[0] * ac[2],
        ab[0] * ac[1] - ab[1] * ac[0],
    ];
    let length = (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2])
        .sqrt()
        .max(f32::EPSILON);
    [cross[0] / length, cross[1] / length, cross[2] / length]
}

struct ThreeMfWriter<'a> {
    zip: ZipWriter<File>,
    spec: &'a GenerationSpec,
    object_count: usize,
}

const COLOR_GROUP_ID: u32 = 1000;

impl<'a> ThreeMfWriter<'a> {
    fn new(spec: &'a GenerationSpec, path: &Path) -> Result<Self> {
        let file = File::create(path).with_context(|| format!("create 3MF {}", path.display()))?;
        let mut zip = ZipWriter::new(file);
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

        zip.start_file("[Content_Types].xml", options)?;
        zip.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>
</Types>"#,
        )?;

        zip.add_directory("_rels/", options)?;
        zip.start_file("_rels/.rels", options)?;
        zip.write_all(
        br#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Target="/3D/3dmodel.model" Id="rel-1" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/>
</Relationships>"#,
    )?;

        zip.add_directory("3D/", options)?;
        zip.start_file("3D/3dmodel.model", options)?;
        if spec.color_output.enabled {
            zip.write_all(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<model unit="millimeter" xml:lang="en-US" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02" xmlns:m="http://schemas.microsoft.com/3dmanufacturing/material/2015/02" requiredextensions="m">
  <metadata name="Title">Terrain Puzzle</metadata>
  <metadata name="Designer">Terrain Puzzle Generator</metadata>
  <resources>
"#
                .as_bytes(),
            )?;
        } else {
            zip.write_all(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<model unit="millimeter" xml:lang="en-US" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02">
  <metadata name="Title">Terrain Puzzle</metadata>
  <metadata name="Designer">Terrain Puzzle Generator</metadata>
  <resources>
"#
                .as_bytes(),
            )?;
        }
        if spec.color_output.enabled {
            writeln!(
                zip,
                "    <m:colorgroup id=\"{COLOR_GROUP_ID}\">\n      <m:color color=\"{}FF\"/>\n      <m:color color=\"{}FF\"/>\n      <m:color color=\"{}FF\"/>\n      <m:color color=\"{}FF\"/>\n      <m:color color=\"{}FF\"/>\n    </m:colorgroup>",
                spec.color_output.rock_color,
                spec.color_output.forest_color,
                spec.color_output.snow_color,
                spec.color_output.water_color,
                spec.color_output.road_color,
            )?;
        }
        Ok(Self {
            zip,
            spec,
            object_count: 0,
        })
    }

    fn write_mesh(&mut self, mesh: &Mesh) -> Result<()> {
        debug_assert_eq!(mesh.triangles.len(), mesh.materials.len());
        let object_id = self.object_count + 1;
        let mut output = BufWriter::with_capacity(64 * 1024, &mut self.zip);
        writeln!(
            output,
            "    <object id=\"{object_id}\" name=\"{}\" type=\"model\"><mesh><vertices>",
            mesh.name
        )?;
        for vertex in &mesh.vertices {
            writeln!(
                output,
                "      <vertex x=\"{:.5}\" y=\"{:.5}\" z=\"{:.5}\"/>",
                vertex[0], vertex[1], vertex[2]
            )?;
        }
        output.write_all(b"    </vertices><triangles>\n")?;
        for (triangle, material) in mesh.triangles.iter().zip(&mesh.materials) {
            if self.spec.color_output.enabled {
                let index = material.material_index();
                writeln!(
                    output,
                    "      <triangle v1=\"{}\" v2=\"{}\" v3=\"{}\" pid=\"{COLOR_GROUP_ID}\" p1=\"{index}\" p2=\"{index}\" p3=\"{index}\"/>",
                    triangle[0], triangle[1], triangle[2],
                )?;
            } else {
                writeln!(
                    output,
                    "      <triangle v1=\"{}\" v2=\"{}\" v3=\"{}\"/>",
                    triangle[0], triangle[1], triangle[2]
                )?;
            }
        }
        output.write_all(b"    </triangles></mesh></object>\n")?;
        output.flush()?;
        self.object_count += 1;
        Ok(())
    }

    fn finish(mut self) -> Result<()> {
        self.zip.write_all(b"  </resources>\n  <build>\n")?;
        let piece_width = self.spec.width_mm / self.spec.columns as f32;
        let piece_height = self.spec.height_mm() / self.spec.rows as f32;
        let spacing = piece_width.min(piece_height) * 0.3;
        for index in 0..self.object_count {
            let row = if self.spec.solid_model {
                0
            } else {
                index as u32 / self.spec.columns
            };
            let column = if self.spec.solid_model {
                0
            } else {
                index as u32 % self.spec.columns
            };
            let tx = column as f32 * (piece_width + spacing);
            let ty = row as f32 * (piece_height + spacing);
            writeln!(
                self.zip,
                "    <item objectid=\"{}\" transform=\"1 0 0 0 1 0 0 0 1 {:.5} {:.5} 0\"/>",
                index + 1,
                tx,
                ty
            )?;
        }
        self.zip.write_all(b"  </build>\n</model>")?;
        self.zip.finish()?;
        Ok(())
    }
}

fn file_artifact(path: &Path, media_type: &str) -> Result<Artifact> {
    Ok(Artifact {
        name: path
            .file_name()
            .and_then(|name| name.to_str())
            .context("artifact has no file name")?
            .to_owned(),
        media_type: media_type.to_owned(),
        bytes: fs::metadata(path)?.len(),
    })
}

pub fn artifact_path(output_dir: &Path, name: &str) -> Option<PathBuf> {
    let candidate = Path::new(name);
    if candidate.components().count() != 1 {
        return None;
    }
    let path = output_dir.join(candidate);
    path.is_file().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::HashMap, io::Read};

    #[test]
    fn shared_edges_are_identical_before_clearance() {
        let spec = GenerationSpec::default();
        let edge_samples = spec.samples_per_piece as usize;
        let left_piece = piece_outline(&spec, 1, 1, true).unwrap();
        let right_piece = piece_outline(&spec, 1, 2, true).unwrap();
        for point in &left_piece[edge_samples..edge_samples * 2] {
            let matching_distance = right_piece
                .iter()
                .map(|candidate| (candidate[0] - point[0]).hypot(candidate[1] - point[1]))
                .fold(f32::INFINITY, f32::min);
            assert!(matching_distance < 0.0001);
        }
    }

    #[test]
    fn shared_seam_keeps_the_requested_minimum_clearance() {
        let spec = GenerationSpec::default();
        let fitted_left = piece_outline(&spec, 1, 1, false).unwrap();
        let fitted_right = piece_outline(&spec, 1, 2, false).unwrap();

        let gap = fitted_left
            .iter()
            .map(|point| point_outline_distance(*point, &fitted_right))
            .chain(
                fitted_right
                    .iter()
                    .map(|point| point_outline_distance(*point, &fitted_left)),
            )
            .fold(f32::INFINITY, f32::min);
        assert!(
            (gap - spec.clearance_mm).abs() < 0.015,
            "minimum shared clearance was {gap} mm"
        );
    }

    #[test]
    fn generated_piece_is_watertight() {
        let mesh = build_piece(&GenerationSpec::default(), None, None, 0, 0).unwrap();
        assert_watertight(&mesh);
    }

    fn assert_watertight(mesh: &Mesh) {
        let mut edges: HashMap<(u32, u32), u32> = HashMap::new();
        for triangle in &mesh.triangles {
            for edge in [
                (triangle[0], triangle[1]),
                (triangle[1], triangle[2]),
                (triangle[2], triangle[0]),
            ] {
                let ordered = if edge.0 < edge.1 {
                    edge
                } else {
                    (edge.1, edge.0)
                };
                *edges.entry(ordered).or_default() += 1;
            }
        }
        let bad_edges = edges
            .iter()
            .filter(|(_, uses)| **uses != 2)
            .take(12)
            .map(|(edge, uses)| {
                (
                    mesh.vertices[edge.0 as usize],
                    mesh.vertices[edge.1 as usize],
                    *uses,
                )
            })
            .collect::<Vec<_>>();
        assert!(bad_edges.is_empty(), "non-manifold edges: {bad_edges:?}");
    }

    #[test]
    fn tray_is_watertight_and_keeps_contours_and_label_colors() {
        let spec = GenerationSpec {
            width_mm: 60.0,
            rows: 2,
            columns: 2,
            place_name: "Mount Rainier".into(),
            tray: TraySpec {
                enabled: true,
                ..TraySpec::default()
            },
            ..GenerationSpec::default()
        };
        let height = HeightField::new(
            3,
            3,
            vec![0.0, 1.0, 2.0, 1.0, 3.0, 5.0, 2.0, 5.0, 8.0],
            "test",
        )
        .unwrap();
        let mesh = build_tray(&spec, Some(&height)).unwrap();
        assert_watertight(&mesh);
        assert!(mesh.materials.contains(&SurfaceClass::Rock));
        assert!(mesh.materials.contains(&SurfaceClass::Forest));
        assert!(mesh.materials.contains(&SurfaceClass::Snow));
        let rim_z = spec.tray.floor_mm + spec.tray.rim_height_mm;
        let raised_label = mesh
            .triangles
            .iter()
            .zip(&mesh.materials)
            .filter(|(_, material)| **material == SurfaceClass::Snow)
            .flat_map(|(triangle, _)| triangle)
            .map(|index| mesh.vertices[*index as usize])
            .collect::<Vec<_>>();
        assert!(raised_label.iter().any(|vertex| vertex[2] > rim_z));
        assert!(
            raised_label
                .iter()
                .all(|vertex| vertex[1] < spec.tray.rim_width_mm)
        );
    }

    #[test]
    fn tray_label_uses_smooth_vector_curves() {
        let label = TrayLabel {
            text: "O".into(),
            origin_x: 1.0,
            baseline_y: 1.0,
            scale: 0.005,
        };
        let mut builder = MeshBuilder::default();
        label.add_embossed_shapes(&mut builder, 3.0).unwrap();
        let mesh = builder.finish("vector-label");
        assert_watertight(&mesh);

        let slanted_side_edges = mesh
            .triangles
            .iter()
            .filter(|triangle| {
                let vertices = triangle.map(|index| mesh.vertices[index as usize]);
                let spans_height = vertices
                    .iter()
                    .map(|vertex| vertex[2])
                    .fold(f32::INFINITY, f32::min)
                    < vertices
                        .iter()
                        .map(|vertex| vertex[2])
                        .fold(f32::NEG_INFINITY, f32::max);
                spans_height
                    && (0..3).any(|index| {
                        let a = vertices[index];
                        let b = vertices[(index + 1) % 3];
                        (a[2] - b[2]).abs() < 0.000_01
                            && (a[0] - b[0]).abs() > 0.000_01
                            && (a[1] - b[1]).abs() > 0.000_01
                    })
            })
            .count();
        assert!(
            slanted_side_edges > 24,
            "expected a smooth O outline, found {slanted_side_edges} curved segments"
        );
    }

    #[test]
    fn tray_exports_separate_stl_and_color_3mf() {
        let output_dir =
            std::env::temp_dir().join(format!("terrain-tray-core-test-{}", std::process::id()));
        if output_dir.exists() {
            std::fs::remove_dir_all(&output_dir).unwrap();
        }
        let spec = GenerationSpec {
            width_mm: 60.0,
            rows: 2,
            columns: 2,
            samples_per_piece: 16,
            tray: TraySpec {
                enabled: true,
                ..TraySpec::default()
            },
            ..GenerationSpec::default()
        };
        let manifest = generate_project(&spec, &output_dir).unwrap();
        assert!(output_dir.join("terrain-tray.stl").is_file());
        assert!(output_dir.join("terrain-tray.3mf").is_file());
        assert!(
            manifest
                .artifacts
                .iter()
                .any(|artifact| artifact.name == "terrain-tray.3mf")
        );

        let file = File::open(output_dir.join("terrain-tray.3mf")).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let mut model = String::new();
        archive
            .by_name("3D/3dmodel.model")
            .unwrap()
            .read_to_string(&mut model)
            .unwrap();
        assert!(model.contains("color=\"#252822FF\""));
        assert!(model.contains("color=\"#E7E4D8FF\""));
        assert!(model.contains("color=\"#F4F3ECFF\""));
        assert!(model.contains("p1=\"1\""));
        assert!(model.contains("p1=\"2\""));

        std::fs::remove_dir_all(output_dir).unwrap();
    }

    #[test]
    fn project_writes_print_artifacts() {
        let output_dir =
            std::env::temp_dir().join(format!("terrain-puzzle-core-test-{}", std::process::id()));
        if output_dir.exists() {
            std::fs::remove_dir_all(&output_dir).unwrap();
        }

        let spec = GenerationSpec {
            rows: 2,
            columns: 2,
            samples_per_piece: 16,
            ..GenerationSpec::default()
        };
        let manifest = generate_project(&spec, &output_dir).unwrap();

        assert!(output_dir.join("terrain-puzzle.3mf").is_file());
        assert!(output_dir.join("piece-1-1.stl").is_file());
        assert!(output_dir.join("preview.json").is_file());
        assert_eq!(
            manifest
                .artifacts
                .iter()
                .filter(|artifact| artifact.name.ends_with(".stl"))
                .count(),
            4
        );

        std::fs::remove_dir_all(output_dir).unwrap();
    }

    #[test]
    fn solid_mode_writes_one_plain_watertight_model() {
        let output_dir =
            std::env::temp_dir().join(format!("terrain-solid-core-test-{}", std::process::id()));
        if output_dir.exists() {
            std::fs::remove_dir_all(&output_dir).unwrap();
        }
        let spec = GenerationSpec {
            rows: 2,
            columns: 2,
            samples_per_piece: 16,
            solid_model: true,
            ..GenerationSpec::default()
        };
        let outline = solid_outline(&spec, 32);
        assert!(outline.iter().all(|point| {
            point[0] == 0.0
                || point[0] == spec.width_mm
                || point[1] == 0.0
                || point[1] == spec.height_mm()
        }));

        let manifest = generate_project(&spec, &output_dir).unwrap();
        assert!(output_dir.join("terrain-solid.stl").is_file());
        assert!(output_dir.join("terrain-solid.3mf").is_file());
        assert!(!output_dir.join("terrain-puzzle.3mf").exists());
        assert!(!output_dir.join("piece-1-1.stl").exists());
        assert_eq!(
            manifest
                .artifacts
                .iter()
                .filter(|artifact| artifact.name.ends_with(".stl"))
                .count(),
            1
        );

        let mesh = build_piece(&spec, None, None, 0, 0).unwrap();
        let mut edges: HashMap<(u32, u32), u32> = HashMap::new();
        for triangle in &mesh.triangles {
            for edge in [
                (triangle[0], triangle[1]),
                (triangle[1], triangle[2]),
                (triangle[2], triangle[0]),
            ] {
                let ordered = if edge.0 < edge.1 {
                    edge
                } else {
                    (edge.1, edge.0)
                };
                *edges.entry(ordered).or_default() += 1;
            }
        }
        assert!(edges.values().all(|uses| *uses == 2));

        std::fs::remove_dir_all(output_dir).unwrap();
    }

    #[test]
    fn solid_mode_supports_maximum_detail() {
        let spec = GenerationSpec {
            samples_per_piece: 128,
            solid_model: true,
            ..GenerationSpec::default()
        };
        let mesh = build_piece(&spec, None, None, 0, 0).unwrap();
        assert!(mesh.vertices.len() > 100_000);
    }

    #[test]
    fn color_project_writes_standard_3mf_properties_and_preview() {
        let output_dir =
            std::env::temp_dir().join(format!("terrain-puzzle-color-test-{}", std::process::id()));
        if output_dir.exists() {
            std::fs::remove_dir_all(&output_dir).unwrap();
        }
        let spec = GenerationSpec {
            rows: 2,
            columns: 2,
            samples_per_piece: 16,
            color_output: ColorOutputSpec {
                enabled: true,
                ..ColorOutputSpec::default()
            },
            ..GenerationSpec::default()
        };
        let height =
            HeightField::new(5, 5, (0..25).map(|value| value as f32).collect(), "test").unwrap();
        let surface = SurfaceField::new(
            5,
            5,
            (0..25)
                .map(|index| match index % 5 {
                    1 => SurfaceClass::Forest,
                    2 => SurfaceClass::Snow,
                    3 => SurfaceClass::Water,
                    4 => SurfaceClass::Road,
                    _ => SurfaceClass::Rock,
                })
                .collect(),
            "test surface",
        )
        .unwrap();

        generate_project_with_fields(&spec, &height, Some(&surface), &output_dir).unwrap();

        let file = File::open(output_dir.join("terrain-puzzle.3mf")).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let mut model = String::new();
        archive
            .by_name("3D/3dmodel.model")
            .unwrap()
            .read_to_string(&mut model)
            .unwrap();
        assert!(
            model.contains(
                "xmlns:m=\"http://schemas.microsoft.com/3dmanufacturing/material/2015/02\""
            )
        );
        assert!(model.contains("<m:colorgroup id=\"1000\">"));
        assert!(model.contains("color=\"#28543AFF\""));
        assert!(model.contains("color=\"#2F76B5FF\""));
        assert!(model.contains("color=\"#D8A33CFF\""));
        assert!(model.contains("pid=\"1000\""));
        assert!(model.contains("p1=\"1\""));
        assert!(model.contains("p1=\"2\""));
        assert!(model.contains("p1=\"3\""));
        assert!(model.contains("p1=\"4\""));
        assert_eq!(model.matches("<object id=").count(), 4);
        assert_eq!(model.matches("<item objectid=").count(), 4);

        let preview: serde_json::Value =
            serde_json::from_slice(&std::fs::read(output_dir.join("preview.json")).unwrap())
                .unwrap();
        assert!(preview["surface_classes"].is_array());
        assert_eq!(preview["surface_palette"]["rock"], "#7C7468");
        assert_eq!(preview["surface_palette"]["water"], "#2F76B5");
        assert_eq!(preview["surface_palette"]["road"], "#D8A33C");
        assert_eq!(preview["surface_source"], "test surface");

        std::fs::remove_dir_all(output_dir).unwrap();
    }

    #[test]
    fn surface_filter_removes_tiny_color_islands() {
        let mut classes = vec![SurfaceClass::Forest; 25];
        classes[12] = SurfaceClass::Snow;
        let mut field = SurfaceField::new(5, 5, classes, "test").unwrap();
        field.filter_small_patches(10.0, 4.0);
        assert_eq!(field.classes[12], SurfaceClass::Forest);
    }

    #[test]
    fn old_color_specs_gain_the_default_water_color() {
        let spec: GenerationSpec = serde_json::from_value(serde_json::json!({
            "color_output": {
                "enabled": true,
                "forest_color": "#28543A",
                "rock_color": "#7C7468",
                "snow_color": "#F4F3EC",
                "minimum_patch_mm": 1.2
            }
        }))
        .unwrap();
        assert!(!spec.solid_model);
        assert_eq!(spec.overlay_samples_per_piece, 112);
        assert_eq!(spec.place_name, "Mount Rainier");
        assert!(!spec.tray.enabled);
        assert!(!spec.buildings.enabled);
        assert_eq!(spec.buildings.z_scale, 5.0);
        assert_eq!(spec.color_output.water_color, "#2F76B5");
        assert_eq!(spec.color_output.road_color, "#D8A33C");
        assert!(spec.color_output.roads_enabled);
        assert_eq!(spec.color_output.road_width_mm, 1.0);
        assert_eq!(spec.color_output.road_height_mm, 0.2);
    }

    #[test]
    fn surface_field_paints_print_width_aware_road_lines() {
        let mut field =
            SurfaceField::new(21, 21, vec![SurfaceClass::Forest; 21 * 21], "test").unwrap();
        field.paint_polyline(&[[0.0, 0.5], [1.0, 0.5]], 20.0, 2.0, SurfaceClass::Road);

        assert_eq!(field.classes[10 * 21 + 10], SurfaceClass::Road);
        assert_eq!(field.classes[9 * 21 + 10], SurfaceClass::Road);
        assert_eq!(field.classes[7 * 21 + 10], SurfaceClass::Forest);
    }

    #[test]
    fn surface_field_paints_scaled_building_heights() {
        let mut field =
            SurfaceField::new(21, 21, vec![SurfaceClass::Rock; 21 * 21], "test").unwrap();
        field.paint_building(
            &[[0.25, 0.25], [0.75, 0.25], [0.75, 0.75], [0.25, 0.75]],
            12.0,
        );
        assert_eq!(field.building_height_at(0.5, 0.5), 12.0);
        assert_eq!(field.building_height_at(0.1, 0.1), 0.0);

        let spec = GenerationSpec {
            width_mm: 100.0,
            ground_span_km: 1.0,
            buildings: BuildingSpec {
                enabled: true,
                z_scale: 2.0,
            },
            ..GenerationSpec::default()
        };
        assert!((building_height_mm(&spec, Some(&field), 0.5, 0.5) - 2.4).abs() < 0.001);
    }

    #[test]
    fn overlays_use_their_independent_detail_level() {
        let mut spec = GenerationSpec::default();
        assert_eq!(spec.effective_samples_per_piece(), 64);
        spec.color_output.enabled = true;
        assert_eq!(spec.effective_samples_per_piece(), 112);
        spec.overlay_samples_per_piece = 48;
        assert_eq!(spec.effective_samples_per_piece(), 64);
    }

    #[test]
    fn roads_raise_the_mesh_by_one_layer() {
        let road_field = SurfaceField::new(3, 3, vec![SurfaceClass::Road; 9], "roads").unwrap();
        let spec = GenerationSpec {
            width_mm: 60.0,
            rows: 2,
            columns: 2,
            samples_per_piece: 16,
            overlay_samples_per_piece: 32,
            color_output: ColorOutputSpec {
                enabled: true,
                roads_enabled: true,
                road_height_mm: 0.2,
                ..ColorOutputSpec::default()
            },
            ..GenerationSpec::default()
        };
        let raised = build_piece(&spec, None, Some(&road_field), 0, 0).unwrap();
        let flat = build_piece(
            &GenerationSpec {
                color_output: ColorOutputSpec {
                    roads_enabled: false,
                    ..spec.color_output.clone()
                },
                ..spec.clone()
            },
            None,
            Some(&road_field),
            0,
            0,
        )
        .unwrap();
        let raised_top = raised
            .vertices
            .iter()
            .map(|vertex| vertex[2])
            .fold(f32::NEG_INFINITY, f32::max);
        let flat_top = flat
            .vertices
            .iter()
            .map(|vertex| vertex[2])
            .fold(f32::NEG_INFINITY, f32::max);
        assert!((raised_top - flat_top - 0.2).abs() < 0.001);
    }

    #[test]
    fn buildings_raise_the_printed_mesh() {
        let mut field = SurfaceField::new(3, 3, vec![SurfaceClass::Rock; 9], "buildings").unwrap();
        field.building_heights_m.fill(12.0);
        let spec = GenerationSpec {
            width_mm: 60.0,
            rows: 2,
            columns: 2,
            samples_per_piece: 16,
            ground_span_km: 1.0,
            buildings: BuildingSpec {
                enabled: true,
                z_scale: 2.0,
            },
            ..GenerationSpec::default()
        };
        let raised = build_piece(&spec, None, Some(&field), 0, 0).unwrap();
        let flat = build_piece(
            &GenerationSpec {
                buildings: BuildingSpec {
                    enabled: false,
                    ..spec.buildings.clone()
                },
                ..spec.clone()
            },
            None,
            Some(&field),
            0,
            0,
        )
        .unwrap();
        let raised_top = raised
            .vertices
            .iter()
            .map(|vertex| vertex[2])
            .fold(f32::NEG_INFINITY, f32::max);
        let flat_top = flat
            .vertices
            .iter()
            .map(|vertex| vertex[2])
            .fold(f32::NEG_INFINITY, f32::max);
        assert!((raised_top - flat_top - 1.44).abs() < 0.001);
    }

    #[test]
    fn tray_contour_grid_uses_submillimetre_cells() {
        let coordinates = regular_coordinates(0.0, 180.0, 0.35);
        let largest = coordinates
            .windows(2)
            .map(|pair| pair[1] - pair[0])
            .fold(0.0, f32::max);
        assert!(largest <= 0.351);
    }

    #[test]
    fn jigsaw_edge_has_overhanging_round_head() {
        let pattern = shared_edge_pattern(0, 1, 0);
        assert_eq!(jigsaw_edge(0.1, pattern)[1], 0.0);
        assert!(jigsaw_edge(0.5, pattern)[1] > 0.99);
        assert!(jigsaw_edge(0.42, pattern)[0] < jigsaw_edge(0.34, pattern)[0] - 0.03);
        assert!(jigsaw_edge(0.58, pattern)[0] > jigsaw_edge(0.66, pattern)[0] + 0.03);
        assert_eq!(jigsaw_edge(0.0, pattern)[1], 0.0);
        assert_eq!(jigsaw_edge(1.0, pattern)[1], 0.0);
    }

    #[test]
    fn puzzle_grid_and_edge_patterns_vary() {
        let spec = GenerationSpec::default();
        let nominal = spec.width_mm / spec.columns as f32;
        let interior = puzzle_grid_point(&spec, 1, 1);
        assert!((interior[0] - nominal).abs() > 0.01);
        assert!((interior[1] - nominal).abs() > 0.01);

        let first = shared_edge_pattern(0, 1, 0);
        let second = shared_edge_pattern(0, 1, 1);
        assert!((first.center - second.center).abs() > 0.001);
        assert!((first.depth_scale - second.depth_scale).abs() > 0.001);
        assert!((first.skew - second.skew).abs() > 0.001);
    }

    #[test]
    fn all_supported_detail_levels_triangulate() {
        for samples_per_piece in [64, 88, 104, 112, 128, 160] {
            let spec = GenerationSpec {
                samples_per_piece,
                ..GenerationSpec::default()
            };
            for row in 0..spec.rows {
                for column in 0..spec.columns {
                    build_piece(&spec, None, None, row, column).unwrap_or_else(|error| {
                        panic!("detail {samples_per_piece}, piece {row}-{column} failed: {error}")
                    });
                }
            }
        }
    }

    #[test]
    fn high_detail_outlines_work_for_every_grid_size() {
        for grid_size in [2, 4, 8, 12, 16] {
            let spec = GenerationSpec {
                rows: grid_size,
                columns: grid_size,
                samples_per_piece: 160,
                ..GenerationSpec::default()
            };
            for row in 0..spec.rows {
                for column in 0..spec.columns {
                    let outline = piece_outline(&spec, row, column, false).unwrap();
                    let points = outline
                        .iter()
                        .map(|point| Point2::new(point[0] as f64, point[1] as f64))
                        .collect::<Vec<_>>();
                    let constraints = (0..outline.len())
                        .map(|index| [index, (index + 1) % outline.len()])
                        .collect::<Vec<_>>();
                    ConstrainedDelaunayTriangulation::<Point2<f64>>::bulk_load_cdt(
                        points,
                        constraints,
                    )
                    .unwrap_or_else(|error| {
                        panic!("grid {grid_size}, piece {row}-{column} failed: {error}")
                    });
                }
            }
        }
    }

    fn point_segment_distance(point: [f32; 2], start: [f32; 2], end: [f32; 2]) -> f32 {
        let segment = [end[0] - start[0], end[1] - start[1]];
        let length_squared = segment[0] * segment[0] + segment[1] * segment[1];
        let t = (((point[0] - start[0]) * segment[0] + (point[1] - start[1]) * segment[1])
            / length_squared.max(f32::EPSILON))
        .clamp(0.0, 1.0);
        (point[0] - start[0] - t * segment[0]).hypot(point[1] - start[1] - t * segment[1])
    }

    fn point_outline_distance(point: [f32; 2], outline: &[[f32; 2]]) -> f32 {
        (0..outline.len())
            .map(|index| {
                point_segment_distance(point, outline[index], outline[(index + 1) % outline.len()])
            })
            .fold(f32::INFINITY, f32::min)
    }
}
