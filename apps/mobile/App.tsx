import { StatusBar } from "expo-status-bar";
import { useCallback, useEffect, useRef, useState } from "react";
import { BackHandler, Image, Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import { SafeAreaProvider, SafeAreaView } from "react-native-safe-area-context";
import { activateKeepAwakeAsync, deactivateKeepAwake } from "expo-keep-awake";

import { Codec, codecFromWire, codecName, codecWireValue, type DisplayInfo } from "@kvm/protocol";
import { decoderAvailable, KvmVideoView, type KvmVideoRef } from "kvm-decoder";

import { clearPairing, loadPairing, savePairing, type StoredPairing } from "./src/lib/pairing";
import { ensureCsprng } from "./src/lib/random";
import { DEFAULT_SETTINGS, loadSettings, saveSettings, setHardwareDecoder, type AppSettings } from "./src/lib/settings";
import type { Frame } from "./src/lib/frames";
import { useSession } from "./src/lib/useSession";
import HomeScreen from "./src/screens/HomeScreen";
import PairScreen from "./src/screens/PairScreen";
import SettingsScreen from "./src/screens/SettingsScreen";
import { colors, radius, space, type } from "./src/ui";

type Tab = "screen" | "settings";
type Route = "home" | "pair" | "session";

/** The system bars are drawn over the app, so every screen keeps clear of them. */
const EDGES = ["top", "right", "bottom", "left"] as const;

export default function App() {
    return (
        <SafeAreaProvider>
            <Monitor />
        </SafeAreaProvider>
    );
}

function Monitor() {
    const [route, setRoute] = useState<Route>("home");
    const [openId, setOpenId] = useState(0);
    const [tab, setTab] = useState<Tab>("screen");
    const [settings, setSettings] = useState<AppSettings>(DEFAULT_SETTINGS);
    const [pairing, setPairing] = useState<StoredPairing | null>(null);
    const [pairError, setPairError] = useState<string | null>(null);
    const [ready, setReady] = useState(false);
    const [reloadToken, setReloadToken] = useState(0);
    const [fullscreen, setFullscreen] = useState(false);

    useEffect(() => {
        void (async () => {
            ensureCsprng();
            setHardwareDecoder(decoderAvailable());
            const storedPairing = await loadPairing();
            const stored = await loadSettings(storedPairing?.deviceId ?? null);
            setSettings(stored);
            setPairing(storedPairing);
            setReady(true);
        })();
    }, []);

    const applyDisplay = useCallback((info: DisplayInfo) => {
        if (!pairing) {
            return;
        }
        setSettings((prev) => {
            if (prev.maxWidth === info.width && prev.maxHeight === info.height && prev.maxFps === info.fps) {
                return prev;
            }
            const next = {
                ...prev,
                maxWidth: info.width,
                maxHeight: info.height,
                maxFps: info.fps,
            };
            void saveSettings(pairing.deviceId, next);
            return next;
        });
    }, [pairing]);

    const session = useSession({
        pairing,
        settings,
        active: route === "session",
        openId,
        reloadToken,
        onDisplayInfo: applyDisplay,
    });

    useEffect(() => {
        if (session.state !== "connected" || !settings.keepAwake) {
            void deactivateKeepAwake();
            return;
        }
        void activateKeepAwakeAsync();
        return () => {
            void deactivateKeepAwake();
        };
    }, [session.state, settings.keepAwake]);

    const updateSettings = useCallback((next: AppSettings) => {
        setSettings((prev) => {
            const streamChanged =
                prev.codecPreference !== next.codecPreference ||
                prev.maxWidth !== next.maxWidth ||
                prev.maxHeight !== next.maxHeight ||
                prev.maxFps !== next.maxFps;
            if (streamChanged) {
                setReloadToken((token) => token + 1);
            }
            return next;
        });
        if (pairing) {
            void saveSettings(pairing.deviceId, next);
        }
    }, [pairing]);

    useEffect(() => {
        if (route === "home") {
            return;
        }
        const subscription = BackHandler.addEventListener("hardwareBackPress", () => {
            if (fullscreen) {
                setFullscreen(false);
                return true;
            }
            setRoute("home");
            return true;
        });
        return () => subscription.remove();
    }, [route, fullscreen]);

    const pair = useCallback(async (blob: string) => {
        setPairError(null);
        try {
            setPairing(await savePairing(blob));
            setRoute("home");
        } catch (cause) {
            setPairError(cause instanceof Error ? cause.message : String(cause));
            throw cause;
        }
    }, []);

    const h264Fallback = useRef(false);
    const failH264 = useCallback(() => {
        if (h264Fallback.current || settings.codecPreference === "jpeg") {
            return;
        }
        h264Fallback.current = true;
        updateSettings({ ...settings, codecPreference: "jpeg" });
    }, [settings, updateSettings]);

    useEffect(() => {
        if (settings.codecPreference === "h264" || settings.codecPreference === "auto") {
            h264Fallback.current = false;
        }
    }, [settings.codecPreference]);

    const unpair = useCallback(async () => {
        await clearPairing();
        setPairing(null);
        setFullscreen(false);
        setRoute("home");
    }, []);

    if (!ready) {
        return (
            <SafeAreaView style={styles.root} edges={EDGES}>
                <StatusBar style="light" />
            </SafeAreaView>
        );
    }

    if (route === "pair") {
        return (
            <SafeAreaView style={styles.root} edges={EDGES}>
                <StatusBar style="light" />
                <PairScreen onPair={pair} onBack={() => setRoute("home")} error={pairError} />
            </SafeAreaView>
        );
    }

    if (route !== "session" || !pairing) {
        return (
            <SafeAreaView style={styles.root} edges={EDGES}>
                <StatusBar style="light" />
                <HomeScreen
                    pairing={pairing}
                    onOpen={() => {
                        setTab("screen");
                        setOpenId((id) => id + 1);
                        setRoute("session");
                    }}
                    onAdd={() => {
                        setPairError(null);
                        setRoute("pair");
                    }}
                />
            </SafeAreaView>
        );
    }

    const connected = session.state === "connected";
    const down =
        (session.state === "error" || session.state === "closed") && session.reconnectingIn === null;
    const leave = () => {
        setFullscreen(false);
        setRoute("home");
    };

    return (
        <SafeAreaView style={styles.root} edges={fullscreen ? [] : EDGES}>
            <StatusBar style="light" hidden={fullscreen} />

            {fullscreen ? null : (
                <View style={styles.topBar}>
                    <View style={styles.leading}>
                        <Pressable accessibilityRole="button" accessibilityLabel="Desktops" onPress={leave}>
                            <Text style={styles.backLabel}>Desktops</Text>
                        </Pressable>
                        <ConnectionPill state={session.state} name={pairing.serverName} />
                    </View>
                    <View style={styles.tabs}>
                        <TabButton label="Screen" active={tab === "screen"} onPress={() => setTab("screen")} />
                        <TabButton label="Settings" active={tab === "settings"} onPress={() => setTab("settings")} />
                    </View>
                </View>
            )}

            {!fullscreen && session.reconnectingIn !== null ? (
                <View style={styles.banner} accessibilityLiveRegion="polite">
                    <Text style={styles.bannerText}>Reconnecting in {session.reconnectingIn} s</Text>
                </View>
            ) : null}

            {tab === "screen" ? (
                <View style={styles.stage}>
                    {connected && session.frame ? (
                        <Picture frame={session.frame} onH264Failed={failH264} />
                    ) : down ? (
                        <View style={styles.stateBlock}>
                            <Text style={type.title}>Open the app on the PC</Text>
                            <Text style={[type.body, styles.stateDetail]}>
                                k-virtual-monitor has to be running on {pairing.serverName} before this phone can connect.
                            </Text>
                            {session.detail ? (
                                <Text style={[type.caption, styles.stateDetail]}>{session.detail}</Text>
                            ) : null}
                            <Pressable
                                accessibilityRole="button"
                                style={styles.action}
                                onPress={() => setReloadToken((token) => token + 1)}
                            >
                                <Text style={styles.actionLabel}>Try again</Text>
                            </Pressable>
                            <Pressable accessibilityRole="button" style={styles.actionSecondary} onPress={leave}>
                                <Text style={styles.backLabel}>Back</Text>
                            </Pressable>
                        </View>
                    ) : (
                        <View style={styles.stateBlock}>
                            <Text style={type.title}>{stateTitle(session.state)}</Text>
                            {session.detail ? <Text style={[type.caption, styles.stateDetail]}>{session.detail}</Text> : null}
                            {session.reconnectingIn !== null ? (
                                <Text style={[type.caption, styles.stateDetail]}>
                                    Next attempt in {session.reconnectingIn} s
                                </Text>
                            ) : null}
                        </View>
                    )}

                    {settings.showStats && session.info && !fullscreen ? (
                        <View style={styles.overlay}>
                            <Text
                                style={styles.overlayText}
                                accessibilityRole="text"
                                accessibilityLabel={`Stream: ${streamCodec(session.info.codec)}, ${session.info.width} by ${session.info.height}, ${session.info.fps} frames per second`}
                            >
                                {streamCodec(session.info.codec)} · {session.info.width}x
                                {session.info.height} · {session.info.fps} fps
                            </Text>
                        </View>
                    ) : null}

                    {fullscreen || down || !session.frame ? null : (
                        <Pressable
                            accessibilityRole="button"
                            accessibilityLabel="Full screen"
                            style={styles.fullButton}
                            onPress={() => setFullscreen(true)}
                        >
                            <Text style={styles.fullLabel}>Full screen</Text>
                        </Pressable>
                    )}
                </View>
            ) : (
                <ScrollView style={styles.scroll}>
                    <SettingsScreen
                        settings={settings}
                        onChange={updateSettings}
                        pairing={pairing}
                        negotiatedCodec={session.info ? session.info.codec : null}
                        onUnpair={() => void unpair()}
                    />
                </ScrollView>
            )}
        </SafeAreaView>
    );
}

function ConnectionPill({ state, name }: { state: string; name: string }) {
    const offline = state === "error" || state === "closed";
    const tone = state === "connected" ? colors.ok : offline ? colors.danger : colors.warn;
    const label = state === "connected" ? "Connected" : offline ? "Offline" : "Connecting";
    return (
        <View
            style={styles.pill}
            accessibilityRole="text"
            accessibilityLabel={`${label} to ${name}`}
        >
            <View style={[styles.pillDot, { backgroundColor: tone }]} />
            <Text style={styles.pillText}>{name}</Text>
        </View>
    );
}

function TabButton({
    label,
    active,
    onPress,
}: {
    label: string;
    active: boolean;
    onPress: () => void;
}) {
    return (
        <Pressable
            accessibilityRole="tab"
            accessibilityState={{ selected: active }}
            accessibilityLabel={label}
            style={[styles.tab, active && styles.tabActive]}
            onPress={onPress}
        >
            <Text style={[styles.tabLabel, active && styles.tabLabelActive]}>{label}</Text>
        </Pressable>
    );
}

const H264_WIRE = codecWireValue(Codec.H264);
const JPEG_WIRE = codecWireValue(Codec.JPEG);

function Picture({ frame, onH264Failed }: { frame: Frame; onH264Failed: () => void }) {
    const video = useRef<KvmVideoRef>(null);
    const h264 = frame.codec === H264_WIRE;

    useEffect(() => {
        if (!h264) {
            return;
        }
        if (!decoderAvailable()) {
            onH264Failed();
            return;
        }
        const view = video.current;
        if (!view) {
            return;
        }
        void view.pushFrame(toBase64(frame.data), frame.keyframe, frame.width, frame.height).catch(() => {
            onH264Failed();
        });
    }, [frame, h264, onH264Failed]);

    if (h264 && decoderAvailable()) {
        return <KvmVideoView ref={video} style={styles.picture} onDecodeError={() => onH264Failed()} />;
    }
    if (frame.codec === JPEG_WIRE) {
        return (
            <Image
                style={styles.picture}
                source={{ uri: `data:image/jpeg;base64,${toBase64(frame.data)}` }}
                resizeMode="contain"
            />
        );
    }
    return null;
}

/** The codec of the live session, from the wire value the server sent. */
function streamCodec(wireValue: number): string {
    const bit = codecFromWire(wireValue);
    return bit === null ? "unknown" : codecName(bit).toUpperCase();
}

function stateTitle(state: string): string {
    return state === "connected" ? "Waiting for the first frame" : "Connecting";
}

function toBase64(bytes: Uint8Array): string {
    let binary = "";
    for (const byte of bytes) {
        binary += String.fromCharCode(byte);
    }
    return globalThis.btoa(binary);
}

const styles = StyleSheet.create({
    root: {
        flex: 1,
        backgroundColor: colors.surface,
    },
    topBar: {
        height: 48,
        flexDirection: "row",
        alignItems: "center",
        justifyContent: "space-between",
        paddingHorizontal: space[4],
        borderBottomWidth: 1,
        borderBottomColor: colors.border,
    },
    leading: {
        flexDirection: "row",
        alignItems: "center",
        gap: space[3],
        flexShrink: 1,
    },
    backLabel: {
        color: colors.ink,
        fontSize: 13,
        fontWeight: "600",
    },
    action: {
        marginTop: space[3],
        backgroundColor: colors.accent,
        borderRadius: radius.control,
        paddingVertical: space[3],
        paddingHorizontal: space[5],
    },
    actionLabel: {
        color: colors.surface,
        fontSize: 15,
        fontWeight: "600",
    },
    actionSecondary: {
        paddingVertical: space[2],
    },
    pill: {
        flexDirection: "row",
        alignItems: "center",
        gap: space[2],
    },
    pillDot: {
        width: 8,
        height: 8,
        borderRadius: 4,
    },
    pillText: {
        fontSize: 13,
        lineHeight: 18,
        color: colors.muted,
    },
    tabs: {
        flexDirection: "row",
        gap: space[2],
    },
    tab: {
        paddingHorizontal: space[3],
        paddingVertical: space[2],
        borderRadius: radius.segment,
        borderWidth: 1,
        borderColor: colors.border,
    },
    tabActive: {
        borderColor: colors.accent,
        backgroundColor: colors.accentSoft,
    },
    tabLabel: {
        fontSize: 13,
        lineHeight: 18,
        color: colors.muted,
    },
    tabLabelActive: {
        color: colors.ink,
        fontWeight: "600",
    },
    banner: {
        height: 28,
        justifyContent: "center",
        backgroundColor: "rgba(42,53,87,0.9)",
    },
    bannerText: {
        color: colors.ink,
        fontSize: 13,
        textAlign: "center",
    },
    stage: {
        flex: 1,
        backgroundColor: colors.surface,
    },
    picture: {
        flex: 1,
    },
    stateBlock: {
        flex: 1,
        alignItems: "center",
        justifyContent: "center",
        gap: space[2],
        paddingHorizontal: space[6],
    },
    stateDetail: {
        textAlign: "center",
        maxWidth: "70%",
    },
    overlay: {
        position: "absolute",
        top: space[2],
        left: space[2],
        backgroundColor: "rgba(11,13,18,0.72)",
        borderRadius: radius.overlay,
        borderWidth: 1,
        borderColor: "rgba(34,40,54,0.9)",
        paddingHorizontal: space[2],
        paddingVertical: space[1],
    },
    overlayText: {
        color: colors.muted,
        fontFamily: "monospace",
        fontSize: 11,
        lineHeight: 14,
        fontVariant: ["tabular-nums"],
    },
    fullButton: {
        position: "absolute",
        top: space[2],
        right: space[2],
        backgroundColor: "rgba(11,13,18,0.72)",
        borderRadius: radius.overlay,
        borderWidth: 1,
        borderColor: "rgba(34,40,54,0.9)",
        paddingHorizontal: space[3],
        paddingVertical: space[2],
    },
    fullLabel: {
        color: colors.ink,
        fontSize: 13,
        fontWeight: "600",
    },
    scroll: {
        flex: 1,
    },
});
