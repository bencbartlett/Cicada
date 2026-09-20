/**
 * The tooltip box (docs/16 §Theme and visual language; docs/17 wave 5 T1):
 * what `installTooltips` reports, rendered once at the root of every screen
 * (`Root`) — a themed box with the hovered element's title, placed by
 * `placeTooltip` below the element (above when there is no room) — below
 * the pointer for a wire or a profiler arc, whose text is an SVG `<title>`
 * child and whose box is no edge to sit under — clamped to the viewport,
 * over every other layer (a modal's backdrop included: the × of About
 * carries a title too) and transparent to the pointer. Named
 * after the component, not `Tooltip.tsx`: that name differs from
 * `tooltip.ts` only in case, and a case-insensitive file system (Windows,
 * macOS) resolves `./Tooltip` to the controller's file.
 */
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { anchorRect, installTooltips, placeTooltip, type TooltipShown } from "./tooltip";

export function TooltipLayer() {
  const [shown, setShown] = useState<TooltipShown | null>(null);
  const boxRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const tooltips = installTooltips(document);
    const unsubscribe = tooltips.subscribe(setShown);
    return () => {
      unsubscribe();
      tooltips.dispose();
    };
  }, []);

  // Place before paint. Measured at the origin: a fixed box left where the
  // previous tooltip sat would be squeezed against the viewport's edge and
  // measure narrower than it will be once moved.
  useLayoutEffect(() => {
    const box = boxRef.current;
    if (shown === null || box === null) return;
    box.style.left = "0px";
    box.style.top = "0px";
    const placed = placeTooltip(anchorRect(shown), box.getBoundingClientRect(), {
      width: window.innerWidth,
      height: window.innerHeight,
    });
    box.style.left = `${placed.left}px`;
    box.style.top = `${placed.top}px`;
    box.dataset.side = placed.side;
  }, [shown]);

  if (shown === null) return null;
  return (
    <div ref={boxRef} className="tooltip" role="tooltip" data-testid="tooltip">
      {shown.text}
    </div>
  );
}
