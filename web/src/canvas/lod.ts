import { useStore } from "@xyflow/react";
import { lodTier, type LodTier } from "./grid";

/** The current zoom LOD tier (docs/16); re-renders only when the tier flips. */
export function useLodTier(): LodTier {
  return useStore((s) => lodTier(s.transform[2]));
}
