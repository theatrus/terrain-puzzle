"use client";

import {
  type CSSProperties,
  type KeyboardEvent as ReactKeyboardEvent,
  type FormEvent,
  type PointerEvent as ReactPointerEvent,
  type WheelEvent as ReactWheelEvent,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";

type GenerationSpec = {
  center_lat: number;
  center_lon: number;
  ground_span_km: number;
  width_mm: number;
  rows: number;
  columns: number;
  base_mm: number;
  relief_mm: number;
  clearance_mm: number;
  samples_per_piece: number;
  overlay_samples_per_piece: number;
  solid_model: boolean;
  straight_piece_sides: boolean;
  puzzle_tabs: boolean;
  place_name: string;
  buildings: {
    enabled: boolean;
    z_scale: number;
  };
  tray: {
    enabled: boolean;
    tray_color: string;
    contour_color: string;
    label_color: string;
    clearance_mm: number;
    rim_width_mm: number;
    floor_mm: number;
    rim_height_mm: number;
    contour_count: number;
  };
  color_output: {
    enabled: boolean;
    forest_color: string;
    rock_color: string;
    snow_color: string;
    water_color: string;
    road_color: string;
    building_color: string;
    roads_enabled: boolean;
    adaptive_road_widths: boolean;
    osm_water_enabled: boolean;
    waterway_coverage_percent: number;
    road_width_mm: number;
    road_height_mm: number;
    bridge_structure: "floating" | "supported";
    bridge_thickness_mm: number;
    minimum_patch_mm: number;
  };
};

type Artifact = {
  name: string;
  media_type: string;
  bytes: number;
};

type Job = {
  id: string;
  status: "queued" | "running" | "complete" | "failed";
  progress: number;
  artifacts: Artifact[];
  error?: string | null;
  spec: GenerationSpec;
};

const DEFAULT_VISUAL_HEIGHT_PERCENT = 37;
const MIN_VISUAL_HEIGHT_PERCENT = 28;
const MAX_VISUAL_HEIGHT_PERCENT = 76;
const VISUAL_HEIGHT_KEYBOARD_STEP = 4;
const WORKSPACE_RESIZER_HEIGHT_PX = 14;

type PreviewData = {
  width: number;
  height: number;
  values: number[];
  rows: number;
  columns: number;
  solid_model?: boolean;
  surface_classes?: number[];
  surface_palette?: {
    rock: string;
    forest: string;
    snow: string;
    water: string;
    road: string;
    building: string;
  };
  surface_coverage?: {
    rock: number;
    forest: number;
    snow: number;
    water: number;
    road: number;
    building: number;
  };
  surface_source?: string;
};

type PlaceResult = {
  display_name: string;
  latitude: number;
  longitude: number;
  category: string;
  kind: string;
};

const IS_TAURI =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

const API_URL =
  IS_TAURI
    ? "http://127.0.0.1:38787"
    : ((typeof process !== "undefined"
        ? process.env.NEXT_PUBLIC_TERRAIN_API_URL
        : undefined) ?? "http://127.0.0.1:8787");

const initialSpec: GenerationSpec = {
  center_lat: 46.8523,
  center_lon: -121.7603,
  ground_span_km: 18,
  width_mm: 180,
  rows: 10,
  columns: 10,
  base_mm: 2.4,
  relief_mm: 14,
  clearance_mm: 0.14,
  samples_per_piece: 64,
  overlay_samples_per_piece: 112,
  solid_model: false,
  straight_piece_sides: false,
  puzzle_tabs: true,
  place_name: "Mount Rainier",
  buildings: {
    enabled: false,
    z_scale: 5,
  },
  tray: {
    enabled: true,
    tray_color: "#252822",
    contour_color: "#E7E4D8",
    label_color: "#F4F3EC",
    clearance_mm: 0.6,
    rim_width_mm: 8,
    floor_mm: 1.6,
    rim_height_mm: 3.2,
    contour_count: 18,
  },
  color_output: {
    enabled: true,
    forest_color: "#28543A",
    rock_color: "#7C7468",
    snow_color: "#F4F3EC",
    water_color: "#2F76B5",
    road_color: "#D8A33C",
    building_color: "#B8A890",
    roads_enabled: true,
    adaptive_road_widths: true,
    osm_water_enabled: true,
    waterway_coverage_percent: 12,
    road_width_mm: 0.7,
    road_height_mm: 0.2,
    bridge_structure: "floating",
    bridge_thickness_mm: 1.2,
    minimum_patch_mm: 1.2,
  },
};

const TILE_SIZE = 256;
const MAX_MERCATOR_LATITUDE = 85.05112878;
const MIN_MAP_ZOOM = 2;
const MAX_MAP_ZOOM = 15;
const MIN_GROUND_SPAN_KM = 1;
const MAX_GROUND_SPAN_KM = 80;

function projectToWorld(longitude: number, latitude: number, zoom: number) {
  const scale = TILE_SIZE * 2 ** zoom;
  const clampedLatitude = Math.max(
    -MAX_MERCATOR_LATITUDE,
    Math.min(MAX_MERCATOR_LATITUDE, latitude),
  );
  const sine = Math.sin((clampedLatitude * Math.PI) / 180);
  return {
    x: ((longitude + 180) / 360) * scale,
    y:
      (0.5 - Math.log((1 + sine) / (1 - sine)) / (4 * Math.PI)) *
      scale,
  };
}

function unprojectFromWorld(x: number, y: number, zoom: number) {
  const scale = TILE_SIZE * 2 ** zoom;
  const longitude = ((((x / scale) * 360) % 360) + 360) % 360 - 180;
  const mercatorY = Math.PI * (1 - (2 * y) / scale);
  const latitude = (Math.atan(Math.sinh(mercatorY)) * 180) / Math.PI;
  return {
    longitude,
    latitude: Math.max(
      -MAX_MERCATOR_LATITUDE,
      Math.min(MAX_MERCATOR_LATITUDE, latitude),
    ),
  };
}

function TerrainMap({
  spec,
  onCenterChange,
  onGroundSpanChange,
}: {
  spec: GenerationSpec;
  onCenterChange: (longitude: number, latitude: number) => void;
  onGroundSpanChange: (groundSpanKm: number) => void;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const dragRef = useRef<{
    pointerId: number;
    startX: number;
    startY: number;
    worldX: number;
    worldY: number;
  } | null>(null);
  const [zoom, setZoom] = useState(9);
  const [size, setSize] = useState({ width: 0, height: 0 });
  const [tilesLoaded, setTilesLoaded] = useState(false);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    const observer = new ResizeObserver(([entry]) => {
      setSize({
        width: Math.round(entry.contentRect.width),
        height: Math.round(entry.contentRect.height),
      });
    });
    observer.observe(container);
    return () => observer.disconnect();
  }, []);

  const worldCenter = useMemo(
    () => projectToWorld(spec.center_lon, spec.center_lat, zoom),
    [spec.center_lat, spec.center_lon, zoom],
  );
  const tiles = useMemo(() => {
    if (!size.width || !size.height) return [];
    const firstX =
      Math.floor((worldCenter.x - size.width / 2) / TILE_SIZE) - 1;
    const lastX =
      Math.floor((worldCenter.x + size.width / 2) / TILE_SIZE) + 1;
    const firstY =
      Math.floor((worldCenter.y - size.height / 2) / TILE_SIZE) - 1;
    const lastY =
      Math.floor((worldCenter.y + size.height / 2) / TILE_SIZE) + 1;
    const tileCount = 2 ** zoom;
    const visibleTiles = [];
    for (let tileY = firstY; tileY <= lastY; tileY += 1) {
      if (tileY < 0 || tileY >= tileCount) continue;
      for (let tileX = firstX; tileX <= lastX; tileX += 1) {
        const wrappedX = ((tileX % tileCount) + tileCount) % tileCount;
        visibleTiles.push({
          key: `${zoom}/${tileX}/${tileY}`,
          url: `https://tile.openstreetmap.org/${zoom}/${wrappedX}/${tileY}.png`,
          left: tileX * TILE_SIZE - worldCenter.x + size.width / 2,
          top: tileY * TILE_SIZE - worldCenter.y + size.height / 2,
        });
      }
    }
    return visibleTiles;
  }, [size, worldCenter, zoom]);

  const metresPerPixel =
    (156543.03392 *
      Math.max(0.1, Math.cos((spec.center_lat * Math.PI) / 180))) /
    2 ** zoom;
  const selectionSize = Math.max(
    8,
    Math.min(
      Math.min(size.width, size.height) * 0.94,
      (spec.ground_span_km * 1000) / metresPerPixel,
    ),
  );
  const groundSpanLabel = Number.isInteger(spec.ground_span_km)
    ? spec.ground_span_km.toFixed(0)
    : spec.ground_span_km.toFixed(1);

  const moveToWorld = useCallback(
    (worldX: number, worldY: number) =>
      unprojectFromWorld(worldX, worldY, zoom),
    [zoom],
  );

  const pointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    event.currentTarget.setPointerCapture(event.pointerId);
    dragRef.current = {
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      worldX: worldCenter.x,
      worldY: worldCenter.y,
    };
  };

  const pointerMove = (event: ReactPointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    const next = moveToWorld(
      drag.worldX - (event.clientX - drag.startX),
      drag.worldY - (event.clientY - drag.startY),
    );
    onCenterChange(next.longitude, next.latitude);
  };

  const pointerUp = (event: ReactPointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    dragRef.current = null;
    const next = moveToWorld(
      drag.worldX - (event.clientX - drag.startX),
      drag.worldY - (event.clientY - drag.startY),
    );
    onCenterChange(next.longitude, next.latitude);
  };

  const changeZoom = useCallback(
    (delta: number) => {
      const nextZoom = Math.max(
        MIN_MAP_ZOOM,
        Math.min(MAX_MAP_ZOOM, zoom + delta),
      );
      if (nextZoom === zoom) return;
      const nextGroundSpan =
        spec.ground_span_km * 2 ** (zoom - nextZoom);
      if (
        nextGroundSpan < MIN_GROUND_SPAN_KM ||
        nextGroundSpan > MAX_GROUND_SPAN_KM
      ) {
        return;
      }
      setZoom(nextZoom);
      onGroundSpanChange(Math.round(nextGroundSpan));
    },
    [onGroundSpanChange, spec.ground_span_km, zoom],
  );

  const wheel = (event: ReactWheelEvent<HTMLDivElement>) => {
    event.preventDefault();
    changeZoom(event.deltaY < 0 ? 1 : -1);
  };

  const canZoomIn =
    zoom < MAX_MAP_ZOOM &&
    spec.ground_span_km / 2 >= MIN_GROUND_SPAN_KM;
  const canZoomOut =
    zoom > MIN_MAP_ZOOM &&
    spec.ground_span_km * 2 <= MAX_GROUND_SPAN_KM;

  return (
    <div className="map-shell">
      <div
        ref={containerRef}
        className="map-canvas"
        aria-label="Terrain map. Drag to choose a place."
        onPointerDown={pointerDown}
        onPointerMove={pointerMove}
        onPointerUp={pointerUp}
        onPointerCancel={() => {
          dragRef.current = null;
        }}
        onWheel={wheel}
        role="application"
      >
        <div className="map-tiles" aria-hidden="true">
          {tiles.map((tile) => (
            // Map tiles must load from their source without image optimization.
            // eslint-disable-next-line @next/next/no-img-element
            <img
              alt=""
              draggable={false}
              key={tile.key}
              onLoad={() => setTilesLoaded(true)}
              src={tile.url}
              style={{ left: tile.left, top: tile.top }}
            />
          ))}
        </div>
        <div
          className="map-selection"
          aria-label={`Selected terrain area: ${groundSpanLabel} km square`}
          data-ground-span-km={spec.ground_span_km}
          data-map-zoom={zoom}
          role="img"
          style={{ height: selectionSize, width: selectionSize }}
        >
          <span>{groundSpanLabel} km</span>
        </div>
      </div>
      <div className="map-zoom" aria-label="Map zoom">
        <button
          type="button"
          aria-label="Zoom in"
          disabled={!canZoomIn}
          onClick={() => changeZoom(1)}
        >
          +
        </button>
        <button
          type="button"
          aria-label="Zoom out"
          disabled={!canZoomOut}
          onClick={() => changeZoom(-1)}
        >
          −
        </button>
      </div>
      <div className="map-crosshair" aria-hidden="true">
        <span />
        <span />
      </div>
      <div className="map-instruction">
        {tilesLoaded ? "Drag the map to choose a place" : "Loading map tiles…"}
      </div>
      <a
        className="map-attribution"
        href="https://www.openstreetmap.org/copyright"
        target="_blank"
        rel="noreferrer"
      >
        © OpenStreetMap
      </a>
    </div>
  );
}

