import { useCallback, useEffect, useMemo, useState } from "react";
import QRCode from "react-qr-code";

import { FRAME_RATES, RESOLUTIONS, type Resolution } from "@kvm/protocol";

import CountUp from "./components/react-bits/CountUp";
import SplitText from "./components/react-bits/SplitText";
import SpotlightCard from "./components/react-bits/SpotlightCard";
import StatusMark from "./components/react-bits/StatusMark";
import TitleBar from "./components/TitleBar";
import {
    createPairing,
    currentPairing,
    getConfig,
    pairedDevices,
    revokeDevice,
    restartCapture,
    saveConfig,
    saveDeviceSettings,
    serverStatus,
    type DeviceInfo,
    type PairingInfo,
    type ServerConfig,
    type ServerStatus,
} from "./lib/api";

const POLL_MS = 2000;

type SaveState = "idle" | "saving" | "saved" | "failed";

const inputClass =
    "h-9 w-full rounded-lg border border-border-subtle bg-surface px-3 font-mono text-sm text-ink transition-colors hover:border-ink-muted/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent focus-visible:ring-offset-2 focus-visible:ring-offset-surface-raised";

const labelClass = "mb-1.5 block font-sans text-xs font-medium text-ink-muted";

/**
 * A dropdown that looks like the inputs. The open list is drawn by the
 * operating system, so only the closed control can be styled.
 */
const selectClass =
    "appearance-none bg-[length:0.75rem] bg-[position:right_0.625rem_center] bg-no-repeat pr-8";

const selectChevron =
    "url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 12 12' fill='none' stroke='%238b93a7' stroke-width='1.5' stroke-linecap='round' stroke-linejoin='round'%3E%3Cpath d='M3 4.5 6 7.5 9 4.5'/%3E%3C/svg%3E\")";

const primaryButton =
    "inline-flex h-9 items-center gap-2 rounded-lg bg-accent px-4 font-sans text-sm font-medium text-surface transition-colors hover:bg-accent/90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent focus-visible:ring-offset-2 focus-visible:ring-offset-surface-raised active:scale-[0.98]";

const secondaryButton =
    "inline-flex h-9 items-center rounded-lg border border-border-subtle bg-surface-raised px-4 font-sans text-sm font-medium text-ink transition-colors hover:border-ink-muted/60 hover:bg-white/5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent focus-visible:ring-offset-2 focus-visible:ring-offset-surface-raised";

const ghostButton =
    "inline-flex h-9 items-center rounded-lg px-3 font-sans text-sm text-ink-muted transition-colors hover:bg-white/5 hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent focus-visible:ring-offset-2 focus-visible:ring-offset-surface-raised";

function formatCountdown(seconds: number): string {
    const minutes = Math.floor(seconds / 60);
    const rest = seconds % 60;
    return `${minutes}:${rest.toString().padStart(2, "0")}`;
}

/** The sizes offered for a phone. The one it has now is always in the list. */
function resolutionOptions(device: DeviceInfo): Resolution[] {
    const known = RESOLUTIONS.some(
        (option) => option.width === device.width && option.height === device.height,
    );
    if (known) {
        return RESOLUTIONS;
    }
    return [
        { label: `${device.width} x ${device.height}`, width: device.width, height: device.height },
        ...RESOLUTIONS,
    ];
}

/** The frame rates offered for a phone, with the one it has now in the list. */
function frameRateOptions(fps: number): number[] {
    if (FRAME_RATES.includes(fps)) {
        return FRAME_RATES;
    }
    return [...FRAME_RATES, fps].sort((left, right) => left - right);
}

function StatTile({ label, children }: { label: string; children: React.ReactNode }) {
    return (
        <div className="rounded-panel border border-border-subtle bg-surface-raised px-4 py-3">
            <p className="font-sans text-xs uppercase tracking-[0.08em] text-ink-muted">{label}</p>
            <div className="mt-1 font-mono text-xl tabular-nums text-ink">{children}</div>
        </div>
    );
}

