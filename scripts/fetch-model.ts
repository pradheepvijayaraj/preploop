import { mkdir, rename, unlink } from "node:fs/promises";
import { dirname, resolve } from "node:path";

const MODEL_URL =
  "https://huggingface.co/mradermacher/granite-embedding-small-english-r2-GGUF/resolve/2b2ad9b58f3e382821e007f4c924101d6bd4a5e2/granite-embedding-small-english-r2.Q8_0.gguf";
const MODEL_SHA256 =
  "d4b41f5d7db712806722103a1c6aba2f0fe99f77740501d6f313c8240641f145";
const MODEL_PATH = resolve(
  import.meta.dir,
  "../src-tauri/models/granite-r2-q8_0.gguf",
);

async function sha256(path: string): Promise<string | null> {
  const file = Bun.file(path);
  if (!(await file.exists())) return null;
  const hasher = new Bun.CryptoHasher("sha256");
  hasher.update(await file.arrayBuffer());
  return hasher.digest("hex");
}

function formatBytes(bytes: number): string {
  if (bytes < 1024 ** 2) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 ** 3) return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
  return `${(bytes / 1024 ** 3).toFixed(2)} GB`;
}

function renderProgress(downloaded: number, total: number | null): void {
  const terminal = process.stdout.isTTY;
  if (!terminal && total !== null) {
    const percentage = Math.floor((downloaded / total) * 100);
    process.stdout.write(`\rDownloading embedding model: ${percentage}%`);
    return;
  }

  if (total === null) {
    process.stdout.write(
      `\rDownloading embedding model: ${formatBytes(downloaded)}`,
    );
    return;
  }

  const percentage = Math.min(100, Math.floor((downloaded / total) * 100));
  const width = 30;
  const filled = Math.floor((percentage / 100) * width);
  const bar = `${"#".repeat(filled)}${"-".repeat(width - filled)}`;
  process.stdout.write(
    `\rDownloading embedding model: [${bar}] ${percentage}% ` +
      `(${formatBytes(downloaded)} / ${formatBytes(total)})`,
  );
}

if ((await sha256(MODEL_PATH)) === MODEL_SHA256) {
  console.log("Embedding model is already present and verified.");
  process.exit(0);
}

await mkdir(dirname(MODEL_PATH), { recursive: true });
const temporaryPath = `${MODEL_PATH}.download`;

try {
  const response = await fetch(MODEL_URL);
  if (!response.ok) {
    throw new Error(
      `Model download failed: ${response.status} ${response.statusText}`,
    );
  }

  const total = Number(response.headers.get("content-length")) || null;
  const reader = response.body?.getReader();
  if (!reader)
    throw new Error("Model download returned an empty response body");

  const writer = Bun.file(temporaryPath).writer();
  let downloaded = 0;
  renderProgress(downloaded, total);
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      await writer.write(value);
      downloaded += value.byteLength;
      renderProgress(downloaded, total);
    }
  } finally {
    // FileSink writes and final flushing can be asynchronous on Windows.
    // Finish closing the file before hashing, renaming, or removing it.
    await writer.end();
  }
  process.stdout.write("\n");

  const actualHash = await sha256(temporaryPath);
  if (actualHash !== MODEL_SHA256) {
    throw new Error(
      `Model checksum mismatch: expected ${MODEL_SHA256}, received ${actualHash}`,
    );
  }

  // Windows does not replace an existing destination with rename(). Remove a
  // stale/corrupt model only after the replacement has passed verification.
  await unlink(MODEL_PATH).catch(() => undefined);
  await rename(temporaryPath, MODEL_PATH);
  console.log(`Downloaded and verified ${MODEL_PATH}`);
} catch (error) {
  await unlink(temporaryPath).catch(() => undefined);
  throw error;
}
