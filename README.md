# Terrain Puzzle

Terrain Puzzle is a local-first topographic puzzle generator. A Rust service
samples worldwide elevation data, builds watertight pieces with round jigsaw
tabs and sockets, and stores job state in SQLite. The web app lets you choose a
place and tune the printable model, including the mesh detail and surface
colors.

An optional shallow tray exports as its own watertight STL and color 3MF. Its
flat well shows smooth, continuous equal-height contour lines as fine color
inlays. Raised text on the front top lip shows the chosen place name, latitude,
and longitude in smooth vector letterforms. Controls set the tray clearance,
rim, floor, line count, and three print colors. The bundled Atkinson
Hyperlegible font keeps the label shape the same on every OS and remains under
its included SIL Open Font License.

Solid terrain mode exports the same mapped relief as one watertight STL and 3MF
model with a straight outer edge and no puzzle seams. It keeps the full source
sampling grid while limiting the single mesh to a safe detail level.

Piece layouts range from 2×2 to 16×16. The default 10×10 layout makes 100
pieces with narrow-necked, round puzzle knobs like a standard jigsaw.

The elevation provider reads Mapzen Terrarium tiles from the AWS Open Data
registry. The service caches elevation, ESA WorldCover, and OpenStreetMap input
under the operating system's user cache directory. OpenStreetMap entries keep
the raw response, so width, density, color, and visibility changes reuse the
same download.

For uncached requests, the service tries a second public Overpass instance when
the first rejects or cannot serve the request. If both fail, generation
continues without that OSM layer. WorldCover water and terrain output remain
available. Concurrent jobs share each cache fill, and the service tries the last
working public instance first on its next request. It retries a failed fetch
once and rejects HTTP 200 responses that contain an Overpass timeout remark, so
it never caches a partial building set. Set `OVERPASS_BASE_URL` to use one
specific Overpass instance.

Color mode reads 10 m ESA WorldCover 2021 data through HTTP range requests. It
maps tree cover, bare ground, snow or ice, and permanent water to editable
forest, rock, snow, and water colors. It also reads prominent roads from
OpenStreetMap through Overpass, then draws motorway, trunk, primary, and
secondary roads as smooth, print-safe vector lines. If none cross the selected
area, it draws paths, footways, bridleways, tracks, and cycleways as a trail
fallback. Rivers, streams, canals, and mapped water areas use the same vector
path so they stay smooth and flush with the terrain. Building footprints keep
their straight mapped edges, with dense local mesh detail along each wall
instead of a blocky whole-map sampling edge. The 3MF stores standard triangle
color properties.
Roads also rise by one configurable print-layer height, which defaults to 0.2
mm. Road width starts at 0.7 mm and can thin automatically in dense road
networks. OpenStreetMap water can be disabled without hiding WorldCover water.
STL files stay single-color but retain the raised road geometry.

Overlay detail is separate from the base terrain setting. It defaults to 112
samples per piece and can rise to 192, giving roads, buildings, water, snow,
forest, and rock boundaries a finer mesh without forcing the same setting on
plain terrain jobs. Generated browser previews use up to 384 samples across the
assembled map.

Building mode reads OpenStreetMap footprints and raises them above the terrain.
It uses tagged height first, then floor count, then an 8 m default. Its own Z
scale controls vertical exaggeration against the map's plan scale. Buildings
can run with or without surface color output. In color output, roofs and walls
use their own editable building material instead of inheriting the land-cover
color beneath each footprint.

Place search uses explicit, user-submitted OpenStreetMap Nominatim queries
through the Rust service. Results are cached in SQLite and outbound requests
are limited to one per second. Set `NOMINATIM_BASE_URL` to use another
compatible service. Review the
[public service policy](https://operations.osmfoundation.org/policies/nominatim/)
before wider or commercial use.

## Requirements

- Rust 1.96 or newer
- Node.js 22.13 or newer

## Run

Start the Rust API:

```bash
cargo run -p terrain-api
```

In a second terminal, start the website:

```bash
npm install
npm run dev
```

Open `http://127.0.0.1:3100`. The Rust API listens on
`http://127.0.0.1:8787`.

## Storage

SQLite and generated jobs live under `data/`, which Git ignores. Set
`TERRAIN_DATA_DIR` to use another directory.

Downloaded map inputs use the standard per-user cache path:

- macOS: `~/Library/Caches/com.theatrus.terrain-puzzle`
- Linux: `$XDG_CACHE_HOME/terrain-puzzle` or `~/.cache/terrain-puzzle`
- Windows: `%LOCALAPPDATA%\theatrus\terrain-puzzle\cache`

Set `TERRAIN_CACHE_DIR` to override that path. The cache keeps elevation PNG
tiles, full ESA WorldCover GeoTIFF tiles, and OpenStreetMap route responses.
Writes use a temporary file and an atomic rename, so a stopped download does
not leave a valid-looking partial tile.

The browser uses `NEXT_PUBLIC_TERRAIN_API_URL` when set. See `.env.example`.

## Check

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
npm test
```

## Project shape

- `crates/terrain-core`: puzzle edges, terrain surface, watertight meshes,
  binary STL, and standards-based 3MF
- `apps/api`: global elevation provider, Axum API, SQLite jobs, background
  generation, ESA WorldCover sampling, and downloads
- `app`: WebGL-free map, color relief preview, print controls, and job downloads

See [the color output plan](docs/color-output-plan.md) for the design and print
checks behind the rock–forest–snow–water–road 3MF workflow.

## Terrain data

Mapzen Terrain Tiles combine several regional and global public elevation
sources. Generated manifests record the source and link to the required
attribution notices:

<https://github.com/tilezen/joerd/blob/master/docs/attribution.md>

Color manifests also record the ESA WorldCover tile and attribution:

<https://worldcover2021.esa.int/download>

When road output is on, manifests also record the OpenStreetMap source and
attribution. Overpass responses use the same OS cache:

<https://www.openstreetmap.org/copyright>
