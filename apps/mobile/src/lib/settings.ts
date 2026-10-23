/**
 * The settings of the phone app.
 *
 * They are stored in the async storage, except the pairing secret, which
 * lives in the secure store.
 */

import AsyncStorage from "@react-native-async-storage/async-storage";

import { Codec } from "@kvm/protocol";

export type CodecPreference = "auto" | "h264" | "h265" | "jpeg";

export interface AppSettings {
    /** Which compression the phone asks the server for. */
    codecPreference: CodecPreference;
    /** Largest frame the phone accepts, in pixels. */
    maxWidth: number;
    maxHeight: number;
    /** Largest frame rate the phone accepts. */
    maxFps: number;
    /** Delay before the first reconnection attempt. */
    reconnectDelayMs: number;
    /** Longest delay between two reconnection attempts. */
    reconnectMaxDelayMs: number;
    /** Keep the screen on while the session is live. */
    keepAwake: boolean;
    /** Show the frame rate and the codec in the viewer. */
    showStats: boolean;
}

export const DEFAULT_SETTINGS: AppSettings = {
    codecPreference: "auto",
    maxWidth: 1920,
    maxHeight: 1200,
    maxFps: 30,
    reconnectDelayMs: 1000,
    reconnectMaxDelayMs: 10000,
    keepAwake: true,
    showStats: true,
};

const STORAGE_KEY = "kvm.settings.v1";

export async function loadSettings(): Promise<AppSettings> {
    try {
        const raw = await AsyncStorage.getItem(STORAGE_KEY);
        if (!raw) {
            return DEFAULT_SETTINGS;
        }
        const parsed = JSON.parse(raw) as Partial<AppSettings>;
        return { ...DEFAULT_SETTINGS, ...parsed };
    } catch {
        return DEFAULT_SETTINGS;
    }
}

export async function saveSettings(settings: AppSettings): Promise<void> {
    await AsyncStorage.setItem(STORAGE_KEY, JSON.stringify(settings));
}

/**
 * The codecs the phone asks for.
 *
 * The phone decodes H.264 and H.265 with the hardware decoder, so both are
 * offered. JPEG is the fallback for the devices without a working hardware
 * decoder, and it is the only one that always works.
 */
export function codecMask(preference: CodecPreference): number {
    switch (preference) {
        case "h264":
            return Codec.H264;
        case "h265":
            return Codec.H265;
        case "jpeg":
            return Codec.JPEG;
        case "auto":
        default:
            return Codec.H264 | Codec.H265 | Codec.JPEG;
    }
}

export const CODEC_PREFERENCES: Array<{ value: CodecPreference; label: string; hint: string }> = [
    { value: "auto", label: "Automatic", hint: "The most compressed codec both ends support" },
    { value: "h264", label: "H.264", hint: "Hardware decoded, works on almost every phone" },
    { value: "h265", label: "H.265", hint: "Less bandwidth, newer phones only" },
    { value: "jpeg", label: "JPEG", hint: "Every frame on its own, the most bandwidth" },
];

export const RESOLUTIONS: Array<{ label: string; width: number; height: number }> = [
    { label: "1920 x 1200", width: 1920, height: 1200 },
    { label: "1408 x 792", width: 1408, height: 792 },
    { label: "1280 x 720", width: 1280, height: 720 },
];

export const FRAME_RATES = [15, 20, 30, 60];
