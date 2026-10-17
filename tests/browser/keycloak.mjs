// Zwei Nutzer über Keycloak (WP_V1_01, SPEC_TEN_INVITES): "demo" lädt ein,
// "demo2" meldet sich über den Einladungslink an, nimmt an und sieht den
// Haushalt mit der Rolle Mitglied.
//
//   COMPOSE_HTTP_PORT=18082 ZAEHLER_SERVER__PUBLIC_URL=http://localhost:18082 \
//     docker compose -f compose.yaml -f compose.keycloak.yaml up -d --build
//   ZAEHLER_URL=http://localhost:18082 node keycloak.mjs
// @test SPEC_TEN_INVITES, SPEC_SEC_OIDC

import { chromium } from "playwright";

const base = (process.env.ZAEHLER_URL ?? "http://localhost:18082").replace(/\/$/, "");
const browser = await chromium.launch({
  executablePath: process.env.CHROMIUM || undefined,
  args: ["--no-sandbox"],
});

async function login(page, user) {
  await page.goto(`${base}/auth/login`);
  await page.click('a[href="/auth/oidc"]');
  await keycloak(page, user);
}

async function keycloak(page, user) {
  await page.waitForSelector("#username");
  await page.fill("#username", user);
  await page.fill("#password", user);
  await Promise.all([page.waitForURL((u) => u.toString().startsWith(base)), page.click("#kc-login")]);
}

function check(ok, what) {
  if (!ok) throw new Error(`fehlgeschlagen: ${what}`);
  console.log(`ok: ${what}`);
}

// Nutzer 1: Haushalt mit Messstelle, Einladung als Mitglied
const ctx1 = await browser.newContext({ locale: "de-DE" });
const anna = await ctx1.newPage();
await login(anna, "demo");
check(anna.url().startsWith(base), "demo über Keycloak angemeldet");
await anna.goto(`${base}/settings/members`);
await anna.selectOption("#invite-role", "member");
await Promise.all([anna.waitForLoadState("load"), anna.click('form[action="/settings/invitations"] button[type="submit"]')]);
const link = await anna.inputValue("#invite-link");
check(link.includes("/invite/"), "Einladungslink erzeugt");

// Nutzer 2: Einladungslink ohne Sitzung führt über die Anmeldung zurück
const ctx2 = await browser.newContext({ locale: "de-DE" });
const bea = await ctx2.newPage();
await bea.goto(link);
if (!bea.url().startsWith(base)) {
  await keycloak(bea, "demo2");
} else if (await bea.$('a[href="/auth/oidc"]')) {
  await bea.click('a[href="/auth/oidc"]');
  await keycloak(bea, "demo2");
}
check(bea.url().includes("/invite/"), "Rücksprung zur Einladung nach der Anmeldung");
await Promise.all([
  bea.waitForURL((u) => !u.toString().includes("/invite/")),
  bea.click('form[action$="/accept"] button[type="submit"]'),
]);
await bea.goto(`${base}/settings/members`);
const members = (await bea.textContent("main")) ?? "";
check(members.includes("Demo Nutzer") && members.includes("Zweite Nutzerin"), "beide Nutzer im Haushalt");
check(!(await bea.$('form[action="/settings/invitations"]')), "Mitglied ohne Owner-Funktionen");

// Nutzer 1 sieht das neue Mitglied
await anna.goto(`${base}/settings/members`);
check(((await anna.textContent("main")) ?? "").includes("Zweite Nutzerin"), "Owner sieht neues Mitglied");
await browser.close();
