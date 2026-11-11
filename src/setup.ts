
export type ToolName = "ytdlp" | "ffmpeg" | "deno";

export interface SetupTools {
  ytdlp: unknown | null;
  ffmpeg: unknown | null;
  jsRuntime: string | null;
}

export interface SetupStep {
  tool: ToolName;
  name: string;
  purpose: string;
}

export const SETUP_STEPS: SetupStep[] = [
  { tool: "ytdlp", name: "yt-dlp", purpose: "Reads each site and downloads from it." },
  { tool: "ffmpeg", name: "ffmpeg", purpose: "Merges HD video with its audio and converts audio." },
  { tool: "deno", name: "Deno", purpose: "The JavaScript runtime YouTube and some other sites need." },
];

/** A tool counts as ready wherever it was found: installed by Claws or already on PATH. */
export function toolReady(tools: SetupTools, tool: ToolName): boolean {
  if (tool === "deno") return tools.jsRuntime !== null;
  return tools[tool] !== null;
}

export function missingTools(tools: SetupTools): ToolName[] {
  return SETUP_STEPS.map((step) => step.tool).filter((tool) => !toolReady(tools, tool));
}

export type StepState = "ready" | "installing" | "waiting" | "error" | "missing";

/** What one row of the setup panel shows. An error outranks everything but a running install. */
export function stepState(ready: boolean, busy: boolean, queued: boolean, error: string | undefined): StepState {
  if (busy) return "installing";
  if (ready) return "ready";
  if (error) return "error";
  return queued ? "waiting" : "missing";
}