function cubicBezier(
  start: [number, number],
  controlA: [number, number],
  controlB: [number, number],
  end: [number, number],
  t: number,
) {
  const inverse = 1 - t;
  const weights = [
    inverse ** 3,
    3 * inverse ** 2 * t,
    3 * inverse * t ** 2,
    t ** 3,
  ];
  return {
    along:
      start[0] * weights[0] +
      controlA[0] * weights[1] +
      controlB[0] * weights[2] +
      end[0] * weights[3],
    offset:
      start[1] * weights[0] +
      controlA[1] * weights[1] +
      controlB[1] * weights[2] +
      end[1] * weights[3],
  };
}

type EdgePattern = {
  center: number;
  radiusAlong: number;
  depthScale: number;
  skew: number;
};

function edgeNoise(seed: bigint, lane: bigint) {
  let value = BigInt.asUintN(
    64,
    seed ^ BigInt.asUintN(64, lane * 0xd6e8feb86659fd93n),
  );
  value ^= value >> 30n;
  value = BigInt.asUintN(64, value * 0xbf58476d1ce4e5b9n);
  value ^= value >> 27n;
  value = BigInt.asUintN(64, value * 0x94d049bb133111ebn);
  value ^= value >> 31n;
  return Number(value >> 40n) / 16777215;
}

function sharedEdgePattern(
  orientation: number,
  line: number,
  segment: number,
): EdgePattern {
  const seed =
    BigInt.asUintN(64, BigInt(orientation) * 0x9e3779b97f4a7c15n) ^
    BigInt.asUintN(64, BigInt(line) * 0xbf58476d1ce4e5b9n) ^
    BigInt.asUintN(64, BigInt(segment) * 0x94d049bb133111ebn);
  return {
    center: 0.43 + edgeNoise(seed, 2n) * 0.14,
    radiusAlong: 0.11 + edgeNoise(seed, 3n) * 0.035,
    depthScale: 0.88 + edgeNoise(seed, 4n) * 0.24,
    skew: (edgeNoise(seed, 5n) - 0.5) * 0.05,
  };
}

function puzzleGridPoint(spec: GenerationSpec, row: number, column: number) {
  const pieceWidth = spec.width_mm / spec.columns;
  const pieceHeight = (spec.width_mm * spec.rows) / spec.columns / spec.rows;
  if (spec.straight_piece_sides) {
    return {
      x: column === spec.columns ? spec.width_mm : column * pieceWidth,
      y:
        row === spec.rows
          ? (spec.width_mm * spec.rows) / spec.columns
          : row * pieceHeight,
    };
  }
  const seed = (BigInt(row) << 32n) | BigInt(column);
  const x =
    column === 0
      ? 0
      : column === spec.columns
        ? spec.width_mm
        : column * pieceWidth +
          (edgeNoise(seed, 0n) - 0.5) * pieceWidth * 0.18;
  const modelHeight = (spec.width_mm * spec.rows) / spec.columns;
  const y =
    row === 0
      ? 0
      : row === spec.rows
        ? modelHeight
        : row * pieceHeight +
          (edgeNoise(seed, 1n) - 0.5) * pieceHeight * 0.18;
  return { x, y };
}

function edgeSign(
  orientation: number,
  segment: number,
  line: number,
  lineCount: number,
) {
  if (line === 0 || line === lineCount) return 0;
  const seed =
    BigInt.asUintN(64, BigInt(orientation) * 0xa24baed4963ee407n) ^
    BigInt.asUintN(64, BigInt(line) * 0x9fb21c651e98df25n) ^
    BigInt.asUintN(64, BigInt(segment) * 0xc13fa9a902a6328fn);
  return edgeNoise(seed, 7n) < 0.5 ? -1 : 1;
}

function jigsawEdge(t: number, pattern: EdgePattern) {
  const radius = pattern.radiusAlong;
  const neck = radius * 0.46;
  const shoulderStart = pattern.center - radius - 0.085;
  const shoulderEnd = pattern.center + radius + 0.085;
  const neckLeft: [number, number] = [pattern.center - neck, 0.18];
  const neckRight: [number, number] = [pattern.center + neck, 0.18];
  const headLeft: [number, number] = [pattern.center - radius, 0.58];
  const headRight: [number, number] = [pattern.center + radius, 0.58];
  const quarterCircle = 0.5522848;
  let point;
  if (t < 0.26) {
    point = { along: (t / 0.26) * shoulderStart, offset: 0 };
  } else if (t < 0.34) {
    point = cubicBezier(
      [shoulderStart, 0],
      [shoulderStart + 0.045, -0.01],
      [neckLeft[0] - 0.025, 0.04],
      neckLeft,
      (t - 0.26) / 0.08,
    );
  } else if (t < 0.42) {
    point = cubicBezier(
      neckLeft,
      [neckLeft[0] + 0.012, 0.34],
      [headLeft[0], 0.45],
      headLeft,
      (t - 0.34) / 0.08,
    );
  } else if (t < 0.5) {
    point = cubicBezier(
      headLeft,
      [
        headLeft[0],
        headLeft[1] + (1 - headLeft[1]) * quarterCircle,
      ],
      [pattern.center - radius * quarterCircle, 1],
      [pattern.center, 1],
      (t - 0.42) / 0.08,
    );
  } else if (t < 0.58) {
    point = cubicBezier(
      [pattern.center, 1],
      [pattern.center + radius * quarterCircle, 1],
      [
        headRight[0],
        headRight[1] + (1 - headRight[1]) * quarterCircle,
      ],
      headRight,
      (t - 0.5) / 0.08,
    );
  } else if (t < 0.66) {
    point = cubicBezier(
      headRight,
      [headRight[0], 0.45],
      [neckRight[0] - 0.012, 0.34],
      neckRight,
      (t - 0.58) / 0.08,
    );
  } else if (t < 0.74) {
    point = cubicBezier(
      neckRight,
      [neckRight[0] + 0.025, 0.04],
      [shoulderEnd - 0.045, -0.01],
      [shoulderEnd, 0],
      (t - 0.66) / 0.08,
    );
  } else {
    point = {
      along: shoulderEnd + ((t - 0.74) / 0.26) * (1 - shoulderEnd),
      offset: 0,
    };
  }
  return {
    along: point.along + pattern.skew * point.offset,
    offset: point.offset,
  };
}

