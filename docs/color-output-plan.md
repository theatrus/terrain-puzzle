# Color output plan

## Goal

Generate a five-color mountain puzzle whose printable surface marks:

- forest in dark green;
- exposed rock and other ground in warm gray;
- snow and ice in off-white.
- permanent water in blue.
- prominent roads in ochre.

The first reference case is the existing Mount Rainier example. The single-color
STL files must remain available. The 3MF file becomes the main color output.

## Product choices

### Use mapped land cover, not elevation alone

Elevation can suggest a snow line or tree line, but it cannot tell forest from
rock. The first version should use ESA WorldCover 2021 at 10 m resolution:

- class 10, tree cover: forest;
- class 60, bare or sparse vegetation: rock;
- class 70, snow and ice: snow;
- class 80, permanent water bodies: water;
- all other classes: rock in the color mode.

OpenStreetMap adds motorway, trunk, primary, and secondary road geometry after
the land-cover mask is clean. Higher road classes get wider print lines. The
generator skips tunnels and keeps bridges visible. The default primary-road
width is 1.0 mm; motorway and trunk lines are wider, while secondary roads and
links are narrower. If no visible prominent road crosses the model, the
generator falls back to paths, footways, bridleways, tracks, and cycleways.
Trails never appear on top of a road network.

The map is static, so snow means mapped snow or ice rather than current seasonal
snow. Water includes lakes, reservoirs, and rivers that are wide enough to
appear in the 10 m data. A later version can add dated Sentinel-2 imagery for
seasonal snow and vector data for narrower streams.

Sources:

- [ESA WorldCover class list](https://developers.google.com/earth-engine/datasets/catalog/ESA_WorldCover_v200)
- [ESA WorldCover project](https://esa-worldcover.org/)
- [OpenStreetMap highway key](https://wiki.openstreetmap.org/wiki/Key:highway)
- [Overpass QL](https://wiki.openstreetmap.org/wiki/Overpass_API/Overpass_QL)

### Paint only the visible terrain surface

The top surface gets forest, rock, snow, water, or roads. Side walls and the
underside use the rock filament. This keeps the pieces strong and cuts filament
changes compared with making each color a solid volume.

The default palette is:

| Surface | Preview color | Suggested filament |
| --- | --- | --- |
| Forest | `#28543A` | dark green matte PLA |
| Rock | `#7C7468` | stone or warm gray matte PLA |
| Snow | `#F4F3EC` | natural white matte PLA |
| Water | `#2F76B5` | medium blue matte PLA |
| Road | `#D8A33C` | ochre or amber matte PLA |

The colors are labels, not fixed filament brands. Bambu Studio should let the
user map each label to an AMS slot.

### Keep color regions printable

Land-cover rasters contain small, noisy patches that do not print well. Before
painting the mesh:

1. resample the land-cover grid in assembled puzzle coordinates;
2. remove isolated regions below a configurable printed size;
3. close one-cell holes;
4. assign each top triangle one flat color from its center point.

Start with a 1.2 mm minimum patch size for a 0.4 mm nozzle. The UI should explain
that smaller patches add filament changes and may vanish in the sliced output.

## Data and Rust model

Add these core types:

```text
SurfaceClass = Forest | Rock | Snow | Water | Road
SurfacePalette = colors and filament labels
SurfaceField = classified raster plus source details
ColorOutputSpec = enabled, palette, minimum patch size, side color
```

The API fetches land-cover tiles and caches Overpass road responses beside the
elevation cache. The job manifest records each data set, license, source URL,
and class mapping.

Mesh generation classifies top triangles in global assembled coordinates. This
keeps color boundaries continuous across neighboring pieces. Bottom and side
triangles always receive the chosen side color.

## 3MF output

Use the standard 3MF Materials and Properties Extension. Add one color group
with the five palette entries, then attach a flat property to each triangle
with `pid` and equal `p1`, `p2`, and `p3` values. Keep geometry and color data
separate in the Rust mesh model.

Do not write Bambu-only project data in the first pass. A standards-based 3MF
is easier to test and still leaves room for a later Bambu project export.

Reference:

- [3MF specifications](https://3mf.io/spec/)

## Website changes

Add a **Color terrain** section below the relief controls:

- Off / Rock–forest–snow–water–road mode;
- five editable color swatches;
- minimum color patch size;
- road output toggle and print width;
- side and underside color;
- a note that snow is not live and narrow streams may fall below 10 m.

The terrain preview should use the same class raster as the export. Add a small
legend and coverage figures, such as `Forest 51% · Rock 31% · Snow 18%`.
Generation should still work if land-cover data fails: show a clear warning and
offer a rock-only export rather than silently inventing classes.

## Delivery stages

### 1. Classification foundation

- Add the surface types and color settings.
- Build a small in-memory class raster.
- Test deterministic classification, filtering, and seam continuity.

### 2. Land-cover provider

- Fetch ESA WorldCover data and cache OpenStreetMap roads for the selected bounds.
- Reproject and sample it beside the elevation field.
- Record source and license details in the manifest.

### 3. Color 3MF

- Add standard 3MF color resources and triangle properties.
- Keep STL output unchanged.
- Add XML and archive tests for every material reference.

### 4. Preview and controls

- Add the color settings, legend, and coverage figures.
- Render the exact exported mask over the relief preview.
- Show missing-data and excessive-color-change warnings.

### 5. Print validation

- Open the Mount Rainier 3MF in the current Bambu Studio release.
- Confirm five named colors map to the intended filaments.
- Confirm all nine pieces stay manifold and retain their snug seams.
- Slice a representative center piece and inspect its top layers.
- Print a small test piece before treating the palette and 1.2 mm patch size as
  final.

## Acceptance checks

- Mount Rainier shows connected green forest low on the mountain, gray exposed
  ground above the tree line, white snow or ice near the summit and glaciers,
  and blue where permanent water is mapped.
- Color boundaries continue across puzzle seams.
- Road widths follow the selected print width and road class.
- The web preview and 3MF triangle classes match.
- The 3MF opens with five usable colors and no mesh repair.
- Each piece remains one closed solid.
- Single-color STL output remains unchanged.
- A missing land-cover tile never produces false color without a warning.
