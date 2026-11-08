// Tests for attributeText in src/ui/dom.ts (compiled by "test:motion" in package.json).
import assert from "node:assert/strict";
import { test } from "node:test";

import { attributeText } from "../../.test-build/ui/dom.js";

test("draggable: false is written out, so thumbnails never export their image URL", () => {
  // Leaving the attribute out would keep the browser default: images are draggable.
  assert.equal(attributeText("draggable", false), "false");
  assert.equal(attributeText("draggable", true), "true");
  assert.equal(attributeText("draggable", "false"), "false");
});

test("ordinary boolean attributes keep their meaning", () => {
  assert.equal(attributeText("disabled", true), "");
  assert.equal(attributeText("disabled", false), null);
  assert.equal(attributeText("hidden", undefined), null);
  assert.equal(attributeText("draggable", undefined), null);
  assert.equal(attributeText("tabindex", 0), "0");
  assert.equal(attributeText("alt", ""), "");
});
