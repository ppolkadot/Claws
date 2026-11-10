// A just-big-enough fake DOM for the component tests (not a test file itself).
//
// It models only what src/ui/components.ts and src/ui/dom.ts touch: elements with
// attributes, children, listeners and focus, plus the <dialog> and Web Animations
// behavior the confirmation sheet depends on.

class FakeEvent {
  constructor(type, init = {}) {
    this.type = type;
    this.defaultPrevented = false;
    this.target = null;
    Object.assign(this, init);
  }
  preventDefault() {
    this.defaultPrevented = true;
  }
  stopPropagation() {
    this.propagationStopped = true;
  }
}

class FakeText {
  constructor(text) {
    this.textContent = String(text);
    this.parentNode = null;
  }
}

class FakeElement {
  constructor(tagName, document) {
    this.tagName = tagName.toUpperCase();
    this.ownerDocument = document;
    this.children = [];
    this.parentNode = null;
    this.attributes = new Map();
    this.listeners = new Map();
    this.className = "";
    this.tabIndex = this.tagName === "BUTTON" ? 0 : -1;
    this.disabled = false;
    this.autofocus = false;
    this.style = { setProperty() {} };
    this.classList = { add() {}, remove() {} };
    this._text = "";
    if (this.tagName === "DIALOG") this.open = false;
  }

  get textContent() {
    return this._text + this.children.map((child) => child.textContent).join("");
  }
  set textContent(text) {
    this.children = [];
    this._text = String(text);
  }

  setAttribute(name, value) {
    this.attributes.set(name, String(value));
  }
  getAttribute(name) {
    return this.attributes.has(name) ? this.attributes.get(name) : null;
  }

  appendChild(child) {
    child.parentNode = this;
    this.children.push(child);
    return child;
  }
  prepend(...nodes) {
    for (const node of nodes) {
      node.parentNode = this;
    }
    this.children.unshift(...nodes);
  }
  removeChild(child) {
    this.children = this.children.filter((c) => c !== child);
    child.parentNode = null;
    return child;
  }
  /** A node that is moved or re-added loses focus, as in a page; one left where it is does not. */
  insertBefore(child, before) {
    this._blurIfFocused(child);
    child.parentNode?.removeChild(child);
    child.parentNode = this;
    const at = before ? this.children.indexOf(before) : -1;
    if (at < 0) this.children.push(child);
    else this.children.splice(at, 0, child);
    return child;
  }
  replaceChildren(...nodes) {
    for (const child of this.children) {
      this._blurIfFocused(child);
      child.parentNode = null;
    }
    this.children = [];
    for (const node of nodes) this.appendChild(node);
  }
  _blurIfFocused(node) {
    if (this.ownerDocument.activeElement === node) this.ownerDocument.activeElement = null;
  }
  get firstChild() {
    return this.children[0] ?? null;
  }
  contains(node) {
    for (let current = node; current; current = current.parentNode) if (current === this) return true;
    return false;
  }
  remove() {
    if (this.parentNode) this.parentNode.removeChild(this);
  }

  addEventListener(type, handler, capture = false) {
    if (!this.listeners.has(type)) this.listeners.set(type, []);
    this.listeners.get(type).push({ handler, capture: capture === true });
  }
  dispatchEvent(event) {
    event.target ??= this;
    const ancestors = [];
    for (let node = this.parentNode; node && node.listeners; node = node.parentNode) ancestors.push(node);
    const run = (node, phase) => {
      for (const { handler, capture } of [...(node.listeners.get(event.type) ?? [])]) {
        if (phase === "capture" ? capture : !capture) handler(event);
      }
    };
    // Like a page: capture listeners from the top down, then the target, then (bubbling:
    // keydown and click reach the ancestors) back up. stopPropagation ends the walk.
    const bubbles = event.bubbles !== false;
    for (const node of bubbles ? [...ancestors].reverse() : []) {
      if (event.propagationStopped) break;
      run(node, "capture");
    }
    if (!event.propagationStopped) {
      run(this, "capture");
      run(this, "bubble");
    }
    for (const node of bubbles ? ancestors : []) {
      if (event.propagationStopped) break;
      run(node, "bubble");
    }
    return !event.defaultPrevented;
  }

  // Just enough selector for ".class": the subtitle chips look their chips up this way.
  matches(selector) {
    return this.className.split(/\s+/).includes(selector.slice(1));
  }
  closest(selector) {
    for (let node = this; node && node.matches; node = node.parentNode) if (node.matches(selector)) return node;
    return null;
  }
  querySelectorAll(selector) {
    return findAllIn(this, (n) => n.matches(selector));
  }

  /** A real click: a disabled button never fires one. */
  click() {
    if (this.disabled) return;
    this.dispatchEvent(new FakeEvent("click"));
  }
  focus() {
    this.ownerDocument.activeElement = this;
  }

  // <dialog>
  showModal() {
    this.open = true;
  }
  close() {
    if (!this.open) return;
    this.open = false;
    this.dispatchEvent(new FakeEvent("close", { bubbles: false }));
  }
  /** Web Animations: the test decides when it finishes or is cancelled. */
  animate(frames, options) {
    const animation = {
      frames,
      options,
      onfinish: null,
      oncancel: null,
      finish() {
        this.onfinish?.();
      },
      cancel() {
        // A cancelled animation fires "cancel", never "finish".
        this.oncancel?.();
      },
    };
    this.animations.push(animation);
    return animation;
  }
  get animations() {
    return (this._animations ??= []);
  }
}

function findAllIn(root, predicate) {
  const found = [];
  for (const child of root.children) {
    if (child.matches && predicate(child)) found.push(child);
    if (child.children) found.push(...findAllIn(child, predicate));
  }
  return found;
}

/** Installs fake `document`, `window` and `matchMedia` globals; returns the document. */
export function installFakeDom({ reducedMotion = false, hidden = false, fakeTimers = false } = {}) {
  const document = {
    hidden,
    activeElement: null,
    createElement: (tag) => new FakeElement(tag, document),
    createElementNS: (_ns, tag) => new FakeElement(tag, document),
    createTextNode: (text) => new FakeText(text),
  };
  document.body = new FakeElement("body", document);
  globalThis.document = document;
  if (fakeTimers) {
    // Timers only run when the test calls document.runTimers(), so waits are deterministic.
    const timers = new Map();
    let nextId = 1;
    document.timers = timers;
    document.runTimers = () => {
      for (const [id, timer] of [...timers]) {
        timers.delete(id);
        timer.fn();
      }
    };
    globalThis.window = {
      setTimeout: (fn, ms) => {
        const id = nextId++;
        timers.set(id, { fn, ms });
        return id;
      },
      clearTimeout: (id) => timers.delete(id),
    };
  } else {
    globalThis.window = {
      setTimeout: globalThis.setTimeout.bind(globalThis),
      clearTimeout: globalThis.clearTimeout.bind(globalThis),
    };
  }
  globalThis.matchMedia = () => ({ matches: reducedMotion });
  return document;
}

export function keydown(key, init = {}) {
  return new FakeEvent("keydown", { key, ...init });
}

export function walk(node, visit) {
  visit(node);
  for (const child of node.children ?? []) walk(child, visit);
}

export function findAll(node, predicate) {
  const found = [];
  walk(node, (n) => {
    if (n.tagName && predicate(n)) found.push(n);
  });
  return found;
}

export { FakeEvent };
