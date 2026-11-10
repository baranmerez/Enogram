export type Locale = "tr" | "en";
export type MessageCatalog = Record<string, Record<Locale, string>>;
export type MessageParams = Record<string, string | number>;
export type Translator<C extends MessageCatalog> = (key: keyof C, params?: MessageParams) => string;

export const localeStorageKey = "enogram.language.v1";
export const defaultLocale: Locale = "tr";

type LocaleStorage = Pick<Storage, "getItem" | "setItem">;
interface LocaleState { locale: Locale; saved: boolean }

export function isLocale(value: unknown): value is Locale {
  return value === "tr" || value === "en";
}

export function intlLocale(locale: Locale): string {
  return locale === "en" ? "en-US" : "tr-TR";
}

export function translate<C extends MessageCatalog>(catalog: C, locale: Locale, key: keyof C, params: MessageParams = {}): string {
  return catalog[key][locale].replace(/\{([a-zA-Z][a-zA-Z0-9]*)\}/g, (placeholder, name: string) => (
    Object.hasOwn(params, name) ? String(params[name]) : placeholder
  ));
}

// A stable external store lets every view change language without remounting
// the workspace or tying Git request effects to a presentation preference.
export function createLocaleStore() {
  let state: LocaleState = { locale: defaultLocale, saved: true };
  let storage: LocaleStorage | undefined;
  let documentRoot: Pick<HTMLElement, "lang"> | undefined;
  const listeners = new Set<() => void>();

  function publish(next: LocaleState) {
    if (documentRoot) documentRoot.lang = next.locale;
    if (state.locale === next.locale && state.saved === next.saved) return;
    state = next;
    for (const listener of listeners) listener();
  }

  return {
    getSnapshot: () => state,
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => { listeners.delete(listener); };
    },
    initialize(nextStorage?: LocaleStorage, root?: Pick<HTMLElement, "lang">) {
      storage = nextStorage;
      documentRoot = root;
      try {
        const saved = storage?.getItem(localeStorageKey);
        publish({ locale: isLocale(saved) ? saved : defaultLocale, saved: storage !== undefined });
      } catch {
        publish({ locale: defaultLocale, saved: false });
      }
    },
    setLocale(locale: Locale): boolean {
      if (!isLocale(locale)) return false;
      let saved = false;
      try {
        if (storage) {
          storage.setItem(localeStorageKey, locale);
          saved = true;
        }
      } catch {
        // The current session still changes language when persistence is denied.
      }
      publish({ locale, saved });
      return saved;
    },
  };
}

export const localeStore = createLocaleStore();
export const getLocale = (): Locale => localeStore.getSnapshot().locale;
export const setLocale = (locale: Locale): boolean => localeStore.setLocale(locale);

export function initializeLocale(): void {
  let storage: LocaleStorage | undefined;
  try { storage = window.localStorage; } catch { /* Storage may be unavailable. */ }
  localeStore.initialize(storage, document.documentElement);
}
