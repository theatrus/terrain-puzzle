"use client";

import {
  type FormEvent,
  type PointerEvent as ReactPointerEvent,
  type WheelEvent as ReactWheelEvent,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";

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
  solid_model: boolean;
  color_output: {
    enabled: boolean;
    forest_color: string;
    rock_color: string;
    snow_color: string;
    water_color: string;
    road_color: string;
    roads_enabled: boolean;
    road_width_mm: number;
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
  };
  surface_coverage?: {
    rock: number;
    forest: number;
    snow: number;
    water: number;
    road: number;
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

const API_URL =
  process.env.NEXT_PUBLIC_TERRAIN_API_URL ?? "http://127.0.0.1:8787";

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
  solid_model: false,
  color_output: {
    enabled: true,
    forest_color: "#28543A",
    rock_color: "#7C7468",
    snow_color: "#F4F3EC",
    water_color: "#2F76B5",
    road_color: "#D8A33C",
    roads_enabled: true,
    road_width_mm: 1,
    minimum_patch_mm: 1.2,
  },
};

const TILE_SIZE = 256;
const MAX_MERCATOR_LATITUDE = 85.05112878;

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
}: {
  spec: GenerationSpec;
  onCenterChange: (longitude: number, latitude: number) => void;
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
    18,
    Math.min(
      Math.min(size.width, size.height) * 0.82,
      (spec.ground_span_km * 1000) / metresPerPixel,
    ),
  );

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

  const wheel = (event: ReactWheelEvent<HTMLDivElement>) => {
    event.preventDefault();
    setZoom((current) =>
      Math.max(2, Math.min(15, current + (event.deltaY < 0 ? 1 : -1))),
    );
  };

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
          style={{ height: selectionSize, width: selectionSize }}
        />
      </div>
      <div className="map-zoom" aria-label="Map zoom">
        <button
          type="button"
          aria-label="Zoom in"
          onClick={() => setZoom((current) => Math.min(15, current + 1))}
        >
          +
        </button>
        <button
          type="button"
          aria-label="Zoom out"
          onClick={() => setZoom((current) => Math.max(2, current - 1))}
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

function shadeColor(color: string, factor: number) {
  const value = color.replace("#", "");
  if (!/^[0-9a-f]{6}$/i.test(value)) return color;
  const channels = [0, 2, 4].map((offset) =>
    Math.max(
      0,
      Math.min(255, Math.round(Number.parseInt(value.slice(offset, offset + 2), 16) * factor)),
    ),
  );
  return `rgb(${channels.join(" ")})`;
}

