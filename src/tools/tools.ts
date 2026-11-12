/** The five practice tools, in sidebar order. Each tool node fills its screen. */
export const TOOL_IDS = ["tuner", "metronome", "drumMachine", "stemSplitter", "fretboard"] as const;

export type ToolId = (typeof TOOL_IDS)[number];

export type ViewId = ToolId | "settings";
