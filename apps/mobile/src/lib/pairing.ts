/**
 * The pairing, stored on the phone.
 *
 * The pairing secret goes to the secure store, the rest of the pairing
 * (the name and the address of the desktop) goes to the regular storage.
 */

import AsyncStorage from "@react-native-async-storage/async-storage";
import * as SecureStore from "expo-secure-store";

import { decodePairingBlob } from "@kvm/protocol";

const SECRET_KEY = "kvm.pairing.secret";
const META_KEY = "kvm.pairing.meta";
const DEVICE_ID_KEY = "kvm.device.id";

interface PairingMeta {
    deviceId: string;
    serverName: string;
    host: string;
    port: number;
}

export interface StoredPairing extends PairingMeta {
    psk: Uint8Array;
}

export async function savePairing(blob: string): Promise<StoredPairing> {
    const parsed = decodePairingBlob(blob.trim());
    const meta: PairingMeta = {
        deviceId: hex(parsed.deviceId),
        serverName: parsed.serverName,
        host: parsed.host,
        port: parsed.port,
    };
    await SecureStore.setItemAsync(SECRET_KEY, hex(parsed.psk));
    await AsyncStorage.setItem(META_KEY, JSON.stringify(meta));
    return { ...meta, psk: parsed.psk };
}

export async function loadPairing(): Promise<StoredPairing | null> {
    const raw = await AsyncStorage.getItem(META_KEY);
    if (!raw) {
        return null;
    }
    const secret = await SecureStore.getItemAsync(SECRET_KEY);
    if (!secret) {
        return null;
    }
    const meta = JSON.parse(raw) as PairingMeta;
    return { ...meta, psk: unhex(secret) };
}

export async function clearPairing(): Promise<void> {
    await SecureStore.deleteItemAsync(SECRET_KEY);
    await AsyncStorage.removeItem(META_KEY);
}

/**
 * The id of this phone. It is generated once and kept, so the desktop can
 * recognise the device across pairings.
 */
export async function deviceId(): Promise<Uint8Array> {
    const stored = await AsyncStorage.getItem(DEVICE_ID_KEY);
    if (stored) {
        return unhex(stored);
    }
    const bytes = new Uint8Array(16);
    for (let i = 0; i < bytes.length; i += 1) {
        bytes[i] = Math.floor(Math.random() * 256);
    }
    await AsyncStorage.setItem(DEVICE_ID_KEY, hex(bytes));
    return bytes;
}

function hex(bytes: Uint8Array): string {
    let out = "";
    for (const byte of bytes) {
        out += byte.toString(16).padStart(2, "0");
    }
    return out;
}

function unhex(text: string): Uint8Array {
    const out = new Uint8Array(text.length / 2);
    for (let i = 0; i < out.length; i += 1) {
        out[i] = Number.parseInt(text.slice(i * 2, i * 2 + 2), 16);
    }
    return out;
}
