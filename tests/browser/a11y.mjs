// Barrierefreiheit aller Seiten (SPEC_A11Y): legt über den
// Entwicklungs-Login Beispieldaten an, besucht jede Seite (je Routenmuster
// einmal) und prüft sie mit axe-core (WCAG 2.2 AA). Verstöße der Stufen
// `serious` und `critical` lassen den Lauf fehlschlagen.
//
//   ZAEHLER_URL=http://localhost:18080 node a11y.mjs
//
// Umgebung: ZAEHLER_URL (Basisadresse), CHROMIUM (Pfad zu einem Chromium,
// sonst der von Playwright), A11Y_REPORT (JSON-Bericht, optional).
// @test SPEC_A11Y

import { readFileSync, writeFileSync } from "node:fs";
import { chromium } from "playwright";
import { AxeBuilder } from "@axe-core/playwright";

const base = (process.env.ZAEHLER_URL ?? "http://localhost:18080").replace(/\/$/, "");
const TAGS = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"];
const FAIL = new Set(["serious", "critical"]);
const SKIP = [
  /^\/auth\//, /^\/static\//, /^\/api\//, /\.json$/, /\/photo$/, /\/file$/,
  /\/download/, /^\/settings\/export/, /^\/sw\.js$/, /^\/manifest/, /\/delete$/,
  /\/revoke$/, /\/remove$/, /^\/settings\/leave$/,
];
const UUID = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/g;

const browser = await chromium.launch({
  executablePath: process.env.CHROMIUM || undefined,
  args: ["--no-sandbox"],
});
const context = await browser.newContext({ locale: "de-DE" });
// CSP-Verstöße und Skriptfehler je Seite sammeln (SPEC_ANA_CHARTS)
await context.addInitScript(() => {
  window.__csp = [];
  document.addEventListener("securitypolicyviolation", (e) => {
    window.__csp.push(`${e.violatedDirective} ${e.blockedURI}`);
  });
});
const page = await context.newPage();
const pageErrors = [];
page.on("pageerror", (e) => pageErrors.push(String(e)));
const api = context.request;

// auf den Start des Servers warten
for (let i = 0; ; i++) {
  const ok = await api.get(`${base}/readyz`).then((r) => r.ok(), () => false);
  if (ok) break;
  if (i === 60) throw new Error(`${base}/readyz nicht erreichbar`);
  await new Promise((r) => setTimeout(r, 1000));
}

async function csrf() {
  const html = await (await api.get(`${base}/settings`)).text();
  const m = html.match(/name="_csrf" value="([^"]+)"/);
  if (!m) throw new Error("kein CSRF-Token");
  return m[1];
}

/** Formular senden, ohne Weiterleitung zu folgen; liefert `Location`. */
async function post(path, form) {
  const res = await api.post(`${base}${path}`, { form, maxRedirects: 0 });
  if (res.status() >= 400) {
    throw new Error(`${path}: HTTP ${res.status()}\n${(await res.text()).slice(0, 400)}`);
  }
  return res.headers()["location"] ?? "";
}

function meterForm(token, name, registers, extra = {}) {
  const form = {
    _csrf: token, name, kind: "electricity", unit: "", serial_number: "",
    rollover_digits: "", decimal_places: "1", schedule_kind: "every_days",
    schedule_value: "30", max_daily_rate: "", billing_month: "", billing_day: "",
    start_at: "2025-01-01T00:00", register_count: String(registers.length), ...extra,
  };
  registers.forEach(([code, direction], i) => {
    form[`register_${i}_id`] = "";
    form[`register_${i}_code`] = code;
    form[`register_${i}_direction`] = direction;
    form[`register_${i}_start`] = "0";
  });
  return form;
}

async function readings(token, meter, perMonth) {
  const html = await (await api.get(`${base}${meter}/readings/new`)).text();
  const fields = [...html.matchAll(/name="(value_[0-9a-f-]+)"/g)].map((m) => m[1]);
  for (let i = 1; i <= 21; i++) {
    const d = new Date(Date.UTC(2025, i, 1));
    const at = d.toISOString().slice(0, 10) + "T00:00";
    const form = { _csrf: token, at, note: "", confirm_warnings: "true" };
    fields.forEach((f, k) => { form[f] = String(perMonth[k % perMonth.length] * i).replace(".", ","); });
    await post(`${meter}/readings`, form);
  }
}

