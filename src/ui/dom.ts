type Child = Node | string | number | null | undefined | false;

interface Props {
  class?: string;
  text?: string;
  attrs?: Record<string, string | number | boolean | undefined>;
  on?: { [K in keyof HTMLElementEventMap]?: (event: HTMLElementEventMap[K]) => void };
  style?: Record<string, string>;
}

/**
 * Builds an element. Text always goes in as a text node, never as HTML, because
 * titles and error messages come from remote sites.
 */
export function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  props: Props = {},
  ...children: Child[]
): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  if (props.class) el.className = props.class;
  if (props.text !== undefined) el.textContent = props.text;
  if (props.attrs) {
    for (const [key, value] of Object.entries(props.attrs)) {
      const text = attributeText(key, value);
      if (text !== null) el.setAttribute(key, text);
    }
  }
  if (props.style) {
    for (const [key, value] of Object.entries(props.style)) el.style.setProperty(key, value);
  }
  if (props.on) {
    for (const [name, handler] of Object.entries(props.on)) {
      el.addEventListener(name, handler as EventListener);
    }
  }
  append(el, children);
  return el;
}

/**
 * The text h() writes for an attribute, or null to leave it out. `draggable` is an
 * enumerated attribute, not a boolean one: leaving it out keeps the browser's default
 * (images and links drag), so `draggable: false` must be written as "false".
 */
export function attributeText(key: string, value: string | number | boolean | undefined): string | null {
  if (value === undefined) return null;
  if (key === "draggable" && typeof value === "boolean") return String(value);
  if (value === false) return null;
  return value === true ? "" : String(value);
}

export function append(parent: Node, children: Child[]) {
  for (const child of children) {
    if (child === null || child === undefined || child === false) continue;
    parent.appendChild(typeof child === "object" ? child : document.createTextNode(String(child)));
  }
}

export function clear(el: Element) {
  while (el.firstChild) el.removeChild(el.firstChild);
}

export function replace(el: Element, ...children: Child[]) {
  clear(el);
  append(el, children);
}
