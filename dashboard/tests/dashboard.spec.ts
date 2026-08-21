import { expect, test } from "@playwright/test";

test("presents the success proof and its limits", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: /p99 hot loop/i })).toBeVisible();
  await expect(page.getByText("Synthetic workload", { exact: true }).first()).toBeVisible();
  await expect(page.getByText(/Paper trading · No live orders/i)).toBeVisible();
  await expect(page.getByRole("heading", { name: /signal only counts/i })).toBeVisible();
  await expect(page.getByRole("heading", { name: /correctness is measured/i })).toBeVisible();
});

test("switches published runs from the history ledger", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Aug 19, 2026" }).click();
  await expect(page.getByText("200×", { exact: true }).first()).toBeVisible();
});

test("does not overflow the mobile viewport", async ({ page }) => {
  await page.goto("/");
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(overflow).toBeLessThanOrEqual(1);
});
