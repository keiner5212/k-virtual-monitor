/**
 * The check of the session, with the client of the phone.
 *
 * The client that runs here is the one of the phone: the same code, the same
 * handshake and the same reconnection policy. The server is the real binary,
 * so the two implementations of the protocol talk to each other, and the
 * pictures that arrive are the ones the decoder of the phone would get.
 */

import { spawn } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { createConnection, createServer } from "node:net";
import { dirname, join, resolve } from "node:path";

import { describe, expect, it } from "vitest";

import { Codec } from "@kvm/protocol";

import { ReconnectingClient, type ConnectOptions, type Transport } from "../src/lib/client";
import type { Frame } from "../src/lib/frames";

/** The root of the workspace. */
const workspace = resolve(dirname(new URL(import.meta.url).pathname), "../../..");
const binary = join(workspace, ".tmp/target/debug/kvm-desktop");
const itWithServer = existsSync(binary) ? it : it.skip;

/** The width and the height the stream of the test has. */
const WIDTH = 320;
const HEIGHT = 240;
/** The rate the server is asked for. */
const FPS = 20;

/** The folder of the workspace where the temporary files are written. */
function scratchDir(): string {
    const directory = join(workspace, ".tmp");
    mkdirSync(directory, { recursive: true });
    return directory;
}

function freePort(): Promise<number> {
    return new Promise((resolvePort, reject) => {
        const probe = createServer();
        probe.on("error", reject);
        probe.listen(0, "127.0.0.1", () => {
            const address = probe.address();
            const port = typeof address === "object" && address ? address.port : 0;
            probe.close(() => resolvePort(port));
        });
    });
}

/**
 * The socket of the test.
 *
 * It is the same socket the phone has, over the same protocol.
 */
function nodeTransport(options: ConnectOptions): Promise<Transport> {
    return new Promise((resolveSocket, reject) => {
        const socket = createConnection({ host: options.host, port: options.port });
        socket.once("error", reject);
        socket.once("connect", () => {
            resolveSocket({
                write: (data) => {
                    socket.write(data);
                },
                onData: (listener) => {
                    socket.on("data", (chunk: Buffer | string) => {
                        const bytes =
                            typeof chunk === "string"
                                ? new TextEncoder().encode(chunk)
                                : new Uint8Array(chunk.buffer, chunk.byteOffset, chunk.byteLength);
                        listener(bytes);
                    });
                },
                onError: (listener) => {
                    socket.on("error", (error) => listener(error));
                },
                onClose: (listener) => {
                    socket.on("close", () => listener());
                },
                setNoDelay: (noDelay) => {
                    socket.setNoDelay(noDelay);
                },
                destroy: () => {
                    socket.destroy();
                },
            });
        });
    });
}

/** The types of the NAL units of an access unit. */
function nalTypes(payload: Uint8Array): number[] {
    const kinds: number[] = [];
    let index = 0;
    while (index + 3 < payload.length) {
        if (
            (payload[index] ?? 0) === 0 &&
            (payload[index + 1] ?? 0) === 0 &&
            (payload[index + 2] ?? 0) === 1
        ) {
            kinds.push((payload[index + 3] ?? 0) & 0x1f);
            index += 4;
            continue;
        }
        index += 1;
    }
    return kinds;
}

/** True when the access unit carries the configuration the decoder needs. */
function hasDecoderConfig(payload: Uint8Array): boolean {
    const kinds = nalTypes(payload);
    return kinds.includes(7) && kinds.includes(8);
}

/** Creates a pairing code and returns the blob. */
async function createPairing(environment: NodeJS.ProcessEnv): Promise<string> {
    return new Promise<string>((resolvePairing, reject) => {
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
            resolvePairing(match[1] as string);
        });
    });
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
        const open = await new Promise<boolean>((resolveOpen) => {
            const probe = createConnection({ host: "127.0.0.1", port });
            probe.once("error", () => resolveOpen(false));
            probe.once("connect", () => {
                probe.destroy();
                resolveOpen(true);
            });
        });
        if (open) {
            return;
        }
        if (Date.now() > deadline) {
            throw new Error(`the server did not start on the port ${port}`);
        }
        await sleep(100);
    }
}

