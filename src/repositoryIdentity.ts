// Repository paths are canonicalized by the backend. This key only reconciles
// Windows display spellings; it does not resolve relative paths or symlinks.
export function repositoryIdentity(path: string): string {
  const windowsPath = path.replace(/\\/g, "/")
    .replace(/^\/\/\?\/UNC\//i, "//")
    .replace(/^\/\/\?\/(?=[a-z]:\/)/i, "");

  if (/^[a-z]:\//i.test(windowsPath)) {
    const normalized = windowsPath.replace(/\/+/g, "/").toLowerCase();
    // A drive root must remain absolute rather than becoming the relative C:.
    return normalized.length === 3 ? normalized : normalized.replace(/\/$/, "");
  }

  if (/^\/\/(?![?.](?:\/|$))[^/]+\/+[^/]+(?:\/|$)/.test(windowsPath)) {
    return `//${windowsPath.slice(2).replace(/\/+/g, "/").replace(/\/$/, "").toLowerCase()}`;
  }

  return path;
}
