/**
 * The pairing screen: scan the code of the desktop, or type it.
 *
 * The code carries the pairing secret, so the phone can prove to the desktop
 * that it is allowed to connect, and the desktop can prove the same back.
 * The six digit safety code is shown on both screens so the user can confirm
 * they paired with the right desktop.
 */

import { CameraView, useCameraPermissions } from "expo-camera";
import { useMemo, useRef, useState } from "react";
import {
    Pressable,
    ScrollView,
    StyleSheet,
    Text,
    TextInput,
    useWindowDimensions,
    View,
} from "react-native";

import { decodePairingBlob, formatSafetyCode, pairingSafetyCode } from "@kvm/protocol";

import { colors, radius, space, type } from "../ui";

interface PairScreenProps {
    onPair: (blob: string) => Promise<void>;
    error: string | null;
}

export default function PairScreen({ onPair, error }: PairScreenProps) {
    const [permission, requestPermission] = useCameraPermissions();
    const [manual, setManual] = useState("");
    const [busy, setBusy] = useState(false);
    const [safety, setSafety] = useState<string | null>(null);
    const [pending, setPending] = useState<string | null>(null);
    const [localError, setLocalError] = useState<string | null>(null);
    const lock = useRef(false);
    const { height } = useWindowDimensions();
    const barcodeSettings = useMemo(() => ({ barcodeTypes: ["qr" as const] }), []);

    const stage = (blob: string) => {
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

    if (!permission) {
        return <View style={styles.scroll} />;
    }

    if (!permission.granted) {
        return (
            <View style={[styles.root, styles.scroll]}>
                <Text style={type.title}>Pair a desktop</Text>
                <Text style={[type.body, styles.explanation]}>
                    The camera reads the pairing code of the desktop. The code never leaves the local network.
                </Text>
                <Pressable
                    accessibilityRole="button"
                    style={styles.primary}
                    onPress={() => void requestPermission()}
                >
                    <Text style={styles.primaryLabel}>Allow the camera</Text>
                </Pressable>
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
            <View style={[styles.scannerBox, { height: cameraHeight(height) }]}>
                <CameraView
                    style={styles.camera}
                    facing="back"
                    autofocus="on"
                    barcodeScannerSettings={barcodeSettings}
                    onBarcodeScanned={(result) => {
                        if (lock.current) {
                            return;
                        }
                        const data = result.data?.trim();
                        if (!data) {
                            return;
                        }
                        lock.current = true;
                        stage(data);
                    }}
                />
                <View pointerEvents="none" style={styles.reticle} />
            </View>
            <Text style={[type.caption, styles.hint]}>Point the camera at the QR code on the desktop</Text>
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

/** The scanner takes the room the screen has, without eating the rest. */
function cameraHeight(height: number): number {
    return Math.round(Math.min(420, Math.max(220, height * 0.46)));
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
    scannerBox: {
        borderRadius: radius.panel,
        borderWidth: 1,
        borderColor: colors.borderStrong,
        overflow: "hidden",
        backgroundColor: colors.raised,
    },
    camera: {
        flex: 1,
    },
    reticle: {
        position: "absolute",
        top: "50%",
        left: "50%",
        width: 200,
        height: 200,
        marginLeft: -100,
        marginTop: -100,
        borderWidth: 2,
        borderColor: colors.accent,
        borderRadius: 12,
    },
    hint: {
        textAlign: "center",
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
