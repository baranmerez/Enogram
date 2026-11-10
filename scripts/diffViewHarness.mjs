import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import vm from "node:vm";
import ts from "typescript";
import * as diffLines from "../src/diffLines.ts";
import * as diffSearch from "../src/diffSearch.ts";

const compiled = ts.transpileModule(await readFile(new URL("../src/DiffView.tsx", import.meta.url), "utf8"), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX, target: ts.ScriptTarget.ES2022 },
}).outputText;
const sameDependencies = (previous, next) => previous && next && previous.length === next.length
  && previous.every((value, index) => Object.is(value, next[index]));

function* elements(value) {
  if (Array.isArray(value)) {
    for (const child of value) yield* elements(child);
  } else if (value && typeof value === "object" && "props" in value) {
    yield value;
    yield* elements(value.props.children);
  }
}

export function textContent(value) {
  if (Array.isArray(value)) return value.map(textContent).join("");
  if (value && typeof value === "object") return textContent(value.props?.children);
  return value === null || value === undefined || typeof value === "boolean" ? "" : String(value);
}

// Exercise the real view's handlers, keyed lifecycle, refs and scroll requests.
// DOM surfaces are deterministic doubles; this does not prove native layout.
export function diffViewHarness(initialProps) {
  const instances = new Map();
  const surfaces = new Map();
  const scrolls = [];
  const outerScroll = { scrollTop: 11, scrollLeft: 17 };
  let props = initialProps;
  let current;
  let cursor = 0;
  let nextId = 0;
  let dirty = true;
  let effects = [];
  let tree;
  let activeElement = null;

  const react = {
    useState(initial) {
      const index = cursor++;
      const instance = current;
      if (!instance.hooks[index]) {
        const entry = { value: typeof initial === "function" ? initial() : initial };
        entry.set = (update) => {
          if (!instance.mounted) return;
          const next = typeof update === "function" ? update(entry.value) : update;
          if (!Object.is(next, entry.value)) { entry.value = next; dirty = true; }
        };
        instance.hooks[index] = entry;
      }
      return [instance.hooks[index].value, instance.hooks[index].set];
    },
    useRef(initial) {
      const index = cursor++;
      current.hooks[index] ??= { current: initial };
      return current.hooks[index];
    },
    useId() { return react.useRef(`diff-test-${++nextId}`).current; },
    useMemo(factory, dependencies) {
      const index = cursor++;
      if (!current.hooks[index] || !sameDependencies(current.hooks[index].dependencies, dependencies)) {
        current.hooks[index] = { value: factory(), dependencies };
      }
      return current.hooks[index].value;
    },
    useEffect(effect, dependencies) {
      const index = cursor++;
      const previous = current.hooks[index];
      if (previous && sameDependencies(previous.dependencies, dependencies)) return;
      const entry = { dependencies, cleanup: previous?.cleanup };
      current.hooks[index] = entry;
      current.effects.push(() => { entry.cleanup?.(); entry.cleanup = effect(); });
    },
  };
  const jsx = (type, elementProps, key) => ({ type, props: elementProps ?? {}, key });
  const modules = {
    react,
    "react/jsx-runtime": { jsx, jsxs: jsx, Fragment: "Fragment" },
    "lucide-react": new Proxy({}, { get: (_target, name) => `icon:${String(name)}` }),
    "./diffLines": diffLines,
    "./diffSearch": diffSearch,
    "./diffView.css": {},
  };
  const exports = {};
  vm.runInNewContext(compiled, {
    exports,
    require(name) {
      assert.ok(Object.hasOwn(modules, name), `Unmapped DiffView import: ${name}`);
      return modules[name];
    },
  }, { filename: "DiffView.tsx" });

  function detach(instance) {
    instance.mounted = false;
    for (const hook of instance.hooks) hook.cleanup?.();
    for (const ref of instance.refs) ref.current = null;
  }

  function renderElement(element, path, owner) {
    if (Array.isArray(element)) return element.map((child, index) => renderElement(child, `${path}/${index}`, owner));
    if (!element || typeof element !== "object" || !("props" in element)) return element;
    if (typeof element.type === "function") {
      const id = `${path}:${element.type.name}:${element.key ?? ""}`;
      let instance = instances.get(id);
      if (!instance) {
        instance = { hooks: [], refs: new Set(), mounted: true };
        instances.set(id, instance);
      }
      instance.seen = true;
      instance.effects = [];
      current = instance;
      cursor = 0;
      const output = renderElement(element.type(element.props), `${id}/render`, instance);
      effects.push(...instance.effects);
      return output;
    }
    let surface = surfaces.get(path);
    if (!surface) {
      surface = {
        focusCount: 0, selectCount: 0,
        scrollTop: 0, scrollLeft: 0, clientTop: 0, clientLeft: 0,
        clientWidth: 320, clientHeight: 300, scrollWidth: 2_000, scrollHeight: 5_000,
        focus(options) { activeElement = this; this.focusCount += 1; this.focusOptions = options; },
        select() { this.selectCount += 1; },
        getBoundingClientRect() {
          return this.rect?.() ?? { top: 0, left: 0, width: 320, height: 23, right: 320, bottom: 23 };
        },
        scrollTo(options) {
          this.scrollTop = options.top;
          this.scrollLeft = options.left;
          scrolls.push({ props: this.node.props, options, scope: "element" });
        },
        scrollIntoView(options) {
          // Model the native regression: this API also scrolls hidden ancestors.
          outerScroll.scrollTop += 175;
          outerScroll.scrollLeft += 421;
          scrolls.push({ props: this.node.props, options, scope: "ancestors" });
        },
        querySelector(selector) {
          const match = /^\[([^=]+)="(\d+)"\]$/.exec(selector);
          assert.ok(match, `Unsupported fixture selector: ${selector}`);
          return [...elements(this.node)].find((node) => String(node.props[match[1]]) === match[2])?.surface ?? null;
        },
      };
      surfaces.set(path, surface);
    }
    const rendered = { ...element, surface, props: { ...element.props, children: renderElement(element.props.children, `${path}/children`, owner) } };
    surface.node = rendered;
    const ref = element.props.ref;
    if (ref && typeof ref === "object") { ref.current = surface; owner.refs.add(ref); }
    return rendered;
  }

  function render() {
    let renders = 0;
    while (dirty) {
      assert.ok(++renders <= 30, "DiffView did not settle after effects");
      dirty = false;
      effects = [];
      for (const instance of instances.values()) instance.seen = false;
      tree = renderElement(jsx(exports.DiffView, props), "root", null);
      for (const [id, instance] of instances) {
        if (!instance.seen) { detach(instance); instances.delete(id); }
      }
      for (const effect of effects) effect();
    }
  }

  return {
    scrolls,
    outerScroll,
    get activeElement() { return activeElement; },
    setProps(change) { props = { ...props, ...change }; dirty = true; },
    flush() { render(); },
    all(predicate, within = tree) { return [...elements(within)].filter(predicate); },
    find(predicate, within = tree) {
      const matches = this.all(predicate, within);
      assert.equal(matches.length, 1, `Expected one element; found ${matches.length}`);
      return matches[0];
    },
    unmount() { for (const instance of instances.values()) detach(instance); instances.clear(); },
  };
}