function sleep(ms: number): Promise<void> {
    return new Promise((resolveSleep) => setTimeout(resolveSleep, ms));
}

/** Waits until the condition holds, or gives up. */
async function waitUntil(condition: () => boolean, timeoutMs: number): Promise<boolean> {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
        if (condition()) {
            return true;
        }
        await sleep(50);
    }
    return condition();
}

interface Server {
    port: number;
    environment: NodeJS.ProcessEnv;
    directory: string;
    child: ReturnType<typeof spawn>;
}

async function startServer(quiet: boolean): Promise<Server> {
    const directory = mkdtempSync(join(scratchDir(), "kvm-phone-"));
    const port = await freePort();
    const environment: NodeJS.ProcessEnv = {
        ...process.env,
        KVM_STORE_PATH: join(directory, "devices.json"),
        KVM_CONFIG_PATH: join(directory, "config.toml"),
        // The test pattern of the server, not the screen of the desktop.
        KVM_SYNTHETIC: "1",
    };
    if (quiet) {
        // The compositor of a real screen only paints when something changes.
        environment.KVM_SYNTHETIC_QUIET = "1";
    }
    writeFileSync(
        environment.KVM_CONFIG_PATH,
        `port = ${port}\nbind = "127.0.0.1"\nwidth = ${WIDTH}\nheight = ${HEIGHT}\nfps = ${FPS}\n`,
    );
    const child = spawn(binary, ["--headless"], { env: environment });
    await waitForPort(port, 10000);
    return { port, environment, directory, child };
}

/** The options of the client, the same ones the phone builds. */
function optionsOf(pairing: string, port: number): ConnectOptions {
    const bytes = hexBytes(pairing.slice("kvm1:".length));
    return {
        host: "127.0.0.1",
        port,
        psk: bytes.slice(17, 49),
        deviceId: bytes.slice(1, 17),
        deviceName: "test phone",
        codecMask: Codec.H264,
        caps: 0b11,
        maxWidth: WIDTH,
        maxHeight: HEIGHT,
        maxFps: FPS,
        transport: nodeTransport,
    };
}

/**
 * Stops the server and waits for it, so the test does not leave it behind.
 *
 * The server has to end by itself, so the test waits for it before it kills
 * it: a server that does not stop on the signal would stay with its encoder
 * and its port.
 */
async function stopServer(child: ReturnType<typeof spawn>): Promise<void> {
    if (child.exitCode !== null || child.signalCode !== null) {
        // It is already gone.
        return;
    }
    const exited = new Promise<void>((resolveExit) => {
        child.once("exit", () => resolveExit());
    });
    child.kill("SIGTERM");
    const stopped = await Promise.race([
        exited.then(() => true),
        sleep(3000).then(() => false),
    ]);
    if (!stopped) {
        // The server has to end by itself: one that waits for a picture that
        // never comes stays with its port and its encoder.
        child.kill("SIGKILL");
        await exited;
        expect(stopped).toBe(true);
    }
}