async function seedSetup(token) {
  const opened = await api.get(`${base}/setup`);
  if (!opened.ok()) throw new Error(`Setup: HTTP ${opened.status()}`);
  const steps = [
    ["grid", { name: "Setup Hausstrom", register: "1.8.0", start_value: "100", has_export: "true", export_start: "20" }],
    ["heat_pump", { mode: "own", name: "Setup Wärmepumpe", register: "main", start_value: "10" }],
    ["pv", {
      system_name: "Setup PV", kind: "surplus", kwp: "9.84", commissioned_on: "2025-01-01",
      generation_name: "Setup Erzeugung", generation_register: "2.8.0", generation_start: "200",
      battery_name: "Setup Speicher", battery_charge: "10", battery_discharge: "8",
    }],
    ["vehicle", {
      name: "Setup E-Auto", model: "Kombi", drive_type: "electric", odometer: "10000",
      wallbox_name: "Setup Wallbox", wallbox_start: "12",
      acquisition_date: "2025-01-01", acquisition_price: "30000",
      acquisition_mode: "mileage", acquisition_odometer: "10000",
      acquisition_lifetime_km: "220000", acquisition_end_value: "0",
    }],
    ["media", { name: "Setup Wasser", kind: "water", unit: "m³", register: "main", start_value: "25" }],
    ["contracts", {
      provider: "Setup Stadtwerke", meter: "grid", second_meter: "heat_pump", direction: "import", starts_on: "2025-01-01",
      position_name: "Strom", base_price: "120", unit_price: "0.32",
    }],
  ];
  for (const [step, fields] of steps) {
    await post(`/setup/${step}`, { _csrf: token, action: "next", enabled: "true", ...fields });
  }
  const uploaded = await api.post(`${base}/setup/photo`, {
    maxRedirects: 0,
    multipart: {
      _csrf: token, object_key: "grid", return_step: "grid",
      photo: {
        name: "zaehler.png", mimeType: "image/png",
        buffer: readFileSync(new URL("../../crates/zaehler-web/static/icon-192.png", import.meta.url)),
      },
    },
  });
  if (uploaded.status() !== 303) throw new Error(`Setupfoto: HTTP ${uploaded.status()}`);
  return steps.map(([step]) => `/setup/${step}`).concat("/setup/summary");
}

async function seed() {
  const login = await api.get(`${base}/auth/dev-login?name=a11y-${Date.now()}`, { maxRedirects: 0 });
  if (login.status() !== 303) throw new Error(`Login: HTTP ${login.status()}`);
  const token = await csrf();
  const grid = await post("/meters", meterForm(token, "Hausanschluss", [["1.8.0", "import"], ["2.8.0", "export"]]));
  const pvMeter = await post("/meters", meterForm(token, "PV Dach", [["2.8.0", "export"]]));
  const water = await post("/meters", meterForm(token, "Wasser", [["main", "import"]], { kind: "water", unit: "m³" }));
  await readings(token, grid, [250, 120]);
  await readings(token, pvMeter, [500]);
  await readings(token, water, [8]);
  const contract = {
    _csrf: token, template: "electricity", positions: "1", direction: "import",
    provider: "Stadtwerke", contract_number: "V-1", starts_on: "2025-01-01", ends_on: "",
    billing_month: "", billing_day: "", note: "", pos0_id: "", pos0_name: "Strom",
    pos0_register: "", pos0_factor: "1", pos0_vat: "19", pos0_components: "1",
    pos0_c0_id: "", pos0_c0_valid_from: "2025-01-01", pos0_c0_base: "119",
    pos0_c0_interval: "year", pos0_c0_price: "0,357", pos0_c0_tiers: "",
    pos0_c0_factor: "", action: "save",
  };
  await post(`${grid}/contracts/new`, contract);
  const car = await post("/vehicles/new", {
    _csrf: token, name: "Familienauto", plate: "M-ZA 1", model: "Kombi",
    drive_type: "combustion", home_charging_meter_id: "", pv_share_pct: "",
    odometer: "10000", start_at: "2025-01-01",
  });
  await post(`${car}/costs`, {
    _csrf: token, booked_on: "2025-03-01", category: "fuel",
    quantity: "45,32", unit_price: "1,789",
  });
  await post(`${car}/acquisition`, {
    _csrf: token, purchased_on: "2025-01-01", price: "30000",
    depreciation_mode: "mileage", odometer_at_purchase: "10000",
    lifetime_km: "220000", end_value: "0",
  });
  await post("/pv/new", {
    _csrf: token, name: "Dach Süd", kind: "surplus", kwp: "9,84",
    commissioned_on: "2025-01-01", azimuth: "180", tilt: "30", inverter: "",
    battery_kwh: "", specific_yield: "950", degradation_pct: "0,5",
    alarm_threshold_pct: "50", grid: grid.replace("/meters/", ""),
    [`gen_${pvMeter.replace("/meters/", "")}`]: "1",
  });
  await post("/settings/tokens", { _csrf: token, name: "Home Assistant", role: "member", expires_on: "" });
  const uploaded = await api.post(`${base}/documents`, {
    maxRedirects: 0,
    multipart: {
      _csrf: token, kind: "receipt",
      file: {
        name: "beleg.png", mimeType: "image/png",
        buffer: readFileSync(new URL("../../crates/zaehler-web/static/icon-192.png", import.meta.url)),
      },
    },
  });
  if (uploaded.status() !== 303) {
    throw new Error(`Fotobeleg: HTTP ${uploaded.status()}\n${(await uploaded.text()).replace(/<[^>]*>/g, " ").replace(/\s+/g, " ").slice(0, 1800)}`);
  }
  const document = uploaded.headers()["location"];
  if (!document?.startsWith("/documents/")) throw new Error("Fotobeleg ohne Weiterleitung");
  const setup = await seedSetup(token);
  return { grid, water, photo: `${document}/view?back=/documents`, setup };
}

