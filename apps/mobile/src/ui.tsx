/**
 * The design tokens of the phone app, and the small building blocks shared by
 * the screens.
 *
 * The palette is the same as the desktop app, so the two look like the same
 * product. The spacing is a 4dp grid, the type has six roles and no more.
 */

import { Pressable, StyleSheet, Switch, Text, View } from "react-native";

export const colors = {
    surface: "#0b0d12",
    raised: "#12151d",
    border: "#222836",
    ink: "#e8ecf5",
    muted: "#8b93a7",
    accent: "#6d8bff",
    accentSoft: "#2a3557",
    ok: "#4ade80",
    warn: "#fbbf24",
    danger: "#f87171",
    /** The only two additions: a pressed surface over raised or surface. */
    pressed: "rgba(255,255,255,0.05)",
    pressedStrong: "rgba(255,255,255,0.08)",
    /** A border that has to be visible on a real screen in daylight. */
    borderStrong: "rgba(139,147,167,0.5)",
};

export const space = {
    1: 4,
    2: 8,
    3: 12,
    4: 16,
    5: 20,
    6: 24,
    8: 32,
    10: 40,
};

export const radius = {
    panel: 14,
    control: 10,
    segment: 8,
    chip: 6,
    overlay: 8,
};

export const type = StyleSheet.create({
    display: { fontSize: 40, lineHeight: 48, fontWeight: "600", color: colors.ink },
    title: { fontSize: 18, lineHeight: 24, fontWeight: "600", color: colors.ink },
    subtitle: { fontSize: 15, lineHeight: 20, fontWeight: "600", color: colors.ink },
    body: { fontSize: 14, lineHeight: 20, color: colors.ink },
    bodyStrong: { fontSize: 14, lineHeight: 20, fontWeight: "500", color: colors.ink },
    caption: { fontSize: 12, lineHeight: 17, color: colors.muted },
    mono: {
        fontSize: 13,
        lineHeight: 18,
        color: colors.ink,
        fontFamily: "monospace",
        fontVariant: ["tabular-nums"],
    },
    monoMuted: {
        fontSize: 13,
        lineHeight: 18,
        color: colors.muted,
        fontFamily: "monospace",
        fontVariant: ["tabular-nums"],
    },
    uppercase: {
        fontSize: 11,
        lineHeight: 14,
        letterSpacing: 1.2,
        color: colors.muted,
    },
});

export function Section({
    title,
    hint,
    children,
}: {
    title: string;
    hint?: string;
    children: React.ReactNode;
}) {
    return (
        <View style={styles.section}>
            <Text accessibilityRole="header" style={type.subtitle}>
                {title}
            </Text>
            {hint ? <Text style={[type.caption, styles.sectionHint]}>{hint}</Text> : null}
            <View style={styles.sectionBody}>{children}</View>
        </View>
    );
}

export function Row({
    label,
    value,
    hint,
    children,
}: {
    label: string;
    value?: string;
    hint?: string;
    children?: React.ReactNode;
}) {
    return (
        <View style={styles.row}>
            <View style={styles.rowText}>
                <Text style={type.body}>{label}</Text>
                {hint ? <Text style={type.caption}>{hint}</Text> : null}
            </View>
            {value ? <Text style={type.monoMuted}>{value}</Text> : null}
            {children}
        </View>
    );
}

/** A row that toggles a switch, with the whole row as the target. */
export function ToggleRow({
    label,
    hint,
    value,
    onChange,
}: {
    label: string;
    hint?: string;
    value: boolean;
    onChange: (value: boolean) => void;
}) {
    return (
        <Pressable
            accessibilityRole="switch"
            accessibilityState={{ checked: value }}
            accessibilityLabel={label}
            style={styles.row}
            onPress={() => onChange(!value)}
        >
            <View style={styles.rowText}>
                <Text style={type.body}>{label}</Text>
                {hint ? <Text style={type.caption}>{hint}</Text> : null}
            </View>
            <Switch
                value={value}
                onValueChange={onChange}
                accessibilityElementsHidden
                importantForAccessibility="no-hide-descendants"
                trackColor={{ false: colors.border, true: colors.accentSoft }}
                thumbColor={value ? colors.accent : colors.muted}
            />
        </Pressable>
    );
}

