/**
 * The phone side of the session.
 *
 * It opens a TCP connection, completes the pairing handshake and then keeps
 * the frame window fed: it acknowledges every frame on receipt, before
 * decoding, so the server is never blocked by the decoder.
 */

import TcpSocket from "react-native-tcp-socket";

import {
    clientHandshake,
    decodeDisplayInfo,
    decodeFrameHeader,
    encodeFrameAck,
    encodeHeartbeat,
    MessageChannel,
    MessageType,
    randomBytes,
    Session,
    type ChannelSocket,
    type DisplayInfo,
    type Hello,
    type Message,
} from "@kvm/protocol";

import { LatestFrame, type Frame } from "./frames";

/** The biggest payload the phone accepts, the same limit the server uses. */
const MAX_PAYLOAD = 4 * 1024 * 1024;

/** How often the phone says hello, so the desktop does not close the session. */
const HEARTBEAT_MS = 30000;

/** Bytes of the frame header that precede the picture. */
const FRAME_HEADER_LEN = 14;

export type ConnectionState = "idle" | "connecting" | "connected" | "error" | "closed";

export interface ClientEvents {
    onState: (state: ConnectionState, detail?: string) => void;
    onDisplayInfo: (info: DisplayInfo) => void;
}

export interface ConnectOptions {
    host: string;
    port: number;
    psk: Uint8Array;
    deviceId: Uint8Array;
    deviceName?: string;
    codecMask: number;
    caps: number;
    maxWidth: number;
    maxHeight: number;
    maxFps: number;
}

export class SessionClient {
    private socket: TcpSocket.Socket | null = null;
    private channel: MessageChannel | null = null;
    private session: Session | null = null;
    private readonly frames: LatestFrame;
    private ackedBytes = 0;
    private highestSeq = 0;
    private closing = false;
    private heartbeatTimer: ReturnType<typeof setInterval> | null = null;
    private heartbeatSentAt = 0;
    private lastRoundTripMs: number | null = null;

    constructor(
        private readonly events: ClientEvents,
        frames: LatestFrame = new LatestFrame(),
    ) {
        this.frames = frames;
    }

    get latestFrame(): LatestFrame {
        return this.frames;
    }

    async connect(options: ConnectOptions): Promise<void> {
        this.events.onState("connecting");
        const socket = await new Promise<TcpSocket.Socket>((resolve, reject) => {
            const created = TcpSocket.createConnection(
                {
                    host: options.host,
                    port: options.port,
                    connectTimeout: 5000,
                },
                () => resolve(created),
            );
            created.on("error", reject);
        });
        socket.setNoDelay(true);
        this.socket = socket;

        const adapter: ChannelSocket = {
            write: (data) => {
                socket.write(data);
            },
            on: (event, listener) => {
                if (event === "data") {
                    socket.on("data", (data) => {
                        listener(data instanceof Uint8Array ? data : new Uint8Array(0));
                    });
                } else if (event === "error") {
                    socket.on("error", (error) => listener(error));
                } else {
                    socket.on("close", () => listener());
                }
            },
        };
        const channel = new MessageChannel(adapter, MAX_PAYLOAD);
        this.channel = channel;

        socket.on("error", (error) => {
            this.events.onState("error", error.message);
        });
        socket.on("close", () => {
            if (!this.closing) {
                this.events.onState("closed");
            }
        });

        const hello: Hello = {
            clientNonce: randomBytes(32),
            deviceId: options.deviceId,
            deviceName: options.deviceName ?? "phone",
            maxWidth: options.maxWidth,
            maxHeight: options.maxHeight,
            maxFps: options.maxFps,
            codecMask: options.codecMask,
            capsRequested: options.caps,
        };

        const keys = await clientHandshake(channel, options.psk, hello);
        this.session = new Session(keys, "client");
        this.events.onState("connected");
        this.startHeartbeat();
        void this.pump();
    }

    /** The round trip time of the last heartbeat, in milliseconds. */
    get roundTripMs(): number | null {
        return this.lastRoundTripMs;
    }

    private async pump(): Promise<void> {
        const channel = this.channel;
        const session = this.session;
        if (!channel || !session) {
            return;
        }
        for (; ;) {
            let message: Message;
            try {
                message = await channel.next();
            } catch {
                if (!this.closing) {
                    this.events.onState("closed");
                }
                return;
            }
            let plaintext: Uint8Array;
            try {
                plaintext = session.open(message.header, message.payload);
            } catch {
                this.events.onState("error", "the session was rejected by the server");
                return;
            }
            if (message.header.type === MessageType.DisplayInfo) {
                this.events.onDisplayInfo(decodeDisplayInfo(plaintext));
            } else if (message.header.type === MessageType.Frame) {
                this.onFrame(plaintext, message.header.seq);
            } else if (message.header.type === MessageType.HeartbeatAck) {
                this.lastRoundTripMs = Date.now() - this.heartbeatSentAt;
            }
        }
    }

