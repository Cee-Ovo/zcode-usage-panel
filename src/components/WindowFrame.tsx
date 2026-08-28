import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect } from "react";

const win = () => getCurrentWindow();

/** Custom title bar: `data-tauri-drag-region` gives native drag-to-move
 *  (double-click toggles maximize/restore). Window-control buttons sit on
 *  top and keep their own click handling. */
export function TitleBar({
  title,
  onRefresh,
}: {
  title: string;
  onRefresh?: () => void;
}) {
  return (
    <div className="zup-titlebar" data-tauri-drag-region>
      <svg width="16" height="16" viewBox="0 0 16 16" aria-hidden>
        <defs>
          <linearGradient id="zup-logo" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#7fd4ff" />
            <stop offset="1" stopColor="#2f7bf6" />
          </linearGradient>
        </defs>
        <rect x="1.5" y="1.5" width="13" height="13" rx="4" fill="url(#zup-logo)" opacity="0.9" />
        <path
          d="M5 5.4h6L5 10.6h6"
          fill="none"
          stroke="#fff"
          strokeWidth="1.6"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      </svg>
      <span className="title" data-tauri-drag-region>
        {title}
      </span>
      <span className="spacer" data-tauri-drag-region />
      <button
        className="zup-nav-item"
        title="立即刷新"
        onClick={() => onRefresh?.()}
        style={{ padding: "4px 10px" }}
      >
        <RefreshIcon />
      </button>
      <button
        className="zup-nav-item"
        title="最小化"
        onClick={() => win().minimize()}
        style={{ padding: "4px 10px" }}
      >
        <MinIcon />
      </button>
      <button
        className="zup-nav-item"
        title="最小化到托盘"
        onClick={() =>
          import("../lib/ipc")
            .then(({ api }) => api.hideMainWindow())
            .catch(() => win().hide())
        }
        style={{ padding: "4px 10px" }}
      >
        <TrayIcon />
      </button>
    </div>
  );
}

/**
 * Window frame: registers the drag region and the 8-direction resize edges.
 * `data-tauri-drag-region` gives us native window dragging; each edge calls
 * Tauri's `startResizeDragging`, which hands the gesture to the OS — the
 * resulting cursor, snapping and DPI behavior are native Win32.
 */
export function WindowFrame() {
  useEffect(() => {
    // Tell the docking engine whether the pointer is over the webview, and
    // whether any overlay (menu/tooltip/select) is open — suppresses
    // auto-hide while the user interacts.
    let inside = false;
    let interacting = false;
    const send = () => {
      import("../lib/ipc").then(({ api }) => {
        api.dockHover(inside).catch(() => {});
        api.dockInteract(interacting).catch(() => {});
      });
    };
    const onEnter = () => {
      inside = true;
      send();
    };
    const onLeave = () => {
      inside = false;
      send();
    };
    const isOverlay = (el: EventTarget | null) => {
      if (!(el instanceof Element)) return false;
      return el.closest(
        '[role="menu"],[role="dialog"],[role="listbox"],[role="combobox"],[role="tooltip"],[data-ogui-overlay],select,option',
      ) !== null;
    };
    const onFocusIn = (e: FocusEvent) => {
      const now = isOverlay(e.target);
      if (now !== interacting) {
        interacting = now;
        send();
      }
    };
    document.addEventListener("mouseenter", onEnter);
    document.addEventListener("mouseleave", onLeave);
    document.addEventListener("focusin", onFocusIn);
    document.addEventListener("focusout", onFocusIn);
    return () => {
      document.removeEventListener("mouseenter", onEnter);
      document.removeEventListener("mouseleave", onLeave);
      document.removeEventListener("focusin", onFocusIn);
      document.removeEventListener("focusout", onFocusIn);
    };
  }, []);

  const startResize = (dir: string) => (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    e.preventDefault();
    getCurrentWindow().startResizeDragging(dir as never).catch(() => {});
  };

  const edge = (dir: string, style: React.CSSProperties, cursor: string): React.CSSProperties => ({
    position: "fixed",
    zIndex: 90,
    cursor,
    ...style,
  });

  return (
    <>
      <div
        onPointerDown={startResize("North")}
        style={edge("North", { top: 0, left: 8, right: 8, height: 5 }, "ns-resize")}
      />
      <div
        onPointerDown={startResize("South")}
        style={edge("South", { bottom: 0, left: 8, right: 8, height: 5 }, "ns-resize")}
      />
      <div
        onPointerDown={startResize("West")}
        style={edge("West", { left: 0, top: 8, bottom: 8, width: 5 }, "ew-resize")}
      />
      <div
        onPointerDown={startResize("East")}
        style={edge("East", { right: 0, top: 8, bottom: 8, width: 5 }, "ew-resize")}
      />
      <div
        onPointerDown={startResize("NorthWest")}
        style={edge("NorthWest", { top: 0, left: 0, width: 10, height: 10 }, "nwse-resize")}
      />
      <div
        onPointerDown={startResize("NorthEast")}
        style={edge("NorthEast", { top: 0, right: 0, width: 10, height: 10 }, "nesw-resize")}
      />
      <div
        onPointerDown={startResize("SouthWest")}
        style={edge("SouthWest", { bottom: 0, left: 0, width: 10, height: 10 }, "nesw-resize")}
      />
      <div
        onPointerDown={startResize("SouthEast")}
        style={edge("SouthEast", { bottom: 0, right: 0, width: 10, height: 10 }, "nwse-resize")}
      />
    </>
  );
}

function MinIcon() {
  return (
    <svg width="11" height="11" viewBox="0 0 11 11">
      <path d="M1 5.5h9" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" />
    </svg>
  );
}

function TrayIcon() {
  return (
    <svg width="12" height="12" viewBox="0 0 12 12">
      <path
        d="M1.5 8.5h9v1.5a1 1 0 0 1-1 1h-7a1 1 0 0 1-1-1z"
        fill="currentColor"
        opacity="0.85"
      />
      <path d="M3.5 5.5h5" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" />
    </svg>
  );
}

function RefreshIcon() {
  return (
    <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
      <polyline
        points="11.5 2 11.5 5 8.5 5"
        stroke="currentColor"
        strokeWidth="1.2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <polyline
        points="0.5 10 0.5 7 3.5 7"
        stroke="currentColor"
        strokeWidth="1.2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path
        d="M1.755 4.5a4.5 4.5 0 0 1 5.67-1.68L11.5 5"
        stroke="currentColor"
        strokeWidth="1.2"
        strokeLinecap="round"
      />
      <path
        d="M0.5 7l2.32 2.18a4.5 4.5 0 0 0 7.425-1.68"
        stroke="currentColor"
        strokeWidth="1.2"
        strokeLinecap="round"
      />
    </svg>
  );
}
