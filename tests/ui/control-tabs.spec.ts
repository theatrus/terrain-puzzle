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

  await page.getByText("STL models").click();
  await expect(page.getByRole("link", { name: "piece-01.stl" })).toHaveAttribute(
    "href",
    /\/downloads\/piece-01\.stl$/,
  );
});
