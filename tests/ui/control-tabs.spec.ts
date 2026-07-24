import { expect, test } from "@playwright/test";

test("switches between the reflowed control panels", async ({ page }) => {
  await page.goto("/");

  const generate = page.getByRole("button", { name: /^Generate/ });
  await expect(page.getByRole("link", { name: "TopoSaic home" })).toContainText(
    "TopoSaic",
  );
  await expect(page.getByRole("link", { name: "TopoSaic home" })).toContainText(
    "Terrain Puzzle",
  );
  const brandIcon = page.locator(".brand-mark");
  await expect(brandIcon).toHaveCSS("background-image", /url\(.+\)/);
  const brandIconUrl = await brandIcon.evaluate((element) => {
    const background = getComputedStyle(element).backgroundImage;
    return background.match(/^url\(["']?(.*?)["']?\)$/)?.[1] ?? "";
  });
  expect(brandIconUrl).toBeTruthy();
  const brandIconResponse = await page.request.get(
    new URL(brandIconUrl, page.url()).href,
  );
  expect(brandIconResponse.ok()).toBe(true);
  await expect(
    page.getByRole("heading", { name: "Shape your terrain" }),
  ).toBeVisible();
  await expect(generate).toHaveAttribute("form", "terrain-controls");
  await expect(page.getByLabel("Find a place")).toBeVisible();
  const modelType = page.getByRole("group", { name: "Model type" });
  const puzzleModel = modelType.getByRole("button", {
    name: /Jigsaw puzzle/,
  });
  const solidModel = modelType.getByRole("button", {
    name: /Solid terrain/,
  });
  await expect(modelType).toBeVisible();
  const puzzleModelBounds = await puzzleModel.boundingBox();
  const solidModelBounds = await solidModel.boundingBox();
  expect(puzzleModelBounds).not.toBeNull();
  expect(solidModelBounds).not.toBeNull();
  expect(puzzleModelBounds!.height).toBeLessThan(64);
  expect(puzzleModelBounds!.width).toBeLessThan(340);
  expect(Math.abs(puzzleModelBounds!.y - solidModelBounds!.y)).toBeLessThan(2);
  await solidModel.click();
  await expect(page.getByRole("group", { name: "Piece layout" })).toBeHidden();
  await puzzleModel.click();
  await expect(page.getByRole("group", { name: "Piece layout" })).toBeVisible();
  const pieceShape = page.getByRole("group", { name: "Piece shape" });
  const preview = page.getByLabel("Interactive 3D terrain preview");
  const straightGrid = pieceShape.getByRole("checkbox", {
    name: /Straight piece sides/,
  });
  const interlockingTabs = pieceShape.getByRole("checkbox", {
    name: /Interlocking tabs/,
  });
  await expect(straightGrid).not.toBeChecked();
  await expect(interlockingTabs).toBeChecked();
  await straightGrid.check();
  await interlockingTabs.uncheck();
  await expect(preview).toHaveAttribute("data-straight-piece-sides", "true");
  await expect(preview).toHaveAttribute("data-puzzle-tabs", "false");
  await expect(page.getByText("Separate pieces with plain cuts")).toBeVisible();
  const relief = page.getByRole("slider", { name: "Terrain relief" });
  await expect(relief).toHaveAttribute("max", "80");
  const initialHeightScale = Number(
    await page
      .getByLabel("Interactive 3D terrain preview")
      .getAttribute("data-height-scale"),
  );
  await relief.fill("80");
  await expect(relief).toHaveValue("80");
  await expect
    .poll(async () =>
      Number(
        await page
          .getByLabel("Interactive 3D terrain preview")
          .getAttribute("data-height-scale"),
      ),
    )
    .toBeGreaterThan(initialHeightScale);

  await page.getByRole("tab", { name: "Surface" }).click();
  const surfaceColors = page.getByRole("group", { name: "Surface colors" });
  await expect(surfaceColors).toBeVisible();
  await expect(page.getByLabel("Find a place")).toBeHidden();
  const floatingBridge = surfaceColors.getByRole("radio", {
    name: "Floating",
  });
  const supportedBridge = surfaceColors.getByRole("radio", {
    name: "Fully supported",
  });
  const bridgeThickness = surfaceColors.getByRole("slider", {
    name: "Floating bridge thickness",
  });
  await expect(floatingBridge).toBeChecked();
  await expect(bridgeThickness).toHaveValue("1.2");
  await supportedBridge.check();
  await expect(supportedBridge).toBeChecked();
  await expect(bridgeThickness).toBeHidden();
  await floatingBridge.check();
  await bridgeThickness.fill("2.4");
  await expect(bridgeThickness).toHaveValue("2.4");
  await surfaceColors.getByRole("checkbox").first().uncheck();

  await page.getByRole("tab", { name: "Buildings" }).click();
  await expect(
    page.getByRole("group", { name: "Mapped buildings" }),
  ).toBeVisible();
  const buildingColor = page.getByLabel("Building color");
  await expect(buildingColor).toHaveValue("#b8a890");
  await buildingColor.fill("#8a5b3d");
  await expect(buildingColor).toHaveValue("#8a5b3d");
  await page
    .getByRole("group", { name: "Mapped buildings" })
    .getByRole("checkbox")
    .check();
  await expect(
    page
      .getByLabel("Surface color legend")
      .getByText("Building", { exact: true }),
  ).toBeVisible();

  await page.getByRole("tab", { name: "Tray" }).click();
  await expect(
    page.getByRole("group", { name: "Shallow terrain tray" }),
  ).toBeVisible();

  await page.getByRole("tab", { name: "Output" }).click();
  await expect(page.getByText("No generation job yet.")).toBeVisible();

  await page.getByRole("tab", { name: "Model" }).click();
  await expect(page.getByLabel("Find a place")).toBeVisible();
});

test("resizes the preview area to make room for controls", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");

  const resizer = page.getByRole("separator", {
    name: "Resize map and 3D preview",
  });
  const visualArea = page.locator(".visual-column");
  const controls = page.locator("#terrain-controls");

  await expect(resizer).toBeVisible();
  await expect(resizer).toHaveAttribute("aria-orientation", "horizontal");
  await expect(resizer).toHaveAttribute("aria-valuenow", "37");

  const initialVisualBounds = await visualArea.boundingBox();
  const initialControlBounds = await controls.boundingBox();
  expect(initialVisualBounds).not.toBeNull();
  expect(initialControlBounds).not.toBeNull();
  expect(
    initialControlBounds!.height /
      (initialVisualBounds!.height + initialControlBounds!.height),
  ).toBeCloseTo(0.63, 2);

  await resizer.focus();
  await page.keyboard.press("Home");
  await expect(resizer).toHaveAttribute("aria-valuenow", "28");

  const smallVisualBounds = await visualArea.boundingBox();
  const largeControlBounds = await controls.boundingBox();
  expect(smallVisualBounds).not.toBeNull();
  expect(largeControlBounds).not.toBeNull();
  expect(smallVisualBounds!.height).toBeLessThan(initialVisualBounds!.height);
  expect(largeControlBounds!.height).toBeGreaterThan(
    initialControlBounds!.height,
  );

  const resizerBounds = await resizer.boundingBox();
  expect(resizerBounds).not.toBeNull();
  if (!resizerBounds) return;
  await page.mouse.move(
    resizerBounds.x + resizerBounds.width / 2,
    resizerBounds.y + resizerBounds.height / 2,
  );
  await page.mouse.down();
  await page.mouse.move(
    resizerBounds.x + resizerBounds.width / 2,
    resizerBounds.y + 120,
    { steps: 6 },
  );
  await page.mouse.up();

  await expect
    .poll(async () => Number(await resizer.getAttribute("aria-valuenow")))
    .toBeGreaterThan(28);
});

test("keeps map zoom and ground span in sync", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");

  const groundSpan = page.getByRole("slider", { name: "Ground span" });
  const selection = page.locator(".map-selection");
  await expect(selection).toHaveAttribute(
    "aria-label",
    "Selected terrain area: 18 km square",
  );
  const initialBounds = await selection.boundingBox();
  expect(initialBounds).not.toBeNull();
  await expect(selection).toHaveAttribute("data-map-zoom", "9");

  await page.getByRole("button", { name: "Zoom in" }).click();
  await expect(groundSpan).toHaveValue("9");
  await expect(selection).toHaveAttribute(
    "aria-label",
    "Selected terrain area: 9 km square",
  );
  await expect(selection).toHaveAttribute("data-map-zoom", "10");

  const zoomedBounds = await selection.boundingBox();
  expect(zoomedBounds).not.toBeNull();
  expect(zoomedBounds!.width).toBeCloseTo(initialBounds!.width, 0);

  await groundSpan.fill("30");
  await expect(selection).toHaveAttribute(
    "aria-label",
    "Selected terrain area: 30 km square",
  );
  await expect(selection).toHaveAttribute("data-ground-span-km", "30");
  const largerBounds = await selection.boundingBox();
  expect(largerBounds).not.toBeNull();
  expect(largerBounds!.width).toBeGreaterThan(zoomedBounds!.width);
});

test("locks a height frame when moving to an adjacent tile", async ({
  page,
}) => {
  const previewSpecs: Array<Record<string, unknown>> = [];
  await page.route("http://127.0.0.1:8787/api/preview", async (route) => {
    const spec = route.request().postDataJSON() as Record<string, unknown>;
    previewSpecs.push(spec);
    const moved = Number(spec.center_lon) > -121.7;
    const minimum = moved ? 80 : 100;
    const datum = spec.elevation_datum_m;
    await route.fulfill({
      json: {
        width: 2,
        height: 2,
        values: [0, 0.3, 0.7, 1],
        minimum_elevation_m: minimum,
        maximum_elevation_m: moved ? 280 : 300,
        height_frame_compatible:
          datum === null || datum === undefined || minimum >= Number(datum),
      },
    });
  });

  await page.goto("/");
  await expect(page.getByText("Live elevation preview")).toBeVisible();

  const minimumHeight = page.getByRole("slider", {
    name: "Minimum piece height",
  });
  await expect(minimumHeight).toHaveValue("2.4");
  await minimumHeight.fill("5");
  await expect(minimumHeight).toHaveValue("5");

  const initialLongitude = Number(
    await page.getByLabel("Longitude").inputValue(),
  );
  await page
    .getByRole("group", { name: "Adjacent tiles" })
    .getByRole("button", { name: /east/i })
    .click();

  await expect(page.getByText(/Moved east by one tile/)).toBeVisible();
  await expect(page.getByText(/Shared datum 96\.0 m/)).toBeVisible();
  await expect
    .poll(async () => Number(await page.getByLabel("Longitude").inputValue()))
    .toBeGreaterThan(initialLongitude);
  await expect(
    page.getByRole("alert").filter({ hasText: "drops below the shared" }),
  ).toBeVisible();
  expect(
    previewSpecs.some(
      (spec) =>
        spec.elevation_datum_m === 96 &&
        Number(spec.elevation_m_per_mm) > 0,
    ),
  ).toBe(true);

  await page.getByRole("button", { name: "Use per-tile height" }).click();
  await expect(page.getByText(/adjacent exports may form a step/)).toBeVisible();
});

test("rotates, zooms, and resets the interactive 3D preview", async ({
  page,
}) => {
  await page.goto("/");

  const preview = page.getByLabel("Interactive 3D terrain preview");
  await expect(preview).toBeVisible();
  await expect(
    page.getByText("Drag to rotate · Scroll or pinch to zoom"),
  ).toBeVisible();
  await expect(preview).toHaveAttribute("data-camera-moved", "false");

  const bounds = await preview.boundingBox();
  expect(bounds).not.toBeNull();
  if (!bounds) return;
  await page.mouse.move(
    bounds.x + bounds.width * 0.68,
    bounds.y + bounds.height * 0.62,
  );
  await page.mouse.down();
  await page.mouse.move(
    bounds.x + bounds.width * 0.42,
    bounds.y + bounds.height * 0.4,
    { steps: 8 },
  );
  await page.mouse.up();
  await expect(preview).toHaveAttribute("data-camera-moved", "true");

  await page.getByRole("button", { name: "Reset view" }).click();
  await expect(preview).toHaveAttribute("data-camera-moved", "false");

  await preview.focus();
  await page.keyboard.press("ArrowLeft");
  await expect(preview).toHaveAttribute("data-camera-moved", "true");
});

test("turns Generate into Cancel while a job is active", async ({ page }) => {
  const jobId = "8b4165dc-9b47-4fa2-9f75-2ea36b9dff45";
  let cancelRequested = false;
  let jobSpec: Record<string, unknown> = {};

  await page.route("http://127.0.0.1:8787/api/**", async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    if (url.pathname === "/api/preview") {
      await route.fulfill({
        json: { width: 2, height: 2, values: [0, 0.3, 0.7, 1] },
      });
      return;
    }
    if (url.pathname === "/api/jobs" && request.method() === "POST") {
      jobSpec = request.postDataJSON();
      await route.fulfill({
        status: 202,
        json: {
          id: jobId,
          status: "running",
          progress: 24,
          artifacts: [],
          spec: jobSpec,
        },
      });
      return;
    }
    if (
      url.pathname === `/api/jobs/${jobId}` &&
      request.method() === "DELETE"
    ) {
      cancelRequested = true;
      await route.fulfill({
        json: {
          id: jobId,
          status: "canceled",
          progress: 24,
          artifacts: [],
          spec: jobSpec,
        },
      });
      return;
    }
    await route.abort();
  });

  await page.goto("/");
  await page.getByRole("button", { name: /^Generate/ }).click();

  const cancel = page.getByRole("button", { name: /^Cancel$/ });
  await expect(cancel).toBeVisible();
  await expect(cancel).toHaveClass(/cancel/);
  await expect(
    page.getByText("Sampling elevation and fetching source tiles…").first(),
  ).toBeVisible();
  const steps = page.getByRole("list", { name: "Generation progress" });
  await expect(steps).toContainText("Elevation");
  await expect(steps).toContainText("60%");
  await expect(page.locator(".job-progress output")).toHaveText("24%");
  await cancel.click();

  await expect(page.getByRole("button", { name: /^Generate/ })).toBeVisible();
  await expect(page.getByText("Generation canceled.").first()).toBeVisible();
  expect(cancelRequested).toBe(true);
});

