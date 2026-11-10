import { useCallback, useSyncExternalStore } from "react";
import { getLocale, localeStore, setLocale, translate, type MessageCatalog, type MessageParams } from "./i18n.ts";

export function useI18n<C extends MessageCatalog>(catalog: C) {
  const state = useSyncExternalStore(localeStore.subscribe, localeStore.getSnapshot, localeStore.getSnapshot);
  const t = useCallback((key: keyof C, params?: MessageParams) => translate(catalog, getLocale(), key, params), [catalog]);
  return { locale: state.locale, localeSaved: state.saved, setLocale, t };
}
