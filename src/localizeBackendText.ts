import type { Locale } from "./i18n.ts";

// These exact application-owned reasons come from remote_settings.rs and
// partial_staging.rs. Unknown Git/process diagnostics remain unchanged.
const turkishBackendText = new Map<string, string>([
  ["This remote has an unsupported name. Edit its settings in Git.", "Bu uzak deponun adı desteklenmiyor. Ayarlarını Git üzerinden düzenleyin."],
  ["This repository uses URL-dependent conditional includes. Edit its remote settings in Git to preserve those configuration rules.", "Bu depo URL'ye bağlı koşullu yapılandırma dosyaları kullanıyor. Bu kuralları korumak için uzak depo ayarlarını Git üzerinden düzenleyin."],
  ["This remote uses inherited, included, or worktree-specific configuration. Edit it in Git so those scopes remain intact.", "Bu uzak depo devralınan, başka dosyadan dahil edilen veya çalışma ağacına özel yapılandırma kullanıyor. Bu kapsamları korumak için Git üzerinden düzenleyin."],
  ["This remote has multiple fetch or push URLs. Edit it in Git to preserve every destination.", "Bu uzak deponun birden fazla Fetch veya Push adresi var. Tüm hedefleri korumak için Git üzerinden düzenleyin."],
  ["This remote uses a custom transport. Edit its settings in Git.", "Bu uzak depo özel bir aktarım yöntemi kullanıyor. Ayarlarını Git üzerinden düzenleyin."],
  ["The repository configuration is missing or inaccessible. Edit it in Git before using remote settings.", "Depo yapılandırması bulunamadı veya erişilemiyor. Uzak depo ayarlarını kullanmadan önce Git üzerinden düzenleyin."],
  ["The repository configuration is a symlink or a non-regular file. Edit its remote settings in Git.", "Depo yapılandırması sembolik bağlantı veya normal dosya dışında bir tür. Uzak depo ayarlarını Git üzerinden düzenleyin."],
  ["The repository configuration is read-only. Make it writable in Git before editing remote settings.", "Depo yapılandırması salt okunur. Uzak depo ayarlarını düzenlemeden önce Git üzerinden yazılabilir duruma getirin."],
  ["The repository configuration has multiple hard links. Edit its remote settings in Git.", "Depo yapılandırmasının birden fazla sabit bağlantısı var. Uzak depo ayarlarını Git üzerinden düzenleyin."],
  ["Resolve the file's merge conflicts before selecting individual lines.", "Tek tek satır seçmeden önce dosyanın merge çakışmalarını çözün."],
  ["Renames, copies, and file-type changes require the whole-file action.", "Yeniden adlandırma, kopyalama ve dosya türü değişiklikleri için dosyanın tamamına işlem uygulanmalıdır."],
  ["Submodules and directories do not support individual-line actions.", "Alt modüllerde ve klasörlerde tek tek satır işlemleri desteklenmiyor."],
  ["Clean/smudge filters, working-tree encoding, and ident conversion require the whole-file action.", "Clean/smudge filtreleri, çalışma ağacı kodlaması ve ident dönüşümü için dosyanın tamamına işlem uygulanmalıdır."],
  ["This diff is truncated. Use the whole-file action.", "Bu fark kısaltılmış. Dosyanın tamamı için işlem yapın."],
  ["Binary files do not support individual-line actions.", "İkili dosyalarda tek tek satır işlemleri desteklenmiyor."],
  ["This diff is not valid UTF-8. Use the whole-file action.", "Bu fark geçerli UTF-8 biçiminde değil. Dosyanın tamamı için işlem yapın."],
  ["Intent-to-add entries and changing file identities require the whole-file action.", "Intent-to-add kayıtları ve değişen dosya kimlikleri için dosyanın tamamına işlem uygulanmalıdır."],
  ["File mode changes and non-UTF-8 index content require the whole-file action.", "Dosya modu değişiklikleri ve UTF-8 olmayan dizin içeriği için dosyanın tamamına işlem uygulanmalıdır."],
  ["This change has no selectable text lines. Use the whole-file action.", "Bu değişiklikte seçilebilir metin satırı yok. Dosyanın tamamı için işlem yapın."],
  ["Symlinks, submodules, and special files require the whole-file action.", "Sembolik bağlantılar, alt modüller ve özel dosyalar için dosyanın tamamına işlem uygulanmalıdır."],
  ["This file exceeds the 4 MiB partial-selection limit. Use the whole-file action.", "Bu dosya kısmi seçim için 4 MiB sınırını aşıyor. Dosyanın tamamı için işlem yapın."],
  ["Git command output exceeded the configured limit.", "Git komutunun çıktısı yapılandırılmış sınırı aştı."],
  ["Invalid index entry.", "Geçersiz dizin kaydı."],
  ["Partial selections require one exact, unmerged index entry.", "Kısmi seçim için dizinde seçili dosyaya ait tek, çakışmasız kayıt gerekir."],
  ["Only ordinary, unmerged text files support individual-line actions.", "Tek tek satır işlemleri yalnızca normal, çakışmasız metin dosyalarında destekleniyor."],
  ["Incomplete diff lines require the whole-file action.", "Eksik fark satırları nedeniyle dosyanın tamamına işlem uygulanmalıdır."],
  ["Partial selection requires a single-file diff.", "Kısmi seçim için tek dosyalık bir fark gerekir."],
  ["Unsupported hunk header.", "Desteklenmeyen değişiklik bölümü başlığı."],
  ["Invalid end-of-file marker.", "Geçersiz dosya sonu işareti."],
  ["Invalid duplicate end-of-file marker.", "Geçersiz, yinelenen dosya sonu işareti."],
  ["Unsupported diff body. Use the whole-file action.", "Fark içeriği desteklenmiyor. Dosyanın tamamı için işlem yapın."],
  ["File mode changes, renames, and copies require the whole-file action.", "Dosya modu değişiklikleri, yeniden adlandırma ve kopyalama için dosyanın tamamına işlem uygulanmalıdır."],
  ["Unsupported diff metadata. Use the whole-file action.", "Fark üst bilgisi desteklenmiyor. Dosyanın tamamı için işlem yapın."],
  ["Only ordinary text-file changes support individual-line actions.", "Tek tek satır işlemleri yalnızca normal metin dosyası değişikliklerinde destekleniyor."],
  ["Invalid hunk range.", "Geçersiz değişiklik bölümü aralığı."],
  ["The hunk range exceeds the partial-selection limit.", "Değişiklik bölümünün aralığı kısmi seçim sınırını aşıyor."],
  ["The diff contains an incomplete hunk.", "Fark, eksik bir değişiklik bölümü içeriyor."],
]);

export function localizeBackendText(value: string | null | undefined, locale: Locale): string | null {
  if (value == null) return null;
  return locale === "tr" ? turkishBackendText.get(value) ?? value : value;
}