    /**
     * Keeps the session alive.
     *
     * The server closes a session that stays silent, and a desktop with a
     * static screen sends no frames at all, so the phone says hello every
     * half minute.
     */
    private startHeartbeat(): void {
        this.heartbeatTimer = setInterval(() => {
            const channel = this.channel;
            const session = this.session;
            if (!channel || !session) {
                return;
            }
            this.heartbeatSentAt = Date.now();
            const payload = encodeHeartbeat({ clientTimeMs: this.heartbeatSentAt });
            const sealed = session.seal(MessageType.Heartbeat, 0, payload);
            channel.send(sealed.header, sealed.payload);
        }, HEARTBEAT_MS);
    }

    private onFrame(plaintext: Uint8Array, seq: number): void {
        if (plaintext.length < FRAME_HEADER_LEN) {
            return;
        }
        const header = decodeFrameHeader(plaintext.subarray(0, FRAME_HEADER_LEN));
        const frame: Frame = {
            ptsMs: header.ptsMs,
            width: header.width,
            height: header.height,
            codec: header.codec,
            keyframe: header.keyframe,
            data: plaintext.subarray(FRAME_HEADER_LEN),
        };
        this.frames.push(frame);

        this.ackedBytes += frame.data.length;
        this.highestSeq = seq;
        this.sendAck();
    }

    private sendAck(): void {
        const channel = this.channel;
        const session = this.session;
        if (!channel || !session) {
            return;
        }
        const payload = encodeFrameAck({ ackedSeq: this.highestSeq, ackedBytes: this.ackedBytes });
        const sealed = session.seal(MessageType.FrameAck, 0, payload);
        channel.send(sealed.header, sealed.payload);
    }

    close(): void {
        this.closing = true;
        if (this.heartbeatTimer) {
            clearInterval(this.heartbeatTimer);
            this.heartbeatTimer = null;
        }
        this.socket?.destroy();
        this.socket = null;
        this.channel = null;
        this.session = null;
        this.events.onState("idle");
    }
}

/**
 * A client that keeps the session alive.
 *
 * The desktop may go away at any moment: it sleeps, the router reboots, the
 * phone walks out of range. The client comes back on its own, with a delay
 * that grows up to a ceiling, so a desktop that is really gone is not
 * hammered with a connection attempt every second.
 */
export class ReconnectingClient {
    private client: SessionClient | null = null;
    private timer: ReturnType<typeof setTimeout> | null = null;
    private attempt = 0;
    private stopped = false;
    private readonly frameBuffer = new LatestFrame();

    constructor(
        private readonly options: ConnectOptions,
        private readonly events: ClientEvents & { onReconnectIn: (seconds: number | null) => void },
        private readonly policy: { delayMs: number; maxDelayMs: number },
    ) { }

    get frames(): LatestFrame {
        return this.frameBuffer;
    }

    start(): void {
        this.stopped = false;
        void this.attemptConnect();
    }

    stop(): void {
        this.stopped = true;
        if (this.timer) {
            clearTimeout(this.timer);
            this.timer = null;
        }
        this.client?.close();
        this.client = null;
        this.events.onReconnectIn(null);
    }

    private async attemptConnect(): Promise<void> {
        if (this.stopped) {
            return;
        }
        const client = new SessionClient(
            {
                onState: (state, detail) => {
                    this.events.onState(state, detail);
                    if (state === "closed" || state === "error") {
                        this.scheduleRetry();
                    }
                },
                onDisplayInfo: (info) => {
                    this.attempt = 0;
                    this.events.onDisplayInfo(info);
                },
            },
            this.frameBuffer,
        );
        this.client = client;
        try {
            await client.connect(this.options);
        } catch (cause) {
            this.events.onState("error", cause instanceof Error ? cause.message : String(cause));
            this.scheduleRetry();
        }
    }

    private scheduleRetry(): void {
        if (this.stopped || this.timer) {
            return;
        }
        const delay = Math.min(this.policy.delayMs * 2 ** this.attempt, this.policy.maxDelayMs);
        this.attempt += 1;
        this.events.onReconnectIn(Math.round(delay / 1000));
        this.timer = setTimeout(() => {
            this.timer = null;
            this.events.onReconnectIn(null);
            this.client?.close();
            void this.attemptConnect();
        }, delay);
    }
}
