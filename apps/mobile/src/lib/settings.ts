/**
 * The settings of the phone app.
 *
 * They are stored in the async storage. The size of the stream is not here:
 * the desktop decides it and the phone shows what it decided.
 */

import AsyncStorage from "@react-native-async-storage/async-storage";

import { Codec } from "@kvm/protocol";

export type CodecPreference = "auto" | "h264" | "h265" | "jpeg";

export interface AppSettings {
    /** Which compression the phone asks the server for. */
    codecPreference: CodecPreference;
    /** Keep the screen on while the session is live. */
    keepAwake: boolean;
    /** Show the frame rate and the codec in the viewer. */
    showStats: boolean;
}

export const DEFAULT_SETTINGS: AppSettings = {
    codecPreference: "auto",
    keepAwake: true,
    showStats: true,
};

const STORAGE_KEY = "kvm.settings.v1";

function storageKey(deviceId: string): string {
    return `${STORAGE_KEY}.${deviceId}`;
}

/**
 * The settings of this phone.
 *
 * Only the known keys are read, so a setting that an older build left behind
 * does not pile up in the state of the app.
 */
export async function loadSettings(deviceId: string | null): Promise<AppSettings> {
    try {
        const key = deviceId ? storageKey(deviceId) : STORAGE_KEY;
        const raw = (await AsyncStorage.getItem(key)) ?? (await AsyncStorage.getItem(STORAGE_KEY));
        if (!raw) {
            return DEFAULT_SETTINGS;
        }
        const parsed = JSON.parse(raw) as Partial<AppSettings>;
        return {
            codecPreference: parsed.codecPreference ?? DEFAULT_SETTINGS.codecPreference,
            keepAwake: parsed.keepAwake ?? DEFAULT_SETTINGS.keepAwake,
            showStats: parsed.showStats ?? DEFAULT_SETTINGS.showStats,
        };
    } catch {
        return DEFAULT_SETTINGS;
    }
}

export async function saveSettings(deviceId: string, settings: AppSettings): Promise<void> {
    await AsyncStorage.setItem(storageKey(deviceId), JSON.stringify(settings));
}

/** False when this build has no H.264 decoder. JPEG is still compressed. */
let hardwareDecoder = true;

export function setHardwareDecoder(ready: boolean): void {
    hardwareDecoder = ready;
}

/**
 * The codecs the phone asks for.
 *
 * H.264 is preferred. JPEG is always included, so a tablet that cannot decode
 * H.264, or a desktop without the encoder, still gets a compressed stream.
 */
export function codecMask(preference: CodecPreference): number {
    if (!hardwareDecoder) {
        return Codec.JPEG;
    }
    switch (preference) {
        case "h264":
            return Codec.H264 | Codec.JPEG;
        case "h265":
            return Codec.H265 | Codec.H264 | Codec.JPEG;
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
