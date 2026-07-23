# Terrain Puzzle

Terrain Puzzle is a local-first topographic puzzle generator. A Rust service
samples worldwide elevation data, builds watertight pieces with round jigsaw
tabs and sockets, and stores job state in SQLite. The web app lets you choose a
place and tune the printable model, including the mesh detail and surface
colors.

Solid terrain mode exports the same mapped relief as one watertight STL and 3MF
model with a straight outer edge and no puzzle seams. It keeps the full source
sampling grid while limiting the single mesh to a safe detail level.

Piece layouts range from 2×2 to 16×16. The default 10×10 layout makes 100
pieces with narrow-necked, round puzzle knobs like a standard jigsaw.

The elevation provider reads Mapzen Terrarium tiles from the AWS Open Data
registry and keeps a local tile cache under `data/dem-cache`.

Color mode reads 10 m ESA WorldCover 2021 data through HTTP range requests. It
maps tree cover, bare ground, snow or ice, and permanent water to editable
forest, rock, snow, and water colors. It also reads prominent roads from
OpenStreetMap through Overpass, then draws motorway, trunk, primary, and
secondary roads at print-safe widths. The 3MF stores standard triangle color
properties. STL files stay single-color.

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
attribution. Overpass responses are cached under `data/road-cache`:

<https://www.openstreetmap.org/copyright>
