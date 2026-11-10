import type { MessageCatalog } from "../i18n.ts";

export const appMessages = {
  "unexpectedError": {
    "tr": "Beklenmeyen bir hata oluştu.",
    "en": "An unexpected error occurred."
  },
  "unknownDate": {
    "tr": "Tarih bilinmiyor",
    "en": "Unknown date"
  },
  "unknownStatus": {
    "tr": "Değişiklik durumu bilinmiyor",
    "en": "Unknown change status"
  },
  "browserEyebrow": {
    "tr": "ENOGRAM · YEREL GIT İNCELEYİCİSİ",
    "en": "ENOGRAM · LOCAL GIT WORKSPACE"
  },
  "desktopRequired": {
    "tr": "Enogram masaüstünde çalışır",
    "en": "Enogram runs on your desktop"
  },
  "browserCopy": {
    "tr": "Depo verileri, Git komutlarını çalıştıran Tauri masaüstü çekirdeğinden okunur. Bu web önizlemesi bilgisayarınızdaki dosyalara erişmez.",
    "en": "Repository data comes from the Tauri desktop core that runs Git commands. This web preview cannot access files on your computer."
  },
  "noRepositoryData": {
    "tr": "Depo verisi gösterilmiyor",
    "en": "No repository data is displayed"
  },
  "repositoryPathRequired": {
    "tr": "Bir depo yolu girin.",
    "en": "Enter a repository path."
  },
  "commitTreeChanged": {
    "tr": "Commit oluşturuldu; commit sırasında stage edilen içerik değişti. Oluşturulan commit’i kontrol edin.",
    "en": "Commit created; staged content changed while committing. Review the created commit."
  },
  "commitCreated": {
    "tr": "Commit oluşturuldu.",
    "en": "Commit created."
  },
  "filesStaged": {
    "tr": "{count} dosya stage edildi.",
    "en": "Files staged: {count}."
  },
  "filesUnstaged": {
    "tr": "{count} dosya unstage edildi.",
    "en": "Files unstaged: {count}."
  },
  "linesStaged": {
    "tr": "{count} değişiklik satırı stage edildi.",
    "en": "Changed lines staged: {count}."
  },
  "linesUnstaged": {
    "tr": "{count} değişiklik satırı unstage edildi.",
    "en": "Changed lines unstaged: {count}."
  },
  "refreshFailure": {
    "tr": "{error}\nDepo durumu yenilenemedi: {refreshError}",
    "en": "{error}\nCould not refresh repository status: {refreshError}"
  },
  "repositoryPath": {
    "tr": "Depo yolu",
    "en": "Repository path"
  },
  "repositoryPathPlaceholder": {
    "tr": "Yerel Git deposunun yolunu girin",
    "en": "Enter a local Git repository path"
  },
  "repositoryPathTitle": {
    "tr": "Açılacak yerel depo klasörünün yolu",
    "en": "Path to the local repository folder to open"
  },
  "open": {
    "tr": "Aç",
    "en": "Open"
  },
  "navigation": {
    "tr": "GEZİNME",
    "en": "NAVIGATION"
  },
  "expandSidebar": {
    "tr": "Kenar çubuğunu aç",
    "en": "Expand sidebar"
  },
  "collapseSidebar": {
    "tr": "Kenar çubuğunu daralt",
    "en": "Collapse sidebar"
  },
  "localRepositoryHeading": {
    "tr": "YEREL DEPO",
    "en": "LOCAL REPOSITORY"
  },
  "readingRepository": {
    "tr": "Depo okunuyor…",
    "en": "Reading repository…"
  },
  "noRepository": {
    "tr": "Depo açık değil",
    "en": "No repository open"
  },
  "worktreesHeading": {
    "tr": "ÇALIŞMA AĞAÇLARI",
    "en": "WORKTREES"
  },
  "worktrees": {
    "tr": "Çalışma ağaçları",
    "en": "Worktrees"
  },
  "bareWorktree": {
    "tr": "{path} — Çalışma dizini yok",
    "en": "{path} — No working directory"
  },
  "openWorktree": {
    "tr": "Çalışma ağacını aç: {path}",
    "en": "Open worktree: {path}"
  },
  "detachedHead": {
    "tr": "Ayrık HEAD",
    "en": "Detached HEAD"
  },
  "noBranch": {
    "tr": "Dal yok",
    "en": "No branch"
  },
  "currentRepository": {
    "tr": "Açık depo",
    "en": "Current repository"
  },
  "noWorktreeInfo": {
    "tr": "Çalışma ağacı bilgisi yok",
    "en": "No worktree information"
  },
  "localGit": {
    "tr": "Yerel Git",
    "en": "Local Git"
  },
  "waitingVersion": {
    "tr": "Sürüm bilgisi bekleniyor",
    "en": "Waiting for version information"
  },
  "manageRemotes": {
    "tr": "Uzak depoları yönet",
    "en": "Manage remotes"
  },
  "remotes": {
    "tr": "Uzak depolar",
    "en": "Remotes"
  },
  "configuredRemotes": {
    "tr": "{count} tanımlı",
    "en": "{count} configured"
  },
  "add": {
    "tr": "Ekle",
    "en": "Add"
  },
  "waitingRepository": {
    "tr": "Depo bekleniyor",
    "en": "Waiting for repository"
  },
  "mcpDescription": {
    "tr": "Yerel MCP erişimi Codex veya Claude ayarlarından yönetilir. Bu pencere istemci bağlantı durumunu izlemez.",
    "en": "Manage local MCP access in your Codex or Claude settings. This window does not monitor client connection status."
  },
  "localMcp": {
    "tr": "Yerel MCP",
    "en": "Local MCP"
  },
  "remoteSetupHint": {
    "tr": "Fetch, Pull ve Push için Uzak depolar bölümünden bir adres ekleyin.",
    "en": "Add a URL in Remotes to use Fetch, Pull, and Push."
  },
  "localRepository": {
    "tr": "Yerel depo",
    "en": "Local repository"
  },
  "focusRepositoryPath": {
    "tr": "Depo yoluna git",
    "en": "Focus repository path"
  },
  "showWorktrees": {
    "tr": "Çalışma ağaçlarını göster",
    "en": "Show worktrees"
  },
  "sidebarWidth": {
    "tr": "Kenar çubuğu genişliği",
    "en": "Sidebar width"
  },
  "viewScope": {
    "tr": "İçerik kapsamı",
    "en": "View scope"
  },
  "working": {
    "tr": "Çalışma",
    "en": "Working"
  },
  "history": {
    "tr": "Geçmiş",
    "en": "History"
  },
  "refresh": {
    "tr": "Geçmişi ve durumu yenile",
    "en": "Refresh history and status"
  },
  "commitHistory": {
    "tr": "Commit geçmişi",
    "en": "Commit history"
  },
  "historySearch": {
    "tr": "Tüm geçmişte arama",
    "en": "Search all history"
  },
  "searchCommits": {
    "tr": "Commit ara",
    "en": "Search commits"
  },
  "historySearchHelp": {
    "tr": "Tüm yerel geçmişte mesaj (başlık ve gövde), yazar/e-posta, SHA veya referans ara. Enter ile ara; Ctrl+Shift+F ile odaklan.",
    "en": "Search all local history by message (subject and body), author/email, SHA, or ref. Press Enter to search; Ctrl+Shift+F to focus."
  },
  "historySearchPlaceholder": {
    "tr": "Tüm geçmişte ara…",
    "en": "Search all history…"
  },
  "searchAllHistory": {
    "tr": "Tüm geçmişte ara",
    "en": "Search all history"
  },
  "searchAllLocalHistory": {
    "tr": "Tüm yerel geçmişte ara",
    "en": "Search all local history"
  },
  "resultsOnPage": {
    "tr": "{count} sonuç bu sayfada",
    "en": "Results on this page: {count}"
  },
  "previousMatchHelp": {
    "tr": "Önceki eşleşme (Shift+Enter)",
    "en": "Previous match (Shift+Enter)"
  },
  "previousCommitMatch": {
    "tr": "Önceki commit eşleşmesi",
    "en": "Previous commit match"
  },
  "nextMatchHelp": {
    "tr": "Sonraki eşleşme (Enter)",
    "en": "Next match (Enter)"
  },
  "nextCommitMatch": {
    "tr": "Sonraki commit eşleşmesi",
    "en": "Next commit match"
  },
  "clearCommitSearch": {
    "tr": "Commit aramasını temizle",
    "en": "Clear commit search"
  },
  "backToGraph": {
    "tr": "Aramayı temizle ve grafiğe dön",
    "en": "Clear search and return to the graph"
  },
  "loadedCommitCount": {
    "tr": "Grafikte yüklenen son commit sayısı",
    "en": "Number of recent commits loaded in the graph"
  },
  "searchFailed": {
    "tr": "Arama tamamlanamadı",
    "en": "Search incomplete"
  },
  "searching": {
    "tr": "Taranıyor",
    "en": "Searching"
  },
  "stopped": {
    "tr": "Durduruldu",
    "en": "Stopped"
  },
  "searchComplete": {
    "tr": "Tarama tamamlandı",
    "en": "Search complete"
  },
  "moreResults": {
    "tr": "Diğer sonuçlar var",
    "en": "More results available"
  },
  "stop": {
    "tr": "Durdur",
    "en": "Stop"
  },
  "previousPage": {
    "tr": "Önceki sonuç sayfası",
    "en": "Previous results page"
  },
  "resumeSearch": {
    "tr": "Aramayı sürdür",
    "en": "Resume search"
  },
  "nextPage": {
    "tr": "Sonraki sonuç sayfası",
    "en": "Next results page"
  },
  "historySearchResults": {
    "tr": "Tüm geçmiş arama sonuçları",
    "en": "All history search results"
  },
  "searchAgain": {
    "tr": "Yeniden ara",
    "en": "Search again"
  },
  "searchingHistory": {
    "tr": "Geçmiş taranıyor…",
    "en": "Searching history…"
  },
  "noMatchingCommits": {
    "tr": "Tüm yerel geçmiş tarandı; eşleşen commit bulunamadı.",
    "en": "All local history searched; no matching commits found."
  },
  "searchStoppedHint": {
    "tr": "Arama durduruldu. Devam edebilir veya yeniden arayabilirsiniz.",
    "en": "Search stopped. You can resume or start a new search."
  },
  "untitledCommit": {
    "tr": "(Başlıksız commit)",
    "en": "(Untitled commit)"
  },
  "openToStart": {
    "tr": "Başlamak için bir depo açın.",
    "en": "Open a repository to get started."
  },
  "noFirstCommit": {
    "tr": "Henüz ilk commit yok.",
    "en": "No first commit yet."
  },
  "noHistory": {
    "tr": "Geçmiş girdisi bulunamadı.",
    "en": "No history entries found."
  },
  "repositoryReadFailed": {
    "tr": "Depo okunamadı.",
    "en": "Could not read the repository."
  },
  "workingFiles": {
    "tr": "Çalışma ağacı dosyaları",
    "en": "Working tree files"
  },
  "commitFiles": {
    "tr": "Commit dosyaları",
    "en": "Commit files"
  },
  "changesHeading": {
    "tr": "DEĞİŞİKLİKLER",
    "en": "CHANGES"
  },
  "commitFilesHeading": {
    "tr": "COMMIT DOSYALARI",
    "en": "COMMIT FILES"
  },
  "visibleTotalFiles": {
    "tr": "Görünen / toplam dosya",
    "en": "Visible / total files"
  },
  "bothLayersHint": {
    "tr": "Bir dosya hem stage edilmiş hem çalışma ağacında değişmiş olabilir.",
    "en": "A file can have both staged and working tree changes."
  },
  "stagedFilesCount": {
    "tr": "{count} stage edilmiş dosya",
    "en": "Staged files: {count}"
  },
  "workingFilesCount": {
    "tr": "{count} çalışma ağacı dosyası",
    "en": "Working tree files: {count}"
  },
  "stagedShort": {
    "tr": "Stage",
    "en": "Staged"
  },
  "fileFilters": {
    "tr": "Dosya filtreleri",
    "en": "File filters"
  },
  "searchFiles": {
    "tr": "Dosya ara",
    "en": "Search files"
  },
  "fileSearchHelp": {
    "tr": "Dosya adı veya yol (Ctrl+F)",
    "en": "File name or path (Ctrl+F)"
  },
  "fileSearchPlaceholder": {
    "tr": "Dosya veya yol…",
    "en": "File or path…"
  },
  "fileStatus": {
    "tr": "Dosya durumu",
    "en": "File status"
  },
  "all": {
    "tr": "Tümü",
    "en": "All"
  },
  "workingFilter": {
    "tr": "Çalışma (W)",
    "en": "Working (W)"
  },
  "stagedFilter": {
    "tr": "Stage (S)",
    "en": "Staged (S)"
  },
  "conflicts": {
    "tr": "Çakışmalar",
    "en": "Conflicts"
  },
  "clearFileFilters": {
    "tr": "Dosya filtrelerini temizle",
    "en": "Clear file filters"
  },
  "visibleStageActions": {
    "tr": "Görünen dosyalar için stage işlemleri",
    "en": "Staging actions for visible files"
  },
  "allStageActions": {
    "tr": "Tüm dosyalar için stage işlemleri",
    "en": "Staging actions for all files"
  },
  "stageVisibleHelp": {
    "tr": "{count} görünen uygun dosyayı stage et{conflicts}",
    "en": "Stage eligible visible files: {count}{conflicts}"
  },
  "unstageVisibleHelp": {
    "tr": "{count} görünen uygun dosyayı unstage et{conflicts}",
    "en": "Unstage eligible visible files: {count}{conflicts}"
  },
  "excludingConflicts": {
    "tr": "; çakışmalı dosyalar hariç",
    "en": "; excluding conflicted files"
  },
  "stageVisible": {
    "tr": "Görünenleri stage et",
    "en": "Stage visible"
  },
  "stageAll": {
    "tr": "Tümünü stage et",
    "en": "Stage all"
  },
  "unstageVisible": {
    "tr": "Görünenleri unstage et",
    "en": "Unstage visible"
  },
  "unstageAll": {
    "tr": "Tümünü unstage et",
    "en": "Unstage all"
  },
  "selectCommitForFiles": {
    "tr": "Dosyalarını incelemek için geçmişten bir commit seçin.",
    "en": "Select a commit from history to review its files."
  },
  "status": {
    "tr": "Durum",
    "en": "Status"
  },
  "file": {
    "tr": "Dosya",
    "en": "File"
  },
  "path": {
    "tr": "Yol",
    "en": "Path"
  },
  "noMatchingFiles": {
    "tr": "Bu filtreyle eşleşen dosya yok.",
    "en": "No files match this filter."
  },
  "cleanWorkingTree": {
    "tr": "Çalışma ağacı temiz.",
    "en": "Working tree is clean."
  },
  "readingStatus": {
    "tr": "Durum okunuyor…",
    "en": "Reading status…"
  },
  "noCommitSelected": {
    "tr": "Commit seçilmedi.",
    "en": "No commit selected."
  },
  "readingCommitFiles": {
    "tr": "Commit dosyaları okunuyor…",
    "en": "Reading commit files…"
  },
  "noCommitChanges": {
    "tr": "Bu commit'te dosya değişikliği yok.",
    "en": "No file changes in this commit."
  },
  "repositoryOpenFailed": {
    "tr": "Depo açılamadı.",
    "en": "Could not open the repository."
  },
  "staged": {
    "tr": "Stage edilmiş",
    "en": "Staged"
  },
  "inWorkingTree": {
    "tr": "Çalışma ağacında",
    "en": "Working tree"
  },
  "historyScope": {
    "tr": "Geçmiş kapsamı",
    "en": "History view"
  },
  "commitMessage": {
    "tr": "Commit mesajı",
    "en": "Commit message"
  },
  "stagedFileSummary": {
    "tr": "{count} dosya stage edilmiş",
    "en": "Files staged: {count}"
  },
  "commitPlaceholder": {
    "tr": "Değişikliği kısaca açıklayın…",
    "en": "Briefly describe the change…"
  },
  "commitAllStagedHint": {
    "tr": "Filtre dışındakiler dahil tüm stage edilen dosyalar commit edilir.",
    "en": "All staged files are committed, including those outside the filter."
  },
  "commitStagedHint": {
    "tr": "Yalnızca stage edilen dosyalar commit edilir.",
    "en": "Only staged files are committed."
  },
  "committing": {
    "tr": "Commit ediliyor…",
    "en": "Committing…"
  },
  "pendingMerge": {
    "tr": "Merge bekliyor. Stage edilmiş değişiklik olmasa da commit ile tamamlayabilirsiniz.",
    "en": "A merge is pending. You can complete it with a commit even without staged changes."
  },
  "conflictWarning": {
    "tr": "{count} çakışmalı dosya var. Commit için çakışmaları çözün. Toplu işlemler bu dosyaları atlar.",
    "en": "Conflicted files: {count}. Resolve them before committing. Bulk actions skip these files."
  },
  "refreshingStatus": {
    "tr": "Depo durumu yeniden okunuyor…",
    "en": "Refreshing repository status…"
  },
  "creatingCommit": {
    "tr": "Commit oluşturuluyor…",
    "en": "Creating commit…"
  },
  "stagingFiles": {
    "tr": "Dosyalar stage ediliyor…",
    "en": "Staging files…"
  },
  "unstagingFiles": {
    "tr": "Dosyalar unstage ediliyor…",
    "en": "Unstaging files…"
  },
  "centerWidth": {
    "tr": "Geçmiş ve fark paneli genişliği",
    "en": "History and diff panel width"
  },
  "selectedFileDiff": {
    "tr": "Seçili dosya farkı",
    "en": "Selected file diff"
  },
  "indexDiffHeading": {
    "tr": "DİZİN · HEAD İLE KARŞILAŞTIRMA",
    "en": "INDEX · COMPARED WITH HEAD"
  },
  "workingDiffHeading": {
    "tr": "ÇALIŞMA AĞACI · DİZİN İLE KARŞILAŞTIRMA",
    "en": "WORKING TREE · COMPARED WITH INDEX"
  },
  "commitDiffHeading": {
    "tr": "COMMIT · İLK EBEVEYN İLE KARŞILAŞTIRMA",
    "en": "COMMIT · COMPARED WITH FIRST PARENT"
  },
  "rootDiffHeading": {
    "tr": "İLK COMMIT · BOŞ AĞAÇ İLE KARŞILAŞTIRMA",
    "en": "ROOT COMMIT · COMPARED WITH EMPTY TREE"
  },
  "noFileSelected": {
    "tr": "Dosya seçilmedi",
    "en": "No file selected"
  },
  "diffLayer": {
    "tr": "Fark katmanı",
    "en": "Diff layer"
  },
  "workingDiffHelp": {
    "tr": "Çalışma ağacı ile dizin arasındaki fark",
    "en": "Difference between the working tree and index"
  },
  "showWorkingDiff": {
    "tr": "Çalışma ağacı farkını göster",
    "en": "Show working tree diff"
  },
  "indexDiffHelp": {
    "tr": "Dizin ile HEAD arasındaki fark",
    "en": "Difference between the index and HEAD"
  },
  "showStagedDiff": {
    "tr": "Stage edilen farkı göster",
    "en": "Show staged diff"
  },
  "indexStatus": {
    "tr": "Dizin: {status}",
    "en": "Index: {status}"
  },
  "workingStatus": {
    "tr": "Çalışma ağacı: {status}",
    "en": "Working tree: {status}"
  },
  "unknownAuthor": {
    "tr": "Yazar bilinmiyor",
    "en": "Unknown author"
  },
  "selectedFileActions": {
    "tr": "Seçili dosyanın stage işlemleri",
    "en": "Staging actions for the selected file"
  },
  "selectedFile": {
    "tr": "Seçili dosya",
    "en": "Selected file"
  },
  "stageSelectedFile": {
    "tr": "Seçili dosyayı stage et",
    "en": "Stage selected file"
  },
  "unresolvedFile": {
    "tr": "Bu dosyada çözülmemiş çakışma var",
    "en": "This file has unresolved conflicts"
  },
  "stageSelectedHelp": {
    "tr": "Seçili dosyanın çalışma ağacı değişikliğini stage et",
    "en": "Stage the working tree changes in the selected file"
  },
  "stage": {
    "tr": "Stage et",
    "en": "Stage"
  },
  "unstageSelectedFile": {
    "tr": "Seçili dosyayı unstage et",
    "en": "Unstage selected file"
  },
  "unstageSelectedHelp": {
    "tr": "Seçili dosyayı unstage et; çalışma dosyasını koru",
    "en": "Unstage the selected file; keep the working file intact"
  },
  "unstage": {
    "tr": "Unstage et",
    "en": "Unstage"
  },
  "checkRepositoryPath": {
    "tr": "Depo yolunu kontrol edin veya başka bir depo açın.",
    "en": "Check the repository path or open another repository."
  },
  "enterRepositoryHint": {
    "tr": "Üstteki alana yerel Git deposunun yolunu yazıp Aç’a basın.",
    "en": "Enter a local Git repository path above and click Open."
  },
  "firstCommitNotCreated": {
    "tr": "İlk commit henüz oluşturulmadı.",
    "en": "The first commit has not been created yet."
  },
  "noChanges": {
    "tr": "Değişiklik yok. Çalışma ağacı temiz.",
    "en": "No changes. The working tree is clean."
  },
  "changeFilterHint": {
    "tr": "Filtreye uyan dosya yok. Aramayı veya durum filtresini değiştirin.",
    "en": "No files match. Change the search or status filter."
  },
  "selectHistoryCommit": {
    "tr": "Geçmişten bir commit seçin.",
    "en": "Select a commit from history."
  },
  "additions": {
    "tr": "Ekleme",
    "en": "Additions"
  },
  "deletions": {
    "tr": "Silme",
    "en": "Deletions"
  },
  "lineCount": {
    "tr": "{count} satır",
    "en": "Lines: {count}"
  },
  "waitingBranch": {
    "tr": "Dal bekleniyor",
    "en": "Waiting for branch"
  },
  "statusSummary": {
    "tr": "{working} çalışma ağacı · {staged} dizin",
    "en": "{working} working tree · {staged} index"
  },
  "noBranchCommits": {
    "tr": "Bu dalda commit yok",
    "en": "No commits on this branch"
  },
  "shortcuts": {
    "tr": "Ctrl+F dosya · Ctrl+Shift+F commit · Ctrl+R yenile",
    "en": "Ctrl+F files · Ctrl+Shift+F commits · Ctrl+R refresh"
  },
  "layoutSaveHelp": {
    "tr": "Düzen bu oturumda geçerli; ayarlar kaydedilemiyor.",
    "en": "Layout applies to this session; settings could not be saved."
  },
  "layoutNotSaved": {
    "tr": "Düzen kaydedilemedi; bu oturumda geçerli.",
    "en": "Layout not saved; applies to this session."
  },
  "clean": {
    "tr": "temiz",
    "en": "clean"
  },
  "indexDiff": {
    "tr": "Dizin farkı",
    "en": "Index diff"
  },
  "workingDiff": {
    "tr": "Çalışma ağacı farkı",
    "en": "Working tree diff"
  },
  "retry": {
    "tr": "Yeniden dene",
    "en": "Retry"
  },
  "loadingHistory": {
    "tr": "Geçmiş yükleniyor",
    "en": "Loading history"
  },
  "language": {
    "tr": "Arayüz dili",
    "en": "Interface language"
  },
  "languageNotSaved": {
    "tr": "Dil tercihi kaydedilemedi; bu oturumda geçerli.",
    "en": "Language preference not saved; applies to this session."
  }
} satisfies MessageCatalog;
