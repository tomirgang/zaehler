// Ergänzungen zu HTMX; die Seiten funktionieren auch ohne JavaScript.
"use strict";

// Nach einem Austausch durch HTMX das erste Feld mit Fehler fokussieren.
document.addEventListener("htmx:afterSwap", () => {
  const field = document.querySelector(".has-error input, .has-error select");
  if (field) {
    field.focus();
  }
});

// @impl SPEC_UI_BUSY
// Ladezustand von Formularen: `aria-busy` am Formular, Spinner und
// `disabled` am auslösenden Button, bis die Antwort da ist.
const busyButtons = new WeakMap();

function submitButton(form, submitter) {
  return submitter || form.querySelector('button[type="submit"], button:not([type])');
}

function setBusy(form, button) {
  form.setAttribute("aria-busy", "true");
  if (button && !button.disabled) {
    button.setAttribute("aria-busy", "true");
    button.disabled = true;
    busyButtons.set(form, button);
  }
}

function clearBusy(form) {
  form.removeAttribute("aria-busy");
  const button = busyButtons.get(form);
  if (button) {
    button.removeAttribute("aria-busy");
    button.disabled = false;
    busyButtons.delete(form);
  }
}

function clearAllBusy() {
  document.querySelectorAll('form[aria-busy="true"]').forEach(clearBusy);
}

// Kein zweites Absenden, solange ein Request läuft (auch per Enter-Taste).
document.addEventListener(
  "submit",
  (e) => {
    if (e.target.getAttribute("aria-busy") === "true") {
      e.preventDefault();
      e.stopImmediatePropagation();
    }
  },
  true,
);

// Normales Absenden ohne HTMX. HTMX ruft `preventDefault` auf und meldet sich
// über `htmx:beforeRequest`. Deaktivieren erst nach dem Event, sonst fehlt der
// Wert des auslösenden Buttons in den Formulardaten.
document.addEventListener("submit", (e) => {
  if (e.defaultPrevented) {
    return;
  }
  const form = e.target;
  const button = submitButton(form, e.submitter);
  setTimeout(() => setBusy(form, button));
});

// Läuft nach `hx-confirm` und nach dem Einsammeln der Formularwerte.
document.addEventListener("htmx:beforeRequest", (e) => {
  const form = e.detail.elt;
  if (form instanceof HTMLFormElement) {
    setBusy(form, submitButton(form, e.detail.requestConfig.triggeringEvent?.submitter));
  }
});

// Auch bei Fehlern, Timeouts und Abbruch.
document.addEventListener("htmx:afterRequest", (e) => {
  if (e.detail.elt instanceof HTMLFormElement) {
    clearBusy(e.detail.elt);
  }
});

// Keinen Ladezustand im HTMX-Verlauf oder im Back-Forward-Cache konservieren.
document.addEventListener("htmx:beforeHistorySave", clearAllBusy);
window.addEventListener("pageshow", (e) => {
  if (e.persisted) {
    clearAllBusy();
  }
});

// @impl SPEC_A11Y
// Breite Tabellen scrollen waagerecht; nur dann sind sie per Tastatur
// fokussierbar, damit sich der Bereich mit den Pfeiltasten bewegen lässt.
function focusableScrollTables() {
  document.querySelectorAll(".table").forEach((t) => {
    if (t.scrollWidth > t.clientWidth + 1) {
      t.setAttribute("tabindex", "0");
    } else {
      t.removeAttribute("tabindex");
    }
  });
}
document.addEventListener("DOMContentLoaded", focusableScrollTables);
document.addEventListener("htmx:afterSettle", focusableScrollTables);
window.addEventListener("resize", focusableScrollTables);

