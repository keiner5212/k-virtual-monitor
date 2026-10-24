/**
 * The session of the phone.
 *
 * The reconnection policy lives in the client, not here, so it can be
 * reasoned about (and tested) without React.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { Dimensions, PixelRatio, Platform } from "react-native";

import type { DisplayInfo } from "@kvm/protocol";

import { ReconnectingClient, type ConnectionState } from "./client";
import type { Frame } from "./frames";
import type { StoredPairing } from "./pairing";
import { codecMask, type AppSettings } from "./settings";

interface UseSessionOptions {
    pairing: StoredPairing | null;
    settings: AppSettings;
    /** The session stays down until the user opens a desktop from the home cards. */
    active: boolean;
    /** Changes each time the user opens a desktop, so the last error is not shown again. */
    openId: number;
    /** Bumped when the user changes the stream or asks to try the connection again. */
    reloadToken: number;
    onDisplayInfo?: (info: DisplayInfo) => void;
}

export function useSession({ pairing, settings, active, openId, reloadToken, onDisplayInfo }: UseSessionOptions) {
    const [state, setState] = useState<ConnectionState>("idle");
    const [detail, setDetail] = useState<string | null>(null);
    const [info, setInfo] = useState<DisplayInfo | null>(null);
    const [frame, setFrame] = useState<Frame | null>(null);
    const [reconnectingIn, setReconnectingIn] = useState<number | null>(null);
    const [trackedOpen, setTrackedOpen] = useState(openId);
    if (trackedOpen !== openId) {
        setTrackedOpen(openId);
        setState("connecting");
        setDetail(null);
        setInfo(null);
        setFrame(null);
        setReconnectingIn(null);
    }

    const clientRef = useRef<ReconnectingClient | null>(null);
    const settingsRef = useRef(settings);
    const onDisplayInfoRef = useRef(onDisplayInfo);
    const [client, setClient] = useState<ReconnectingClient | null>(null);

    useEffect(() => {
        settingsRef.current = settings;
        onDisplayInfoRef.current = onDisplayInfo;
    }, [settings, onDisplayInfo]);

    const connect = useCallback(() => {
        if (!pairing) {
            return;
        }
        const current = settingsRef.current;
        const client = new ReconnectingClient(
            {
                host: pairing.host,
                port: pairing.port,
                psk: pairing.psk,
                deviceId: pairing.deviceIdBytes,
                deviceName: deviceName(),
                codecMask: codecMask(current.codecPreference),
                caps: 0b11,
                maxWidth: capability().width,
                maxHeight: capability().height,
                maxFps: 60,
            },
            {
                onState: (next, message) => {
                    setState(next);
                    setDetail(message ?? null);
                },
                onDisplayInfo: (displayInfo) => {
                    setInfo(displayInfo);
                    onDisplayInfoRef.current?.(displayInfo);
                },
                onReconnectIn: (seconds) => setReconnectingIn(seconds),
            },
        );
        clientRef.current = client;
        setClient(client);
        client.start();
    }, [pairing]);

    useEffect(() => {
        if (!pairing || !active) {
            return;
        }
        const timer = setTimeout(connect, 0);
        return () => {
            clearTimeout(timer);
            clientRef.current?.stop();
            clientRef.current = null;
        };
    }, [connect, pairing, active, reloadToken]);

    useEffect(() => {
        if (!client) {
            return;
        }
        return client.frames.subscribe(setFrame);
    }, [client]);

    return { state, detail, info, frame, reconnectingIn };
}

/** The name of the phone, so the desktop can tell the devices apart. */
function deviceName(): string {
    // React Native keeps the model of the device in the platform constants,
    // but the types of the library do not name it.
    const constants = Platform.constants as unknown as { Model?: string };
    const model = constants.Model;
    return model && model !== "unknown" ? model : "phone";
}

/**
 * The biggest frame the phone can show, in pixels.
 *
 * The desktop picks the size and never sends more than this.
 */
function capability(): { width: number; height: number } {
    const { width, height } = Dimensions.get("window");
    const ratio = PixelRatio.get();
    return {
        width: Math.max(1, Math.round(width * ratio)),
        height: Math.max(1, Math.round(height * ratio)),
    };
}
