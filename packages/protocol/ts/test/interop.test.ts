/**
 * The interoperability check.
 *
 * The server is written in Rust and the client in TypeScript. This test runs
 * the real server binary and connects to it with the real TypeScript client,
 * over a real socket, so a difference between the two implementations shows
 * up here instead of on a phone.
 */

import { spawn } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { createConnection, createServer } from "node:net";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import {
    clientHandshake,
    codecWireValue,
    Codec,
    decodeDisplayInfo,
    decodeFrameHeader,
    encodeFrameAck,
    FRAME_HEADER_LEN,
    MessageChannel,
    MessageType,
    Session,
    type ChannelSocket,
    type Message,
} from "../src/index.js";

const binary = fileURLToPath(
    new URL("../../../../.tmp/target/debug/kvm-desktop", import.meta.url),
);
const itWithServer = existsSync(binary) ? it : it.skip;

/** The folder of the workspace where the temporary files are written. */
function scratchDir(): string {
    const directory = fileURLToPath(new URL("../../../../.tmp/", import.meta.url));
    mkdirSync(directory, { recursive: true });
    return directory;
}

function freePort(): Promise<number> {
    return new Promise((resolve, reject) => {
        const probe = createServer();
        probe.on("error", reject);
        probe.listen(0, "127.0.0.1", () => {
            const address = probe.address();
            const port = typeof address === "object" && address ? address.port : 0;
            probe.close(() => resolve(port));
        });
    });
}

function connect(port: number): Promise<ChannelSocket> {
    return new Promise((resolve, reject) => {
        const socket = createConnection({ host: "127.0.0.1", port });
        socket.once("error", reject);
        socket.once("connect", () => {
            resolve({
                write: (data: Uint8Array) => {
                    socket.write(Buffer.from(data));
                },
                on: (event: string, listener: (payload?: unknown) => void) => {
                    socket.on(event, (payload) => {
                        if (event === "data") {
                            listener(new Uint8Array(payload as Buffer));
                        } else {
                            listener(payload);
                        }
                    });
                },
            });
        });
    });
}

describe("the Rust server and the TypeScript client", () => {
    itWithServer("complete the handshake and exchange frames", async () => {
        const directory = mkdtempSync(join(scratchDir(), "kvm-interop-"));
        const port = await freePort();
        const environment = {
            ...process.env,
            KVM_STORE_PATH: join(directory, "devices.json"),
            KVM_CONFIG_PATH: join(directory, "config.toml"),
            // The test pattern of the server, not the screen of the desktop:
            // the check has to be the same everywhere.
            KVM_SYNTHETIC: "1",
        };
        writeFileSync(
            environment.KVM_CONFIG_PATH,
            `port = ${port}\nbind = "127.0.0.1"\nwidth = 320\nheight = 240\nfps = 20\n`,
        );

        const pairing = await createPairing(environment);

        const server = spawn(binary, ["--headless"], { env: environment });
        try {
            await waitForPort(port, 5000);

            const socket = await connect(port);
            const channel = new MessageChannel(socket, 4 * 1024 * 1024);
            const keys = await clientHandshake(channel, pskOf(pairing), helloOf(pairing));
            const session = new Session(keys, "client");

            const first = await channel.next();
            expect(first.header.type).toBe(MessageType.DisplayInfo);
            const info = decodeDisplayInfo(session.open(first.header, first.payload));
            expect(info.width).toBe(320);
            expect(info.height).toBe(240);

            let frames = 0;
            let acked = 0;
            for (; ;) {
                const message: Message = await channel.next();
                if (message.header.type !== MessageType.Frame) {
                    continue;
                }
                const plaintext = session.open(message.header, message.payload);
                const header = decodeFrameHeader(plaintext.subarray(0, FRAME_HEADER_LEN));
                expect(header.width).toBe(320);
                // The picture of the synthetic source is an H.264 access unit.
                expect(header.codec).toBe(codecWireValue(Codec.H264));
                expect(plaintext[FRAME_HEADER_LEN]).toBe(0x00);
                expect(plaintext[FRAME_HEADER_LEN + 1]).toBe(0x00);
                expect(plaintext[FRAME_HEADER_LEN + 2]).toBe(0x00);
                expect(plaintext[FRAME_HEADER_LEN + 3]).toBe(0x01);
                frames += 1;
                acked = message.header.seq;
                const ack = encodeFrameAck({
                    ackedSeq: acked,
                    ackedBytes: plaintext.length,
                });
                const sealed = session.seal(MessageType.FrameAck, 0, ack);
                channel.send(sealed.header, sealed.payload);
                if (frames >= 3) {
                    break;
                }
            }
            expect(frames).toBe(3);
        } finally {
            server.kill();
            rmSync(directory, { recursive: true, force: true });
        }
    });

    itWithServer("accept a pairing code created while the server runs", async () => {
        const directory = mkdtempSync(join(scratchDir(), "kvm-interop-"));
        const port = await freePort();
        const environment = {
            ...process.env,
            KVM_STORE_PATH: join(directory, "devices.json"),
            KVM_CONFIG_PATH: join(directory, "config.toml"),
            KVM_SYNTHETIC: "1",
        };
        writeFileSync(
            environment.KVM_CONFIG_PATH,
            `port = ${port}\nbind = "127.0.0.1"\nwidth = 320\nheight = 240\nfps = 20\n`,
        );

        const server = spawn(binary, ["--headless"], { env: environment });
        try {
            await waitForPort(port, 5000);

            // The code is created after the server is already listening, the
            // way the desktop creates it from its interface.
            const pairing = await createPairing(environment);

            const socket = await connect(port);
            const channel = new MessageChannel(socket, 4 * 1024 * 1024);
            const keys = await clientHandshake(channel, pskOf(pairing), helloOf(pairing));
            const session = new Session(keys, "client");

            const first = await channel.next();
            expect(first.header.type).toBe(MessageType.DisplayInfo);
            const info = decodeDisplayInfo(session.open(first.header, first.payload));
            expect(info.width).toBe(320);
        } finally {
            server.kill();
            rmSync(directory, { recursive: true, force: true });
        }
    });
});

