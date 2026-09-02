import { access } from "node:fs/promises";
import { resolve } from "node:path";

async function exists(path: string): Promise<boolean> {
  try {
    await access(path);
    return true;
  } catch {
    return false;
  }
}

async function run(command: string[]): Promise<void> {
  const child = Bun.spawn(command, {
    stderr: "inherit",
    stdin: "inherit",
    stdout: "inherit",
  });
  if ((await child.exited) !== 0) {
    process.exit(child.exitCode ?? 1);
  }
}

const tauriBinary = resolve(import.meta.dir, "../node_modules/.bin/tauri");

if (await exists(tauriBinary)) {
  console.log(
    "JavaScript dependencies are already installed; skipping bun install.",
  );
} else {
  console.log("JavaScript dependencies are missing; installing from bun.lock.");
  await run(["bun", "install", "--frozen-lockfile"]);
}

await run(["bun", "run", "model:fetch"]);
