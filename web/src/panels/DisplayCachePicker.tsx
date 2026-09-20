/**
 * The display cache's budget control (docs/16 §Settings; the D1 contract):
 * a `<select>` over the offered sizes and the session's current budget
 * beside it. A per-user choice the WRITER applies to the session now and on
 * every connect (`store.chooseDisplayCache`); an observer's is kept for when
 * it holds the lease. Shared by the settings menu and the profiler's caches
 * section (v0.1 wave 5 P1).
 */
import { DISPLAY_CACHE_CHOICES_MIB } from "../protocol/messages";
import { canWrite, useCicada } from "../state/store";
import { displayCacheLabel, shortBytes } from "./format";

export function DisplayCachePicker({ testId = "settings-display-cache" }: { testId?: string }) {
  const settings = useCicada((s) => s.settings);
  const chooseDisplayCache = useCicada((s) => s.chooseDisplayCache);
  const caches = useCicada((s) => s.caches);
  const writer = useCicada(canWrite);
  return (
    <span className="tb-cache-pick">
      <select
        value={settings.displayCacheMib === null ? "" : String(settings.displayCacheMib)}
        onChange={(e) => chooseDisplayCache(e.target.value === "" ? null : Number(e.target.value))}
        title={
          writer
            ? "resizes this session's display cache now and on every connect (the lease holder's preference wins)"
            : "kept for when you hold the write lease — only the lease holder resizes the session's cache"
        }
        data-testid={testId}
      >
        <option value="">server default</option>
        {DISPLAY_CACHE_CHOICES_MIB.map((mib) => (
          <option key={mib} value={String(mib)}>
            {displayCacheLabel(mib)}
          </option>
        ))}
      </select>
      <span className="faint" data-testid={`${testId}-now`}>
        {caches === null ? "session: …" : `session: ${shortBytes(caches.display.budget)}`}
      </span>
    </span>
  );
}
