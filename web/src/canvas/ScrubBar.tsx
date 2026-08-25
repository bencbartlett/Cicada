/**
 * The buffer bar under a scrub-cached slider's track (docs/16 §Sliders,
 * docs/12 §Speculative warming; v0.1 item 5 S2) — ONE component for both
 * slider widgets (the canvas `ParamWidget`, the params panel): a segment per
 * step position, the warm ones filled, the position the thumb is on marked,
 * a subtle pulse on the cold segments while the server's worker still has
 * work (`warming`), and the cold segments in the warn hue when the byte cap
 * stopped it (`capped`). Everything it draws is the server's: the slider's
 * `param.scrub` from the last snapshot / delta merged with the
 * `scrub_progress` overlay the store keeps between them (`mergeScrub`);
 * observers see the same from the broadcast. Nothing when the slider is
 * not scrub-cached or cannot be (`showsScrubBar`).
 *
 * The data attributes (`data-positions`, `data-warmed`, `data-warming`,
 * `data-current`, `data-capped`) are the oracle the e2e reads; the
 * segments are the picture.
 */
import type { ScrubView } from "../protocol/messages";
import { currentPosition, scrubBarTitle, showsScrubBar } from "../state/scrub";
import "./scrub.css";

interface Props {
  node: string;
  /** The merged view (`mergeScrub(param.scrub, progress)`); undefined / off / ineligible draw nothing. */
  scrub: ScrubView | undefined;
  /** The value the thumb shows right now (a drag's own value, the pending value, or the committed one). */
  value: number;
  min: number;
  step: number;
}

export function ScrubBar({ node, scrub, value, min, step }: Props) {
  if (!showsScrubBar(scrub)) return null;
  const warm = new Set(scrub.warmed);
  const current = currentPosition(value, min, step, scrub.positions);
  const title = scrubBarTitle(scrub);
  const classes = ["scrub-bar"];
  if (scrub.warming) classes.push("warming");
  if (scrub.capped === true) classes.push("capped");
  return (
    <span
      className={classes.join(" ")}
      role="img"
      aria-label={title}
      title={title}
      data-testid={`scrub-bar-${node}`}
      data-positions={scrub.positions}
      data-warmed={scrub.warmed.length}
      data-warming={scrub.warming}
      data-current={current}
      data-capped={scrub.capped === true ? "true" : undefined}
    >
      {Array.from({ length: scrub.positions }, (_, i) => (
        <i
          key={i}
          className={`scrub-seg${warm.has(i) ? " warm" : ""}${i === current ? " current" : ""}`}
          data-index={i}
        />
      ))}
    </span>
  );
}
