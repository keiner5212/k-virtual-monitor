/**
 * The settings screen.
 *
 * The compression section shows what the phone asks for and what the server
 * actually chose, so the user can tell whether the stream is compressed and
 * whether it is decoded in hardware.
 */

import { Alert, Pressable, StyleSheet, Text, View } from "react-native";

import { codecFromWire, codecName, Codec } from "@kvm/protocol";

import type { StoredPairing } from "../lib/pairing";
import {
    CODEC_PREFERENCES,
    FRAME_RATES,
    RESOLUTIONS,
    type AppSettings,
    type CodecPreference,
} from "../lib/settings";
import { Chip, colors, radius, Row, Section, Segmented, Stepper, ToggleRow, type } from "../ui";

interface SettingsScreenProps {
    settings: AppSettings;
    onChange: (settings: AppSettings) => void;
    pairing: StoredPairing | null;
    negotiatedCodec: number | null;
    onUnpair: () => void;
}

const RECONNECT_DELAYS = [500, 1000, 2000, 5000];
const RECONNECT_MAX_DELAYS = [5000, 10000, 30000, 60000];

export default function SettingsScreen({
    settings,
    onChange,
    pairing,
    negotiatedCodec,
    onUnpair,
}: SettingsScreenProps) {
    const detected = negotiatedCodec === null ? null : codecFromWire(negotiatedCodec);
    const preference = CODEC_PREFERENCES.find(
        (option) => option.value === settings.codecPreference,
    );
    const mismatch =
        detected !== null &&
        settings.codecPreference !== "auto" &&
        detected !== codecBit(settings.codecPreference);

    return (
        <View style={styles.root}>
            <Section
                title="Compression"
                hint="The server picks the most compressed codec both ends support."
            >
                <Segmented<CodecPreference>
                    options={CODEC_PREFERENCES.map((option) => ({ value: option.value, label: option.label }))}
                    value={settings.codecPreference}
                    onChange={(codecPreference) => onChange({ ...settings, codecPreference })}
                    accessibilityLabel="Compression"
                />
                <View style={styles.detected}>
                    <Text style={type.caption}>Detected</Text>
                    <View style={styles.detectedRow}>
                        <Text style={type.mono}>
                            {detected === null ? "Not connected" : codecName(detected).toUpperCase()}
                        </Text>
                        {detected !== null ? <DecoderChip codec={detected} /> : null}
                    </View>
                    <Text style={type.caption}>{detected === null ? "not connected yet" : decoderNote(detected)}</Text>
                    {mismatch ? (
                        <Text style={[type.caption, styles.mismatch]}>
                            The desktop chose a different codec.
                        </Text>
                    ) : null}
                </View>
                <View style={styles.hintBox}>
                    <Text style={type.caption}>{preference?.hint}</Text>
                </View>
            </Section>

            <Section title="Stream" hint="The largest frame and frame rate the phone accepts.">
                <Row label="Resolution">
                    <Stepper
                        value={settings.maxWidth}
                        options={RESOLUTIONS.map((resolution) => resolution.width)}
                        onChange={(maxWidth) => {
                            const resolution = RESOLUTIONS.find((option) => option.width === maxWidth);
                            onChange({
                                ...settings,
                                maxWidth,
                                maxHeight: resolution?.height ?? settings.maxHeight,
                            });
                        }}
                        format={(width) =>
                            RESOLUTIONS.find((option) => option.width === width)?.label ?? String(width)
                        }
                        accessibilityLabel="Resolution"
                    />
                </Row>
                <Row label="Frame rate">
                    <Stepper
                        value={settings.maxFps}
                        options={FRAME_RATES}
                        onChange={(maxFps) => onChange({ ...settings, maxFps })}
                        format={(fps) => `${fps} fps`}
                        accessibilityLabel="Frame rate"
                    />
                </Row>
            </Section>

            <Section
                title="Connection"
                hint="How fast the phone comes back when the desktop goes away."
            >
                <Row label="First retry">
                    <Stepper
                        value={settings.reconnectDelayMs}
                        options={RECONNECT_DELAYS}
                        onChange={(reconnectDelayMs) => onChange({ ...settings, reconnectDelayMs })}
                        format={(delay) => `${delay / 1000} s`}
                        accessibilityLabel="First retry"
                    />
                </Row>
                <Row label="Longest wait">
                    <Stepper
                        value={settings.reconnectMaxDelayMs}
                        options={RECONNECT_MAX_DELAYS}
                        onChange={(reconnectMaxDelayMs) => onChange({ ...settings, reconnectMaxDelayMs })}
                        format={(delay) => `${delay / 1000} s`}
                        accessibilityLabel="Longest wait"
                    />
                </Row>
            </Section>

            <Section title="Screen">
                <ToggleRow
                    label="Keep the screen on"
                    hint="Stops the screen from sleeping while a session is live."
                    value={settings.keepAwake}
                    onChange={(keepAwake) => onChange({ ...settings, keepAwake })}
                />
                <ToggleRow
                    label="Show the stream statistics"
                    hint="Shows the codec, the resolution and the frame rate over the picture."
                    value={settings.showStats}
                    onChange={(showStats) => onChange({ ...settings, showStats })}
                />
            </Section>

            <Section title="Pairing">
                {pairing ? (
                    <>
                        <Row label="Desktop" value={pairing.serverName} />
                        <Row label="Address" value={`${pairing.host}:${pairing.port}`} />
                        <Pressable
                            accessibilityRole="button"
                            style={styles.dangerButton}
                            onPress={() =>
                                Alert.alert(
                                    "Forget this desktop?",
                                    "The desktop has to be paired again before it can send pictures.",
                                    [
                                        { text: "Cancel", style: "cancel" },
                                        { text: "Forget", style: "destructive", onPress: () => onUnpair() },
                                    ],
                                )
                            }
                        >
                            <Text style={styles.dangerLabel}>Forget this desktop</Text>
                        </Pressable>
                    </>
                ) : (
                    <Text style={[type.body, styles.empty]}>No desktop paired yet.</Text>
                )}
            </Section>
        </View>
    );
}

