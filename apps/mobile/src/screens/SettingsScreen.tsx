/**
 * The settings screen.
 *
 * The compression section shows what the phone asks for and what the server
 * actually chose, so the user can tell whether the stream is compressed and
 * whether it is decoded in hardware.
 */

import { Alert, Pressable, StyleSheet, Text, View } from "react-native";

import { codecFromWire, codecName, Codec, type DisplayInfo } from "@kvm/protocol";

import type { StoredPairing } from "../lib/pairing";
import { CODEC_PREFERENCES, type AppSettings, type CodecPreference } from "../lib/settings";
import { Chip, colors, radius, Row, Section, Segmented, space, ToggleRow, type } from "../ui";

interface SettingsScreenProps {
    settings: AppSettings;
    onChange: (settings: AppSettings) => void;
    pairing: StoredPairing | null;
    /** What the desktop decided for this phone. */
    streamInfo: DisplayInfo | null;
    onUnpair: () => void;
}

export default function SettingsScreen({
    settings,
    onChange,
    pairing,
    streamInfo,
    onUnpair,
}: SettingsScreenProps) {
    const detected = streamInfo === null ? null : codecFromWire(streamInfo.codec);
    const preference = CODEC_PREFERENCES.find(
        (option) => option.value === settings.codecPreference,
    );
    const mismatch =
        detected !== null &&
        settings.codecPreference !== "auto" &&
        detected !== codecBit(settings.codecPreference);
    const live = streamInfo !== null;
    const valueStyle = live ? type.mono : type.monoMuted;

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

            <Section
                title="Stream"
                hint="The desktop sets the frame size and the frame rate for every phone."
            >
                <Row label="Resolution">
                    <Text style={valueStyle}>
                        {live ? `${streamInfo.width} x ${streamInfo.height}` : "Not connected"}
                    </Text>
                </Row>
                <Row label="Frame rate">
                    <Text style={valueStyle}>{live ? `${streamInfo.fps} fps` : "Not connected"}</Text>
                </Row>
                <Row label="Codec">
                    <Text style={valueStyle}>
                        {detected === null ? "Not connected" : codecName(detected).toUpperCase()}
                    </Text>
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
        paddingHorizontal: space[4],
        paddingTop: space[4],
        paddingBottom: space[10],
    },
    detected: {
        paddingHorizontal: space[4],
        paddingBottom: space[3],
        gap: space[1],
    },
    detectedRow: {
        flexDirection: "row",
        alignItems: "center",
        gap: space[2],
    },
    mismatch: {
        color: colors.warn,
    },
    hintBox: {
        paddingHorizontal: space[4],
        paddingBottom: space[3],
    },
    empty: {
        padding: space[4],
    },
    dangerButton: {
        margin: space[3],
        paddingVertical: space[3],
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
