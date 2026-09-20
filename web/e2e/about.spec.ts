/**
 * About (docs/16 §Settings; docs/17 wave 5 R1) against the REAL `cicada
 * serve` from `playwright.config.ts`: the settings menu's last entry opens
 * the dialog, whose version · commit · built are exactly what `GET
 * /api/version` answers (the same object `hello.version` carried — the
 * build the binary stamped), the protocol is the server's, the threads are
 * the suite's `--threads 2`, a click on the commit puts it on the
 * clipboard, and Esc closes the dialog.
 */
import { expect, test } from "@playwright/test";
import config from "../playwright.config";

const meta = config.metadata as { token: string; serveArgs: string[] };
const TOKEN = meta.token;
const PIPELINE = "02-solids.cic";

interface VersionInfo {
  semver: string;
  commit: string;
  built: string;
}

test("About shows the build /api/version reports, copies the commit, closes on Esc", async ({ page, context }) => {
  const response = await page.request.get(`/api/version?token=${TOKEN}`);
  expect(response.ok(), await response.text()).toBeTruthy();
  const version = (await response.json()) as VersionInfo;
  expect(version.semver.length).toBeGreaterThan(0);
  expect(version.commit).toMatch(/^([0-9a-f]{12,}(-dirty)?|unknown)$/);
  expect(version.built).toMatch(/^\d{4}-\d{2}-\d{2}$/);
  const threads = meta.serveArgs[meta.serveArgs.indexOf("--threads") + 1];
  if (threads === undefined || !/^[1-9]\d*$/.test(threads)) {
    throw new Error(`the suite's serve args carry no explicit --threads N: ${meta.serveArgs.join(" ")}`);
  }

  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await page.goto(`/?token=${TOKEN}&pipeline=${PIPELINE}`);
  await expect(page.getByTestId("app")).toBeVisible();
  await expect(page.getByTestId("tb-conn")).toContainText("open");

  await page.getByTestId("tb-settings").click();
  await expect(page.getByRole("dialog", { name: "settings" })).toBeVisible();
  await page.getByTestId("tb-about").click();
  const dialog = page.getByTestId("about-dialog");
  await expect(dialog).toBeVisible();
  await expect(page.getByRole("dialog", { name: "settings" })).toHaveCount(0);

  await expect(page.getByTestId("about-version")).toHaveText(version.semver);
  await expect(page.getByTestId("about-commit")).toHaveText(version.commit);
  await expect(page.getByTestId("about-built")).toHaveText(version.built);
  await expect(page.getByTestId("about-protocol")).toHaveText("1");
  await expect(page.getByTestId("about-threads")).toHaveText(threads);
  await expect(page.getByTestId("about-engine")).toHaveText(`cicada ${version.semver}`);
  await expect(page.getByTestId("about-notes")).toHaveAttribute(
    "href",
    `https://github.com/bencbartlett/Cicada/releases/tag/v${version.semver}`,
  );

  await page.getByTestId("about-commit").click();
  await expect(page.getByTestId("about-copied")).toHaveText("copied");
  const onClipboard = await page.evaluate(() => navigator.clipboard.readText());
  expect(onClipboard).toBe(version.commit);

  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
});
