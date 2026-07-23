import { expect, test } from "@playwright/test";

test("switches between the reflowed control panels", async ({ page }) => {
  await page.goto("/");

  const generate = page.getByRole("button", { name: /^Generate/ });
  await expect(
    page.getByRole("heading", { name: "Shape your terrain" }),
  ).toBeVisible();
  await expect(generate).toHaveAttribute("form", "terrain-controls");
  await expect(page.getByLabel("Find a place")).toBeVisible();

  await page.getByRole("tab", { name: "Surface" }).click();
  await expect(
    page.getByRole("group", { name: "Surface colors" }),
  ).toBeVisible();
  await expect(page.getByLabel("Find a place")).toBeHidden();

  await page.getByRole("tab", { name: "Buildings" }).click();
  await expect(
    page.getByRole("group", { name: "Mapped buildings" }),
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
