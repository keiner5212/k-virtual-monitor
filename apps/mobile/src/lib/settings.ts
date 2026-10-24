/**
 * The settings of the phone app.
 *
 * They are stored in the async storage. The size of the stream is not here:
 * the desktop decides it and the phone shows what it decided.
 */

import AsyncStorage from "@react-native-async-storage/async-storage";

import { Codec } from "@kvm/protocol";

export interface AppSettings {
    /** Keep the screen on while the session is live. */
    keepAwake: boolean;
    /** Show the frame rate and the codec in the viewer. */
    showStats: boolean;
}

export const DEFAULT_SETTINGS: AppSettings = {
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

/**
 * The codecs the phone asks for.
 *
 * Every picture of the stream is compressed with H.264. There is no other
 * choice: a picture that travels alone costs far more than the link can
 * carry, and the phone cannot draw it as fast as it arrives.
 */
export function codecMask(): number {
    return Codec.H264;
}