const { grid, water, photo, setup } = await seed();
const start = [
  "/", "/meters", "/meters/new", grid, `${grid}/edit`, `${grid}/readings/new`,
  `${grid}/history`, `${grid}/swap`, `${grid}/costs`,
  `${grid}/contracts/new?template=electricity`, water, "/costs", "/capture",
  "/import", "/documents", "/vehicles", "/vehicles/new", "/pv",
  "/pv/new", "/settings", "/settings/members", "/settings/notifications",
  "/settings/tokens", "/setup", photo, ...setup,
];

const results = [];
let failed = 0;
for (const width of [360, 1280]) {
await page.setViewportSize({ width, height: 900 });
await page.goto(base, { waitUntil: "networkidle" });
const toggle = page.locator(".menu-toggle");
if (width === 360) {
  await toggle.click();
  if (await toggle.getAttribute("aria-expanded") !== "true") throw new Error("Menü öffnet nicht");
  await page.keyboard.press("Escape");
  if (await toggle.getAttribute("aria-expanded") !== "false" || !await toggle.evaluate((el) => el === document.activeElement)) {
    throw new Error("Escape schließt das Menü nicht mit Fokusrückgabe");
  }
  await toggle.press("Enter");
  if (await toggle.getAttribute("aria-expanded") !== "true") throw new Error("Menü öffnet nicht per Tastatur");
  await page.locator("main h1").click();
  if (await toggle.getAttribute("aria-expanded") !== "false") throw new Error("Klick außerhalb schließt Menü nicht");
  await toggle.click();
  await page.locator('#topnav a[href="/meters"]').click();
  await page.waitForURL(`${base}/meters`);
  if (await toggle.getAttribute("aria-expanded") !== "false") throw new Error("Menü bleibt nach Navigation offen");
} else if (await toggle.isVisible()) {
  throw new Error("Menüschaltfläche auf großem Bildschirm sichtbar");
}
await page.goto(`${base}${photo}`, { waitUntil: "networkidle" });
await page.keyboard.press("Escape");
await page.waitForURL(`${base}/documents`);
const seen = new Set();
const queue = [...start];
let pageCount = 0;
while (queue.length > 0 && pageCount < 120) {
  const path = queue.shift();
  const pattern = path.replace(UUID, ":id").replace(/\?.*$/, "");
  if (seen.has(pattern) || SKIP.some((r) => r.test(path))) continue;
  seen.add(pattern);
  pageCount += 1;
  const res = await page.goto(`${base}${path}`, { waitUntil: "networkidle" });
  if (!res || res.status() >= 400) {
    console.log(`FEHLER ${width}px ${path}: HTTP ${res?.status()}`);
    failed += 1;
    continue;
  }
  if (path !== "/") {
    const breadcrumb = page.locator("nav.breadcrumb");
    const valid = await breadcrumb.count() === 1 && await breadcrumb.evaluate((nav) => {
      const items = Array.from(nav.querySelectorAll("ol > li"));
      const last = items.at(-1);
      return items[0]?.querySelector("a")?.getAttribute("href") === "/"
        && last?.getAttribute("aria-current") === "page" && !last.querySelector("a");
    });
    if (!valid) throw new Error(`Breadcrumb fehlt oder ist ungültig: ${path}`);
    if (width === 360) {
      const visible = await breadcrumb.locator("li").evaluateAll((items) =>
        items.filter((item) => getComputedStyle(item).display !== "none").length);
      if (visible !== 1) throw new Error(`Mobile Breadcrumb zeigt nicht genau den Rücksprung: ${path}`);
    }
  }
  // Diagramme außerhalb geschlossener <details> sind gezeichnet
  const charts = await page.$$eval(".chart[data-chart]", (nodes) =>
    nodes
      .filter((n) => !n.closest("details:not([open])"))
      .map((n) => ({ chart: n.dataset.chart, drawn: !!n.querySelector("canvas, svg") })),
  );
  const csp = await page.evaluate(() => window.__csp);
  const undrawn = charts.filter((c) => !c.drawn).map((c) => c.chart);
  if (csp.length > 0 || pageErrors.length > 0 || undrawn.length > 0) {
    console.log(`FEHLER ${path}: CSP ${JSON.stringify(csp)}, Skript ${JSON.stringify(pageErrors)}, nicht gezeichnet ${JSON.stringify(undrawn)}`);
    failed += 1;
  } else if (charts.length > 0) {
    console.log(`Diagramme ${path}: ${charts.map((c) => c.chart).join(", ")}`);
  }
  pageErrors.length = 0;
  const scan = await new AxeBuilder({ page }).withTags(TAGS).analyze();
  const bad = scan.violations.filter((v) => FAIL.has(v.impact));
  results.push({ path, width, violations: scan.violations });
  if (width === 360 && await page.evaluate(() => document.documentElement.scrollWidth > innerWidth + 1)) {
    console.log(`FEHLER ${path}: horizontaler Seiten-Overflow bei 360px`);
    console.log(await page.evaluate(() => Array.from(document.querySelectorAll("main *"))
      .filter((el) => el.getBoundingClientRect().right > innerWidth + 1 && !el.closest(".table"))
      .slice(0, 8).map((el) => `${el.tagName}.${el.className}: ${Math.round(el.getBoundingClientRect().right)}`).join(", ")));
    failed += 1;
  }
  for (const v of scan.violations) {
    const mark = FAIL.has(v.impact) ? "FEHLER" : "Hinweis";
    console.log(`${mark} ${path} [${v.impact}] ${v.id}: ${v.help}`);
    for (const n of v.nodes.slice(0, 3)) console.log(`    ${n.target.join(" ")}`);
  }
  failed += bad.length;
  console.log(`geprüft ${path}: ${scan.violations.length} Befunde, davon ${bad.length} serious/critical`);
  // weitere Seiten über Links derselben Herkunft
  const links = await page.$$eval("a[href]", (as) => as.map((a) => a.getAttribute("href")));
  for (const href of links) {
    if (href && href.startsWith("/") && !href.startsWith("//")) queue.push(href.split("#")[0]);
  }
}
}

