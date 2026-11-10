import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import vm from "node:vm";
import ts from "typescript";
import * as diffLines from "../src/diffLines.ts";
import * as repositoryActionModel from "../src/repositoryActionModel.ts";
import * as repositoryIdentity from "../src/repositoryIdentity.ts";
import * as workingChanges from "../src/workingChanges.ts";
import * as workspacePreferences from "../src/workspacePreferences.ts";
import * as workspaceSearch from "../src/workspaceSearch.ts";

const source = await readFile(new URL("../src/App.tsx", import.meta.url), "utf8");
const compiled = ts.transpileModule(`${source}\nexport { RepositoryApp };`, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX, target: ts.ScriptTarget.ES2022 },
}).outputText;

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

// Run the real component's handlers and effects with deterministic hook storage
// and deferred Tauri calls. This covers state/IPC races, not DOM or WebView layout.
export function repositoryAppHarness({ localStorage } = {}) {
  const hooks = [];
  const requests = [];
  const storedValues = new Map();
  let cursor = 0;
  let dirty = true;
  let pendingEffects = [];
  let tree;

  const react = {
    useState(initial) {
      const index = cursor++;
      if (!hooks[index]) {
        const entry = { value: typeof initial === "function" ? initial() : initial };
        entry.set = (update) => {
          const next = typeof update === "function" ? update(entry.value) : update;
          if (!Object.is(next, entry.value)) {
            entry.value = next;
            dirty = true;
          }
        };
        hooks[index] = entry;
      }
      return [hooks[index].value, hooks[index].set];
    },
    useRef(initial) {
      const index = cursor++;
      hooks[index] ??= { current: initial };
      return hooks[index];
    },
    useMemo(factory, dependencies) {
      const index = cursor++;
      if (!hooks[index] || !sameDependencies(hooks[index].dependencies, dependencies)) {
        hooks[index] = { value: factory(), dependencies };
      }
      return hooks[index].value;
    },
    useCallback(callback, dependencies) {
      return react.useMemo(() => callback, dependencies);
    },
    useEffect(effect, dependencies) {
      const index = cursor++;
      const previous = hooks[index];
      if (previous && sameDependencies(previous.dependencies, dependencies)) return;
      const entry = { dependencies, cleanup: previous?.cleanup };
      hooks[index] = entry;
      pendingEffects.push(() => {
        entry.cleanup?.();
        entry.cleanup = effect();
      });
    },
  };

  const invoke = (command, args) => new Promise((resolve, reject) => {
    requests.push({ command, args, resolve, reject, taken: false });
  });
  const jsx = (type, props, key) => ({ type, props: props ?? {}, key });
  const modules = {
    "react": react,
    "react/jsx-runtime": { jsx, jsxs: jsx, Fragment: "Fragment" },
    "@tauri-apps/api/core": { isTauri: () => true, invoke },
    "lucide-react": new Proxy({}, { get: (_target, name) => `icon:${String(name)}` }),
    "./DiffView": { DiffView: "DiffView" },
    "./diffLines": diffLines,
    "./HistoryList": { HistoryList: "HistoryList" },
    "./RepositoryActions": { RepositoryActions: "RepositoryActions" },
    "./repositoryActionModel": repositoryActionModel,
    "./repositoryIdentity": repositoryIdentity,
    "./workingChanges": workingChanges,
    "./workspacePreferences": workspacePreferences,
    "./workspaceSearch": workspaceSearch,
    "./workspaceControls.css": {},
  };
  const exports = {};
  vm.runInNewContext(compiled, {
    exports,
    require(name) {
      assert.ok(Object.hasOwn(modules, name), `Unmapped App import: ${name}`);
      return modules[name];
    },
    window: {
      addEventListener() {}, removeEventListener() {},
      localStorage: localStorage ?? {
        getItem: (key) => storedValues.get(key) ?? null,
        setItem: (key, value) => storedValues.set(key, String(value)),
      },
    },
    document: { querySelector: () => null },
  }, { filename: "App.tsx" });

  function render() {
    let renders = 0;
    while (dirty) {
      assert.ok(++renders <= 30, "Component did not settle after effects");
      dirty = false;
      cursor = 0;
      pendingEffects = [];
      tree = exports.RepositoryApp();
      for (const effect of pendingEffects) effect();
    }
  }

  return {
    requests,
    async flush() {
      for (let turn = 0; turn < 30; turn += 1) {
        render();
        await new Promise((resolve) => setImmediate(resolve));
        if (!dirty) return;
      }
      assert.fail("Component did not settle after deferred calls");
    },
    all(predicate) {
      return [...elements(tree)].filter(predicate);
    },
    find(predicate) {
      const found = this.all(predicate);
      assert.equal(found.length, 1, `Expected one element; found ${found.length}`);
      return found[0];
    },
    take(command) {
      const request = requests.find((entry) => entry.command === command && !entry.taken);
      assert.ok(request, `No pending ${command} request`);
      request.taken = true;
      return request;
    },
    unmount() {
      for (const hook of hooks) hook.cleanup?.();
    },
  };
}
