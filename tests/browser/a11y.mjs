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

import { writeFileSync } from "node:fs";
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
  await post("/vehicles/new", {
    _csrf: token, name: "Familienauto", plate: "M-ZA 1", model: "Kombi",
    drive_type: "combustion", home_charging_meter_id: "", pv_share_pct: "",
    odometer: "10000", start_at: "2025-01-01",
  });
  await post("/pv/new", {
    _csrf: token, name: "Dach Süd", kind: "surplus", kwp: "9,84",
    commissioned_on: "2025-01-01", azimuth: "180", tilt: "30", inverter: "",
    battery_kwh: "", specific_yield: "950", degradation_pct: "0,5",
    alarm_threshold_pct: "50", grid: grid.replace("/meters/", ""),
    [`gen_${pvMeter.replace("/meters/", "")}`]: "1",
  });
  await post("/settings/tokens", { _csrf: token, name: "Home Assistant", role: "member", expires_on: "" });
  return { grid, water };
}

const { grid, water } = await seed();
const start = [
  "/", "/meters", "/meters/new", grid, `${grid}/edit`, `${grid}/readings/new`,
  `${grid}/history`, `${grid}/swap`, `${grid}/costs`,
  `${grid}/contracts/new?template=electricity`, water, "/costs", "/capture",
  "/import", "/documents", "/vehicles", "/vehicles/new", "/pv",
  "/pv/new", "/settings", "/settings/members", "/settings/notifications",
  "/settings/tokens",
];

const seen = new Set();
const queue = [...start];
const results = [];
let failed = 0;
while (queue.length > 0 && results.length < 120) {
  const path = queue.shift();
  const pattern = path.replace(UUID, ":id").replace(/\?.*$/, "");
  if (seen.has(pattern) || SKIP.some((r) => r.test(path))) continue;
  seen.add(pattern);
  const res = await page.goto(`${base}${path}`, { waitUntil: "networkidle" });
  if (!res || res.status() >= 400) {
    console.log(`${path}: HTTP ${res?.status()} (übersprungen)`);
    continue;
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
  results.push({ path, violations: scan.violations });
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

if (process.env.A11Y_REPORT) writeFileSync(process.env.A11Y_REPORT, JSON.stringify(results, null, 2));
console.log(`${results.length} Seiten geprüft, ${failed} Verstöße serious/critical`);
await browser.close();
process.exit(failed > 0 ? 1 : 0);
