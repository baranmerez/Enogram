import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import vm from "node:vm";
import ts from "typescript";
import * as repositoryActionModel from "../src/repositoryActionModel.ts";
import * as repositoryIdentity from "../src/repositoryIdentity.ts";
import * as i18n from "../src/i18n.ts";
import * as controls from "../src/locales/controls.ts";
import * as backendText from "../src/localizeBackendText.ts";
import * as theme from "../src/theme.ts";

const compiled = await Promise.all(["ThemePanel", "RemoteManagerDialog", "RepositoryActions"].map(async (name) => ({
  name,
  code: ts.transpileModule(await readFile(new URL(`../src/${name}.tsx`, import.meta.url), "utf8"), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX, target: ts.ScriptTarget.ES2022 },
  }).outputText,
})));

function sameDependencies(previous, next) {
  return previous && next && previous.length === next.length
    && previous.every((value, index) => Object.is(value, next[index]));
}

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

// Run the real controls, reconciling keyed child state and attaching dialog
// refs before effects. The native surface models open/close and detach behavior;
// this is a component lifecycle check, not browser layout or focus acceptance.
export function repositoryActionsHarness(initialProps, initialLocale = "tr") {
  const instances = new Map();
  const closeEvents = [];
  let props = initialProps;
  let current;
  let cursor = 0;
  let nextId = 0;
  let dirty = true;
  let effects = [];
  let tree;
  let locale = initialLocale;
  const translators = new Map();
  const changeLocale = (next) => { locale = next; dirty = true; };

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
      const entry = instance.hooks[index];
      return [entry.value, entry.set];
    },
    useRef(initial) {
      const index = cursor++;
      current.hooks[index] ??= { current: initial };
      return current.hooks[index];
    },
    useId() { return react.useRef(`test-${++nextId}`).current; },
    useMemo(factory, dependencies) {
      const index = cursor++;
      if (!current.hooks[index] || !sameDependencies(current.hooks[index].dependencies, dependencies)) {
        current.hooks[index] = { value: factory(), dependencies };
      }
      return current.hooks[index].value;
    },
    useCallback(callback, dependencies) { return react.useMemo(() => callback, dependencies); },
    useLayoutEffect(effect, dependencies) { react.useEffect(effect, dependencies); },
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
    "react-dom": { createPortal: (element) => element },
    "lucide-react": new Proxy({}, { get: (_target, name) => `icon:${String(name)}` }),
    "./theme": theme,
    "./themePanel.css": {},
    "./repositoryActionModel": repositoryActionModel,
    "./repositoryIdentity": repositoryIdentity,
    "./i18n.ts": i18n,
    "./locales/controls.ts": controls,
    "./localizeBackendText.ts": backendText,
    "./useI18n": { useI18n(catalog) {
      if (!translators.has(catalog)) translators.set(catalog, (key, params) => i18n.translate(catalog, locale, key, params));
      return { locale, t: translators.get(catalog), setLocale: changeLocale, localeSaved: true };
    } },
    "./repositoryActions.css": {},
  };
  for (const { name, code } of compiled) {
    const exports = {};
    vm.runInNewContext(code, {
      exports,
      require(moduleName) {
        assert.ok(Object.hasOwn(modules, moduleName), `Unmapped component import: ${moduleName}`);
        return modules[moduleName];
      },
      document: { body: {} },
    }, { filename: `${name}.tsx` });
    modules[`./${name}`] = exports;
  }

  function detach(instance) {
    instance.mounted = false;
    for (const hook of instance.hooks) hook.cleanup?.();
    for (const ref of instance.refs) {
      if (ref.current) { ref.current.isConnected = false; ref.current.open = false; }
      ref.current = null;
    }
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
      instance.currentRefs = new Set();
      current = instance;
      cursor = 0;
      const output = renderElement(element.type(element.props), `${id}/render`, instance);
      for (const ref of instance.refs) {
        if (!instance.currentRefs.has(ref)) { ref.current.isConnected = false; ref.current.open = false; ref.current = null; }
      }
      instance.refs = instance.currentRefs;
      effects.push(...instance.effects);
      return output;
    }
    const rendered = { ...element, props: { ...element.props, children: renderElement(element.props.children, `${path}/children`, owner) } };
    const ref = element.props.ref;
    if (ref && typeof ref === "object") {
      if (!ref.current) {
        ref.current = {
          open: false, isConnected: true, showCount: 0,
          showModal() { assert.ok(this.isConnected, "Cannot show a detached dialog"); this.open = true; this.showCount += 1; },
          close() { if (this.open) { this.open = false; closeEvents.push(this); } },
          focus() {},
        };
      }
      ref.current.props = rendered.props;
      owner.currentRefs.add(ref);
    }
    return rendered;
  }

  function render() {
    let renders = 0;
    while (dirty) {
      assert.ok(++renders <= 30, "Components did not settle after effects");
      dirty = false;
      effects = [];
      for (const instance of instances.values()) instance.seen = false;
      tree = renderElement(jsx(modules["./RepositoryActions"].RepositoryActions, props), "root", null);
      for (const [id, instance] of instances) {
        if (!instance.seen) { detach(instance); instances.delete(id); }
      }
      for (const effect of effects) effect();
    }
  }

  return {
    setProps(change) { props = { ...props, ...change }; dirty = true; },
    setLocale: changeLocale,
    async flush() {
      for (let turn = 0; turn < 30; turn += 1) {
        render();
        for (const surface of closeEvents.splice(0)) if (surface.isConnected) surface.props.onClose?.();
        await new Promise((resolve) => setImmediate(resolve));
        if (!dirty && !closeEvents.length) return;
      }
      assert.fail("Components did not settle after deferred calls");
    },
    all(predicate, within = tree) { return [...elements(within)].filter(predicate); },
    find(predicate, within = tree) {
      const matches = this.all(predicate, within);
      assert.equal(matches.length, 1, `Expected one element; found ${matches.length}`);
      return matches[0];
    },
    unmount() { for (const instance of instances.values()) detach(instance); instances.clear(); },
  };
}
