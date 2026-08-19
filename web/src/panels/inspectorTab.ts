/**
 * Which inspector tab is showing (Inspect · Params · Text). UI-only state
 * shared by the inspector, the "show in text" actions, and the settings
 * menu's text-panel toggle — kept out of the frozen store.
 */
import { create } from "zustand";

export type InspectorTab = "inspect" | "params" | "text";

interface InspectorTabState {
  tab: InspectorTab;
  setTab: (tab: InspectorTab) => void;
}

export const useInspectorTab = create<InspectorTabState>((set) => ({
  tab: "inspect",
  setTab: (tab) => set({ tab }),
}));
