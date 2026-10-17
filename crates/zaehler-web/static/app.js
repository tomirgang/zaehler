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

// @impl SPEC_UI_PWA
// Service Worker ohne Offline-Cache; macht die App installierbar.
if ("serviceWorker" in navigator) {
  window.addEventListener("load", () => {
    navigator.serviceWorker.register("/sw.js").catch(() => {});
  });
}