// @impl SPEC_SETUP_FLOW
// Bedingte Felder: `data-show-if` mit durch `;` getrennten Bedingungen
// (`feld`, `feld=a|b`, `feld!=a|b`), ausgewertet gegen das eigene Formular
// oder das in `data-show-form` genannte. Ausgeblendete Felder werden
// deaktiviert und daher nicht gesendet; ohne JavaScript bleibt alles sichtbar.
function conditionValue(form, name) {
  const el = form.elements.namedItem(name);
  if (!el || el.disabled) {
    return "";
  }
  if (el instanceof HTMLInputElement && el.type === "checkbox") {
    return el.checked ? el.value : "";
  }
  return el.value.trim();
}

function conditionHolds(form, condition) {
  const match = /^([\w-]+)(?:(!?=)(.*))?$/.exec(condition.trim());
  if (!match) {
    return true;
  }
  const value = conditionValue(form, match[1]);
  if (!match[2]) {
    return value !== "";
  }
  const listed = match[3].split("|").includes(value);
  return match[2] === "=" ? listed : !listed;
}

function updateConditionalFields() {
  // Dokumentreihenfolge: Bedingungen verweisen auf vorangehende Felder
  document.querySelectorAll("[data-show-if]").forEach((el) => {
    const form = el.dataset.showForm
      ? document.getElementById(el.dataset.showForm)
      : el.closest("form");
    if (!form) {
      return;
    }
    const visible = el.dataset.showIf
      .split(";")
      .filter((c) => c.trim() !== "")
      .every((c) => conditionHolds(form, c));
    el.hidden = !visible;
    const controls = el.matches("input, select, textarea, button")
      ? [el]
      : el.querySelectorAll("input, select, textarea, button");
    controls.forEach((c) => {
      c.disabled = !visible;
    });
  });
}
document.addEventListener("DOMContentLoaded", updateConditionalFields);
document.addEventListener("htmx:afterSettle", updateConditionalFields);
document.addEventListener("change", updateConditionalFields);
document.addEventListener("input", (e) => {
  if (e.target.closest("form")?.querySelector("[data-show-if]")) {
    updateConditionalFields();
  }
});
// Nach „Zurück“ im Browser stellt der Cache Formularwerte wieder her.
window.addEventListener("pageshow", updateConditionalFields);

// @impl SPEC_UI_PWA
// Service Worker ohne Offline-Cache; macht die App installierbar.
if ("serviceWorker" in navigator) {
  window.addEventListener("load", () => {
    navigator.serviceWorker.register("/sw.js").catch(() => {});
  });
}

// @impl SPEC_UI_MENU
// Menü auf kleinen Bildschirmen. Die Klasse `js` blendet die Navigation erst
// aus, wenn dieses Skript läuft; ohne JavaScript bleibt sie sichtbar.
document.documentElement.classList.add("js");

function menuParts() {
  return [document.querySelector(".menu-toggle"), document.getElementById("topnav")];
}

function setMenu(open, focusToggle) {
  const [toggle, menu] = menuParts();
  if (!toggle || !menu) {
    return;
  }
  toggle.setAttribute("aria-expanded", open ? "true" : "false");
  menu.classList.toggle("open", open);
  if (!open && focusToggle) {
    toggle.focus();
  }
}

function menuOpen() {
  const [toggle] = menuParts();
  return toggle !== null && toggle.getAttribute("aria-expanded") === "true";
}

document.addEventListener("htmx:afterSwap", () => setMenu(false, false));

document.addEventListener("click", (e) => {
  if (e.target.closest(".menu-toggle")) {
    setMenu(!menuOpen(), false);
  } else if (menuOpen() && !e.target.closest("#topnav")) {
    setMenu(false, false);
  }
});

// @impl SPEC_UI_PHOTO_VIEW
// Escape schließt zuerst das Menü, sonst eine Ansicht mit `data-close`.
document.addEventListener("keydown", (e) => {
  if (e.key !== "Escape") {
    return;
  }
  if (menuOpen()) {
    setMenu(false, true);
    return;
  }
  const close = document.querySelector("[data-close]");
  if (close) {
    close.click();
  }
});
