/**
 * The pairing screen: scan the code of the desktop, or type it.
 *
 * The code carries the pairing secret, so the phone can prove to the desktop
 * that it is allowed to connect, and the desktop can prove the same back.
 * The six digit safety code is shown on both screens so the user can confirm
 * they paired with the right desktop.
 */

import { CameraView, useCameraPermissions, type BarcodeScanningResult } from "expo-camera";
import { useCallback, useMemo, useRef, useState } from "react";
import {
    Pressable,
    ScrollView,
    StyleSheet,
    Text,
    TextInput,
    View,
} from "react-native";

import { decodePairingBlob, formatSafetyCode, pairingSafetyCode } from "@kvm/protocol";

import { colors, radius, space, type } from "../ui";

interface PairScreenProps {
    onPair: (blob: string) => Promise<void>;
    onBack: () => void;
    error: string | null;
}

export default function PairScreen({ onPair, onBack, error }: PairScreenProps) {
    const [permission, requestPermission] = useCameraPermissions();
    const [manual, setManual] = useState("");
    const [busy, setBusy] = useState(false);
    const [cameraOn, setCameraOn] = useState(false);
    const [torch, setTorch] = useState(false);
    const [safety, setSafety] = useState<string | null>(null);
    const [pending, setPending] = useState<string | null>(null);
    const [localError, setLocalError] = useState<string | null>(null);
    const lock = useRef(false);
    const barcodeSettings = useMemo(() => ({ barcodeTypes: ["qr" as const] }), []);

    const stage = useCallback((blob: string) => {
        const trimmed = blob.trim();
        try {
            setSafety(formatSafetyCode(pairingSafetyCode(decodePairingBlob(trimmed))));
            setPending(trimmed);
            setLocalError(null);
        } catch {
            lock.current = false;
            setSafety(null);
            setPending(null);
            setLocalError("That is not a pairing code. Point the camera at the code on the desktop.");
        }
    }, []);

    const onBarcodeScanned = useCallback((result: BarcodeScanningResult) => {
        if (lock.current) {
            return;
        }
        const payload = `${result.raw || result.data || ""}`.trim();
        if (!payload || payload === "null") {
            setLocalError("The camera read an empty code. Point it at the code on the desktop.");
            return;
        }
        lock.current = true;
        setCameraOn(false);
        stage(payload);
    }, [stage]);

    const scan = async () => {
        const current = permission?.granted ? permission : await requestPermission();
        if (!current.granted) {
            setLocalError("The camera is needed to scan. Paste the code instead.");
            return;
        }
        lock.current = false;
        setLocalError(null);
        setCameraOn(true);
    };

    const confirm = async () => {
        if (!pending || busy) {
            return;
        }
        setBusy(true);
        try {
            await onPair(pending);
        } catch {
            // The error is shown by the caller.
        } finally {
            setBusy(false);
        }
    };

    const scanAgain = () => {
        lock.current = false;
        setSafety(null);
        setPending(null);
        setLocalError(null);
    };

    if (cameraOn) {
        return (
            <View style={styles.scanner}>
                <CameraView
                    style={styles.camera}
                    facing="back"
                    mode="picture"
                    enableTorch={torch}
                    barcodeScannerSettings={barcodeSettings}
                    onBarcodeScanned={onBarcodeScanned}
                    onMountError={(event) => {
                        setCameraOn(false);
                        setLocalError(event.message || "The camera could not open.");
                    }}
                    accessible={false}
                    importantForAccessibility="no"
                    accessibilityElementsHidden
                />
                <View style={styles.scannerShade} pointerEvents="none" importantForAccessibility="no" />
                {localError ? (
                    <View style={styles.scannerError}>
                        <Text
                            accessibilityRole="alert"
                            accessibilityLiveRegion="assertive"
                            numberOfLines={3}
                            style={[styles.error, styles.scannerErrorText]}
                        >
                            {localError}
                        </Text>
                    </View>
                ) : null}
                <Text
                    style={styles.scannerHint}
                    accessibilityRole="text"
                    accessibilityLabel="Camera viewfinder. Fill the frame with the code shown on the desktop."
                >
                    Fill the frame with the code on the desktop
                </Text>
                <View style={styles.scannerBar}>
                    <Pressable accessibilityRole="button" style={[styles.secondary, styles.scannerButton]} onPress={() => setTorch((on) => !on)}>
                        <Text style={styles.secondaryLabel}>{torch ? "Light off" : "Light"}</Text>
                    </Pressable>
                    <Pressable accessibilityRole="button" style={[styles.secondary, styles.scannerButton]} onPress={() => setCameraOn(false)}>
                        <Text style={styles.secondaryLabel}>Close</Text>
                    </Pressable>
                </View>
            </View>
        );
    }

    if (safety && pending) {
        return (
            <View style={[styles.root, styles.scroll]}>
                <Text style={type.title}>Compare the code</Text>
                <View style={styles.card} accessible accessibilityLabel={`Safety code ${safety}. Compare it with the code on the desktop.`}>
                    <Text style={type.uppercase}>Compare this code with your phone</Text>
                    <View style={styles.codeRow}>
                        {safety.split(" ").map((group) => (
                            <Text key={group} style={styles.code}>
                                {group}
                            </Text>
                        ))}
                    </View>
                    <Text style={type.body}>These six digits must match the desktop.</Text>
                    <Text style={type.caption}>
                        They prove this phone paired with that computer. If they differ, scan again.
                    </Text>
                </View>
                <Pressable
                    accessibilityRole="button"
                    style={[styles.primary, busy && styles.primaryDisabled]}
                    onPress={() => void confirm()}
                    disabled={busy}
                >
                    <Text style={styles.primaryLabel}>{busy ? "Pairing..." : "Codes match"}</Text>
                </Pressable>
                <Pressable accessibilityRole="button" style={styles.secondary} onPress={scanAgain}>
                    <Text style={styles.secondaryLabel}>Scan again</Text>
                </Pressable>
                <Pressable accessibilityRole="button" style={styles.secondary} onPress={onBack}>
                    <Text style={styles.secondaryLabel}>Back</Text>
                </Pressable>
                {error ? (
                    <Text accessibilityLiveRegion="assertive" style={styles.error}>
                        {error}
                    </Text>
                ) : null}
            </View>
        );
    }

    return (
        <View style={styles.screen}>
            <Text style={type.title}>Pair a desktop</Text>
            <Text style={[type.body, styles.explanation]}>
                The scanner reads the code shown on the PC. The code never leaves the local network.
            </Text>
            <Pressable
                accessibilityRole="button"
                style={styles.primary}
                onPress={() => void scan()}
            >
                <Text style={styles.primaryLabel}>Scan the code</Text>
            </Pressable>
            <Pressable accessibilityRole="button" style={styles.secondary} onPress={onBack}>
                <Text style={styles.secondaryLabel}>Back</Text>
            </Pressable>
            <ScrollView
                style={styles.scroll}
                contentContainerStyle={styles.manual}
                keyboardShouldPersistTaps="handled"
            >
                <Text style={type.bodyStrong}>Pairing code</Text>
                <TextInput
                    accessibilityLabel="Pairing code"
                    accessibilityHint="Paste the code shown on the desktop"
                    style={styles.input}
                    value={manual}
                    onChangeText={setManual}
                    placeholder="kvm1:..."
                    placeholderTextColor={colors.muted}
                    autoCapitalize="none"
                    autoCorrect={false}
                    autoComplete="off"
                    spellCheck={false}
                    multiline
                />
                <Pressable
                    accessibilityRole="button"
                    accessibilityState={{ disabled: !manual.trim() || busy }}
                    style={[styles.primary, (!manual.trim() || busy) && styles.primaryDisabled]}
                    onPress={() => stage(manual)}
                    disabled={!manual.trim() || busy}
                >
                    <Text style={styles.primaryLabel}>Pair</Text>
                </Pressable>
                {localError || error ? (
                    <Text accessibilityLiveRegion="assertive" style={styles.error}>
                        {localError ?? error}
                    </Text>
                ) : null}
            </ScrollView>
        </View>
    );
}

