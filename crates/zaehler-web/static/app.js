// Ergänzungen zu HTMX; die Seiten funktionieren auch ohne JavaScript.
"use strict";

// Nach einem Austausch durch HTMX das erste Feld mit Fehler fokussieren.
document.addEventListener("htmx:afterSwap", () => {
  const field = document.querySelector(".has-error input, .has-error select");
  if (field) {
    field.focus();
  }
});
