import { StatusBar } from "expo-status-bar";
import { useCallback, useEffect, useState } from "react";
import { Image, Pressable, SafeAreaView, ScrollView, StyleSheet, Text, View } from "react-native";
import { activateKeepAwakeAsync, deactivateKeepAwake } from "expo-keep-awake";

import { codecName } from "@kvm/protocol";

import { clearPairing, loadPairing, savePairing, type StoredPairing } from "./src/lib/pairing";
import { ensureCsprng } from "./src/lib/random";
import { DEFAULT_SETTINGS, loadSettings, saveSettings, type AppSettings } from "./src/lib/settings";
import { useSession } from "./src/lib/useSession";
import PairScreen from "./src/screens/PairScreen";
import SettingsScreen from "./src/screens/SettingsScreen";
import { colors, radius, space, type } from "./src/ui";

type Tab = "screen" | "settings";

export default function App() {
    const [tab, setTab] = useState<Tab>("screen");
    const [settings, setSettings] = useState<AppSettings>(DEFAULT_SETTINGS);
    const [pairing, setPairing] = useState<StoredPairing | null>(null);
    const [pairError, setPairError] = useState<string | null>(null);
    const [ready, setReady] = useState(false);

    useEffect(() => {
        void (async () => {
            ensureCsprng();
            const [stored, storedPairing] = await Promise.all([loadSettings(), loadPairing()]);
            setSettings(stored);
            setPairing(storedPairing);
            setReady(true);
        })();
    }, []);

    const session = useSession({ pairing, settings });

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
        setSettings(next);
        void saveSettings(next);
    }, []);

    const pair = useCallback(async (blob: string) => {
        setPairError(null);
        try {
            setPairing(await savePairing(blob));
        } catch (cause) {
            setPairError(cause instanceof Error ? cause.message : String(cause));
            throw cause;
        }
    }, []);

    const unpair = useCallback(async () => {
        await clearPairing();
        setPairing(null);
    }, []);

    if (!ready) {
        return (
            <SafeAreaView style={styles.root}>
                <StatusBar style="light" />
            </SafeAreaView>
        );
    }

    if (!pairing) {
        return (
            <SafeAreaView style={styles.root}>
                <StatusBar style="light" />
                <PairScreen onPair={pair} error={pairError} />
            </SafeAreaView>
        );
    }

    const connected = session.state === "connected";

    return (
        <SafeAreaView style={styles.root}>
            <StatusBar style="light" />

            <View style={styles.topBar}>
                <ConnectionPill state={session.state} name={pairing.serverName} />
                <View style={styles.tabs}>
                    <TabButton label="Screen" active={tab === "screen"} onPress={() => setTab("screen")} />
                    <TabButton label="Settings" active={tab === "settings"} onPress={() => setTab("settings")} />
                </View>
            </View>

            {!connected && session.state !== "idle" ? (
                <View style={styles.banner} accessibilityLiveRegion="polite">
                    <Text style={styles.bannerText}>
                        {session.state === "error"
                            ? `Connection error: ${session.detail ?? "unknown"}`
                            : session.reconnectingIn !== null
                                ? `Reconnecting in ${session.reconnectingIn} s`
                                : "Connecting..."}
                    </Text>
                </View>
            ) : null}

            {tab === "screen" ? (
                <View style={styles.stage}>
                    {connected && session.frame ? (
                        <Image
                            style={styles.picture}
                            source={{ uri: `data:image/jpeg;base64,${toBase64(session.frame.data)}` }}
                            resizeMode="contain"
                        />
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

                    {settings.showStats && session.info ? (
                        <View style={styles.overlay}>
                            <Text
                                style={styles.overlayText}
                                accessibilityRole="text"
                                accessibilityLabel={`Stream: ${codecName(session.info.codec)}, ${session.info.width} by ${session.info.height}, ${session.info.fps} frames per second`}
                            >
                                {codecName(session.info.codec).toUpperCase()} · {session.info.width}x
                                {session.info.height} · {session.info.fps} fps
                            </Text>
                        </View>
                    ) : null}
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
    const tone = state === "connected" ? colors.ok : state === "error" ? colors.danger : colors.warn;
    const label =
        state === "connected" ? "Connected" : state === "error" ? "Error" : "Connecting";
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

function stateTitle(state: string): string {
    switch (state) {
        case "connecting":
            return "Connecting";
        case "connected":
            return "Waiting for the first frame";
        case "error":
            return "Connection error";
        case "closed":
            return "Disconnected";
        default:
            return "Idle";
    }
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
        paddingVertical: 6,
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
        top: 10,
        left: 10,
        backgroundColor: "rgba(11,13,18,0.72)",
        borderRadius: radius.overlay,
        borderWidth: 1,
        borderColor: "rgba(34,40,54,0.9)",
        paddingHorizontal: 10,
        paddingVertical: 5,
    },
    overlayText: {
        color: colors.muted,
        fontFamily: "monospace",
        fontSize: 11,
        lineHeight: 14,
        fontVariant: ["tabular-nums"],
    },
    scroll: {
        flex: 1,
    },
});