export function Segmented<T extends string | number>({
    options,
    value,
    onChange,
    mono = false,
    accessibilityLabel,
}: {
    options: Array<{ value: T; label: string }>;
    value: T;
    onChange: (value: T) => void;
    mono?: boolean;
    accessibilityLabel?: string;
}) {
    return (
        <View style={styles.segmented}>
            {options.map((option) => {
                const active = option.value === value;
                return (
                    <Pressable
                        key={String(option.value)}
                        accessibilityRole="radio"
                        accessibilityState={{ selected: active }}
                        accessibilityLabel={
                            accessibilityLabel
                                ? `${accessibilityLabel}, ${option.label}${active ? ", selected" : ""}`
                                : undefined
                        }
                        onPress={() => onChange(option.value)}
                        style={[styles.segment, active && styles.segmentActive]}
                    >
                        <Text
                            style={[
                                mono ? type.mono : type.body,
                                styles.segmentLabel,
                                active && styles.segmentLabelActive,
                            ]}
                        >
                            {option.label}
                        </Text>
                    </Pressable>
                );
            })}
        </View>
    );
}

export function Stepper({
    value,
    options,
    onChange,
    format,
    mono = true,
    accessibilityLabel,
}: {
    value: number;
    options: number[];
    onChange: (value: number) => void;
    format?: (value: number) => string;
    mono?: boolean;
    accessibilityLabel?: string;
}) {
    return (
        <Segmented
            options={options.map((option) => ({
                value: option,
                label: format ? format(option) : String(option),
            }))}
            value={value}
            onChange={onChange}
            mono={mono}
            accessibilityLabel={accessibilityLabel}
        />
    );
}

/** A small tag that pairs a color with a word, never color alone. */
export function Chip({ label, tone }: { label: string; tone: "ok" | "warn" | "muted" }) {
    const color = tone === "ok" ? colors.ok : tone === "warn" ? colors.warn : colors.muted;
    return (
        <View style={[styles.chip, { borderColor: color }]}>
            <Text style={[type.caption, { color }]}>{label}</Text>
        </View>
    );
}

export function PrimaryButton({
    label,
    onPress,
    disabled = false,
    busy = false,
}: {
    label: string;
    onPress: () => void;
    disabled?: boolean;
    busy?: boolean;
}) {
    return (
        <Pressable
            accessibilityRole="button"
            accessibilityState={{ disabled }}
            onPress={onPress}
            style={[styles.primary, (disabled || busy) && styles.primaryDisabled]}
        >
            <Text style={styles.primaryLabel}>{busy ? "Pairing..." : label}</Text>
        </Pressable>
    );
}

const styles = StyleSheet.create({
    section: {
        marginBottom: space[5],
    },
    sectionTitle: {
        fontSize: 15,
        lineHeight: 20,
        fontWeight: "600",
        color: colors.ink,
    },
    sectionHint: {
        marginTop: space[1],
    },
    sectionBody: {
        marginTop: space[3],
        borderRadius: radius.panel,
        borderWidth: 1,
        borderColor: colors.border,
        backgroundColor: colors.raised,
        overflow: "hidden",
    },
    row: {
        flexDirection: "row",
        alignItems: "flex-start",
        justifyContent: "space-between",
        paddingHorizontal: space[4],
        paddingVertical: space[3],
        gap: space[3],
        minHeight: 48,
    },
    rowText: {
        flexShrink: 1,
        gap: space[1],
    },
    segmented: {
        flexDirection: "row",
        flexWrap: "wrap",
        gap: space[2],
        padding: space[3],
    },
    segment: {
        minHeight: 44,
        justifyContent: "center",
        paddingHorizontal: space[3],
        paddingVertical: space[2],
        borderRadius: radius.segment,
        borderWidth: 1,
        borderColor: colors.borderStrong,
        backgroundColor: colors.surface,
    },
    segmentActive: {
        borderColor: colors.accent,
        backgroundColor: colors.accentSoft,
    },
    segmentLabel: {
        fontSize: 14,
        lineHeight: 20,
        color: colors.muted,
    },
    segmentLabelActive: {
        color: colors.ink,
        fontWeight: "600",
    },
    chip: {
        borderWidth: 1,
        borderRadius: radius.chip,
        paddingHorizontal: space[2],
        paddingVertical: space[1],
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
    primaryLabel: {
        color: colors.surface,
        fontSize: 15,
        fontWeight: "600",
    },
});
