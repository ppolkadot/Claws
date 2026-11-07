// No Python dependency for ordinary builds. Only the generator can approve new bytes,
// after checking serialized glyph advances; Vite must package those same bytes.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const digest = (path) => createHash("sha256").update(readFileSync(path)).digest("hex");

export function verifyFonts(root = repo, distribution = false) {
  const approved = JSON.parse(readFileSync(join(root, "scripts/font-integrity.json"), "utf8"));
  const css = distribution
    ? readdirSync(join(root, "dist/assets")).filter((file) => file.endsWith(".css"))
      .map((file) => readFileSync(join(root, "dist/assets", file), "utf8")).join("\n")
    : readFileSync(join(root, "src/styles/fonts.css"), "utf8");
  const faces = css.match(/@font-face\s*\{[^}]+\}/g) ?? [];
  assert.equal(faces.length, 4, "Expected four bundled Polaris faces");
  const base = distribution ? css : readFileSync(join(root, "src/styles/base.css"), "utf8");
  assert.match(base, /--font:\s*["']?Claws Polaris["']?\s*,/, "Interface must use the private bundled face");
  if (!distribution) {
    assert.ok(readFileSync(join(root, "src/main.ts"), "utf8").includes('13px "Claws Polaris"'), "Startup must preload the bundled face");
  }
  for (const weight of [400, 500, 600, 700]) {
    const face = faces.find((text) => new RegExp(`font-weight:\\s*${weight}\\s*[;}]`).test(text));
    assert.match(face ?? "", /font-family:\s*["']?Claws Polaris["']?\s*;/, `Missing private font face ${weight}`);
    assert.doesNotMatch(face, /local\s*\(/i, "Installed fonts must not replace bundled Polaris");
    let path;
    if (distribution) {
      const filename = face.match(new RegExp(`/assets/(salted-flutters-polaris-${weight}-[\\w-]+\\.woff2)`))?.[1];
      assert.ok(filename, `Missing fingerprinted output for ${weight}`);
      path = join(root, "dist/assets", filename);
    } else {
      const filename = `salted-flutters-polaris-${weight}.woff2`;
      assert.ok(face.includes(`../assets/fonts/${filename}`), `Font ${weight} must be a Vite import`);
      path = join(root, "src/assets/fonts", filename);
    }
    assert.equal(digest(path), approved[weight], `Unapproved Polaris ${weight} bytes; regenerate with python scripts/build-fonts.py`);
  }
  if (distribution) {
    // Only the four referenced, fingerprinted faces may ship: no orphaned or legacy copies.
    const referenced = new Set(faces.map((text) => text.match(/\/assets\/(salted-flutters-polaris-[^)"']+\.woff2)/)?.[1]));
    const packaged = readdirSync(join(root, "dist/assets")).filter((file) => /^salted-flutters-polaris-.*\.woff2$/.test(file));
    const stale = packaged.filter((file) => !referenced.has(file));
    assert.deepEqual(stale, [], "Stale packaged Polaris files in dist/assets");
    const legacy = existsSync(join(root, "dist/fonts")) ? readdirSync(join(root, "dist/fonts")).filter((file) => file.endsWith(".woff2")) : [];
    assert.deepEqual(legacy, [], "Legacy unfingerprinted fonts in dist/fonts");
  }
  const sourceLicense = digest(join(root, "assets/fonts/polaris/LICENSE.txt"));
  assert.equal(digest(join(root, distribution ? "dist/fonts/LICENSE.txt" : "public/fonts/LICENSE.txt")), sourceLicense, "Font license must remain unmodified");
}

// import.meta.main survives junctions and symlinks; comparing argv paths silently skipped the check.
if (import.meta.main) {
  verifyFonts(repo, process.argv.includes("--dist"));
  console.log("Polaris font integrity verified" + (process.argv.includes("--dist") ? " in the production bundle" : ""));
}
