/**
 * A minimal context menu (docs/16 §Context menus). Positioned at the pointer
 * (pane-relative), closes on outside press / Esc / any item.
 */
import { useEffect, useRef } from "react";

export interface MenuItem {
  label: string;
  onClick?: () => void;
  disabled?: boolean;
  danger?: boolean;
  /** Visual separator above this item. */
  separator?: boolean;
  hint?: string;
}

interface Props {
  left: number;
  top: number;
  title?: string;
  items: MenuItem[];
  onClose: () => void;
}

export function ContextMenu({ left, top, title, items, onClose }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const onDown = (event: PointerEvent) => {
      if (ref.current !== null && !ref.current.contains(event.target as Node)) onClose();
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("pointerdown", onDown, true);
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("pointerdown", onDown, true);
      window.removeEventListener("keydown", onKey, true);
    };
  }, [onClose]);

  return (
    <div
      ref={ref}
      className="cv-menu nodrag nopan nowheel"
      style={{ left, top }}
      role="menu"
      onContextMenu={(event) => event.preventDefault()}
      data-testid="context-menu"
    >
      {title && <div className="cv-menu-title mono">{title}</div>}
      {items.map((item, index) => (
        <button
          type="button"
          key={index}
          role="menuitem"
          className={`cv-menu-item${item.danger ? " danger" : ""}${item.separator ? " sep" : ""}`}
          disabled={item.disabled}
          title={item.hint}
          onClick={() => {
            onClose();
            item.onClick?.();
          }}
        >
          <span>{item.label}</span>
          {item.hint && <span className="cv-menu-hint faint">{item.hint}</span>}
        </button>
      ))}
    </div>
  );
}