const styles = StyleSheet.create({
    scroll: {
        flex: 1,
    },
    screen: {
        flex: 1,
        padding: space[4],
        gap: space[3],
    },
    scanner: {
        flex: 1,
        backgroundColor: "#000",
    },
    camera: {
        flex: 1,
    },
    scannerShade: {
        position: "absolute",
        top: "18%",
        left: "12%",
        right: "12%",
        bottom: "28%",
        borderWidth: 2,
        borderColor: colors.accent,
        borderRadius: 12,
    },
    scannerHint: {
        position: "absolute",
        left: space[4],
        right: space[4],
        bottom: 88,
        textAlign: "center",
        color: colors.ink,
        fontSize: 14,
        lineHeight: 20,
    },
    scannerError: {
        position: "absolute",
        bottom: "82%",
        marginBottom: space[2],
        alignSelf: "center",
        maxWidth: "76%",
        backgroundColor: colors.raised,
        borderWidth: 1,
        borderColor: colors.danger,
        borderRadius: radius.overlay,
        paddingHorizontal: space[3],
        paddingVertical: space[2],
    },
    scannerErrorText: {
        textAlign: "center",
    },
    scannerBar: {
        position: "absolute",
        left: space[4],
        right: space[4],
        bottom: space[4],
        flexDirection: "row",
        gap: space[3],
    },
    scannerButton: {
        flex: 1,
        backgroundColor: colors.raised,
    },
    manual: {
        gap: space[3],
        paddingBottom: space[6],
    },
    root: {
        padding: space[4],
        gap: space[3],
    },
    explanation: {
        color: colors.muted,
    },
    label: {
        marginTop: space[2],
    },
    input: {
        minHeight: 72,
        borderRadius: radius.control,
        borderWidth: 1,
        borderColor: colors.borderStrong,
        backgroundColor: colors.raised,
        color: colors.ink,
        padding: space[3],
        fontFamily: "monospace",
        fontSize: 12,
        lineHeight: 17,
        textAlignVertical: "top",
    },
    primary: {
        borderRadius: radius.control,
        backgroundColor: colors.accent,
        paddingVertical: space[3],
        alignItems: "center",
    },
    primaryDisabled: {
        opacity: 0.5,
    },
    secondary: {
        borderRadius: radius.control,
        borderWidth: 1,
        borderColor: colors.borderStrong,
        paddingVertical: space[3],
        alignItems: "center",
    },
    secondaryLabel: {
        color: colors.ink,
        fontSize: 15,
        fontWeight: "600",
    },
    primaryLabel: {
        color: colors.surface,
        fontSize: 15,
        fontWeight: "600",
    },
    error: {
        color: colors.danger,
        fontSize: 13,
        lineHeight: 18,
    },
    card: {
        backgroundColor: colors.raised,
        borderWidth: 1,
        borderColor: colors.accentSoft,
        borderRadius: radius.panel,
        paddingVertical: space[5],
        paddingHorizontal: space[4],
        alignItems: "center",
        gap: space[2],
    },
    codeRow: {
        flexDirection: "row",
        gap: space[4],
    },
    code: {
        fontFamily: "monospace",
        fontSize: 40,
        lineHeight: 48,
        fontWeight: "600",
        color: colors.ink,
        letterSpacing: 4,
        fontVariant: ["tabular-nums"],
    },
});
