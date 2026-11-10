import { Check, FileCode2, GitBranch, Layers2, Moon, Palette, RotateCcw, Sparkles, Sun, X } from "lucide-react";
import { type CSSProperties, type FormEvent, useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from "react";
import {
  applyTheme,
  createThemeTokens,
  defaultThemeSettings,
  loadThemeSettings,
  normalizeHexColor,
  normalizeThemeSettings,
  saveThemeSettings,
  settingsForPreset,
  themeLimits,
  themePresets,
  type ButtonStyle,
  type ThemeMode,
  type ThemePreset,
  type ThemeSettings,
} from "./theme";
import "./themePanel.css";

const buttonStyles: { value: ButtonStyle; label: string }[] = [
  { value: "solid", label: "Dolu" },
  { value: "soft", label: "Yumuşak" },
  { value: "outline", label: "Çizgili" },
  { value: "glass", label: "Cam" },
];

function useTheme() {
  const [loaded] = useState(() => loadThemeSettings());
  const [settings, setSettings] = useState(loaded.settings);
  const [storageStatus, setStorageStatus] = useState(loaded.storageStatus);

  useLayoutEffect(() => applyTheme(settings), [settings]);

  const updateTheme = useCallback((value: ThemeSettings) => {
    const next = normalizeThemeSettings(value);
    setSettings(next);
    setStorageStatus(saveThemeSettings(next) ? "available" : "unavailable");
  }, []);

  return { settings, storageStatus, updateTheme };
}

function presetStyle(preset: ThemePreset): CSSProperties {
  const tokens = createThemeTokens(preset.settings);
  return {
    "--preset-canvas": tokens["--canvas"],
    "--preset-panel": tokens["--panel-raised"],
    "--preset-line": tokens["--line"],
    "--preset-text": tokens["--muted"],
    "--preset-accent": tokens["--blue"],
    "--preset-selection": tokens["--selection"],
  } as CSSProperties;
}

const presetStyles = new Map(themePresets.map((preset) => [preset.id, presetStyle(preset)]));

interface ThemePanelProps {
  density?: "compact" | "comfortable";
  onDensityChange?: (density: "compact" | "comfortable") => void;
  onResetLayout?: () => void;
}

export function ThemePanel({ density = "compact", onDensityChange, onResetLayout }: ThemePanelProps = {}) {
  const { settings, storageStatus, updateTheme } = useTheme();
  const [dialogOpen, setDialogOpen] = useState(false);
  const [seedInput, setSeedInput] = useState(settings.seed);
  const [draftMode, setDraftMode] = useState<ThemeMode>(settings.mode);
  const [seedError, setSeedError] = useState<string | null>(null);
  const [generatorStatus, setGeneratorStatus] = useState("");
  const dialogRef = useRef<HTMLDialogElement>(null);
  const seedRef = useRef<HTMLInputElement>(null);
  const dialogId = useId();
  const headingId = useId();
  const seedId = useId();
  const seedErrorId = useId();
  const opacityId = useId();
  const blurId = useId();
  const buttonGroupName = useId();
  const tokens = useMemo(() => createThemeTokens(settings), [settings]);
  const activePreset = themePresets.find((preset) => preset.id === settings.preset);
  const paletteName = activePreset?.name ?? "Özel palet";
  const hasGlass = settings.surface === "glass";
  const seedPreview = normalizeHexColor(seedInput) ?? settings.seed;

  useEffect(() => {
    setSeedInput(settings.seed);
    setDraftMode(settings.mode);
    setSeedError(null);
  }, [settings.seed, settings.mode]);

  function openDialog() {
    if (!dialogRef.current || dialogRef.current.open) return;
    dialogRef.current.showModal();
    setDialogOpen(true);
  }

  function closeDialog() {
    dialogRef.current?.close();
  }

  function choosePreset(preset: ThemePreset) {
    updateTheme(settingsForPreset(preset.id));
    setSeedInput(preset.settings.seed);
    setDraftMode(preset.settings.mode);
    setSeedError(null);
    setGeneratorStatus("");
  }

  function generatePalette(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const seed = normalizeHexColor(seedInput);
    if (!seed) {
      setSeedError("#75AEF2 gibi 3 veya 6 haneli bir renk girin.");
      setGeneratorStatus("");
      seedRef.current?.focus();
      return;
    }
    updateTheme({ ...settings, preset: "custom", seed, mode: draftMode });
    setSeedInput(seed);
    setSeedError(null);
    setGeneratorStatus("Yeni renk paleti uygulandı.");
  }

  function restoreDefault() {
    updateTheme({ ...defaultThemeSettings });
    setSeedInput(defaultThemeSettings.seed);
    setDraftMode(defaultThemeSettings.mode);
    setSeedError(null);
    setGeneratorStatus("Varsayılan tema geri yüklendi.");
  }

  return (
    <>
      <button
        type="button"
        className="theme-trigger"
        onClick={openDialog}
        aria-haspopup="dialog"
        aria-controls={dialogId}
        aria-expanded={dialogOpen}
        title="Görünüm ve tema"
      >
        <Palette size={16} strokeWidth={1.7} aria-hidden="true" />
        <span>Görünüm</span>
        <i className="theme-trigger-dot" aria-hidden="true" />
      </button>

      <dialog
        ref={dialogRef}
        id={dialogId}
        className="theme-dialog"
        aria-labelledby={headingId}
        onClose={() => setDialogOpen(false)}
      >
        <header className="theme-dialog-header">
          <div className="theme-dialog-mark" aria-hidden="true"><Palette size={21} strokeWidth={1.6} /></div>
          <div className="theme-heading-copy">
            <span className="theme-eyebrow">ÇALIŞMA ALANINI KİŞİSELLEŞTİR</span>
            <h2 id={headingId}>Görünüm</h2>
          </div>
          <span className="theme-live-label"><i aria-hidden="true" /> Anında uygulanır</span>
          <button type="button" className="theme-close" aria-label="Görünüm ayarlarını kapat" onClick={closeDialog} autoFocus>
            <X size={18} aria-hidden="true" />
          </button>
        </header>

        <div className="theme-dialog-body">
          <div className="theme-palette-column">
            <section className="theme-section theme-layout" aria-labelledby={`${headingId}-layout`}>
              <div className="theme-layout-heading">
                <h3 id={`${headingId}-layout`}>Yerleşim</h3>
                <button type="button" className="theme-layout-reset" onClick={onResetLayout} disabled={!onResetLayout}>
                  <RotateCcw size={12} aria-hidden="true" /> Düzeni sıfırla
                </button>
              </div>
              <div className="theme-density-row">
                <div className="theme-segmented" role="group" aria-label="Çalışma alanı yoğunluğu">
                  <button type="button" aria-pressed={density === "compact"} disabled={!onDensityChange} onClick={() => onDensityChange?.("compact")}>Sıkı</button>
                  <button type="button" aria-pressed={density === "comfortable"} disabled={!onDensityChange} onClick={() => onDensityChange?.("comfortable")}>Rahat</button>
                </div>
                <span>{density === "compact" ? "Daha fazla satır" : "Daha geniş aralıklar"}</span>
              </div>
            </section>

            <section className="theme-section" aria-labelledby={`${headingId}-presets`}>
              <div className="theme-section-heading">
                <h3 id={`${headingId}-presets`}>Hazır temalar</h3>
                <span>Bir dokunuşla yeni bir atmosfer</span>
              </div>
              <div className="theme-presets">
                {themePresets.map((preset) => (
                  <button
                    key={preset.id}
                    type="button"
                    className={`theme-preset${settings.preset === preset.id ? " selected" : ""}`}
                    style={presetStyles.get(preset.id)}
                    aria-pressed={settings.preset === preset.id}
                    onClick={() => choosePreset(preset)}
                  >
                    <span className="theme-preset-scene" aria-hidden="true">
                      <span className="theme-preset-top"><i /><i /><i /></span>
                      <span className="theme-preset-sidebar"><i /><i /><i /></span>
                      <span className="theme-preset-editor"><i /><i /><i /><i /></span>
                      <span className="theme-preset-glow" />
                      {settings.preset === preset.id && <span className="theme-preset-check"><Check size={12} strokeWidth={2.5} /></span>}
                    </span>
                    <span className="theme-preset-copy"><strong>{preset.name}</strong><small>{preset.description}</small></span>
                  </button>
                ))}
              </div>
            </section>

            <section className="theme-generator theme-section" aria-labelledby={`${headingId}-generator`}>
              <div className="theme-section-heading">
                <h3 id={`${headingId}-generator`}><Sparkles size={14} aria-hidden="true" /> Kendi rengini üret</h3>
                <span>Tek renkten uyumlu bir palet.</span>
              </div>
              <form className="theme-generator-form" onSubmit={generatePalette} noValidate>
                <div className="theme-seed-row">
                  <label className="theme-color-picker" title="Renk seç" style={{ "--seed-preview": seedPreview } as CSSProperties}>
                    <input
                      type="color"
                      aria-label="Başlangıç rengini seç"
                      value={seedPreview}
                      onChange={(event) => { setSeedInput(event.target.value.toUpperCase()); setSeedError(null); setGeneratorStatus(""); }}
                    />
                    <span aria-hidden="true" />
                  </label>
                  <div className="theme-hex-field">
                    <label htmlFor={seedId}>Başlangıç rengi</label>
                    <input
                      ref={seedRef}
                      id={seedId}
                      value={seedInput}
                      maxLength={7}
                      spellCheck={false}
                      autoComplete="off"
                      autoCapitalize="characters"
                      aria-invalid={Boolean(seedError)}
                      aria-describedby={seedError ? seedErrorId : undefined}
                      onChange={(event) => { setSeedInput(event.target.value); setSeedError(null); setGeneratorStatus(""); }}
                    />
                  </div>
                  <button type="submit" className="theme-generate-button"><Sparkles size={13} aria-hidden="true" /> Üret</button>
                </div>
                <div className="theme-mode-row">
                  <span>Renk modu</span>
                  <div className="theme-segmented" role="group" aria-label="Üretilecek temanın renk modu">
                    <button type="button" aria-pressed={draftMode === "dark"} onClick={() => { setDraftMode("dark"); setGeneratorStatus(""); }}><Moon size={13} aria-hidden="true" /> Koyu</button>
                    <button type="button" aria-pressed={draftMode === "light"} onClick={() => { setDraftMode("light"); setGeneratorStatus(""); }}><Sun size={13} aria-hidden="true" /> Açık</button>
                  </div>
                </div>
                {seedError && <p className="theme-field-error" id={seedErrorId} role="alert">{seedError}</p>}
                <p className="theme-generator-note" aria-live="polite">{generatorStatus || "Yazı ve durum renkleri okunabilirliğe göre dengelenir."}</p>
              </form>
            </section>
          </div>

          <div className="theme-detail-column">
            <section className="theme-section" aria-labelledby={`${headingId}-preview`}>
              <div className="theme-section-heading theme-preview-heading">
                <h3 id={`${headingId}-preview`}>Canlı önizleme</h3>
                <span>{paletteName} <span aria-hidden="true">·</span> {settings.mode === "dark" ? "Koyu" : "Açık"}</span>
              </div>
              <div className="theme-live-preview">
                <div className="theme-preview-toolbar"><FileCode2 size={13} aria-hidden="true" /><span>theme.css</span><span className="theme-preview-branch"><GitBranch size={11} aria-hidden="true" /> main</span></div>
                <div className="theme-preview-code" aria-label="Tema renkleriyle örnek kod görünümü">
                  <div><span className="theme-preview-line-number">1</span><code>:root &#123;</code></div>
                  <div className="theme-preview-add"><span className="theme-preview-line-number">2</span><code>+  --accent: {tokens["--blue"]};</code></div>
                  <div className="theme-preview-remove"><span className="theme-preview-line-number">3</span><code>−  --surface: previous;</code></div>
                  <div><span className="theme-preview-line-number">4</span><code>&#125;</code></div>
                </div>
                <div className="theme-preview-bottom">
                  <span className="theme-preview-status"><Check size={12} aria-hidden="true" /> Hazır</span>
                  <button type="button" className="theme-preview-button" onClick={() => setGeneratorStatus("Düğme biçimi çalışma alanına uygulandı.")} aria-label="Seçili düğme biçimini dene">Düğmeyi dene</button>
                </div>
              </div>
              <div className="theme-palette-swatches" aria-label="Uygulanan renk paleti">
                {[
                  { name: "Arka plan", token: "--canvas" },
                  { name: "Yüzey", token: "--panel-raised" },
                  { name: "Vurgu", token: "--blue" },
                  { name: "Eklenen", token: "--green" },
                  { name: "Silinen", token: "--red" },
                ].map((swatch) => <span key={swatch.token} title={`${swatch.name}: ${tokens[swatch.token as keyof typeof tokens]}`} style={{ background: tokens[swatch.token as keyof typeof tokens] }}><span className="theme-sr-only">{swatch.name}: {tokens[swatch.token as keyof typeof tokens]}</span></span>)}
                <span className="theme-palette-caption">Dengeli kontrast</span>
              </div>
            </section>

            <fieldset className="theme-section theme-button-fieldset">
              <legend>Düğme biçimi</legend>
              <div className="theme-button-options">
                {buttonStyles.map((style) => (
                  <label key={style.value} className={`theme-button-option${settings.buttonStyle === style.value ? " selected" : ""}`}>
                    <input
                      className="theme-sr-only"
                      type="radio"
                      name={buttonGroupName}
                      value={style.value}
                      checked={settings.buttonStyle === style.value}
                      onChange={() => updateTheme({ ...settings, buttonStyle: style.value })}
                    />
                    <span className={`theme-style-sample ${style.value}`} aria-hidden="true">Aa</span>
                    <span>{style.label}</span>
                  </label>
                ))}
              </div>
            </fieldset>

            <section className="theme-section theme-material" aria-labelledby={`${headingId}-material`}>
              <div className="theme-material-heading">
                <h3 id={`${headingId}-material`}><Layers2 size={14} aria-hidden="true" /> Yüzey</h3>
                <div className="theme-segmented" role="group" aria-label="Yüzey malzemesi">
                  <button type="button" aria-pressed={!hasGlass} onClick={() => updateTheme({ ...settings, surface: "solid" })}>Mat</button>
                  <button type="button" aria-pressed={hasGlass} onClick={() => updateTheme({ ...settings, surface: "glass" })}>Sıvı cam</button>
                </div>
              </div>
              <div className={`theme-glass-controls${hasGlass ? "" : " inactive"}`}>
                <div className="theme-range-heading"><label htmlFor={opacityId}>Opaklık</label><output htmlFor={opacityId}>%{settings.opacity}</output></div>
                <input id={opacityId} type="range" min={themeLimits.minOpacity} max={themeLimits.maxOpacity} step={1} value={settings.opacity} disabled={!hasGlass} onChange={(event) => updateTheme({ ...settings, opacity: Number(event.target.value) })} />
                <div className="theme-range-heading"><label htmlFor={blurId}>Bulanıklık</label><output htmlFor={blurId}>{settings.blur} px</output></div>
                <input id={blurId} type="range" min={themeLimits.minBlur} max={themeLimits.maxBlur} step={1} value={settings.blur} disabled={!hasGlass} onChange={(event) => updateTheme({ ...settings, blur: Number(event.target.value) })} />
              </div>
              <p className="theme-material-note">{hasGlass ? "Cam derinliği panelleri etkiler; kod alanı net kalır." : "Cam ayarlarını kullanmak için Sıvı cam seçin."}</p>
            </section>
          </div>
        </div>

        {storageStatus !== "available" && <p className="theme-storage-notice" role="status">{storageStatus === "unavailable" ? "Görünüm bu oturumda uygulanıyor, ancak bu cihazda kaydedilemiyor." : "Kaydedilen görünümün geçersiz ayarları varsayılan değerlerle açıldı."}</p>}

        <footer className="theme-dialog-footer">
          <button type="button" className="theme-reset" onClick={restoreDefault}><RotateCcw size={13} aria-hidden="true" /> Temayı sıfırla</button>
          <span className="theme-save-note"><Check size={12} aria-hidden="true" /> {storageStatus === "available" ? "Tercihler bu cihazda saklanır" : "Bu oturumda geçerli"}</span>
          <button type="button" className="theme-done" onClick={closeDialog}>Tamam</button>
        </footer>
      </dialog>
    </>
  );
}
