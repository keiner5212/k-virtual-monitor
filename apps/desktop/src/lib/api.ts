import { invoke } from "@tauri-apps/api/core";

export interface ServerStatus {
    running: boolean;
    port: number;
    address: string;
    sessions: number;
    pairedDevices: number;
    codec: string | null;
    fps: number | null;
    deviceName: string | null;
}

export interface PairingInfo {
    blob: string;
    safetyCode: string;
    expiresInSeconds: number;
}

export interface DeviceInfo {
    id: string;
    name: string;
    paired: boolean;
}

export interface ServerConfig {
    port: number;
    bind: string;
    server_name: string;
    width: number;
    height: number;
    fps: number;
    jpeg_quality: number;
    codec: string;
    max_sessions: number;
    max_pending_handshakes: number;
    handshake_timeout_secs: number;
    session_idle_timeout_secs: number;
}

export function serverStatus(): Promise<ServerStatus> {
    return invoke<ServerStatus>("server_status");
}

export function createPairing(): Promise<PairingInfo> {
    return invoke<PairingInfo>("create_pairing");
}

export function pairedDevices(): Promise<DeviceInfo[]> {
    return invoke<DeviceInfo[]>("paired_devices");
}

export function getConfig(): Promise<ServerConfig> {
    return invoke<ServerConfig>("get_config");
}

export function saveConfig(config: ServerConfig): Promise<ServerConfig> {
    return invoke<ServerConfig>("save_config", { config });
}
