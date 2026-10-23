import { invoke } from "@tauri-apps/api/core";

export interface ServerStatus {
  running: boolean;
  port: number;
  address: string;
  sessions: number;
  pairedDevices: number;
}

export interface PairingInfo {
  blob: string;
  safetyCode: string;
  expiresInSeconds: number;
}

export function serverStatus(): Promise<ServerStatus> {
  return invoke<ServerStatus>("server_status");
}

export function createPairing(): Promise<PairingInfo> {
  return invoke<PairingInfo>("create_pairing");
}

export function pairedDevices(): Promise<string[]> {
  return invoke<string[]>("paired_devices");
}
