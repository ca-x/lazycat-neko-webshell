import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import vm from "node:vm";
import { stripTypeScriptTypes } from "node:module";
import { getBuiltinThemeSource, listBuiltinThemeNames, parseGhosttyTheme } from "restty";

const root = new URL("../", import.meta.url);
const read = (path) => readFile(new URL(path, root), "utf8");
const hashes = {};
async function source(path) {
  const text = await read(path);
  hashes[path] = createHash("sha256").update(text).digest("hex");
  return stripTypeScriptTypes(text, { mode: "strip" });
}
function initializer(file, name) {
  const match = file.match(new RegExp(`^(?:export )?const ${name}\\s*=\\s*(\\[[\\s\\S]*?^\\]|\\{[\\s\\S]*?^\\}|[^\\n]+);`, "m"));
  assert(match, `Missing or unsupported declaration: ${name}`);
  return match[1];
}
function evaluate(node, bindings = {}) {
  return vm.runInNewContext(`(${node})`, bindings, { timeout: 1000 });
}

const config = await source("src/frontend/src/config.ts");
const inkstone = await source("src/frontend/src/inkstone-themes.ts");
const registry = await source("src/frontend/src/theme-registry.ts");
const inkstoneThemes = evaluate(initializer(inkstone, "INKSTONE_THEMES"));
const webThemes = evaluate(initializer(config, "THEMES"), { INKSTONE_THEMES: inkstoneThemes });
const prefix = evaluate(initializer(registry, "BUILTIN_THEME_PREFIX"));
const inkstoneLicense = inkstone.match(/^\/\/ License: (.+)$/m)?.[1]?.trim();
assert(inkstoneLicense, "Missing Inkstone license attribution");
const settings = initializer(config, "DEFAULT_SETTINGS");
function defaultValue(name) {
  const property = settings.match(new RegExp(`^  ${name}: ([^\\n]+),$`, "m"));
  assert(property, `Missing default ${name}`);
  return evaluate(property[1]);
}
const restty = JSON.parse(await read("node_modules/restty/package.json"));
const requested = JSON.parse(await read("package.json")).dependencies.restty;
assert.equal(restty.version, requested, "Install the pinned Restty version before exporting");
const fallbackSource = getBuiltinThemeSource("Ghostty Default Style Dark");
assert(fallbackSource, "Restty default theme is missing");
const fallback = parseGhosttyTheme(fallbackSource).colors;
function rgb(color) {
  assert(color && typeof color === "object", "Missing RGB color");
  assert(color.a === undefined || color.a === 255, "Transparent theme colors need explicit native support");
  for (const channel of [color.r, color.g, color.b]) assert(Number.isInteger(channel) && channel >= 0 && channel <= 255);
  return color.r * 65536 + color.g * 256 + color.b;
}
function terminalColor(value, foreground, background) {
  if (value === "cell-foreground") return foreground;
  if (value === "cell-background") return background;
  return rgb(value);
}
const themes = [...webThemes, ...listBuiltinThemeNames().map((name) => ({ id: `${prefix}${name}`, label: name, ghosttyName: name }))].map((theme) => {
  const ghosttySource = theme.ghosttySource ?? getBuiltinThemeSource(theme.ghosttyName ?? "");
  assert(ghosttySource, `Unknown Ghostty theme: ${theme.id}`);
  const parsed = parseGhosttyTheme(ghosttySource).colors;
  const foreground = rgb(parsed.foreground ?? fallback.foreground);
  const background = rgb(parsed.background ?? fallback.background);
  return {
    id: theme.id,
    label: theme.label,
    origin: theme.ghosttySource ? "inkstone" : "restty",
    ghostty_name: theme.ghosttyName ?? null,
    ghostty_source: ghosttySource,
    background,
    foreground,
    cursor: rgb(parsed.cursor ?? parsed.foreground ?? fallback.cursor),
    cursor_text: rgb(parsed.cursorText ?? parsed.background ?? fallback.cursorText),
    selection_background: terminalColor(parsed.selectionBackground ?? fallback.selectionBackground, foreground, background),
    selection_foreground: terminalColor(parsed.selectionForeground ?? fallback.selectionForeground, foreground, background),
    palette: Array.from({ length: 256 }, (_, index) => {
      if (parsed.palette[index]) return rgb(parsed.palette[index]);
      if (index < 16) return rgb(fallback.palette[index]);
      if (index >= 232) return (8 + (index - 232) * 10) * 0x010101;
      const offset = index - 16;
      const level = (value) => value === 0 ? 0 : 55 + value * 40;
      return level(Math.floor(offset / 36)) * 65536 + level(Math.floor(offset / 6) % 6) * 256 + level(offset % 6);
    }),
  };
});
assert.equal(new Set(themes.map((theme) => theme.id)).size, themes.length, "Duplicate theme IDs");
const catalog = {
  version: 1,
  generator: "scripts/export-native-terminal-themes.mjs",
  sources: hashes,
  restty_version: restty.version,
  defaults: { theme_id: defaultValue("themeId"), font_size: defaultValue("fontSize") },
  licenses: await Promise.all([
    ["restty", `vendor/restty/${restty.version}/LICENSE`],
    ["inkstone", `src/frontend/${inkstoneLicense}`],
  ].map(async ([id, path]) => ({ id, path, text: await read(path) }))),
  themes,
};
assert(themes.some((theme) => theme.id === catalog.defaults.theme_id));
const destination = new URL("remoted/themes/catalog.json", root);
const output = `${JSON.stringify(catalog, null, 2)}\n`;
if (process.argv.includes("--check")) {
  assert.equal(await readFile(destination, "utf8"), output, `Stale catalog; run node ${fileURLToPath(import.meta.url)}`);
} else {
  const { mkdir } = await import("node:fs/promises");
  await mkdir(new URL("remoted/themes/", root), { recursive: true });
  await writeFile(destination, output);
}
console.log(`${process.argv.includes("--check") ? "Checked" : "Exported"} ${themes.length} terminal themes`);
