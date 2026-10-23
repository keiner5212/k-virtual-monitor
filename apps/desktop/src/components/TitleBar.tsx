import { getCurrentWindow } from "@tauri-apps/api/window";

interface TitleBarProps {
    serverName: string;
}

const buttonClass =
    "no-drag flex h-10 w-11 items-center justify-center text-ink-muted transition-colors duration-150 hover:bg-white/5 hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent focus-visible:ring-inset";

export default function TitleBar({ serverName }: TitleBarProps) {
    // The window is only available inside the Tauri webview. Reading it
    // lazily keeps the screen renderable in a plain browser, which is how the
    // layout is reviewed.
    const appWindow = () => getCurrentWindow();

    return (
        <header className="drag-region flex h-10 shrink-0 items-center justify-between border-b border-border-subtle bg-surface-raised pl-4 pr-1">
            <div className="drag-region flex items-center gap-2.5">
                <div className="flex h-5 w-5 items-center justify-center rounded-md bg-accent-soft text-[11px] font-bold text-accent">
                    K
                </div>
                <span className="font-sans text-xs font-medium text-ink">k-virtual-monitor</span>
                <span className="h-3 w-px bg-border-subtle" />
                <span className="hidden font-sans text-xs text-ink-muted min-[680px]:inline">{serverName}</span>
            </div>

            <div className="no-drag flex items-center">
                <button
                    type="button"
                    className={buttonClass}
                    aria-label="Minimize window"
                    onClick={() => void appWindow().minimize()}
                >
                    <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
                        <path d="M2 6h8" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
                    </svg>
                </button>
                <button
                    type="button"
                    className={buttonClass}
                    aria-label="Maximize window"
                    onClick={() => void appWindow().toggleMaximize()}
                >
                    <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
                        <rect
                            x="2.5"
                            y="2.5"
                            width="7"
                            height="7"
                            rx="1"
                            fill="none"
                            stroke="currentColor"
                            strokeWidth="1.5"
                        />
                    </svg>
                </button>
                <button
                    type="button"
                    className={`${buttonClass} hover:bg-danger hover:text-surface`}
                    aria-label="Close window"
                    onClick={() => void appWindow().close()}
                >
                    <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
                        <path
                            d="M3 3l6 6M9 3l-6 6"
                            stroke="currentColor"
                            strokeWidth="1.5"
                            strokeLinecap="round"
                        />
                    </svg>
                </button>
            </div>
        </header>
    );
}