function codecBit(preference: CodecPreference): number {
    switch (preference) {
        case "h264":
            return Codec.H264;
        case "h265":
            return Codec.H265;
        case "jpeg":
            return Codec.JPEG;
        default:
            return 0;
    }
}

function DecoderChip({ codec }: { codec: number }) {
    if (codec === Codec.H264 || codec === Codec.H265) {
        return <Chip label="hardware" tone="ok" />;
    }
    if (codec === Codec.JPEG) {
        return <Chip label="software" tone="warn" />;
    }
    return null;
}

function decoderNote(codec: number): string {
    if (codec === Codec.H264 || codec === Codec.H265) {
        return "decoded in hardware, the usual choice";
    }
    if (codec === Codec.JPEG) {
        return "decoded in software, the most bandwidth";
    }
    return "the decoder is not reported";
}

const styles = StyleSheet.create({
    root: {
        paddingHorizontal: 16,
        paddingTop: 16,
        paddingBottom: 48,
    },
    detected: {
        paddingHorizontal: 16,
        paddingBottom: 12,
        gap: 2,
    },
    detectedRow: {
        flexDirection: "row",
        alignItems: "center",
        gap: 8,
    },
    mismatch: {
        color: colors.warn,
    },
    hintBox: {
        paddingHorizontal: 16,
        paddingBottom: 12,
    },
    empty: {
        padding: 16,
    },
    dangerButton: {
        margin: 12,
        paddingVertical: 11,
        borderRadius: radius.control,
        borderWidth: 1,
        borderColor: colors.danger,
        alignItems: "center",
    },
    dangerLabel: {
        color: colors.danger,
        fontSize: 14,
        fontWeight: "600",
    },
});
