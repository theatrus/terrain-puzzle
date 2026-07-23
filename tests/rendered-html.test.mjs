import assert from "node:assert/strict";
import { access, readFile } from "node:fs/promises";
import test from "node:test";

const projectRoot = new URL("../", import.meta.url);

async function render() {
  const workerUrl = new URL("../dist/server/index.js", import.meta.url);
  workerUrl.searchParams.set("test", `${process.pid}-${Date.now()}`);
  const { default: worker } = await import(workerUrl.href);

  return worker.fetch(
    new Request("http://localhost/", {
      headers: { accept: "text/html" },
    }),
    {
      ASSETS: {
        fetch: async () => new Response("Not found", { status: 404 }),
      },
    },
    {
      waitUntil() {},
      passThroughOnException() {},
    },
  );
}

test("server-renders TopoSaic", async () => {
  const response = await render();
  assert.equal(response.status, 200);
  assert.match(response.headers.get("content-type") ?? "", /^text\/html\b/i);

  const html = await response.text();
  assert.match(html, /<title>TopoSaic — Terrain Puzzle<\/title>/i);
  assert.match(html, />TopoSaic</);
  assert.match(html, /<small>Terrain Puzzle<\/small>/);
  assert.match(html, /Shape your terrain/);
  assert.match(html, /role="tablist"/);
  assert.match(html, />Model</);
  assert.match(html, />Surface</);
  assert.match(html, />Buildings</);
  assert.match(html, />Tray</);
  assert.match(html, />Output</);
  assert.match(html, /id="terrain-controls"/);
  assert.match(html, /Solid terrain/);
  assert.match(html, /Straight piece sides/);
  assert.match(html, /Interlocking tabs/);
  assert.match(html, /tab-less pieces with plain cuts/);
  assert.match(html, /Shallow terrain tray/);
  assert.match(html, /Tray place label/);
  assert.match(html, /Selected terrain area: 18 km square/);
  assert.match(html, /Mapped buildings/);
  assert.match(html, /Building color/);
  assert.match(html, /Render roads/);
  assert.match(html, /OpenStreetMap waterways/);
  assert.match(html, /Maximum waterway coverage/);
  assert.match(html, /major waterways only/);
  assert.match(html, /Thin dense road networks/);
  assert.match(html, /Overlay detail/);
  assert.match(html, /Road layer height/);
  assert.match(html, /Tagged bridges span between their terrain-height abutments/);
  assert.match(html, /#B8A890/i);
  assert.match(html, /SQLite/);
  assert.doesNotMatch(html, /codex-preview|Your site is taking shape/i);
});

test("removes starter-only files and metadata", async () => {
  const [page, layout, packageJson] = await Promise.all([
    readFile(new URL("../app/page.tsx", import.meta.url), "utf8"),
    readFile(new URL("../app/layout.tsx", import.meta.url), "utf8"),
    readFile(new URL("../package.json", import.meta.url), "utf8"),
  ]);

  assert.match(page, /TerrainStudio/);
  assert.match(layout, /TopoSaic — Terrain Puzzle/);
  assert.doesNotMatch(packageJson, /react-loading-skeleton|drizzle/);
  await assert.rejects(access(new URL("../app/_sites-preview", projectRoot)));
});
