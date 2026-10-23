import { StatusBar } from "expo-status-bar";
import { useCallback, useEffect, useState } from "react";
import {
    Image,
    Pressable,
    SafeAreaView,
    ScrollView,
    StyleSheet,
    Text,
    View,
} from "react-native";
import { activateKeepAwakeAsync, deactivateKeepAwake } from "expo-keep-awake";

import { codecName } from "@kvm/protocol";

import { clearPairing, deviceId, loadPairing, savePairing, type StoredPairing } from "./src/lib/pairing";
import { DEFAULT_SETTINGS, loadSettings, saveSettings, type AppSettings } from "./src/lib/settings";
import { useSession } from "./src/lib/useSession";
import PairScreen from "./src/screens/PairScreen";
import SettingsScreen from "./src/screens/SettingsScreen";
import { colors } from "./src/ui";

type Tab = "screen" | "settings";

export default function App() {
    const [tab, setTab] = useState<Tab>("screen");
    const [settings, setSettings] = useState<AppSettings>(DEFAULT_SETTINGS);
    const [pairing, setPairing] = useState<StoredPairing | null>(null);
    const [phoneId, setPhoneId] = useState<Uint8Array | null>(null);
    const [pairError, setPairError] = useState<string | null>(null);
    const [ready, setReady] = useState(false);

    useEffect(() => {
        void (async () => {
            const [stored, storedPairing, id] = await Promise.all([
                loadSettings(),
                loadPairing(),
                deviceId(),
            ]);
            setSettings(stored);
            setPairing(storedPairing);
            setPhoneId(id);
            setReady(true);
        })();
    }, []);

    const session = useSession({ pairing, settings, deviceId: phoneId });

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

    return (
        <SafeAreaView style={styles.root}>
            <StatusBar style="light" />
            <View style={styles.tabs}>
                <TabButton label="Screen" active={tab === "screen"} onPress={() => setTab("screen")} />
                <TabButton label="Settings" active={tab === "settings"} onPress={() => setTab("settings")} />
            </View>

            {tab === "screen" ? (
                <View style={styles.screen}>
                    {session.frame ? (
                        <Image
                            style={styles.image}
                            source={{ uri: `data:image/jpeg;base64,${toBase64(session.frame.data)}` }}
                            resizeMode="contain"
                        />
                    ) : (
                        <View style={styles.center}>
                            <Text style={styles.state}>{stateLabel(session.state)}</Text>
                            {session.detail ? <Text style={styles.detail}>{session.detail}</Text> : null}
                            {session.reconnectingIn !== null ? (
                                <Text style={styles.detail}>Reconnecting in {session.reconnectingIn} s</Text>
                            ) : null}
                        </View>
                    )}
                    {settings.showStats && session.info ? (
                        <View style={styles.stats}>
                            <Text style={styles.statsText}>
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
        <Pressable style={[styles.tab, active && styles.tabActive]} onPress={onPress}>
            <Text style={[styles.tabLabel, active && styles.tabLabelActive]}>{label}</Text>
        </Pressable>
    );
}

function stateLabel(state: string): string {
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
    tabs: {
        flexDirection: "row",
        gap: 8,
        padding: 12,
        borderBottomWidth: 1,
        borderBottomColor: colors.border,
    },
    tab: {
        paddingHorizontal: 14,
        paddingVertical: 8,
        borderRadius: 9,
        borderWidth: 1,
        borderColor: colors.border,
    },
    tabActive: {
        borderColor: colors.accent,
        backgroundColor: colors.accentSoft,
    },
    tabLabel: {
        color: colors.muted,
        fontSize: 14,
    },
    tabLabelActive: {
        color: colors.ink,
        fontWeight: "600",
    },
    screen: {
        flex: 1,
    },
    image: {
        flex: 1,
    },
    center: {
        flex: 1,
        alignItems: "center",
        justifyContent: "center",
        gap: 6,
    },
    state: {
        color: colors.ink,
        fontSize: 16,
    },
    detail: {
        color: colors.muted,
        fontSize: 13,
    },
    stats: {
        position: "absolute",
        top: 10,
        right: 10,
        paddingHorizontal: 10,
        paddingVertical: 5,
        borderRadius: 8,
        backgroundColor: "#12151dcc",
    },
    statsText: {
        color: colors.muted,
        fontSize: 11,
    },
    scroll: {
        flex: 1,
    },
});
