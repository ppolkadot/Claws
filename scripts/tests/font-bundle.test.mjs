import assert from "node:assert/strict";
import { readFileSync, cpSync, mkdtempSync, rmSync, writeFileSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { verifyFonts } from "../check-fonts.mjs";
import { test } from "node:test";

const root = new URL("../../", import.meta.url);
const read = (path) => readFileSync(new URL(path, root), "utf8");

test("runtime faces use fingerprinted bundled assets without installed-font substitution", () => {
  const css = read("src/styles/fonts.css");
  assert.doesNotMatch(css, /local\s*\(/i);
  for (const weight of [400, 500, 600, 700]) {
    assert.ok(css.includes(`../assets/fonts/salted-flutters-polaris-${weight}.woff2`));
  }
  assert.ok(read("src/styles/base.css").includes('--font: "Claws Polaris"'));
  assert.ok(read("src/main.ts").includes('13px "Claws Polaris"'));
});

test("normal builds verify the source fonts and the final distribution", () => {
  const { scripts } = JSON.parse(read("package.json"));
  assert.match(scripts.build, /^node scripts\/check-fonts\.mjs &&/);
  assert.match(scripts.build, /vite build && node scripts\/check-fonts\.mjs --dist$/);
});

function fixture(t) {
  const dir = mkdtempSync(join(tmpdir(), "claws-font-guard-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  for (const path of ["src/assets/fonts", "src/styles/fonts.css", "src/styles/base.css", "src/main.ts",
    "scripts/font-integrity.json", "assets/fonts/polaris/LICENSE.txt", "public/fonts/LICENSE.txt"]) {
    cpSync(new URL(path, root), join(dir, path), { recursive: true });
  }
  return dir;
}

test("ordinary build rejects a replaced or corrupt font before packaging", (t) => {
  const dir = fixture(t);
  verifyFonts(dir);
  writeFileSync(join(dir, "src/assets/fonts/salted-flutters-polaris-700.woff2"), "unrepaired replacement");
  assert.throws(() => verifyFonts(dir), /Unapproved Polaris 700 bytes/);
});

test("ordinary build rejects the installed-font fallback", (t) => {
  const dir = fixture(t);
  const path = join(dir, "src/styles/fonts.css");
  writeFileSync(path, readFileSync(path, "utf8").replace('format("woff2")', 'format("woff2"), local("Salted Flutters Polaris")'));
  assert.throws(() => verifyFonts(dir), /Installed fonts must not replace/);
});

test("packaged font bytes and fingerprinted CSS are both verified", (t) => {
  const dir = fixture(t);
  mkdirSync(join(dir, "dist/assets"), { recursive: true });
  cpSync(join(dir, "public/fonts"), join(dir, "dist/fonts"), { recursive: true });
  let css = readFileSync(join(dir, "src/styles/fonts.css"), "utf8") + readFileSync(join(dir, "src/styles/base.css"), "utf8");
  for (const weight of [400, 500, 600, 700]) {
    const filename = `salted-flutters-polaris-${weight}-testhash.woff2`;
    cpSync(join(dir, `src/assets/fonts/salted-flutters-polaris-${weight}.woff2`), join(dir, "dist/assets", filename));
    css = css.replace(`../assets/fonts/salted-flutters-polaris-${weight}.woff2`, `/assets/${filename}`);
  }
  writeFileSync(join(dir, "dist/assets/index.css"), css);
  verifyFonts(dir, true);
  writeFileSync(join(dir, "dist/assets/salted-flutters-polaris-400-testhash.woff2"), "stale packaged font");
  assert.throws(() => verifyFonts(dir, true), /Unapproved Polaris 400 bytes/);
});

test("packaged checks reject orphaned and legacy font files", (t) => {
  const dir = fixture(t);
  mkdirSync(join(dir, "dist/assets"), { recursive: true });
  cpSync(join(dir, "public/fonts"), join(dir, "dist/fonts"), { recursive: true });
  let css = readFileSync(join(dir, "src/styles/fonts.css"), "utf8") + readFileSync(join(dir, "src/styles/base.css"), "utf8");
  for (const weight of [400, 500, 600, 700]) {
    const filename = `salted-flutters-polaris-${weight}-testhash.woff2`;
    cpSync(join(dir, `src/assets/fonts/salted-flutters-polaris-${weight}.woff2`), join(dir, "dist/assets", filename));
    css = css.replace(`../assets/fonts/salted-flutters-polaris-${weight}.woff2`, `/assets/${filename}`);
  }
  writeFileSync(join(dir, "dist/assets/index.css"), css);
  verifyFonts(dir, true);
  writeFileSync(join(dir, "dist/assets/salted-flutters-polaris-400-oldhash.woff2"), "orphan");
  assert.throws(() => verifyFonts(dir, true), /Stale packaged Polaris files/);
  rmSync(join(dir, "dist/assets/salted-flutters-polaris-400-oldhash.woff2"));
  writeFileSync(join(dir, "dist/fonts/salted-flutters-polaris-400.woff2"), "legacy");
  assert.throws(() => verifyFonts(dir, true), /Legacy unfingerprinted fonts/);
});

test("the command-line check runs through a junction or symlinked path", async (t) => {
  const { symlinkSync } = await import("node:fs");
  const { spawnSync } = await import("node:child_process");
  const { fileURLToPath } = await import("node:url");
  const dir = mkdtempSync(join(tmpdir(), "claws-font-link-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const link = join(dir, "repo");
  symlinkSync(fileURLToPath(root), link, "junction");
  const run = spawnSync(process.execPath, [join(link, "scripts", "check-fonts.mjs")], { encoding: "utf8" });
  assert.equal(run.status, 0, run.stderr);
  assert.match(run.stdout, /Polaris font integrity verified/);
});