test("keeps direct artifact downloads in the web app", async ({ page }) => {
  await page.route("http://127.0.0.1:8787/api/**", async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    if (url.pathname === "/api/preview") {
      await route.fulfill({
        json: { width: 2, height: 2, values: [0, 0.3, 0.7, 1] },
      });
      return;
    }
    if (url.pathname === "/api/jobs" && request.method() === "POST") {
      await route.fulfill({
        json: {
          id: "e2ba221e-a689-4b59-9d5f-ae9b883596a1",
          status: "complete",
          progress: 100,
          artifacts: [
            {
              name: "terrain.3mf",
              media_type: "model/3mf",
              bytes: 1_048_576,
            },
            {
              name: "manifest.json",
              media_type: "application/json",
              bytes: 1024,
            },
            {
              name: "piece-01.stl",
              media_type: "model/stl",
              bytes: 2048,
            },
          ],
          spec: request.postDataJSON(),
        },
      });
      return;
    }
    if (
      url.pathname.endsWith("/downloads/terrain.3mf") &&
      request.method() === "GET"
    ) {
      await route.fulfill({
        body: "3mf data",
        headers: {
          "content-disposition": 'attachment; filename="terrain.3mf"',
          "content-type": "model/3mf",
        },
      });
      return;
    }
    await route.abort();
  });

  await page.goto("/");
  await page.getByRole("button", { name: /^Generate/ }).click();

  const model = page.getByRole("link", { name: /terrain\.3mf/ });
  await expect(model).toBeVisible();
  await expect(model).toHaveAttribute(
    "href",
    "http://127.0.0.1:8787/api/jobs/e2ba221e-a689-4b59-9d5f-ae9b883596a1/downloads/terrain.3mf",
  );
  const completedSteps = page.getByRole("list", {
    name: "Generation progress",
  });
  await expect(completedSteps).toContainText("Print files");
  await expect(completedSteps).toContainText("Ready");

  const download = page.waitForEvent("download");
  await model.click();
  await expect(model).toContainText("Sent to browser");
  expect((await download).suggestedFilename()).toBe("terrain.3mf");
  await expect(
    page.getByText("Sent terrain.3mf to your browser downloads."),
  ).toBeVisible();

  await page.getByText("STL models").click();
  await expect(
    page.getByRole("link", { name: /piece-01\.stl/ }),
  ).toHaveAttribute("href", /\/downloads\/piece-01\.stl$/);
});
