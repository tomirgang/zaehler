// Prüft alle Mermaid-Blöcke der Doku mit der Mermaid-CLI (mmdc).
// Mermaid rendert erst im Browser; ohne diese Prüfung fallen Syntaxfehler im
// Sphinx-Build nicht auf. Aufruf im Image ghcr.io/mermaid-js/mermaid-cli:
//   MMDC=/home/mermaidcli/node_modules/.bin/mmdc MMDC_ARGS="-p /puppeteer-config.json" \
//     node docs/tools/check-mermaid.mjs docs
import { mkdtempSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, relative } from "node:path";
import { spawnSync } from "node:child_process";

const root = process.argv[2] ?? "docs";
const mmdc = process.env.MMDC ?? "mmdc";
const extraArgs = (process.env.MMDC_ARGS ?? "").split(" ").filter(Boolean);
const skip = new Set(["_build", ".venv", "node_modules"]);

function* markdownFiles(dir) {
  for (const entry of readdirSync(dir)) {
    if (skip.has(entry)) continue;
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) yield* markdownFiles(path);
    else if (path.endsWith(".md")) yield path;
  }
}

const tmp = mkdtempSync(join(tmpdir(), "mermaid-"));
let count = 0;
let failed = 0;
for (const file of markdownFiles(root)) {
  const text = readFileSync(file, "utf8");
  for (const match of text.matchAll(/```\{mermaid\}\n([\s\S]*?)```/g)) {
    count += 1;
    const line = text.slice(0, match.index).split("\n").length;
    const input = join(tmp, `d${count}.mmd`);
    writeFileSync(input, match[1]);
    const result = spawnSync(mmdc, [...extraArgs, "-i", input, "-o", join(tmp, `d${count}.svg`)], { encoding: "utf8" });
    const location = `${relative(".", file)}:${line}`;
    if (result.status === 0) {
      console.log(`OK      ${location}`);
    } else {
      failed += 1;
      const detail = (result.stderr || result.stdout || "").split("\n").filter((l) => /error|expect/i.test(l)).slice(0, 3);
      console.error(`FEHLER  ${location}\n        ${detail.join("\n        ")}`);
    }
  }
}
console.log(`${count} Diagramme geprüft, ${failed} fehlerhaft`);
process.exit(failed > 0 ? 1 : 0);