/** Creates a pairing code and returns the blob. */
function createPairing(environment: NodeJS.ProcessEnv): Promise<string> {
    return new Promise<string>((resolve, reject) => {
        const child = spawn(binary, ["--pair"], { env: environment });
        let output = "";
        child.stdout.on("data", (chunk) => {
            output += String(chunk);
        });
        child.on("error", reject);
        child.on("close", (code) => {
            const match = output.match(/pairing code: (\S+)/);
            if (code !== 0 || !match) {
                reject(new Error(`the pairing failed: ${output}`));
                return;
            }
            resolve(match[1] as string);
        });
    });
}

/** The pairing secret of the blob, the same one the phone would keep. */
function pskOf(blob: string): Uint8Array {
    const bytes = hexBytes(blob.slice("kvm1:".length));
    // version(1) device_id(16) psk(32)
    return bytes.slice(17, 49);
}

function helloOf(blob: string) {
    const bytes = hexBytes(blob.slice("kvm1:".length));
    return {
        clientNonce: new Uint8Array(32).fill(7),
        deviceId: bytes.slice(1, 17),
        deviceName: "interop",
        maxWidth: 320,
        maxHeight: 240,
        maxFps: 20,
        codecMask: codecWireValue(Codec.H264),
        capsRequested: 3,
    };
}

function hexBytes(text: string): Uint8Array {
    const out = new Uint8Array(text.length / 2);
    for (let i = 0; i < out.length; i += 1) {
        out[i] = Number.parseInt(text.slice(i * 2, i * 2 + 2), 16);
    }
    return out;
}

async function waitForPort(port: number, timeoutMs: number): Promise<void> {
    const deadline = Date.now() + timeoutMs;
    for (; ;) {
        const open = await new Promise<boolean>((resolve) => {
            const probe = createConnection({ host: "127.0.0.1", port });
            probe.once("error", () => resolve(false));
            probe.once("connect", () => {
                probe.destroy();
                resolve(true);
            });
        });
        if (open) {
            return;
        }
        if (Date.now() > deadline) {
            throw new Error(`the server did not start on the port ${port}`);
        }
        await new Promise((resolve) => setTimeout(resolve, 100));
    }
}