const noJs = await browser.newContext({ javaScriptEnabled: false, viewport: { width: 360, height: 900 }, storageState: await context.storageState() });
const noJsPage = await noJs.newPage();
await noJsPage.goto(base);
if (!await noJsPage.locator("#topnav").isVisible()) throw new Error("Navigation ohne JavaScript unsichtbar");
await noJs.close();

const completed = await post("/setup/complete", { _csrf: await csrf() });
if (completed !== "/") throw new Error(`Setup nicht abgeschlossen: ${completed}`);
const dashboard = await (await api.get(base)).text();
for (const name of ["Setup Hausstrom", "Setup Wärmepumpe", "Setup E-Auto", "Setup Wasser"]) {
  if (!dashboard.includes(name)) throw new Error(`Setup-Objekt fehlt nach Abschluss: ${name}`);
}
const pvList = await (await api.get(`${base}/pv`)).text();
if (!pvList.includes("Setup PV")) throw new Error("Setup-PV fehlt nach Abschluss");

if (process.env.A11Y_REPORT) writeFileSync(process.env.A11Y_REPORT, JSON.stringify(results, null, 2));
console.log(`${results.length} Seiten geprüft, ${failed} Verstöße serious/critical`);
await browser.close();
process.exit(failed > 0 ? 1 : 0);
