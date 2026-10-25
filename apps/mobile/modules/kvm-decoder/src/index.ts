import type { Ref } from "react";
import { requireNativeModule, requireNativeViewManager } from "expo-modules-core";
import type { StyleProp, ViewStyle } from "react-native";

type NativeDecoder = {
    isAvailable?: () => boolean;
};

export type KvmVideoRef = {
    pushFrame: (
        payload: Uint8Array,
        keyframe: boolean,
        width: number,
        height: number,
        ptsMs: number,
    ) => Promise<void>;
};

/** The failure of the decoder, with the reason the native side gave. */
export type DecodeErrorEvent = {
    nativeEvent?: { message?: string };
};

type VideoProps = {
    ref?: Ref<KvmVideoRef>;
    style?: StyleProp<ViewStyle>;
    onDecodeError?: (event: DecodeErrorEvent) => void;
};

export const KvmVideoView = requireNativeViewManager<VideoProps>("KvmDecoder");

export function decoderAvailable(): boolean {
    try {
        const native = requireNativeModule("KvmDecoder") as NativeDecoder;
        return typeof native.isAvailable === "function" ? native.isAvailable() : true;
    } catch {
        return false;
    }
}
