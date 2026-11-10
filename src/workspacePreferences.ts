export type WorkspaceDensity = "compact" | "comfortable";
export interface PanelWidths { sidebar: number; center: number }
export interface WorkspacePreferences {
  density: WorkspaceDensity;
  sidebarCollapsed: boolean;
  panelWidths: PanelWidths;
}

export const workspacePreferencesKey = "enogram.workspace.v1";
export const workspaceLimits = { sidebarMin: 196, sidebarMax: 380, centerMin: 370, centerMax: 1600, diffMin: 320, collapsedSidebar: 54, handle: 6 };
export const defaultWorkspacePreferences: WorkspacePreferences = {
  density: "compact", sidebarCollapsed: false, panelWidths: { sidebar: 224, center: 470 },
};

function boundedWidth(value: unknown, fallback: number, min: number, max: number): number {
  return typeof value === "number" && Number.isFinite(value) ? Math.round(Math.max(min, Math.min(max, value))) : fallback;
}

export function normalizeWorkspacePreferences(value: unknown): WorkspacePreferences {
  const input = value && typeof value === "object" ? value as Partial<WorkspacePreferences> : {};
  return {
    density: input.density === "comfortable" ? "comfortable" : "compact",
    sidebarCollapsed: input.sidebarCollapsed === true,
    panelWidths: {
      sidebar: boundedWidth(input.panelWidths?.sidebar, defaultWorkspacePreferences.panelWidths.sidebar, workspaceLimits.sidebarMin, workspaceLimits.sidebarMax),
      center: boundedWidth(input.panelWidths?.center, defaultWorkspacePreferences.panelWidths.center, workspaceLimits.centerMin, workspaceLimits.centerMax),
    },
  };
}

export function loadWorkspacePreferences(storage?: Pick<Storage, "getItem">): WorkspacePreferences {
  try { return normalizeWorkspacePreferences(JSON.parse(storage?.getItem(workspacePreferencesKey) ?? "null")); }
  catch { return normalizeWorkspacePreferences(null); }
}

export function saveWorkspacePreferences(settings: WorkspacePreferences, storage?: Pick<Storage, "setItem">): boolean {
  try {
    if (!storage) return false;
    storage.setItem(workspacePreferencesKey, JSON.stringify(normalizeWorkspacePreferences(settings)));
    return true;
  } catch { return false; }
}

// Fitting a temporary narrow window must not overwrite the preferred layout.
export function fitPanelWidths(preferred: PanelWidths, available: number, collapsed: boolean): PanelWidths {
  const { sidebarMin, sidebarMax, centerMin, diffMin, collapsedSidebar, handle } = workspaceLimits;
  const sidebar = Math.max(sidebarMin, Math.min(preferred.sidebar, sidebarMax, available - centerMin - diffMin - handle * 2));
  const center = Math.max(centerMin, Math.min(preferred.center, available - (collapsed ? collapsedSidebar : sidebar) - diffMin - handle * 2));
  return { sidebar, center };
}
