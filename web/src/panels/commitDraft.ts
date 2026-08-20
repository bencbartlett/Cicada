/**
 * The commit message being written — shared by the Git tab's form and the
 * Ctrl+S dialog so a message started in one is there in the other, and
 * survives a tab switch. UI-only; cleared when a commit lands.
 */
import { create } from "zustand";

interface CommitDraftState {
  draft: string;
  setDraft: (draft: string) => void;
  clear: () => void;
}

export const useCommitDraft = create<CommitDraftState>((set) => ({
  draft: "",
  setDraft: (draft) => set({ draft }),
  clear: () => set({ draft: "" }),
}));
