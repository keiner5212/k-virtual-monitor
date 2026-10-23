/**
 * The session of the phone.
 *
 * The reconnection policy lives in the client, not here, so it can be
 * reasoned about (and tested) without React.
 */

import { useCallback, useEffect, useRef, useState } from "react";

import type { DisplayInfo } from "@kvm/protocol";

import { ReconnectingClient, type ConnectionState } from "./client";
import type { Frame } from "./frames";
import type { StoredPairing } from "./pairing";
import { codecMask, type AppSettings } from "./settings";

interface UseSessionOptions {
    pairing: StoredPairing | null;
    settings: AppSettings;
    deviceId: Uint8Array | null;
}

export function useSession({ pairing, settings, deviceId }: UseSessionOptions) {
    const [state, setState] = useState<ConnectionState>("idle");
    const [detail, setDetail] = useState<string | null>(null);
    const [info, setInfo] = useState<DisplayInfo | null>(null);
    const [frame, setFrame] = useState<Frame | null>(null);
    const [reconnectingIn, setReconnectingIn] = useState<number | null>(null);

    const clientRef = useRef<ReconnectingClient | null>(null);
    const settingsRef = useRef(settings);
    const [client, setClient] = useState<ReconnectingClient | null>(null);

    useEffect(() => {
        settingsRef.current = settings;
    }, [settings]);

    const connect = useCallback(() => {
        if (!pairing || !deviceId) {
            return;
        }
        const current = settingsRef.current;
        const client = new ReconnectingClient(
            {
                host: pairing.host,
                port: pairing.port,
                psk: pairing.psk,
                deviceId,
                codecMask: codecMask(current.codecPreference),
                caps: 0b11,
                maxWidth: current.maxWidth,
                maxHeight: current.maxHeight,
                maxFps: current.maxFps,
            },
            {
                onState: (next, message) => {
                    setState(next);
                    setDetail(message ?? null);
                },
                onDisplayInfo: (displayInfo) => setInfo(displayInfo),
                onReconnectIn: (seconds) => setReconnectingIn(seconds),
            },
            { delayMs: current.reconnectDelayMs, maxDelayMs: current.reconnectMaxDelayMs },
        );
        clientRef.current = client;
        setClient(client);
        client.start();
    }, [deviceId, pairing]);

    useEffect(() => {
        if (!pairing || !deviceId) {
            return;
        }
        const timer = setTimeout(connect, 0);
        return () => {
            clearTimeout(timer);
            clientRef.current?.stop();
            clientRef.current = null;
        };
    }, [connect, deviceId, pairing]);

    useEffect(() => {
        if (!client) {
            return;
        }
        return client.frames.subscribe(setFrame);
    }, [client]);

    return { state, detail, info, frame, reconnectingIn };
}
