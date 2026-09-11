import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname } from "node:path";
import type { MonitorState } from "./types.js";

function emptyState(): MonitorState {
  return { deployments: {} };
}

/** Loads persisted pagination state, or a fresh empty one if the file
 * doesn't exist yet (first run) or is unreadable/corrupt (logged, not
 * thrown — losing the cursor just means re-deriving a start ledger, see
 * poller.ts, not a crash). */
export async function loadState(path: string): Promise<MonitorState> {
  try {
    const raw = await readFile(path, "utf8");
    const parsed = JSON.parse(raw) as unknown;
    if (
      typeof parsed === "object" &&
      parsed !== null &&
      "deployments" in parsed
    ) {
      return parsed as MonitorState;
    }
    console.warn(
      `[state] ${path} didn't contain the expected shape; starting fresh.`,
    );
    return emptyState();
  } catch (err) {
    const code = (err as NodeJS.ErrnoException).code;
    if (code === "ENOENT") {
      return emptyState();
    }
    console.warn(
      `[state] Couldn't read ${path} (${(err as Error).message}); starting fresh.`,
    );
    return emptyState();
  }
}

/** Writes state atomically (write to a sibling temp file, then rename) so a
 * crash mid-write never leaves a half-written, unparseable state file
 * behind — the poller would otherwise permanently lose its cursor and have
 * to re-derive a start ledger every restart. */
export async function saveState(
  path: string,
  state: MonitorState,
): Promise<void> {
  const dir = dirname(path);
  await mkdir(dir, { recursive: true });
  const tmpPath = `${path}.${process.pid}.tmp`;
  await writeFile(tmpPath, JSON.stringify(state, null, 2), "utf8");
  await rename(tmpPath, path);
}