function ReliefPreview({
  spec,
  preview,
}: {
  spec: GenerationSpec;
  preview: PreviewData | null;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ratio = Math.min(window.devicePixelRatio || 1, 2);
    const width = canvas.clientWidth;
    const height = canvas.clientHeight;
    canvas.width = width * ratio;
    canvas.height = height * ratio;
    const context = canvas.getContext("2d");
    if (!context) return;
    context.scale(ratio, ratio);
    context.clearRect(0, 0, width, height);

    const samples = preview?.width ?? 32;
    const points: { x: number; y: number; z: number }[][] = [];
    const seedA = Math.sin((spec.center_lat * Math.PI) / 180) * 1.7;
    const seedB = Math.cos((spec.center_lon * Math.PI) / 180) * 1.3;
    for (let y = 0; y < samples; y += 1) {
      const row = [];
      for (let x = 0; x < samples; x += 1) {
        const u = x / (samples - 1);
        const v = y / (samples - 1);
        const z =
          preview?.values[y * samples + x] ??
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
          })();
        row.push({
          x: width * 0.5 + (u - v) * width * 0.38,
          y:
            height * 0.2 +
            (u + v) * height * 0.27 -
            z * Math.min(92, spec.relief_mm * 5),
          z,
        });
      }
      points.push(row);
    }

    const projectedPoint = (u: number, v: number) => {
      const sampleX = Math.max(0, Math.min(samples - 1, u * (samples - 1)));
      const sampleY = Math.max(0, Math.min(samples - 1, v * (samples - 1)));
      const x0 = Math.floor(sampleX);
      const y0 = Math.floor(sampleY);
      const x1 = Math.min(samples - 1, x0 + 1);
      const y1 = Math.min(samples - 1, y0 + 1);
      const tx = sampleX - x0;
      const ty = sampleY - y0;
      const blend = (key: "x" | "y" | "z") => {
        const bottom =
          points[y0][x0][key] * (1 - tx) + points[y0][x1][key] * tx;
        const top =
          points[y1][x0][key] * (1 - tx) + points[y1][x1][key] * tx;
        return bottom * (1 - ty) + top * ty;
      };
      return { x: blend("x"), y: blend("y"), z: blend("z") };
    };

    for (let y = samples - 2; y >= 0; y -= 1) {
      for (let x = 0; x < samples - 1; x += 1) {
        const a = points[y][x];
        const b = points[y][x + 1];
        const c = points[y + 1][x + 1];
        const d = points[y + 1][x];
        const averageHeight = (a.z + b.z + c.z + d.z) / 4;
        const shade = Math.round(46 + averageHeight * 72);
        const surfaceClass = spec.color_output.enabled
          ? preview?.surface_classes?.[y * samples + x]
          : undefined;
        const palette = preview?.surface_palette;
        const surfaceColor =
          surfaceClass === 1
            ? palette?.forest
            : surfaceClass === 2
              ? palette?.snow
              : surfaceClass === 3
                ? palette?.water
                : surfaceClass === 4
                  ? palette?.road
                  : palette?.rock;
        context.beginPath();
        context.moveTo(a.x, a.y);
        context.lineTo(b.x, b.y);
        context.lineTo(c.x, c.y);
        context.lineTo(d.x, d.y);
        context.closePath();
        context.fillStyle = surfaceColor
          ? shadeColor(surfaceColor, 0.78 + averageHeight * 0.28)
          : `hsl(75 28% ${shade}%)`;
        context.fill();
      }
    }

    context.strokeStyle = "rgba(15, 25, 23, 0.72)";
    context.lineWidth = 1.7;
    const modelHeight = (spec.width_mm * spec.rows) / spec.columns;
    const baseDepth =
      Math.min(spec.width_mm / spec.columns, modelHeight / spec.rows) * 0.17;
    if (!spec.solid_model) {
      for (let edgeColumn = 1; edgeColumn < spec.columns; edgeColumn += 1) {
        for (let row = 0; row < spec.rows; row += 1) {
          const start = puzzleGridPoint(spec, row, edgeColumn);
          const end = puzzleGridPoint(spec, row + 1, edgeColumn);
          const pattern = sharedEdgePattern(1, edgeColumn, row);
          const sign = edgeSign(1, row, edgeColumn, spec.columns);
          context.beginPath();
          for (let step = 0; step <= 64; step += 1) {
            const t = step / 64;
            const edgePoint = puzzleEdgePoint(
              start,
              end,
              pattern,
              sign,
              t,
              baseDepth,
            );
            const point = projectedPoint(
              edgePoint.x / spec.width_mm,
              edgePoint.y / modelHeight,
            );
            if (step === 0) context.moveTo(point.x, point.y);
            else context.lineTo(point.x, point.y);
          }
          context.stroke();
        }
      }
      for (let edgeRow = 1; edgeRow < spec.rows; edgeRow += 1) {
        for (let column = 0; column < spec.columns; column += 1) {
          const start = puzzleGridPoint(spec, edgeRow, column);
          const end = puzzleGridPoint(spec, edgeRow, column + 1);
          const pattern = sharedEdgePattern(0, edgeRow, column);
          const sign = edgeSign(0, column, edgeRow, spec.rows);
          context.beginPath();
          for (let step = 0; step <= 64; step += 1) {
            const t = step / 64;
            const edgePoint = puzzleEdgePoint(
              start,
              end,
              pattern,
              sign,
              t,
              baseDepth,
            );
            const point = projectedPoint(
              edgePoint.x / spec.width_mm,
              edgePoint.y / modelHeight,
            );
            if (step === 0) context.moveTo(point.x, point.y);
            else context.lineTo(point.x, point.y);
          }
          context.stroke();
        }
      }
    }
  }, [preview, spec]);

  return (
    <div className="relief-shell">
      <canvas ref={canvasRef} className="relief-canvas" />
      {spec.color_output.enabled && (
        <div className="color-legend" aria-label="Surface color legend">
          {(
            [
              ["Forest", "forest", spec.color_output.forest_color],
              ["Rock", "rock", spec.color_output.rock_color],
              ["Snow", "snow", spec.color_output.snow_color],
              ["Water", "water", spec.color_output.water_color],
              ["Road", "road", spec.color_output.road_color],
            ] as const
          )
            .filter(
              ([, key]) => key !== "road" || spec.color_output.roads_enabled,
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
          {preview ? "Generated terrain" : "Fast shape preview"} ·{" "}
          {spec.solid_model
            ? `${Math.max(96, Math.min(spec.samples_per_piece * 2, 256))} mesh samples`
            : `${spec.samples_per_piece} samples/piece`}
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
  const [job, setJob] = useState<Job | null>(null);
  const [preview, setPreview] = useState<PreviewData | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [placeQuery, setPlaceQuery] = useState("");
  const [placeResults, setPlaceResults] = useState<PlaceResult[]>([]);
  const [placeMessage, setPlaceMessage] = useState<string | null>(null);
  const [searchingPlaces, setSearchingPlaces] = useState(false);

  const update = useCallback(
    <Key extends keyof GenerationSpec>(key: Key, value: GenerationSpec[Key]) =>
      setSpec((current) => ({ ...current, [key]: value })),
    [],
  );
  const updateColor = useCallback(
    <Key extends keyof GenerationSpec["color_output"]>(
      key: Key,
      value: GenerationSpec["color_output"][Key],
    ) =>
      setSpec((current) => ({
        ...current,
        color_output: { ...current.color_output, [key]: value },
      })),
    [],
  );

  const onCenterChange = useCallback((longitude: number, latitude: number) => {
    setSpec((current) => ({
      ...current,
      center_lat: Number(latitude.toFixed(5)),
      center_lon: Number(longitude.toFixed(5)),
    }));
  }, []);

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
    setPlaceResults([]);
    setPlaceMessage(`Map moved to ${place.display_name.split(",")[0]}.`);
    setPreview(null);
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
            setPreview((await previewResponse.json()) as PreviewData);
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
    setPreview(null);
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
    } catch (error) {
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

  const statusLabel = useMemo(() => {
    if (!job) return null;
    if (job.status === "complete") return "Your print files are ready.";
    if (job.status === "failed") return job.error ?? "Generation failed.";
    if (job.status === "queued") return "Waiting for the generator…";
    if (job.progress < 40) return "Sampling global elevation…";
    if (job.progress < 65 && spec.color_output.enabled) {
      return spec.color_output.roads_enabled
        ? "Mapping land cover and prominent roads…"
        : "Mapping forest, rock, snow, and water…";
    }
    return job.spec.solid_model
      ? "Building one watertight terrain model…"
      : "Building watertight pieces…";
  }, [job, spec.color_output.enabled, spec.color_output.roads_enabled]);

  return (
    <main className="studio">
      <header className="topbar">
        <a className="brand" href="#" aria-label="Terrain Puzzle Studio home">
          <span className="brand-mark" aria-hidden="true">
            T6
          </span>
          <span>
            Terrain Puzzle
            <small>Rust mesh studio</small>
          </span>
        </a>
        <div className="build-state">
          <span />
          Local engine · SQLite
        </div>
      </header>

      <section className="hero">
        <div>
          <p className="eyebrow">Make a place you can hold</p>
          <h1>Turn any landscape into a puzzle.</h1>
        </div>
        <p className="hero-copy">
          Pick a place, tune the relief, then print it as one solid terrain
          model or a full jigsaw.
        </p>
      </section>

      <div className="workspace">
        <section className="visual-column" aria-label="Place and model preview">
          <TerrainMap spec={spec} onCenterChange={onCenterChange} />
          <ReliefPreview spec={spec} preview={preview} />
        </section>

        <form className="controls" onSubmit={submit}>
          <div className="panel-heading">
            <span>01</span>
            <div>
              <h2>Shape your terrain</h2>
              <p>All sizes use millimetres.</p>
            </div>
          </div>

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
            max={35}
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

          <fieldset className="color-controls" aria-label="Surface colors">
            <div className="color-heading">
              <div>
                <strong className="color-title">Surface colors</strong>
                <p>Paint the 3MF from mapped land cover and roads.</p>
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
                      ["Road", "road_color"],
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
                      checked={spec.color_output.roads_enabled}
                      onChange={(event) =>
                        updateColor("roads_enabled", event.target.checked)
                      }
                    />
                    <span>Prominent roads</span>
                  </label>
                  <small>Motorway through secondary road</small>
                </div>
                {spec.color_output.roads_enabled && (
                  <RangeField
                    label="Road print width"
                    value={spec.color_output.road_width_mm}
                    unit=" mm"
                    min={0.6}
                    max={4}
                    step={0.2}
                    onChange={(value) => updateColor("road_width_mm", value)}
                  />
                )}
                <p className="color-note">
                  Water shows mapped permanent lakes, reservoirs, and rivers.
                  Narrow streams below 10 m may not appear. Roads come from
                  OpenStreetMap, with wider lines for higher road classes.
                  Tunnels stay hidden. Snow is not live. Sides and bottoms use
                  the rock color.
                </p>
              </>
            )}
          </fieldset>

          <fieldset className="model-mode">
            <legend>Model type</legend>
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
                <small>Separate interlocking pieces</small>
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
          </fieldset>

          {!spec.solid_model && (
            <fieldset className="piece-grid">
              <legend>Piece layout</legend>
            {[4, 6, 8, 10, 12].map((count) => (
              <button
                type="button"
                className={
                  spec.rows === count && spec.columns === count ? "active" : ""
                }
                key={count}
                onClick={() =>
                  setSpec((current) => ({
                    ...current,
                    rows: count,
                    columns: count,
                  }))
                }
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
                <span>{count}×{count}</span>
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
                  onChange={(event) => update("rows", Number(event.target.value))}
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
                  About {(spec.width_mm / spec.columns).toFixed(1)} mm wide each
                </small>
              </div>
            </div>
            {spec.width_mm / spec.columns < 10 && (
              <p className="piece-warning">
                These pieces are under 10 mm wide. Increase print width for
                stronger knobs and easier handling.
              </p>
            )}
            </fieldset>
          )}

          <div className="engine-note">
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
              <>
                <strong>
                  <a
                    href="https://worldcover2021.esa.int/download"
                    target="_blank"
                    rel="noreferrer"
                  >
                    ESA WorldCover 2021 surface classes
                  </a>
                </strong>
                {spec.color_output.roads_enabled && (
                  <strong>
                    <a
                      href="https://www.openstreetmap.org/copyright"
                      target="_blank"
                      rel="noreferrer"
                    >
                      OpenStreetMap road data
                    </a>
                  </strong>
                )}
              </>
            )}
            <p>
              The job saves source details and required notices in its manifest.
            </p>
          </div>

          <button className="generate-button" type="submit" disabled={submitting}>
            <span>{submitting ? "Starting…" : "Generate print files"}</span>
            <span aria-hidden="true">↗</span>
          </button>

          {(message || job) && (
            <section
              className={`job-card ${job?.status ?? "notice"}`}
              aria-live="polite"
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
                    .map((artifact) => (
                      <a
                        key={artifact.name}
                        href={`${API_URL}/api/jobs/${job.id}/downloads/${artifact.name}`}
                      >
                        <span>{artifact.name}</span>
                        <small>
                          {(artifact.bytes / 1024 / 1024).toFixed(1)} MB
                        </small>
                      </a>
                    ))}
                  <details>
                    <summary>
                      {job.spec.solid_model
                        ? "Solid terrain STL"
                        : "Separate STL pieces"}
                    </summary>
                    <div>
                      {job.artifacts
                        .filter((artifact) => artifact.name.endsWith(".stl"))
                        .map((artifact) => (
                          <a
                            key={artifact.name}
                            href={`${API_URL}/api/jobs/${job.id}/downloads/${artifact.name}`}
                          >
                            {artifact.name}
                          </a>
                        ))}
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