function puzzleEdgePoint(
  start: { x: number; y: number },
  end: { x: number; y: number },
  pattern: EdgePattern,
  sign: number,
  t: number,
  baseDepth: number,
) {
  const deltaX = end.x - start.x;
  const deltaY = end.y - start.y;
  const length = Math.max(Number.EPSILON, Math.hypot(deltaX, deltaY));
  const edge = sign === 0 ? { along: t, offset: 0 } : jigsawEdge(t, pattern);
  const depth = baseDepth * pattern.depthScale;
  return {
    x:
      start.x +
      deltaX * edge.along -
      (deltaY / length) * sign * depth * edge.offset,
    y:
      start.y +
      deltaY * edge.along +
      (deltaX / length) * sign * depth * edge.offset,
  };
}

function ReliefPreview({
  spec,
  preview,
  previewState,
}: {
  spec: GenerationSpec;
  preview: PreviewData | null;
  previewState: "shape" | "loading" | "elevation" | "generated";
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const renderErrorRef = useRef<HTMLParagraphElement>(null);
  const controlsRef = useRef<OrbitControls | null>(null);
  const viewRef = useRef<{
    position: [number, number, number];
    target: [number, number, number];
  } | null>(null);
  const resetViewRef = useRef(false);
  const [resetViewKey, setResetViewKey] = useState(0);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    if (renderErrorRef.current) renderErrorRef.current.hidden = true;
    let renderer: THREE.WebGLRenderer;
    try {
      renderer = new THREE.WebGLRenderer({
        canvas,
        antialias: true,
        alpha: true,
        powerPreference: "high-performance",
      });
    } catch {
      if (renderErrorRef.current) renderErrorRef.current.hidden = false;
      return;
    }

    const scene = new THREE.Scene();
    const sampleWidth = preview?.width ?? 32;
    const sampleHeight = preview?.height ?? sampleWidth;
    const rawHeightScale = (spec.relief_mm / spec.width_mm) * 4.2;
    const heightScale = Math.max(
      0.12,
      rawHeightScale <= 0.48
        ? rawHeightScale
        : 0.48 + Math.log1p((rawHeightScale - 0.48) * 1.6) / 1.6,
    );
    canvas.dataset.heightScale = heightScale.toFixed(4);
    const seedA = Math.sin((spec.center_lat * Math.PI) / 180) * 1.7;
    const seedB = Math.cos((spec.center_lon * Math.PI) / 180) * 1.3;
    const heightValues = Array.from(
      { length: sampleWidth * sampleHeight },
      (_, index) => {
        const x = index % sampleWidth;
        const y = Math.floor(index / sampleWidth);
        const u = x / Math.max(1, sampleWidth - 1);
        const v = y / Math.max(1, sampleHeight - 1);
        return (
          preview?.values[index] ??
          (() => {
            const ridge =
              Math.sin((u * 9.2 + seedA) * 1.2) * 0.19 +
              Math.cos((v * 7.1 - seedB) * 1.4) * 0.14;
            const folds =
              Math.abs(Math.sin((u * 3.8 + v * 5.6 + seedB) * Math.PI)) *
              0.17;
            const dx = u - (0.54 + seedB * 0.05);
            const dy = v - (0.48 + seedA * 0.05);
            const peak = Math.exp(-(dx * dx * 5.5 + dy * dy * 7)) * 0.63;
            return Math.max(0.03, Math.min(1, 0.12 + ridge + folds + peak));
          })()
        );
      },
    );

    const heightAt = (u: number, v: number) => {
      const sampleX = Math.max(
        0,
        Math.min(sampleWidth - 1, u * (sampleWidth - 1)),
      );
      const sampleY = Math.max(
        0,
        Math.min(sampleHeight - 1, v * (sampleHeight - 1)),
      );
      const x0 = Math.floor(sampleX);
      const y0 = Math.floor(sampleY);
      const x1 = Math.min(sampleWidth - 1, x0 + 1);
      const y1 = Math.min(sampleHeight - 1, y0 + 1);
      const tx = sampleX - x0;
      const ty = sampleY - y0;
      const bottom =
        heightValues[y0 * sampleWidth + x0] * (1 - tx) +
        heightValues[y0 * sampleWidth + x1] * tx;
      const top =
        heightValues[y1 * sampleWidth + x0] * (1 - tx) +
        heightValues[y1 * sampleWidth + x1] * tx;
      return bottom * (1 - ty) + top * ty;
    };

    const palette = {
      rock:
        preview?.surface_palette?.rock ?? spec.color_output.rock_color,
      forest:
        preview?.surface_palette?.forest ?? spec.color_output.forest_color,
      snow:
        preview?.surface_palette?.snow ?? spec.color_output.snow_color,
      water:
        preview?.surface_palette?.water ?? spec.color_output.water_color,
      road:
        preview?.surface_palette?.road ?? spec.color_output.road_color,
      building:
        preview?.surface_palette?.building ?? spec.color_output.building_color,
    };
    const classColor = (surfaceClass?: number) =>
      surfaceClass === 1
        ? palette.forest
        : surfaceClass === 2
          ? palette.snow
          : surfaceClass === 3
            ? palette.water
            : surfaceClass === 4
              ? palette.road
              : surfaceClass === 5
                ? palette.building
                : spec.color_output.enabled
                  ? palette.rock
                  : "#74846B";
    const positions: number[] = [];
    const colors: number[] = [];
    const normals: number[] = [];
    const normalAt = (x: number, y: number) => {
      const left = Math.max(0, x - 1);
      const right = Math.min(sampleWidth - 1, x + 1);
      const down = Math.max(0, y - 1);
      const up = Math.min(sampleHeight - 1, y + 1);
      const spanX = Math.max(1, right - left) / Math.max(1, sampleWidth - 1);
      const spanY = Math.max(1, up - down) / Math.max(1, sampleHeight - 1);
      const slopeX =
        ((heightValues[y * sampleWidth + right] -
          heightValues[y * sampleWidth + left]) *
          heightScale) /
        spanX;
      const slopeY =
        ((heightValues[up * sampleWidth + x] -
          heightValues[down * sampleWidth + x]) *
          heightScale) /
        spanY;
      return new THREE.Vector3(-slopeX, 1, -slopeY).normalize();
    };
    const addVertex = (
      x: number,
      y: number,
      color: THREE.Color,
    ) => {
      const u = x / Math.max(1, sampleWidth - 1);
      const v = y / Math.max(1, sampleHeight - 1);
      positions.push(
        u - 0.5,
        heightValues[y * sampleWidth + x] * heightScale,
        v - 0.5,
      );
      colors.push(color.r, color.g, color.b);
      const normal = normalAt(x, y);
      normals.push(normal.x, normal.y, normal.z);
    };
    for (let y = 0; y < sampleHeight - 1; y += 1) {
      for (let x = 0; x < sampleWidth - 1; x += 1) {
        const surfaceClass =
          spec.color_output.enabled || spec.buildings.enabled
          ? preview?.surface_classes?.[y * sampleWidth + x]
          : undefined;
        const color = new THREE.Color(classColor(surfaceClass));
        addVertex(x, y, color);
        addVertex(x, y + 1, color);
        addVertex(x + 1, y, color);
        addVertex(x + 1, y, color);
        addVertex(x, y + 1, color);
        addVertex(x + 1, y + 1, color);
      }
    }

    const terrainGeometry = new THREE.BufferGeometry();
    terrainGeometry.setAttribute(
      "position",
      new THREE.Float32BufferAttribute(positions, 3),
    );
    terrainGeometry.setAttribute(
      "color",
      new THREE.Float32BufferAttribute(colors, 3),
    );
    terrainGeometry.setAttribute(
      "normal",
      new THREE.Float32BufferAttribute(normals, 3),
    );
    const terrainMaterial = new THREE.MeshStandardMaterial({
      color: 0xffffff,
      metalness: 0,
      roughness: 0.86,
      side: THREE.DoubleSide,
      vertexColors: true,
    });
    const terrainMesh = new THREE.Mesh(terrainGeometry, terrainMaterial);
    scene.add(terrainMesh);

    const baseDepth = 0.055;
    const baseGeometry = new THREE.BoxGeometry(1, baseDepth, 1);
    const baseMaterial = new THREE.MeshStandardMaterial({
      color: new THREE.Color(palette.rock).multiplyScalar(0.68),
      metalness: 0,
      roughness: 0.92,
    });
    const baseMesh = new THREE.Mesh(baseGeometry, baseMaterial);
    baseMesh.position.y = -baseDepth / 2;
    scene.add(baseMesh);

    const lineMaterial = new THREE.LineBasicMaterial({
      color: 0x14201d,
      opacity: 0.72,
      transparent: true,
    });
    const pointOnTerrain = (u: number, v: number) =>
      new THREE.Vector3(
        u - 0.5,
        heightAt(u, v) * heightScale + 0.0025,
        v - 0.5,
      );
    const perimeter = [
      ...Array.from({ length: sampleWidth }, (_, x) =>
        pointOnTerrain(x / Math.max(1, sampleWidth - 1), 0),
      ),
      ...Array.from({ length: sampleHeight - 1 }, (_, y) =>
        pointOnTerrain(1, (y + 1) / Math.max(1, sampleHeight - 1)),
      ),
      ...Array.from({ length: sampleWidth - 1 }, (_, x) =>
        pointOnTerrain(
          (sampleWidth - 2 - x) / Math.max(1, sampleWidth - 1),
          1,
        ),
      ),
      ...Array.from({ length: sampleHeight - 2 }, (_, y) =>
        pointOnTerrain(
          0,
          (sampleHeight - 2 - y) / Math.max(1, sampleHeight - 1),
        ),
      ),
    ];
    perimeter.push(perimeter[0].clone());
    scene.add(
      new THREE.Line(
        new THREE.BufferGeometry().setFromPoints(perimeter),
        lineMaterial,
      ),
    );

    const modelHeight = (spec.width_mm * spec.rows) / spec.columns;
    const puzzleTabDepth =
      Math.min(spec.width_mm / spec.columns, modelHeight / spec.rows) * 0.17;
    if (!spec.solid_model) {
      for (let edgeColumn = 1; edgeColumn < spec.columns; edgeColumn += 1) {
        for (let row = 0; row < spec.rows; row += 1) {
          const start = puzzleGridPoint(spec, row, edgeColumn);
          const end = puzzleGridPoint(spec, row + 1, edgeColumn);
          const pattern = sharedEdgePattern(1, edgeColumn, row);
          const sign = spec.puzzle_tabs
            ? edgeSign(1, row, edgeColumn, spec.columns)
            : 0;
          const points = [];
          for (let step = 0; step <= 48; step += 1) {
            const t = step / 48;
            const edgePoint = puzzleEdgePoint(
              start,
              end,
              pattern,
              sign,
              t,
              puzzleTabDepth,
            );
            points.push(
              pointOnTerrain(
                edgePoint.x / spec.width_mm,
                edgePoint.y / modelHeight,
              ),
            );
          }
          scene.add(
            new THREE.Line(
              new THREE.BufferGeometry().setFromPoints(points),
              lineMaterial,
            ),
          );
        }
      }
      for (let edgeRow = 1; edgeRow < spec.rows; edgeRow += 1) {
        for (let column = 0; column < spec.columns; column += 1) {
          const start = puzzleGridPoint(spec, edgeRow, column);
          const end = puzzleGridPoint(spec, edgeRow, column + 1);
          const pattern = sharedEdgePattern(0, edgeRow, column);
          const sign = spec.puzzle_tabs
            ? edgeSign(0, column, edgeRow, spec.rows)
            : 0;
          const points = [];
          for (let step = 0; step <= 48; step += 1) {
            const t = step / 48;
            const edgePoint = puzzleEdgePoint(
              start,
              end,
              pattern,
              sign,
              t,
              puzzleTabDepth,
            );
            points.push(
              pointOnTerrain(
                edgePoint.x / spec.width_mm,
                edgePoint.y / modelHeight,
              ),
            );
          }
          scene.add(
            new THREE.Line(
              new THREE.BufferGeometry().setFromPoints(points),
              lineMaterial,
            ),
          );
        }
      }
    }

    scene.add(new THREE.HemisphereLight(0xffffff, 0x39433c, 1.8));
    const keyLight = new THREE.DirectionalLight(0xfff8df, 2.7);
    keyLight.position.set(-1.4, 2.1, 1.5);
    scene.add(keyLight);
    const fillLight = new THREE.DirectionalLight(0xb9d8ff, 0.8);
    fillLight.position.set(1.3, 0.7, -1);
    scene.add(fillLight);

    const camera = new THREE.PerspectiveCamera(36, 1, 0.01, 20);
    const cameraScale = Math.max(1, heightScale * 1.5);
    const defaultTarget: [number, number, number] = [
      0,
      heightScale * 0.35,
      0,
    ];
    const savedView = resetViewRef.current ? null : viewRef.current;
    if (savedView) {
      camera.position.fromArray(savedView.position);
    } else {
      camera.position.set(
        0.92 * cameraScale,
        defaultTarget[1] + 0.72 * cameraScale,
        1.08 * cameraScale,
      );
    }
    const controls = new OrbitControls(camera, canvas);
    controls.target.fromArray(savedView?.target ?? defaultTarget);
    controls.enableDamping = false;
    controls.enablePan = false;
    controls.minDistance = 0.72;
    controls.maxDistance = 3.1 * cameraScale;
    controls.minPolarAngle = 0.12;
    controls.maxPolarAngle = Math.PI / 2 - 0.025;
    controls.rotateSpeed = 0.72;
    controls.zoomSpeed = 0.85;
    controls.zoomToCursor = true;
    controls.update();
    controlsRef.current = controls;
    canvas.dataset.cameraMoved = savedView ? "true" : "false";
    resetViewRef.current = false;
    const initialPosition = camera.position.clone();
    const initialTarget = controls.target.clone();

    renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
    renderer.outputColorSpace = THREE.SRGBColorSpace;
    renderer.toneMapping = THREE.ACESFilmicToneMapping;
    renderer.toneMappingExposure = 1.05;

    const render = () => renderer.render(scene, camera);
    const resize = () => {
      const width = Math.max(1, canvas.clientWidth);
      const height = Math.max(1, canvas.clientHeight);
      renderer.setSize(width, height, false);
      camera.aspect = width / height;
      camera.updateProjectionMatrix();
      render();
    };
    const onViewChange = () => {
      const positionDelta =
        camera.position.distanceToSquared(initialPosition);
      const targetDelta = controls.target.distanceToSquared(initialTarget);
      const cameraMoved = positionDelta > 1e-12 || targetDelta > 1e-12;
      if (cameraMoved) {
        canvas.dataset.cameraMoved = "true";
        viewRef.current = {
          position: camera.position.toArray(),
          target: controls.target.toArray(),
        };
      }
      render();
    };
    controls.addEventListener("change", onViewChange);
    const observer = new ResizeObserver(resize);
    observer.observe(canvas);
    resize();

    return () => {
      if (!resetViewRef.current && canvas.dataset.cameraMoved === "true") {
        viewRef.current = {
          position: camera.position.toArray(),
          target: controls.target.toArray(),
        };
      }
      observer.disconnect();
      controls.removeEventListener("change", onViewChange);
      controls.dispose();
      controlsRef.current = null;
      scene.traverse((object) => {
        if (object instanceof THREE.Mesh || object instanceof THREE.Line) {
          object.geometry.dispose();
          const materials = Array.isArray(object.material)
            ? object.material
            : [object.material];
          materials.forEach((material) => material.dispose());
        }
      });
      renderer.dispose();
    };
  }, [preview, resetViewKey, spec]);

  const keyboardOrbit = (event: ReactKeyboardEvent<HTMLCanvasElement>) => {
    const controls = controlsRef.current;
    if (!controls) return;
    switch (event.key) {
      case "ArrowLeft":
        controls.rotateLeft(Math.PI / 18);
        break;
      case "ArrowRight":
        controls.rotateLeft(-Math.PI / 18);
        break;
      case "ArrowUp":
        controls.rotateUp(Math.PI / 24);
        break;
      case "ArrowDown":
        controls.rotateUp(-Math.PI / 24);
        break;
      case "+":
      case "=":
        controls.dollyIn(1.12);
        break;
      case "-":
      case "_":
        controls.dollyOut(1.12);
        break;
      default:
        return;
    }
    event.preventDefault();
    controls.update();
  };

  return (
    <div className="relief-shell">
      <canvas
        ref={canvasRef}
        className="relief-canvas"
        aria-label="Interactive 3D terrain preview"
        data-camera-moved="false"
        data-puzzle-tabs={spec.puzzle_tabs}
        data-straight-piece-sides={spec.straight_piece_sides}
        onKeyDown={keyboardOrbit}
        tabIndex={0}
      />
      <p ref={renderErrorRef} className="preview-render-error" hidden>
        This system could not start the 3D preview.
      </p>
      <div className="preview-orbit-controls" aria-label="3D preview controls">
        <span>Drag to rotate · Scroll or pinch to zoom</span>
        <button
          type="button"
          onClick={() => {
            resetViewRef.current = true;
            viewRef.current = null;
            setResetViewKey((current) => current + 1);
          }}
        >
          Reset view
        </button>
      </div>
      {(spec.color_output.enabled || spec.buildings.enabled) && (
        <div className="color-legend" aria-label="Surface color legend">
          {(
            [
              ["Forest", "forest", spec.color_output.forest_color],
              ["Rock", "rock", spec.color_output.rock_color],
              ["Snow", "snow", spec.color_output.snow_color],
              ["Water", "water", spec.color_output.water_color],
              ["Route", "road", spec.color_output.road_color],
              ["Building", "building", spec.color_output.building_color],
            ] as const
          )
            .filter(
              ([, key]) => {
                if (!spec.color_output.enabled) {
                  return (
                    key === "rock" ||
                    (key === "building" && spec.buildings.enabled)
                  );
                }
                return (
                  (key !== "road" || spec.color_output.roads_enabled) &&
                  (key !== "building" || spec.buildings.enabled)
                );
              },
            )
            .map(([label, key, color]) => (
              <span key={key}>
                <i
                  style={{
                    background: preview?.surface_palette?.[key] ?? color,
                  }}
                />
                {label}
                {preview?.surface_coverage && (
                  <small>{preview.surface_coverage[key].toFixed(0)}%</small>
                )}
              </span>
            ))}
        </div>
      )}
      <div className="preview-label">
        <span>
          {previewState === "generated"
            ? "Generated terrain"
            : previewState === "elevation"
              ? "Live elevation preview"
              : previewState === "loading"
                ? "Loading local elevation"
                : "Fast shape preview"}{" "}
          ·{" "}
          {spec.solid_model
            ? `${Math.max(
                96,
                Math.min(
                  Math.max(
                    spec.samples_per_piece * 2,
                    spec.color_output.enabled || spec.buildings.enabled
                      ? spec.overlay_samples_per_piece
                      : 0,
                  ),
                  256,
                ),
              )} mesh samples`
            : `${Math.max(
                spec.samples_per_piece,
                spec.color_output.enabled || spec.buildings.enabled
                  ? spec.overlay_samples_per_piece
                  : 0,
              )} samples/piece`}
        </span>
        <strong>
          {spec.solid_model
            ? "One solid terrain model"
            : `${spec.columns} × ${spec.rows} pieces`}
        </strong>
      </div>
    </div>
  );
}

