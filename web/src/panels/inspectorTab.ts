/**
 * Which inspector tab is showing (Inspect · Params · Profile · Git · Text).
 * UI-only state shared by the inspector, the "show in text" actions, the top
 * bar's git chip and profile button, the caches indicator, the settings
 * menu's text-panel toggle and the keyboard map (Esc closes the profiler) —
 * kept out of the frozen store.
 */
import { create } from "zustand";

export type InspectorTab = "inspect" | "params" | "profile" | "git" | "text";

/** A section of the profiler a caller wants scrolled into view when the tab opens (the caches indicator's click). */
export type ProfileSection = "caches";

interface InspectorTabState {
  tab: InspectorTab;
  /** Set by `openProfile(section)`, consumed once by the profiler when it has rendered the section. */
  profileFocus: ProfileSection | null;
  setTab: (tab: InspectorTab) => void;
  /** Open the profiler, optionally on one of its sections (v0.1 wave 5 P1: the caches indicator's click lands on the caches section). */
  openProfile: (section?: ProfileSection) => void;
  /** The profiler scrolled the requested section into view. */
  consumeProfileFocus: () => void;
}

export const useInspectorTab = create<InspectorTabState>((set) => ({
  tab: "inspect",
  profileFocus: null,
  setTab: (tab) => set({ tab }),
  openProfile: (section) => set({ tab: "profile", profileFocus: section ?? null }),
  consumeProfileFocus: () => set({ profileFocus: null }),
}));