export default function App() {
    const [status, setStatus] = useState<ServerStatus | null>(null);
    const [config, setConfig] = useState<ServerConfig | null>(null);
    const [draft, setDraft] = useState<ServerConfig | null>(null);
    const [devices, setDevices] = useState<DeviceInfo[]>([]);
    const [pairing, setPairing] = useState<PairingInfo | null>(null);
    const [now, setNow] = useState(() => Date.now());
    const [error, setError] = useState<string | null>(null);
    const [saveState, setSaveState] = useState<SaveState>("idle");
    const [pickedId, setPickedId] = useState<string | null>(null);
    const [editedDevice, setEditedDevice] = useState<DeviceInfo | null>(null);
    const selectedId = devices.some((device) => device.id === pickedId)
        ? pickedId
        : (devices[0]?.id ?? null);
    const selectedDevice = devices.find((device) => device.id === selectedId) ?? null;
    const deviceDraft = editedDevice?.id === selectedId ? editedDevice : selectedDevice;

    const refresh = useCallback(async () => {
        try {
            const [nextStatus, nextDevices, nextPairing] = await Promise.all([
                serverStatus(),
                pairedDevices(),
                currentPairing(),
            ]);
            setStatus(nextStatus);
            setDevices(nextDevices);
            setPairing((current) => {
                if (!nextPairing) {
                    return null;
                }
                if (!current) {
                    return nextPairing;
                }
                return nextPairing.expiresAt >= current.expiresAt ? nextPairing : current;
            });
            setError(null);
        } catch (cause) {
            setError(cause instanceof Error ? cause.message : String(cause));
        }
    }, []);

    useEffect(() => {
        const initial = setTimeout(() => void refresh(), 0);
        const timer = setInterval(() => void refresh(), POLL_MS);
        return () => {
            clearTimeout(initial);
            clearInterval(timer);
        };
    }, [refresh]);

    useEffect(() => {
        let cancelled = false;
        void getConfig()
            .then((loaded) => {
                if (cancelled) {
                    return;
                }
                setConfig(loaded);
                setDraft(loaded);
            })
            .catch((cause: unknown) => {
                setError(cause instanceof Error ? cause.message : String(cause));
            });
        return () => {
            cancelled = true;
        };
    }, []);

    useEffect(() => {
        const timer = setInterval(() => setNow(Date.now()), 1000);
        return () => clearInterval(timer);
    }, []);

    const pair = useCallback(async () => {
        setError(null);
        try {
            const created = await createPairing();
            setPairing(created);
        } catch (cause) {
            setError(cause instanceof Error ? cause.message : String(cause));
        }
    }, []);

    const revoke = useCallback(async (id: string) => {
        try {
            await revokeDevice(id);
            await refresh();
        } catch (cause) {
            setError(cause instanceof Error ? cause.message : String(cause));
        }
    }, [refresh]);

    const copyBlob = useCallback(async () => {
        if (!pairing) {
            return;
        }
        try {
            await navigator.clipboard.writeText(pairing.blob);
        } catch (cause) {
            setError(cause instanceof Error ? cause.message : String(cause));
        }
    }, [pairing]);

    const save = useCallback(async () => {
        if (!draft) {
            return;
        }
        setSaveState("saving");
        try {
            const saved = await saveConfig(draft);
            setConfig(saved);
            setDraft(saved);
            setSaveState("saved");
        } catch (cause) {
            setSaveState("failed");
            setError(cause instanceof Error ? cause.message : String(cause));
        }
    }, [draft]);

    const saveDevice = useCallback(async () => {
        if (!deviceDraft) {
            return;
        }
        setError(null);
        try {
            await saveDeviceSettings({
                id: deviceDraft.id,
                width: deviceDraft.width,
                height: deviceDraft.height,
                fps: deviceDraft.fps,
                jpegQuality: deviceDraft.jpegQuality,
            });
            setEditedDevice(null);
            await refresh();
        } catch (cause) {
            setError(cause instanceof Error ? cause.message : String(cause));
        }
    }, [deviceDraft, refresh]);

    const serverName = config?.server_name ?? "k-virtual-monitor";
    const live = Boolean(status?.sessions);
    const remaining = pairing
        ? Math.max(0, Math.round((pairing.expiresAt * 1000 - now) / 1000))
        : 0;
    const expired = pairing !== null && remaining <= 0;

    const statusView = useMemo(() => {
        if (!status?.running) {
            return { mark: "pending" as const, color: "var(--color-ink-muted)", label: "Server stopped" };
        }
        if (live) {
            return { mark: "running" as const, color: "var(--color-ok)", label: "Streaming" };
        }
        return { mark: "running" as const, color: "var(--color-accent)", label: "Waiting for a phone" };
    }, [status?.running, live]);

    return (
        <div className="flex h-screen flex-col overflow-hidden bg-surface text-ink">
            <TitleBar serverName={serverName} />

            <div className="min-h-0 flex-1 overflow-y-auto">
                <section className="grid grid-cols-1 gap-6 px-4 pt-5 pb-4 lg:grid-cols-[minmax(0,1fr)_20rem] lg:px-6">
                    <div>
                        <SplitText
                            text={serverName}
                            tag="h1"
                            effect="rise"
                            splitType="chars"
                            trigger="load"
                            duration={0.6}
                            delay={28}
                            staggerFrom="start"
                            mask
                            className="font-sans text-2xl font-semibold tracking-[-0.02em] text-ink"
                        />
                        <div
                            role="status"
                            aria-live="polite"
                            className="mt-3 flex items-center gap-2 font-sans text-sm text-ink-muted"
                        >
                            <StatusMark status={statusView.mark} color={statusView.color} size={18} strokeWidth={2} />
                            <span>{statusView.label}</span>
                            {status?.deviceName ? <span className="text-ink-muted">· {status.deviceName}</span> : null}
                        </div>
                        <p className="mt-1 font-mono text-sm text-ink-muted">
                            {status ? `${status.address}:${status.port}` : "not configured"}
                        </p>
                        {status?.capture ? (
                            <p className="mt-2 font-sans text-sm text-ink">{status.capture}</p>
                        ) : null}
                        {status?.capture.startsWith("Virtual screen was not created") ? (
                            <button type="button" className={`${secondaryButton} mt-3`} onClick={() => void restartCapture()}>
                                Try again
                            </button>
                        ) : null}
                    </div>

                    <div className={`grid grid-cols-2 gap-3 ${status?.running ? "" : "opacity-60"}`}>
                        <StatTile label="Sessions">
                            <CountUp
                                to={status?.sessions ?? 0}
                                from={0}
                                duration={0.5}
                                mode="count"
                                trigger="load"
                                easing="easeOut"
                            />
                        </StatTile>
                        <StatTile label="Paired">
                            <CountUp
                                to={devices.length}
                                from={0}
                                duration={0.5}
                                mode="count"
                                trigger="load"
                                easing="easeOut"
                            />
                        </StatTile>
                        <StatTile label="Stream">{status?.codec ?? (status?.running ? "Waiting" : "Idle")}</StatTile>
                        <StatTile label="Frame rate">
                            {status?.fps ? `${status.fps} fps` : status?.running ? "--" : "0 fps"}
                        </StatTile>
                    </div>
                </section>

                {error ? (
                    <p role="alert" className="mx-4 mb-4 rounded-lg border border-danger/40 px-3 py-2 font-sans text-sm text-danger lg:mx-6">
                        {error}
                    </p>
                ) : null}

                <section className="mx-4 mb-4 rounded-panel border border-border-subtle bg-surface-raised p-5 lg:mx-6">
                    <h2 className="font-sans text-base font-semibold text-ink">Pair a phone</h2>
                    <p className="mt-1 font-sans text-sm text-ink-muted">
                        Scan the pairing code with the phone app. The six digits are a check: the phone shows the same digits after the scan. If they differ, a different device scanned the code.
                    </p>

                    {live ? (
                        <p className="mt-4 font-sans text-sm text-ink">The phone is connected.</p>
                    ) : pairing && !expired ? (
                        <div className="mt-4">
                            <SpotlightCard
                                className="relative overflow-hidden rounded-panel border border-accent-soft bg-surface p-6 text-center"
                                spotlightColor="rgba(109, 139, 255, 0.18)"
                                ambient={false}
                                borderGlow={0}
                                intensity={0.5}
                            >
                                <p className="font-sans text-xs uppercase tracking-[0.12em] text-ink-muted">
                                    Scan this code with the phone
                                </p>
                                <div className="mt-4 flex justify-center">
                                    <div className="rounded-lg bg-white p-3">
                                        <QRCode
                                            value={pairing.blob}
                                            size={336}
                                            level="L"
                                            bgColor="#ffffff"
                                            fgColor="#0b0d12"
                                        />
                                    </div>
                                </div>
                                <p className="mt-5 font-sans text-xs uppercase tracking-[0.12em] text-ink-muted">
                                    Compare this code with your phone
                                </p>
                                <p className="mt-2 font-sans text-sm text-ink-muted">
                                    After the scan, the phone shows these six digits. They must match. That is how you know the phone paired with this computer.
                                </p>
                                <p className="mt-3 font-mono text-5xl font-semibold tracking-[0.08em] text-ink">
                                    {pairing.safetyCode.split(" ").map((group, index) => (
                                        <span key={group}>
                                            {index > 0 ? <span className="text-ink-muted"> </span> : null}
                                            {group}
                                        </span>
                                    ))}
                                </p>
                                <p
                                    className={`mt-3 font-sans text-xs ${remaining < 60 ? "text-warn" : "text-ink-muted"}`}
                                >
                                    Expires in {formatCountdown(remaining)}
                                </p>
                            </SpotlightCard>

                            <div className="mt-4">
                                <label htmlFor="pairing-blob" className={labelClass}>
                                    Pairing code
                                </label>
                                <div className="flex items-start gap-2">
                                    <input
                                        id="pairing-blob"
                                        readOnly
                                        value={pairing.blob}
                                        className={`${inputClass} h-16 select-text overflow-auto text-xs`}
                                    />
                                    <button type="button" className={ghostButton} onClick={() => void copyBlob()}>
                                        Copy
                                    </button>
                                </div>
                            </div>

                            <div className="mt-4 flex items-center gap-2">
                                <button type="button" className={secondaryButton} onClick={() => void pair()}>
                                    Create new code
                                </button>
                            </div>
                        </div>
                    ) : expired ? (
                        <div className="mt-4">
                            <p className="font-sans text-sm text-warn">This code expired. Create a new one.</p>
                            <button type="button" className={`${primaryButton} mt-3`} onClick={() => void pair()}>
                                Create pairing code
                            </button>
                        </div>
                    ) : (
                        <div className="mt-4">
                            <p className="font-sans text-sm text-ink-muted">
                                No pairing code yet. Create one, then scan it with the phone app.
                            </p>
                            <button type="button" className={`${primaryButton} mt-3`} onClick={() => void pair()}>
                                Create pairing code
                            </button>
                        </div>
                    )}
                </section>

                <section className="mx-4 mb-6 rounded-panel border border-border-subtle bg-surface-raised p-5 lg:mx-6">
                    <h2 className="font-sans text-base font-semibold text-ink">Paired devices</h2>
                    {devices.length === 0 ? (
                        <p className="mt-2 font-sans text-sm text-ink-muted">
                            No phone paired yet. Create a pairing code and scan it with the phone.
                        </p>
                    ) : (
                        <ul className="mt-3 flex flex-col gap-2">
                            {devices.map((device) => (
                                <li
                                    key={device.id}
                                    className="rounded-lg border border-border-subtle bg-surface px-3 py-2"
                                >
                                    <div className="flex items-center justify-between gap-3">
                                        <button
                                            type="button"
                                            className="font-mono text-sm text-ink"
                                            onClick={() => {
                                                setPickedId(device.id);
                                                setEditedDevice(null);
                                            }}
                                        >
                                            {device.name}
                                            {device.connected ? " · live" : ""}
                                            <span className="ml-2 font-mono text-xs text-ink-muted">
                                                {device.id.slice(0, 6)}
                                            </span>
                                        </button>
                                        <button
                                            type="button"
                                            className={ghostButton}
                                            onClick={() => void revoke(device.id)}
                                        >
                                            Revoke
                                        </button>
                                    </div>
                                    {deviceDraft && selectedId === device.id ? (
                                        <div className="mt-3 grid grid-cols-1 gap-3 min-[720px]:grid-cols-2">
                                            <div>
                                                <label className={labelClass} htmlFor={`resolution-${device.id}`}>
                                                    Resolution
                                                </label>
                                                <select
                                                    id={`resolution-${device.id}`}
                                                    className={`${inputClass} ${selectClass}`}
                                                    style={{ backgroundImage: selectChevron }}
                                                    value={`${deviceDraft.width}x${deviceDraft.height}`}
                                                    onChange={(event) => {
                                                        const [width, height] = event.target.value
                                                            .split("x")
                                                            .map(Number);
                                                        setEditedDevice({
                                                            ...deviceDraft,
                                                            width: width || 0,
                                                            height: height || 0,
                                                        });
                                                    }}
                                                >
                                                    {resolutionOptions(deviceDraft).map((option) => (
                                                        <option
                                                            key={`${option.width}x${option.height}`}
                                                            value={`${option.width}x${option.height}`}
                                                        >
                                                            {option.label}
                                                        </option>
                                                    ))}
                                                </select>
                                            </div>
                                            <div>
                                                <label className={labelClass} htmlFor={`fps-${device.id}`}>
                                                    Frame rate
                                                </label>
                                                <select
                                                    id={`fps-${device.id}`}
                                                    className={`${inputClass} ${selectClass}`}
                                                    style={{ backgroundImage: selectChevron }}
                                                    value={deviceDraft.fps}
                                                    onChange={(event) => {
                                                        setEditedDevice({
                                                            ...deviceDraft,
                                                            fps: Number(event.target.value) || 0,
                                                        });
                                                    }}
                                                >
                                                    {frameRateOptions(deviceDraft.fps).map((fps) => (
                                                        <option key={fps} value={fps}>
                                                            {fps} fps
                                                        </option>
                                                    ))}
                                                </select>
                                            </div>
                                            <div>
                                                <label className={labelClass} htmlFor={`quality-${device.id}`}>
                                                    Quality
                                                </label>
                                                <div className="flex items-center gap-3">
                                                    <input
                                                        id={`quality-${device.id}`}
                                                        type="range"
                                                        min={1}
                                                        max={100}
                                                        className="h-9 w-full accent-[var(--color-accent)]"
                                                        value={deviceDraft.jpegQuality}
                                                        onChange={(event) => {
                                                            setEditedDevice({
                                                                ...deviceDraft,
                                                                jpegQuality: Number(event.target.value),
                                                            });
                                                        }}
                                                    />
                                                    <span className="w-8 text-right font-mono text-sm text-ink">
                                                        {deviceDraft.jpegQuality}
                                                    </span>
                                                </div>
                                            </div>
                                            <div className="flex items-end">
                                                <button type="button" className={primaryButton} onClick={() => void saveDevice()}>
                                                    Save for this phone
                                                </button>
                                            </div>
                                        </div>
                                    ) : null}
                                </li>
                            ))}
                        </ul>
                    )}
                </section>

                <section className="mx-4 mb-6 rounded-panel border border-border-subtle bg-surface-raised p-5 lg:mx-6">
                    <div className="flex items-center justify-between">
                        <h2 className="font-sans text-base font-semibold text-ink">Server settings</h2>
                        <div className="flex items-center gap-2 font-sans text-xs text-ink-muted">
                            {saveState === "saving" ? (
                                <>
                                    <StatusMark status="running" color="var(--color-accent)" size={14} />
                                    <span>Saving</span>
                                </>
                            ) : saveState === "saved" ? (
                                <>
                                    <StatusMark status="done" color="var(--color-ok)" size={14} />
                                    <span>Saved</span>
                                </>
                            ) : saveState === "failed" ? (
                                <>
                                    <StatusMark status="failed" color="var(--color-danger)" size={14} />
                                    <span>Could not save</span>
                                </>
                            ) : null}
                        </div>
                    </div>

                    {draft ? (
                        <div className="mt-4 grid grid-cols-1 gap-x-4 gap-y-3 min-[720px]:grid-cols-2">
                            <div>
                                <label htmlFor="port" className={labelClass}>
                                    Port
                                </label>
                                <input
                                    id="port"
                                    type="number"
                                    min={1024}
                                    max={65535}
                                    className={inputClass}
                                    value={draft.port}
                                    onChange={(event) =>
                                        setDraft({ ...draft, port: Number(event.target.value) || 0 })
                                    }
                                />
                            </div>
                            <div>
                                <label htmlFor="server-name" className={labelClass}>
                                    Server name
                                </label>
                                <input
                                    id="server-name"
                                    type="text"
                                    maxLength={64}
                                    className={`${inputClass} font-sans`}
                                    value={draft.server_name}
                                    onChange={(event) => setDraft({ ...draft, server_name: event.target.value })}
                                />
                            </div>
                            <div>
                                <label htmlFor="max-sessions" className={labelClass}>
                                    Max sessions
                                </label>
                                <input
                                    id="max-sessions"
                                    type="number"
                                    min={1}
                                    max={16}
                                    className={inputClass}
                                    value={draft.max_sessions}
                                    onChange={(event) =>
                                        setDraft({ ...draft, max_sessions: Number(event.target.value) || 0 })
                                    }
                                />
                            </div>
                        </div>
                    ) : null}

                    <div className="mt-4 flex items-center justify-between border-t border-border-subtle pt-4">
                        <p className="font-sans text-xs text-ink-muted">Port and name apply on restart. Screen size is per phone.</p>
                        <button
                            type="button"
                            className={primaryButton}
                            onClick={() => void save()}
                            disabled={!draft}
                        >
                            Save
                        </button>
                    </div>
                </section>
            </div>
        </div>
    );
}
