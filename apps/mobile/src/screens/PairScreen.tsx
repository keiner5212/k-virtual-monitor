/**
 * The pairing screen: scan the code of the desktop, or type it.
 *
 * The code carries the pairing secret, so the phone can prove to the desktop
 * that it is allowed to connect, and the desktop can prove the same back.
 */

import { CameraView, useCameraPermissions } from "expo-camera";
import { useState } from "react";
import { Pressable, StyleSheet, Text, TextInput, View } from "react-native";

import { colors } from "../ui";

interface PairScreenProps {
    onPair: (blob: string) => Promise<void>;
    error: string | null;
}

export default function PairScreen({ onPair, error }: PairScreenProps) {
    const [permission, requestPermission] = useCameraPermissions();
    const [scanned, setScanned] = useState(false);
    const [manual, setManual] = useState("");
    const [busy, setBusy] = useState(false);

    const submit = async (blob: string) => {
        if (busy) {
            return;
        }
        setBusy(true);
        try {
            await onPair(blob);
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
                <Text style={styles.title}>Pair a desktop</Text>
                <Text style={styles.body}>
                    The camera reads the pairing code of the desktop. The code never leaves the local
                    network.
                </Text>
                <Pressable style={styles.primary} onPress={() => void requestPermission()}>
                    <Text style={styles.primaryLabel}>Allow the camera</Text>
                </Pressable>
            </View>
        );
    }

    return (
        <View style={styles.root}>
            <Text style={styles.title}>Pair a desktop</Text>
            <View style={styles.cameraBox}>
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
            </View>
            <Text style={styles.body}>Or paste the pairing code.</Text>
            <TextInput
                style={styles.input}
                value={manual}
                onChangeText={setManual}
                placeholder="kvm1:..."
                placeholderTextColor={colors.muted}
                autoCapitalize="none"
                autoCorrect={false}
                multiline
            />
            <Pressable
                style={[styles.primary, (!manual.trim() || busy) && styles.disabled]}
                onPress={() => void submit(manual)}
                disabled={!manual.trim() || busy}
            >
                <Text style={styles.primaryLabel}>{busy ? "Pairing..." : "Pair"}</Text>
            </Pressable>
            {error ? <Text style={styles.error}>{error}</Text> : null}
        </View>
    );
}

const styles = StyleSheet.create({
    root: {
        flex: 1,
        padding: 16,
        gap: 12,
    },
    title: {
        color: colors.ink,
        fontSize: 20,
        fontWeight: "700",
    },
    body: {
        color: colors.muted,
        fontSize: 13,
    },
    cameraBox: {
        height: 320,
        borderRadius: 14,
        overflow: "hidden",
        borderWidth: 1,
        borderColor: colors.border,
        backgroundColor: colors.raised,
    },
    camera: {
        flex: 1,
    },
    input: {
        minHeight: 72,
        borderRadius: 10,
        borderWidth: 1,
        borderColor: colors.border,
        backgroundColor: colors.raised,
        color: colors.ink,
        padding: 12,
        fontSize: 12,
    },
    primary: {
        borderRadius: 10,
        backgroundColor: colors.accent,
        paddingVertical: 13,
        alignItems: "center",
    },
    primaryLabel: {
        color: colors.surface,
        fontSize: 15,
        fontWeight: "600",
    },
    disabled: {
        opacity: 0.5,
    },
    error: {
        color: colors.danger,
        fontSize: 13,
    },
});
