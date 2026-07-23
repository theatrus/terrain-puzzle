# Third-party licenses and data

The Apache-2.0 license in `LICENSE` covers the TopoSaic source code and
documentation. It does not relicense third-party software, the bundled font, or
map data.

## Bundled font

`assets/fonts/AtkinsonHyperlegible-Regular.ttf` is Copyright 2020 Braille
Institute of America, Inc. It is distributed under the SIL Open Font License,
Version 1.1. The complete license is in `assets/fonts/OFL.txt`.

## Software dependencies

Rust and Node packages keep their own licenses. Their package metadata and
installed copies contain the applicable license texts. The dependency audit
found permissive licenses, Unicode data licenses, MPL-2.0 components, and
LGPL-3.0-or-later libvips binaries used by the web build toolchain. It found no
GPL-only or AGPL package.

Anyone who distributes a compiled application or a web build must include the
notices and license texts required by the exact dependency versions in that
distribution. `Cargo.lock` and `package-lock.json` record those versions.

## Map and terrain data

Generated projects may contain data from sources whose terms are separate from
Apache-2.0. Each generated `manifest.json` records the sources used for that
project.

- OpenStreetMap roads, trails, waterways, buildings, and search results use
  OpenStreetMap data under the Open Data Commons Open Database License (ODbL).
  Public use must credit OpenStreetMap and state that its data is available
  under the ODbL. See <https://www.openstreetmap.org/copyright>.
- ESA WorldCover 2021 v200 is available under CC BY 4.0. Published maps, models,
  and data products must include: “© ESA WorldCover project 2021 / Contains
  modified Copernicus Sentinel data (2021) processed by ESA WorldCover
  consortium.” See <https://esa-worldcover.org/en/data-access>.
- Mapzen Terrain Tiles combine regional and global elevation sources with
  source-specific credit requirements. Use the attribution recorded in the
  generated manifest and consult
  <https://github.com/tilezen/joerd/blob/master/docs/attribution.md>.

When a physical print or static image is shared publicly, place the required
credits near the work or in its packaging or accompanying documentation. Do
not rely on the software license as a substitute for data attribution.