describe("the session of the phone", () => {
    itWithServer(
        "keeps the rate of the stream and gives the decoder what it needs",
        async () => {
            const server = await startServer(true);
            try {
                const pairing = await createPairing(server.environment);
                const frames: Frame[] = [];
                const states: string[] = [];
                const client = new ReconnectingClient(optionsOf(pairing, server.port), {
                    onState: (state) => states.push(state),
                    onDisplayInfo: () => undefined,
                    onReconnectIn: () => undefined,
                });
                client.start();
                client.frames.subscribe((frame: Frame) => frames.push(frame));

                // The first picture takes a moment: the screen is quiet.
                expect(await waitUntil(() => frames.length > 0, 10000)).toBe(true);
                expect(states).toContain("connected");

                // The encoder is in the middle of a sequence when the phone
                // arrives, so the first picture may be a delta picture. The
                // decoder waits for the next key frame, which comes within a
                // second.
                expect(await waitUntil(() => frames.some((frame) => frame.keyframe), 5000)).toBe(
                    true,
                );
                const first = frames.find((frame) => frame.keyframe) as Frame;
                expect(first.width).toBe(WIDTH);
                expect(first.height).toBe(HEIGHT);
                expect(hasDecoderConfig(first.data)).toBe(true);

                // The rate of the stream, with a screen that does not change.
                const started = Date.now();
                expect(await waitUntil(() => frames.length >= FPS * 3, 15000)).toBe(true);
                const elapsed = (Date.now() - started) / 1000;
                const rate = frames.length / elapsed;
                expect(rate).toBeGreaterThan(FPS * 0.7);

                // Every key frame has to carry the configuration of the
                // decoder, or the phone cannot start.
                const keys = frames.filter((frame) => frame.keyframe);
                expect(keys.length).toBeGreaterThan(0);
                for (const key of keys) {
                    expect(hasDecoderConfig(key.data)).toBe(true);
                }

                // The pictures have to be a stream a decoder can read.
                const written = frames.length;
                const path = join(scratchDir(), "kvm-phone-stream.h264");
                writeFileSync(path, joinFrames(frames));
                const decoded = await decodeWithFfmpeg(path);
                if (decoded !== null) {
                    expect(decoded.errors).toBe("");
                    expect(decoded.frames).toBeGreaterThanOrEqual(written);
                }

                client.stop();
            } finally {
                await stopServer(server.child);
                rmSync(server.directory, { recursive: true, force: true });
            }
        },
        60000,
    );

    itWithServer(
        "comes back when the desktop goes away and comes back",
        async () => {
            const server = await startServer(false);
            try {
                const pairing = await createPairing(server.environment);
                let connected = 0;
                const client = new ReconnectingClient(optionsOf(pairing, server.port), {
                    onState: (state) => {
                        if (state === "connected") {
                            connected += 1;
                        }
                    },
                    onDisplayInfo: () => undefined,
                    onReconnectIn: () => undefined,
                });
                client.start();
                expect(await waitUntil(() => connected >= 1, 10000)).toBe(true);

                // The desktop goes away.
                await stopServer(server.child);
                await sleep(1500);

                // And comes back.
                const again = spawn(binary, ["--headless"], { env: server.environment });
                try {
                    await waitForPort(server.port, 10000);
                    expect(await waitUntil(() => connected >= 2, 15000)).toBe(true);
                } finally {
                    await stopServer(again);
                }
                client.stop();
            } finally {
                await stopServer(server.child);
                rmSync(server.directory, { recursive: true, force: true });
            }
        },
        60000,
    );
});

/** The pictures of the session, one after the other. */
function joinFrames(frames: Frame[]): Uint8Array {
    let total = 0;
    for (const frame of frames) {
        total += frame.data.length;
    }
    const out = new Uint8Array(total);
    let offset = 0;
    for (const frame of frames) {
        out.set(frame.data, offset);
        offset += frame.data.length;
    }
    return out;
}

/**
 * Decodes a stream with ffmpeg, the way a decoder of the phone would.
 *
 * The result is null when ffmpeg is not installed.
 */
async function decodeWithFfmpeg(path: string): Promise<{ frames: number; errors: string } | null> {
    const probe = spawn("ffprobe", [
        "-v",
        "error",
        "-count_frames",
        "-select_streams",
        "v:0",
        "-show_entries",
        "stream=nb_read_frames",
        "-of",
        "csv=p=0",
        path,
    ]);
    let output = "";
    probe.stdout.on("data", (chunk) => {
        output += String(chunk);
    });
    probe.stderr.on("data", () => undefined);
    const done = new Promise<number | null>((resolveCount) => {
        probe.on("error", () => resolveCount(null));
        probe.on("close", () => {
            const frames = Number.parseInt(output.trim(), 10);
            resolveCount(Number.isNaN(frames) ? null : frames);
        });
    });
    const count = await done;
    if (count === null) {
        return null;
    }
    const decoder = spawn("ffmpeg", ["-v", "error", "-i", path, "-f", "null", "-"]);
    let errors = "";
    decoder.stderr.on("data", (chunk) => {
        errors += String(chunk);
    });
    const finished = new Promise<void>((resolveDone) => {
        decoder.on("error", () => resolveDone());
        decoder.on("close", () => resolveDone());
    });
    await finished;
    return { frames: count, errors: errors.trim() };
}
