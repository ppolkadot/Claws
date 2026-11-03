// Runs yt-dlp with the app's arguments and prints when each phase starts and ends,
// so the slow part of a download can be identified.
// Usage: node phases.mjs <yt-dlp.exe> <info.json> <outdir> [extra args...]
import { spawn } from "node:child_process";
import { execSync } from "node:child_process";
import readline from "node:readline";

const [, , ytdlp, infoJson, outDir, ...extra] = process.argv;
const ffmpegDir = execSync("where ffmpeg").toString().split(/\r?\n/)[0].replace(/\\ffmpeg\.exe$/i, "");

const args = [
  "--no-colors", "--ignore-config", "--ffmpeg-location", ffmpegDir, "--js-runtimes", "node",
  "--newline", "--no-warnings", "--no-playlist", "--progress", "--no-simulate", "--windows-filenames",
  "--retries", "5", "--fragment-retries", "5", "--concurrent-fragments", "8",
  "--progress-template", "download:DL|%(progress.status)s|%(progress.filename)s",
  "--progress-template", "postprocess:PP|%(progress.status)s|%(progress.postprocessor)s",
  "-f", "bv*+ba/b", "-S", "res:720,vcodec:h264,acodec:aac", "--merge-output-format", "mp4",
  ...extra,
  "-P", outDir, "-o", "v.%(ext)s", "--load-info-json", infoJson,
];

const t0 = performance.now();
const stamp = () => `${((performance.now() - t0) / 1000).toFixed(2)}s`;
const seen = new Map();
const child = spawn(ytdlp, args, { env: { ...process.env, PYTHONIOENCODING: "utf-8" } });
const onLine = (line) => {
  const [tag, status, what] = line.split("|");
  if (tag !== "DL" && tag !== "PP") return;
  const key = tag === "DL" ? `download ${what.split(".").slice(-2).join(".")}` : `post ${what}`;
  if (!seen.has(key)) { seen.set(key, true); console.log(`${stamp()}  start  ${key}`); }
  if (status === "finished") console.log(`${stamp()}  end    ${key}`);
};
readline.createInterface({ input: child.stdout }).on("line", onLine);
readline.createInterface({ input: child.stderr }).on("line", onLine);
child.on("close", (code) => console.log(`${stamp()}  exit ${code}`));