function RangeField({
  label,
  value,
  unit,
  min,
  max,
  step,
  onChange,
}: {
  label: string;
  value: number;
  unit: string;
  min: number;
  max: number;
  step: number;
  onChange: (value: number) => void;
}) {
  return (
    <label className="range-field">
      <span>
        {label}
        <output>
          {value}
          {unit}
        </output>
      </span>
      <input
        aria-label={label}
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(event) => onChange(Number(event.target.value))}
      />
    </label>
  );
}

export function TerrainStudio() {
  const [spec, setSpec] = useState(initialSpec);
  const [visualHeightPercent, setVisualHeightPercent] = useState(
    DEFAULT_VISUAL_HEIGHT_PERCENT,
  );
  const [activeSection, setActiveSection] = useState<
    "model" | "surface" | "buildings" | "tray" | "output"
  >("model");
  const [job, setJob] = useState<Job | null>(null);
  const [generatedPreview, setGeneratedPreview] =
    useState<PreviewData | null>(null);
  const [elevationPreview, setElevationPreview] =
    useState<PreviewData | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [savingArtifact, setSavingArtifact] = useState<string | null>(null);
  const [placeQuery, setPlaceQuery] = useState("");
  const [placeResults, setPlaceResults] = useState<PlaceResult[]>([]);
  const [placeMessage, setPlaceMessage] = useState<string | null>(null);
  const [searchingPlaces, setSearchingPlaces] = useState(false);
  const workspaceRef = useRef<HTMLDivElement>(null);
  const resizePointerRef = useRef<number | null>(null);

  const setVisualHeightFromPointer = useCallback((clientY: number) => {
    const bounds = workspaceRef.current?.getBoundingClientRect();
    if (!bounds || bounds.height <= WORKSPACE_RESIZER_HEIGHT_PX) return;
    const usableHeight = bounds.height - WORKSPACE_RESIZER_HEIGHT_PX;
    const previewHeight =
      clientY - bounds.top - WORKSPACE_RESIZER_HEIGHT_PX / 2;
    const nextPercent = (previewHeight / usableHeight) * 100;
    setVisualHeightPercent(
      Math.min(
        MAX_VISUAL_HEIGHT_PERCENT,
        Math.max(MIN_VISUAL_HEIGHT_PERCENT, nextPercent),
      ),
    );
  }, []);

  const resizePointerDown = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      if (event.button !== 0) return;
      event.preventDefault();
      resizePointerRef.current = event.pointerId;
      event.currentTarget.setPointerCapture(event.pointerId);
      setVisualHeightFromPointer(event.clientY);
    },
    [setVisualHeightFromPointer],
  );

  const resizePointerMove = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      if (resizePointerRef.current !== event.pointerId) return;
      setVisualHeightFromPointer(event.clientY);
    },
    [setVisualHeightFromPointer],
  );

  const resizePointerUp = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      if (resizePointerRef.current !== event.pointerId) return;
      setVisualHeightFromPointer(event.clientY);
      resizePointerRef.current = null;
      if (event.currentTarget.hasPointerCapture(event.pointerId)) {
        event.currentTarget.releasePointerCapture(event.pointerId);
      }
    },
    [setVisualHeightFromPointer],
  );

  const resizeKeyDown = useCallback(
    (event: ReactKeyboardEvent<HTMLDivElement>) => {
      let nextPercent: number | null = null;
      if (event.key === "ArrowUp") {
        nextPercent = visualHeightPercent - VISUAL_HEIGHT_KEYBOARD_STEP;
      } else if (event.key === "ArrowDown") {
        nextPercent = visualHeightPercent + VISUAL_HEIGHT_KEYBOARD_STEP;
      } else if (event.key === "Home") {
        nextPercent = MIN_VISUAL_HEIGHT_PERCENT;
      } else if (event.key === "End") {
        nextPercent = MAX_VISUAL_HEIGHT_PERCENT;
      }
      if (nextPercent === null) return;
      event.preventDefault();
      setVisualHeightPercent(
        Math.min(
          MAX_VISUAL_HEIGHT_PERCENT,
          Math.max(MIN_VISUAL_HEIGHT_PERCENT, nextPercent),
        ),
      );
    },
    [visualHeightPercent],
  );

  const update = useCallback(
    <Key extends keyof GenerationSpec>(key: Key, value: GenerationSpec[Key]) => {
      setGeneratedPreview(null);
      setSpec((current) => ({ ...current, [key]: value }));
    },
    [],
  );
  const updateColor = useCallback(
    <Key extends keyof GenerationSpec["color_output"]>(
      key: Key,
      value: GenerationSpec["color_output"][Key],
    ) => {
      setGeneratedPreview(null);
      setSpec((current) => ({
        ...current,
        color_output: { ...current.color_output, [key]: value },
      }));
    },
    [],
  );
  const updateTray = useCallback(
    <Key extends keyof GenerationSpec["tray"]>(
      key: Key,
      value: GenerationSpec["tray"][Key],
    ) => {
      setGeneratedPreview(null);
      setSpec((current) => ({
        ...current,
        tray: { ...current.tray, [key]: value },
      }));
    },
    [],
  );
  const updateBuildings = useCallback(
    <Key extends keyof GenerationSpec["buildings"]>(
      key: Key,
      value: GenerationSpec["buildings"][Key],
    ) => {
      setGeneratedPreview(null);
      setSpec((current) => ({
        ...current,
        buildings: { ...current.buildings, [key]: value },
      }));
    },
    [],
  );

  const onCenterChange = useCallback((longitude: number, latitude: number) => {
    setGeneratedPreview(null);
    setSpec((current) => ({
      ...current,
      center_lat: Number(latitude.toFixed(5)),
      center_lon: Number(longitude.toFixed(5)),
    }));
  }, []);

  useEffect(() => {
    const controller = new AbortController();
    const timer = window.setTimeout(async () => {
      setElevationPreview(null);
      setPreviewLoading(true);
      const previewSpec: GenerationSpec = {
        ...initialSpec,
        center_lat: spec.center_lat,
        center_lon: spec.center_lon,
        ground_span_km: spec.ground_span_km,
        color_output: {
          ...initialSpec.color_output,
          enabled: false,
          roads_enabled: false,
          osm_water_enabled: false,
        },
        buildings: { ...initialSpec.buildings, enabled: false },
        tray: { ...initialSpec.tray, enabled: false },
      };
      try {
        const response = await fetch(`${API_URL}/api/preview`, {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify(previewSpec),
          signal: controller.signal,
        });
        if (!response.ok) return;
        setElevationPreview((await response.json()) as PreviewData);
      } catch (error) {
        if (!(error instanceof DOMException && error.name === "AbortError")) {
          setElevationPreview(null);
        }
      } finally {
        if (!controller.signal.aborted) setPreviewLoading(false);
      }
    }, 450);
    return () => {
      window.clearTimeout(timer);
      controller.abort();
    };
  }, [spec.center_lat, spec.center_lon, spec.ground_span_km]);

  const searchPlaces = async () => {
    const query = placeQuery.trim();
    if (query.length < 2) {
      setPlaceMessage("Enter at least two characters.");
      setPlaceResults([]);
      return;
    }
    setSearchingPlaces(true);
    setPlaceMessage(null);
    try {
      const response = await fetch(
        `${API_URL}/api/places?q=${encodeURIComponent(query)}`,
      );
      const payload = await response.json();
      if (!response.ok) {
        throw new Error(payload.error ?? "Place search failed");
      }
      const results = payload as PlaceResult[];
      setPlaceResults(results);
      if (results.length === 0) {
        setPlaceMessage("No matching places found.");
      }
    } catch (error) {
      setPlaceResults([]);
      setPlaceMessage(
        error instanceof Error ? error.message : "Place search failed.",
      );
    } finally {
      setSearchingPlaces(false);
    }
  };

  const choosePlace = (place: PlaceResult) => {
    onCenterChange(place.longitude, place.latitude);
    setPlaceQuery(place.display_name);
    update("place_name", place.display_name.split(",")[0].trim().slice(0, 48));
    setPlaceResults([]);
    setPlaceMessage(`Map moved to ${place.display_name.split(",")[0]}.`);
    setGeneratedPreview(null);
  };

  useEffect(() => {
    if (!job || !["queued", "running"].includes(job.status)) return;
    const timer = window.setInterval(async () => {
      try {
        const response = await fetch(`${API_URL}/api/jobs/${job.id}`);
        if (!response.ok) throw new Error("Could not read the job");
        const nextJob = (await response.json()) as Job;
        setJob(nextJob);
        if (nextJob.status === "complete") {
          const previewResponse = await fetch(
            `${API_URL}/api/jobs/${nextJob.id}/downloads/preview.json`,
          );
          if (previewResponse.ok) {
            setGeneratedPreview((await previewResponse.json()) as PreviewData);
          }
        }
      } catch {
        setMessage("The generator stopped responding. The job is safe in SQLite.");
      }
    }, 900);
    return () => window.clearInterval(timer);
  }, [job]);

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    setSubmitting(true);
    setMessage(null);
    setJob(null);
    setGeneratedPreview(null);
    try {
      const response = await fetch(`${API_URL}/api/jobs`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(spec),
      });
      const payload = await response.json();
      if (!response.ok) {
        throw new Error(payload.error ?? "Generation could not start");
      }
      setJob(payload as Job);
      setActiveSection("output");
    } catch (error) {
      setActiveSection("output");
      setMessage(
        error instanceof TypeError
          ? "Start the local Rust generator, then try again."
          : error instanceof Error
            ? error.message
            : "Generation could not start.",
      );
    } finally {
      setSubmitting(false);
    }
  };

  const saveDesktopArtifact = async (artifact: Artifact) => {
    if (!job || !IS_TAURI) return;
    setSavingArtifact(artifact.name);
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const savedBytes = await invoke<number | null>("save_artifact", {
        jobId: job.id,
        artifactName: artifact.name,
      });
      if (savedBytes === null) return;
      setMessage(`Saved ${artifact.name}.`);
    } catch (error) {
      setMessage(
        error instanceof Error
          ? error.message
          : typeof error === "string"
            ? error
            : `Could not save ${artifact.name}.`,
      );
    } finally {
      setSavingArtifact(null);
    }
  };

  const statusLabel = useMemo(() => {
    if (!job) return null;
    if (job.status === "complete") return "Your print files are ready.";
    if (job.status === "failed") return job.error ?? "Generation failed.";
    if (job.status === "queued") return "Waiting for the generator…";
    if (job.progress < 40) return "Sampling global elevation…";
    if (
      job.progress < 65 &&
      (job.spec.color_output.enabled || job.spec.buildings.enabled)
    ) {
      if (job.spec.buildings.enabled && !job.spec.color_output.enabled) {
        return "Mapping building footprints…";
      }
      if (job.spec.buildings.enabled) {
        return job.spec.color_output.roads_enabled
          ? "Mapping land cover, routes, and buildings…"
          : "Mapping land cover and buildings…";
      }
      return job.spec.color_output.roads_enabled
        ? "Mapping land cover, roads, or fallback trails…"
        : "Mapping forest, rock, snow, and water…";
    }
    return job.spec.solid_model
      ? "Building one watertight terrain model…"
      : "Building watertight pieces…";
  }, [job]);

  const preview = generatedPreview ?? elevationPreview;
  const previewState = generatedPreview
    ? "generated"
    : elevationPreview
      ? "elevation"
      : previewLoading
        ? "loading"
        : "shape";

  return (
    <main className="studio">
      <header className="topbar">
        <a className="brand" href="#" aria-label="TopoSaic home">
          <span className="brand-mark" aria-hidden="true" />
          <span>
            TopoSaic
            <small>Terrain Puzzle</small>
          </span>
        </a>
        <div className="topbar-actions">
          <div className="build-state">
            <span />
            {job ? statusLabel : "Local engine · SQLite"}
          </div>
          <button
            className="topbar-generate"
            type="submit"
            form="terrain-controls"
            disabled={submitting}
          >
            {submitting ? "Starting…" : "Generate"}
            <span aria-hidden="true">↗</span>
          </button>
        </div>
      </header>

      <div
        className="workspace"
        ref={workspaceRef}
        style={
          {
            "--visual-share": `${visualHeightPercent}fr`,
            "--controls-share": `${100 - visualHeightPercent}fr`,
          } as CSSProperties
        }
      >
        <section className="visual-column" aria-label="Place and model preview">
          <TerrainMap
            spec={spec}
            onCenterChange={onCenterChange}
            onGroundSpanChange={(groundSpanKm) =>
              update("ground_span_km", groundSpanKm)
            }
          />
          <ReliefPreview
            spec={spec}
            preview={preview}
            previewState={previewState}
          />
        </section>

        <div
          aria-label="Resize map and 3D preview"
          aria-orientation="horizontal"
          aria-valuemax={MAX_VISUAL_HEIGHT_PERCENT}
          aria-valuemin={MIN_VISUAL_HEIGHT_PERCENT}
          aria-valuenow={Math.round(visualHeightPercent)}
          aria-valuetext={`${Math.round(visualHeightPercent)}% preview height`}
          className="workspace-resizer"
          onDoubleClick={() =>
            setVisualHeightPercent(DEFAULT_VISUAL_HEIGHT_PERCENT)
          }
          onKeyDown={resizeKeyDown}
          onLostPointerCapture={() => {
            resizePointerRef.current = null;
          }}
          onPointerCancel={() => {
            resizePointerRef.current = null;
          }}
          onPointerDown={resizePointerDown}
          onPointerMove={resizePointerMove}
          onPointerUp={resizePointerUp}
          role="separator"
          tabIndex={0}
          title="Drag to resize the map and 3D preview"
        />

        <form className="controls" id="terrain-controls" onSubmit={submit}>
          <div className="panel-heading">
            <div>
              <h1>Shape your terrain</h1>
              <p>Choose a section. Generate stays within reach.</p>
            </div>
          </div>

          <div
            className="control-tabs"
            role="tablist"
            aria-label="Terrain settings"
          >
            {(
              [
                ["model", "Model"],
                ["surface", "Surface"],
                ["buildings", "Buildings"],
                ["tray", "Tray"],
                ["output", "Output"],
              ] as const
            ).map(([key, label]) => (
              <button
                key={key}
                type="button"
                role="tab"
                aria-selected={activeSection === key}
                className={activeSection === key ? "active" : ""}
                onClick={() => setActiveSection(key)}
              >
                {label}
                {key === "output" && job && (
                  <span className={`tab-status ${job.status}`} />
                )}
              </button>
            ))}
          </div>

          <section
            className="control-section model-controls"
            hidden={activeSection !== "model"}
          >
            <div className="place-search">
              <label htmlFor="place-search-input">Find a place</label>
              <div className="place-search-row">
                <input
                  id="place-search-input"
                  type="search"
                  value={placeQuery}
                  placeholder="Mountain, park, city…"
                  onChange={(event) => setPlaceQuery(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") {
                      event.preventDefault();
                      void searchPlaces();
                    }
                  }}
                />
                <button
                  type="button"
                  disabled={searchingPlaces}
                  onClick={() => void searchPlaces()}
                >
                  {searchingPlaces ? "Searching…" : "Search"}
                </button>
              </div>
              {placeMessage && (
                <p className="place-search-message" role="status">
                  {placeMessage}
                </p>
              )}
              {placeResults.length > 0 && (
                <ul className="place-results" aria-label="Place search results">
                  {placeResults.map((place) => (
                    <li
                      key={`${place.latitude}-${place.longitude}-${place.display_name}`}
                    >
                      <button type="button" onClick={() => choosePlace(place)}>
                        <span>{place.display_name}</span>
                        <small>
                          {place.category} · {place.kind.replaceAll("_", " ")}
                        </small>
                      </button>
                    </li>
                  ))}
                </ul>
              )}
              <p className="place-search-note">
                Submit-only search sends public place names to{" "}
                <a
                  href="https://www.openstreetmap.org/copyright"
                  target="_blank"
                  rel="noreferrer"
                >
                  OpenStreetMap
                </a>
                . Do not enter private information.
              </p>
            </div>

            <div className="coordinate-row">
              <label>
                Latitude
                <input
                  type="number"
                  step="0.00001"
                  value={spec.center_lat}
                  onChange={(event) =>
                    update("center_lat", Number(event.target.value))
                  }
                />
              </label>
              <label>
                Longitude
                <input
                  type="number"
                  step="0.00001"
                  value={spec.center_lon}
                  onChange={(event) =>
                    update("center_lon", Number(event.target.value))
                  }
                />
              </label>
            </div>
            <label className="place-label-field">
              Tray place label
              <input
                type="text"
                maxLength={48}
                required
                value={spec.place_name}
                onChange={(event) => update("place_name", event.target.value)}
              />
              <small>The tray adds the coordinates after this name.</small>
            </label>

            <div
              className="model-mode"
              role="group"
              aria-label="Model type"
            >
              <strong className="model-mode-label">Model type</strong>
              <button
                type="button"
                className={!spec.solid_model ? "active" : ""}
                onClick={() => update("solid_model", false)}
              >
                <span className="mode-mark puzzle-mark" aria-hidden="true">
                  <i />
                  <i />
                  <i />
                  <i />
                </span>
                <span>
                  <strong>Jigsaw puzzle</strong>
                  <small>
                    {spec.puzzle_tabs
                      ? "Separate interlocking pieces"
                      : "Separate pieces with plain cuts"}
                  </small>
                </span>
              </button>
              <button
                type="button"
                className={spec.solid_model ? "active" : ""}
                onClick={() => update("solid_model", true)}
              >
                <span className="mode-mark solid-mark" aria-hidden="true" />
                <span>
                  <strong>Solid terrain</strong>
                  <small>One watertight model, no cuts</small>
                </span>
              </button>
            </div>

            <RangeField
              label="Ground span"
              value={spec.ground_span_km}
              unit=" km"
              min={1}
              max={80}
              step={1}
              onChange={(value) => update("ground_span_km", value)}
            />
            <RangeField
              label="Print width"
              value={spec.width_mm}
              unit=" mm"
              min={80}
              max={300}
              step={5}
              onChange={(value) => update("width_mm", value)}
            />
            <RangeField
              label="Terrain relief"
              value={spec.relief_mm}
              unit=" mm"
              min={3}
              max={80}
              step={1}
              onChange={(value) => update("relief_mm", value)}
            />
            <RangeField
              label="Mesh detail"
              value={spec.samples_per_piece}
              unit={spec.solid_model ? "" : " samples/piece"}
              min={32}
              max={128}
              step={8}
              onChange={(value) => update("samples_per_piece", value)}
            />
            {(spec.color_output.enabled || spec.buildings.enabled) && (
              <RangeField
                label="Overlay detail"
                value={spec.overlay_samples_per_piece}
                unit=" samples/piece"
                min={64}
                max={192}
                step={8}
                onChange={(value) =>
                  update("overlay_samples_per_piece", value)
                }
              />
            )}
            {!spec.solid_model && (
              <RangeField
                label="Fit clearance"
                value={spec.clearance_mm}
                unit=" mm"
                min={0}
                max={0.4}
                step={0.02}
                onChange={(value) => update("clearance_mm", value)}
              />
            )}
          </section>

          <fieldset
            className="color-controls control-section surface-controls"
            aria-label="Surface colors"
            hidden={activeSection !== "surface"}
          >
            <div className="color-heading">
              <div>
                <strong className="color-title">Surface colors</strong>
                <p>Paint the 3MF from mapped land cover and routes.</p>
              </div>
              <label className="color-toggle">
                <input
                  type="checkbox"
                  checked={spec.color_output.enabled}
                  onChange={(event) =>
                    updateColor("enabled", event.target.checked)
                  }
                />
                <span>{spec.color_output.enabled ? "On" : "Off"}</span>
              </label>
            </div>
            {spec.color_output.enabled && (
              <>
                <div className="color-swatches">
                  {(
                    [
                      ["Forest", "forest_color"],
                      ["Rock", "rock_color"],
                      ["Snow", "snow_color"],
                      ["Water", "water_color"],
                      ["Route", "road_color"],
                    ] as const
                  ).map(([label, key]) => (
                    <label key={key}>
                      <input
                        type="color"
                        value={spec.color_output[key]}
                        onChange={(event) => updateColor(key, event.target.value)}
                      />
                      <span>{label}</span>
                      <code>{spec.color_output[key].toUpperCase()}</code>
                    </label>
                  ))}
                </div>
                <RangeField
                  label="Smallest color patch"
                  value={spec.color_output.minimum_patch_mm}
                  unit=" mm"
                  min={0.4}
                  max={4}
                  step={0.2}
                  onChange={(value) => updateColor("minimum_patch_mm", value)}
                />
                <div className="road-options">
                  <label className="color-toggle">
                    <input
                      type="checkbox"
                      checked={spec.color_output.osm_water_enabled}
                      onChange={(event) =>
                        updateColor("osm_water_enabled", event.target.checked)
                      }
                    />
                    <span>OpenStreetMap waterways</span>
                  </label>
                  <small>
                    Adds smooth rivers, streams, canals, and mapped water areas
                  </small>
                </div>
                {spec.color_output.osm_water_enabled && (
                  <RangeField
                    label="Maximum waterway coverage"
                    value={spec.color_output.waterway_coverage_percent}
                    unit="%"
                    min={0}
                    max={100}
                    step={1}
                    onChange={(value) =>
                      updateColor("waterway_coverage_percent", value)
                    }
                  />
                )}
                {spec.color_output.osm_water_enabled && (
                  <p className="control-hint">
                    Keeps rivers and canals, then adds the longest streams up to
                    this share of the print surface. Set 0% for major waterways
                    only or 100% for every mapped stream. Lakes are unchanged.
                  </p>
                )}
                <div className="road-options">
                  <label className="color-toggle">
                    <input
                      type="checkbox"
                      checked={spec.color_output.roads_enabled}
                      onChange={(event) =>
                        updateColor("roads_enabled", event.target.checked)
                      }
                    />
                    <span>Render roads</span>
                  </label>
                  <small>Falls back to trails when no roads cross the map</small>
                </div>
                {spec.color_output.roads_enabled && (
                  <>
                    <RangeField
                      label="Route print width"
                      value={spec.color_output.road_width_mm}
                      unit=" mm"
                      min={0.4}
                      max={4}
                      step={0.1}
                      onChange={(value) => updateColor("road_width_mm", value)}
                    />
                    <div className="road-options">
                      <label className="color-toggle">
                        <input
                          type="checkbox"
                          checked={spec.color_output.adaptive_road_widths}
                          onChange={(event) =>
                            updateColor(
                              "adaptive_road_widths",
                              event.target.checked,
                            )
                          }
                        />
                        <span>Thin dense road networks</span>
                      </label>
                      <small>
                        Reduces route width as mapped road coverage rises
                      </small>
                    </div>
                    <RangeField
                      label="Road layer height"
                      value={spec.color_output.road_height_mm}
                      unit=" mm"
                      min={0.08}
                      max={0.4}
                      step={0.02}
                      onChange={(value) => updateColor("road_height_mm", value)}
                    />
                    <div
                      className="road-options bridge-options"
                      role="group"
                      aria-label="Bridge structure"
                    >
                      <strong>Bridge structure</strong>
                      <label className="color-toggle">
                        <input
                          type="radio"
                          name="bridge-structure"
                          checked={
                            spec.color_output.bridge_structure === "floating"
                          }
                          onChange={() =>
                            updateColor("bridge_structure", "floating")
                          }
                        />
                        <span>Floating</span>
                      </label>
                      <small>Uses a thick deck between the abutments</small>
                      <label className="color-toggle">
                        <input
                          type="radio"
                          name="bridge-structure"
                          checked={
                            spec.color_output.bridge_structure === "supported"
                          }
                          onChange={() =>
                            updateColor("bridge_structure", "supported")
                          }
                        />
                        <span>Fully supported</span>
                      </label>
                      <small>
                        Fills from the deck down to the mapped ground or water
                      </small>
                    </div>
                    {spec.color_output.bridge_structure === "floating" && (
                      <RangeField
                        label="Floating bridge thickness"
                        value={spec.color_output.bridge_thickness_mm}
                        unit=" mm"
                        min={0.4}
                        max={6}
                        step={0.2}
                        onChange={(value) =>
                          updateColor("bridge_thickness_mm", value)
                        }
                      />
                    )}
                  </>
                )}
                <p className="color-note">
                  WorldCover supplies permanent water. OpenStreetMap waterways
                  add smooth lakes, rivers, streams, and canals when enabled.
                  Routes come from OpenStreetMap. The generator uses prominent
                  roads first, then trails only when no roads cross the model.
                  Tagged bridges can use thick floating decks or solid support
                  down to mapped ground or water. Untagged routes follow the
                  terrain. Tunnels stay hidden. The road layer height controls
                  the colored top surface, not bridge thickness. Snow is not
                  live. Sides and bottoms use the rock color.
                </p>
              </>
            )}
          </fieldset>

          <fieldset
            className="color-controls building-controls control-section"
            aria-label="Mapped buildings"
            hidden={activeSection !== "buildings"}
          >
            <div className="color-heading">
              <div>
                <strong className="color-title">Mapped buildings</strong>
                <p>Raise OpenStreetMap building footprints above the terrain.</p>
              </div>
              <label className="color-toggle">
                <input
                  type="checkbox"
                  checked={spec.buildings.enabled}
                  onChange={(event) =>
                    updateBuildings("enabled", event.target.checked)
                  }
                />
                <span>{spec.buildings.enabled ? "On" : "Off"}</span>
              </label>
            </div>
            <div className="color-swatches building-color-swatch">
              <label>
                <input
                  aria-label="Building color"
                  type="color"
                  value={spec.color_output.building_color}
                  onChange={(event) =>
                    updateColor("building_color", event.target.value)
                  }
                />
                <span>Building color</span>
                <code>{spec.color_output.building_color.toUpperCase()}</code>
              </label>
            </div>
            {spec.buildings.enabled && (
              <>
                <RangeField
                  label="Building Z scale"
                  value={spec.buildings.z_scale}
                  unit="×"
                  min={0.5}
                  max={30}
                  step={0.5}
                  onChange={(value) => updateBuildings("z_scale", value)}
                />
                <p className="color-note">
                  Buildings use exact mapped footprints, flat roofs, straight
                  vertical walls, and their own 3MF color material. 1× keeps
                  true height against the map width. Higher values make small
                  buildings easier to print. Tagged heights are used first,
                  then floor count, then an 8 m default.
                </p>
              </>
            )}
          </fieldset>

          {!spec.solid_model && (
            <fieldset className="piece-grid" hidden={activeSection !== "model"}>
              <legend>Piece layout</legend>
              {[4, 6, 8, 10, 12].map((count) => (
                <button
                  type="button"
                  className={
                    spec.rows === count && spec.columns === count
                      ? "active"
                      : ""
                  }
                  key={count}
                  onClick={() => {
                    setGeneratedPreview(null);
                    setSpec((current) => ({
                      ...current,
                      rows: count,
                      columns: count,
                    }));
                  }}
                >
                  <span
                    className="mini-grid"
                    style={{
                      gridTemplateColumns: `repeat(${count}, 1fr)`,
                    }}
                  >
                    {Array.from({ length: count * count }).map((_, index) => (
                      <i key={index} />
                    ))}
                  </span>
                  <span>
                    {count}×{count}
                  </span>
                  <small>{count * count} pieces</small>
                </button>
              ))}
              <div className="piece-custom">
                <label>
                  Columns
                  <select
                    value={spec.columns}
                    onChange={(event) =>
                      update("columns", Number(event.target.value))
                    }
                  >
                    {Array.from({ length: 15 }, (_, index) => index + 2).map(
                      (count) => (
                        <option key={count} value={count}>
                          {count}
                        </option>
                      ),
                    )}
                  </select>
                </label>
                <label>
                  Rows
                  <select
                    value={spec.rows}
                    onChange={(event) =>
                      update("rows", Number(event.target.value))
                    }
                  >
                    {Array.from({ length: 15 }, (_, index) => index + 2).map(
                      (count) => (
                        <option key={count} value={count}>
                          {count}
                        </option>
                      ),
                    )}
                  </select>
                </label>
                <div>
                  <strong>{spec.rows * spec.columns} pieces</strong>
                  <small>
                    About {(spec.width_mm / spec.columns).toFixed(1)} mm wide
                    each
                  </small>
                </div>
              </div>
              <div
                className="piece-shape-options"
                role="group"
                aria-label="Piece shape"
              >
                <label>
                  <input
                    type="checkbox"
                    checked={spec.straight_piece_sides}
                    onChange={(event) =>
                      update("straight_piece_sides", event.target.checked)
                    }
                  />
                  <span>
                    <strong>Straight piece sides</strong>
                    <small>Align each cut instead of warping the grid</small>
                  </span>
                </label>
                <label>
                  <input
                    type="checkbox"
                    checked={spec.puzzle_tabs}
                    onChange={(event) =>
                      update("puzzle_tabs", event.target.checked)
                    }
                  />
                  <span>
                    <strong>Interlocking tabs</strong>
                    <small>Turn off for tab-less pieces with plain cuts</small>
                  </span>
                </label>
              </div>
              {spec.width_mm / spec.columns < 10 && (
                <p className="piece-warning">
                  These pieces are under 10 mm wide. Increase print width for
                  stronger pieces and easier handling.
                </p>
              )}
            </fieldset>
          )}

          <fieldset
            className="color-controls tray-controls control-section"
            aria-label="Shallow terrain tray"
            hidden={activeSection !== "tray"}
          >
            <div className="color-heading">
              <div>
                <strong className="color-title">Shallow terrain tray</strong>
                <p>A fitted base for the terrain or puzzle pieces.</p>
              </div>
              <label className="color-toggle">
                <input
                  type="checkbox"
                  checked={spec.tray.enabled}
                  onChange={(event) =>
                    updateTray("enabled", event.target.checked)
                  }
                />
                <span>{spec.tray.enabled ? "On" : "Off"}</span>
              </label>
            </div>
            {spec.tray.enabled && (
              <>
                <div className="color-swatches">
                  {(
                    [
                      ["Tray", "tray_color"],
                      ["Contours", "contour_color"],
                      ["Label", "label_color"],
                    ] as const
                  ).map(([label, key]) => (
                    <label key={key}>
                      <input
                        type="color"
                        value={spec.tray[key]}
                        onChange={(event) =>
                          updateTray(key, event.target.value)
                        }
                      />
                      <span>{label}</span>
                      <code>{String(spec.tray[key]).toUpperCase()}</code>
                    </label>
                  ))}
                </div>
                <RangeField
                  label="Tray clearance"
                  value={spec.tray.clearance_mm}
                  unit=" mm"
                  min={0.2}
                  max={2}
                  step={0.1}
                  onChange={(value) => updateTray("clearance_mm", value)}
                />
                <RangeField
                  label="Rim width"
                  value={spec.tray.rim_width_mm}
                  unit=" mm"
                  min={5}
                  max={16}
                  step={0.5}
                  onChange={(value) => updateTray("rim_width_mm", value)}
                />
                <RangeField
                  label="Floor thickness"
                  value={spec.tray.floor_mm}
                  unit=" mm"
                  min={1}
                  max={4}
                  step={0.2}
                  onChange={(value) => updateTray("floor_mm", value)}
                />
                <RangeField
                  label="Rim height above floor"
                  value={spec.tray.rim_height_mm}
                  unit=" mm"
                  min={2}
                  max={8}
                  step={0.2}
                  onChange={(value) => updateTray("rim_height_mm", value)}
                />
                <RangeField
                  label="Contour lines"
                  value={spec.tray.contour_count}
                  unit=""
                  min={5}
                  max={60}
                  step={1}
                  onChange={(value) => updateTray("contour_count", value)}
                />
                <p className="color-note">
                  The color 3MF prints contour lines on the flat tray floor and
                  the place name, latitude, and longitude as raised shapes on
                  the top front lip. The job also includes a plain STL.
                </p>
              </>
            )}
          </fieldset>

          <div className="output-intro" hidden={activeSection !== "output"}>
            <strong>{job ? statusLabel : "No generation job yet."}</strong>
            <p>
              Generate a model to collect its color 3MF, tray, manifest, and
              optional STL files here.
            </p>
          </div>

          <div className="engine-note" hidden={activeSection !== "output"}>
            <span>Print source</span>
            <strong>
              <a
                href="https://github.com/tilezen/joerd/blob/master/docs/attribution.md"
                target="_blank"
                rel="noreferrer"
              >
                Global Mapzen elevation tiles
              </a>
            </strong>
            {spec.color_output.enabled && (
              <strong>
                <a
                  href="https://worldcover2021.esa.int/download"
                  target="_blank"
                  rel="noreferrer"
                >
                  ESA WorldCover 2021 surface classes
                </a>
              </strong>
            )}
            {((spec.color_output.enabled &&
              spec.color_output.roads_enabled) ||
              spec.buildings.enabled) && (
              <strong>
                <a
                  href="https://www.openstreetmap.org/copyright"
                  target="_blank"
                  rel="noreferrer"
                >
                  OpenStreetMap route and building data
                </a>
              </strong>
            )}
            <p>
              The job saves source details and required notices in its manifest.
            </p>
          </div>

          {(message || job) && (
            <section
              className={`job-card ${job?.status ?? "notice"}`}
              aria-live="polite"
              hidden={activeSection !== "output"}
            >
              <div>
                <span className="status-dot" />
                <strong>{message ?? statusLabel}</strong>
              </div>
              {job && job.status !== "failed" && (
                <div className="progress-track">
                  <span style={{ width: `${job.progress}%` }} />
                </div>
              )}
              {job?.status === "complete" && (
                <div className="downloads">
                  {job.artifacts
                    .filter(
                      (artifact) =>
                        artifact.name.endsWith(".3mf") ||
                        artifact.name === "manifest.json",
                    )
                    .map((artifact) =>
                      IS_TAURI ? (
                        <button
                          key={artifact.name}
                          type="button"
                          disabled={savingArtifact !== null}
                          onClick={() => void saveDesktopArtifact(artifact)}
                        >
                          <span>{artifact.name}</span>
                          <small>
                            {savingArtifact === artifact.name
                              ? "Choosing…"
                              : `${(artifact.bytes / 1024 / 1024).toFixed(1)} MB`}
                          </small>
                        </button>
                      ) : (
                        <a
                          key={artifact.name}
                          href={`${API_URL}/api/jobs/${job.id}/downloads/${artifact.name}`}
                        >
                          <span>{artifact.name}</span>
                          <small>
                            {(artifact.bytes / 1024 / 1024).toFixed(1)} MB
                          </small>
                        </a>
                      ),
                    )}
                  <details>
                    <summary>STL models</summary>
                    <div>
                      {job.artifacts
                        .filter((artifact) => artifact.name.endsWith(".stl"))
                        .map((artifact) =>
                          IS_TAURI ? (
                            <button
                              key={artifact.name}
                              type="button"
                              disabled={savingArtifact !== null}
                              onClick={() => void saveDesktopArtifact(artifact)}
                            >
                              {savingArtifact === artifact.name
                                ? `Choosing ${artifact.name}…`
                                : artifact.name}
                            </button>
                          ) : (
                            <a
                              key={artifact.name}
                              href={`${API_URL}/api/jobs/${job.id}/downloads/${artifact.name}`}
                            >
                              {artifact.name}
                            </a>
                          ),
                        )}
                    </div>
                  </details>
                </div>
              )}
            </section>
          )}
        </form>
      </div>
    </main>
  );
}
