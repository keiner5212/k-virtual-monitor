/**
 * The pairing screen: scan the code of the desktop, or type it.
 *
 * The code carries the pairing secret, so the phone can prove to the desktop
 * that it is allowed to connect, and the desktop can prove the same back.
 * The six digit safety code is shown on both screens so the user can confirm
 * they paired with the right desktop.
 */

import { CameraView, useCameraPermissions } from "expo-camera";
import { useState } from "react";
import { Pressable, StyleSheet, Text, TextInput, View } from "react-native";

import { decodePairingBlob, formatSafetyCode, pairingSafetyCode } from "@kvm/protocol";

import { colors, radius, space, type } from "../ui";

interface PairScreenProps {
    onPair: (blob: string) => Promise<void>;
    error: string | null;
}

export default function PairScreen({ onPair, error }: PairScreenProps) {
    const [permission, requestPermission] = useCameraPermissions();
    const [scanned, setScanned] = useState(false);
    const [manual, setManual] = useState("");
    const [busy, setBusy] = useState(false);
    const [safety, setSafety] = useState<string | null>(null);

    const submit = async (blob: string) => {
        if (busy) {
            return;
        }
        setBusy(true);
        try {
            // The safety code comes from the secret inside the code, so it can be
            // shown before the connection is even established.
            try {
                setSafety(formatSafetyCode(pairingSafetyCode(decodePairingBlob(blob.trim()))));
            } catch {
                setSafety(null);
            }
            await onPair(blob);
        } catch {
            // The error is shown by the caller.
        } finally {
            setBusy(false);
        }
    };

    if (!permission) {
        return <View style={styles.root} />;
    }

    if (!permission.granted) {
        return (
            <View style={styles.root}>
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

    if (safety) {
        return (
            <View style={styles.root}>
                <Text style={type.title}>Compare the code</Text>
                <View style={styles.card} accessible accessibilityLabel={`Safety code ${safety.replace(" ", " ")}. Compare it with the code on the desktop.`}>
                    <Text style={type.uppercase}>Compare with the desktop</Text>
                    <View style={styles.codeRow}>
                        {safety.split(" ").map((group) => (
                            <Text key={group} style={styles.code}>
                                {group}
                            </Text>
                        ))}
                    </View>
                    <Text style={type.body}>The two codes must match.</Text>
                    <Text style={type.caption}>If they do not, stop and scan again.</Text>
                </View>
            </View>
        );
    }

    return (
        <View style={styles.root}>
            <Text style={type.title}>Pair a desktop</Text>

            <View style={styles.scannerBox}>
                <CameraView
                    style={styles.camera}
                    barcodeScannerSettings={{ barcodeTypes: ["qr"] }}
                    onBarcodeScanned={
                        scanned
                            ? undefined
                            : (result) => {
                                setScanned(true);
                                void submit(result.data);
                            }
                    }
                />
                <View pointerEvents="none" style={styles.reticle} />
            </View>
            <Text style={[type.caption, styles.hint]}>Point the camera at the QR code on the desktop</Text>

            <Text style={[type.bodyStrong, styles.label]}>Pairing code</Text>
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
            <Text style={type.caption}>
                Open the desktop app, choose Pair a phone, and copy the code.
            </Text>

            <Pressable
                accessibilityRole="button"
                accessibilityState={{ disabled: !manual.trim() || busy }}
                style={[styles.primary, (!manual.trim() || busy) && styles.primaryDisabled]}
                onPress={() => void submit(manual)}
                disabled={!manual.trim() || busy}
            >
                <Text style={styles.primaryLabel}>{busy ? "Pairing..." : "Pair"}</Text>
            </Pressable>

            {error ? (
                <Text accessibilityLiveRegion="assertive" style={styles.error}>
                    {error}
                </Text>
            ) : null}
        </View>
    );
}

const styles = StyleSheet.create({
    root: {
        flex: 1,
        padding: space[4],
        gap: space[3],
    },
    explanation: {
        color: colors.muted,
    },
    scannerBox: {
        height: 320,
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
        paddingVertical: 13,
        alignItems: "center",
    },
    primaryDisabled: {
        opacity: 0.5,
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
        gap: 16,
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
