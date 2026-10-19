// Bilder des Benutzerhandbuchs aus der Demo-Instanz (siehe seed.py).
// Aufruf aus tests/browser (dort ist Playwright installiert):
//   node ../../docs/tools/handbuch/shots.mjs
// Umgebung: ZAEHLER_URL (Standard http://localhost:18080), CHROMIUM (Pfad
// zu einem Chromium, sonst der von Playwright).
import { createRequire } from "node:module";
import { readFileSync, rmSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
const here = dirname(fileURLToPath(import.meta.url));
const repo = join(here, "..", "..", "..");
const require = createRequire(join(repo, "tests", "browser", "package.json"));
const { chromium } = require("playwright");
const ids = JSON.parse(readFileSync(join(here, "ids.json"), "utf8"));
const M = ids.meters, base = (process.env.ZAEHLER_URL ?? "http://localhost:18080").replace(/\/$/, "");
const out = join(repo, "docs", "handbuch", "bilder");
rmSync(out, { recursive: true, force: true }); mkdirSync(out, { recursive: true });
// [Name, Pfad, Modus]: "full", Zahl = Höhe ab oben, "h2:Text" = Abschnitt mit Überschrift
const desktop = [
  ["uebersicht", "/", 1250],
  ["messstellen", "/meters", "full"],
  ["messstelle-auswertung", `/meters/${M.Hausstrom}`, 1150],
  ["messstelle-prognose", `/meters/${M.Hausstrom}`, "chart:0"],
  ["messstelle-jahresverlauf", `/meters/${M.Hausstrom}`, "chart:1"],
  ["messstelle-ablesungen", `/meters/${M.Wasser}`, "from:Ablesungen:560"],
  ["ablesung-erfassen", `/meters/${M.Hausstrom}/readings/new`, "full"],
  ["messstelle-anlegen", "/meters/new", "full"],
  ["zaehlerwechsel", `/meters/${M.Wasser}/swap`, "full"],
  ["foto-erfassung", "/capture", "full"],
  ["verlauf", `/meters/${M.Wasser}/history`, "full"],
  ["kosten-messstelle", `/meters/${M.Hausstrom}/costs`, 1300],
  ["vertrag", `/contracts/${ids.contract}`, 1100],
  ["kostenuebersicht", "/costs?year=2026", "full"],
  ["pv-uebersicht", "/pv", "full"],
  ["pv-anlage", `/pv/${ids.pv}`, 1300],
  ["pv-hausanschluss", `/pv/grid/${M.Hausstrom}`, 1300],
  ["pv-sammelerfassung", `/pv/grid/${M.Hausstrom}/readings`, "full"],
  ["fahrzeuge", "/vehicles", "full"],
  ["fahrzeug", `/vehicles/${M["VW ID.3"]}`, 1250],
  ["fahrzeug-anschaffung", `/vehicles/${M["Skoda Octavia"]}`, "h2:Anschaffung"],
  ["fahrzeug-laden", `/vehicles/${M["VW ID.3"]}`, "h2:Ladevorgänge"],
  ["kostenbuch", `/vehicles/${M["Skoda Octavia"]}/costs?year=2025`, 1200],
  ["dokumente", "/documents", "full"],
  ["import", "/import", "full"],
  ["einstellungen", "/settings", "full"],
  ["mitglieder", "/settings/members", "full"],
  ["api-tokens", "/settings/tokens", "full"],
  ["benachrichtigungen", "/settings/notifications", "full"],
  ["setup-assistent", "/setup/vehicle", "full"],
];
const mobile = [["mobil-uebersicht", "/"], ["mobil-ablesung", `/meters/${M.Wasser}/readings/new`]];
const browser = await chromium.launch({ executablePath: process.env.CHROMIUM || undefined, args: ["--no-sandbox", "--lang=de-DE"], env: { ...process.env, LANG: "de_DE.UTF-8", LC_ALL: "de_DE.UTF-8" } });
async function shoot(list, viewport) {
  const ctx = await browser.newContext({ viewport, locale: "de-DE", deviceScaleFactor: 1 });
  const page = await ctx.newPage();
  await page.goto(base + "/auth/dev-login?name=anna");
  for (const [name, path, mode = "view"] of list) {
    const res = await page.goto(base + path, { waitUntil: "networkidle" });
    if (name === "setup-assistent") {
      await page.check('input[name="enabled"]');
      await page.selectOption("#drive_type", "electric");
      await page.selectOption("#home_meter", "wallbox");
      await page.check("#has_acquisition");
      await page.fill("#name", "VW ID.3");
      await page.fill("#model", "ID.3 Pro");
    }
    await page.waitForTimeout(900);
    const file = `${out}/${name}.png`;
    if (mode === "full") await page.screenshot({ path: file, fullPage: true });
    else if (mode === "view") await page.screenshot({ path: file });
    else if (typeof mode === "number") {
      const h = await page.evaluate(() => document.documentElement.scrollHeight);
      await page.screenshot({ path: file, fullPage: true, clip: { x: 0, y: 0, width: viewport.width, height: Math.min(mode, h) } });
    } else if (mode.startsWith("chart:")) {
      const el = page.locator(".chart").nth(Number(mode.slice(6)));
      await el.scrollIntoViewIfNeeded(); await page.waitForTimeout(500);
      await el.screenshot({ path: file });
    } else if (mode.startsWith("from:")) {
      const [, text, height] = mode.split(":");
      const box = await page.locator("h2", { hasText: text }).first().boundingBox();
      const top = box.y + (await page.evaluate(() => window.scrollY)) - 10;
      await page.screenshot({ path: file, fullPage: true, clip: { x: 0, y: top, width: viewport.width, height: Number(height) } });
    } else {
      const text = mode.slice(3);
      const heading = page.locator("h2", { hasText: text }).first();
      const section = heading.locator("xpath=ancestor::section[1]");
      const el = (await section.count()) ? section : heading.locator("xpath=..");
      await el.screenshot({ path: file });
    }
    console.log(res.status(), name, mode);
  }
  await ctx.close();
}
await shoot(mobile, { width: 390, height: 844 });
await shoot(desktop, { width: 1280, height: 900 });
await browser.close();
