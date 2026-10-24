/**
 * The frame sizes and rates both ends support.
 *
 * The desktop picks the size for every phone and the phone accepts it, so
 * the list lives here and both apps read the same one.
 */

export interface Resolution {
    label: string;
    width: number;
    height: number;
}

export const RESOLUTIONS: Resolution[] = [
    { label: "1920 x 1200", width: 1920, height: 1200 },
    { label: "1920 x 1080", width: 1920, height: 1080 },
    { label: "1408 x 792", width: 1408, height: 792 },
    { label: "1280 x 720", width: 1280, height: 720 },
];

export const FRAME_RATES: number[] = [15, 20, 30, 60];

/** The label of a size, even when it is not one of the ones in the list. */
export function resolutionLabel(width: number, height: number): string {
    return (
        RESOLUTIONS.find((option) => option.width === width && option.height === height)?.label ??
        `${width} x ${height}`
    );
}
