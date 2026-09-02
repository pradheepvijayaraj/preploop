import { describe, expect, test } from "bun:test";
import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

import { isNewer } from "./check-version.ts";
import { prepareReleaseAssets } from "./prepare-release-assets.ts";

const ROOT = resolve(import.meta.dir, "..");

describe("release version", () => {
  test("compares semantic-version precedence", () => {
    expect(isNewer("1.0.1", "1.0.0")).toBe(true);
    expect(isNewer("1.0.0", "1.0.0-rc.1")).toBe(true);
    expect(isNewer("1.0.0-rc.2", "1.0.0-rc.1")).toBe(true);
    expect(isNewer("1.0.0", "1.0.0")).toBe(false);
    expect(isNewer("0.9.0", "1.0.0")).toBe(false);
    expect(
      isNewer("999999999999999999999999.0.0", "999999999999999999999998.0.0"),
    ).toBe(true);
  });
});

describe("release assets", () => {
  test("prepares the complete matrix with duplicate macOS source names", () => {
    const config = JSON.parse(
      readFileSync(join(ROOT, "src-tauri/tauri.conf.json"), "utf8"),
    ) as { version: string; plugins: { updater: { pubkey: string } } };
    const version = config.version;
    const publicText = Buffer.from(
      config.plugins.updater.pubkey,
      "base64",
    ).toString("utf8");
    const publicKey = Buffer.from(publicText.split(/\r?\n/)[1], "base64");
    const signaturePacket = Buffer.concat([
      Buffer.from("Ed"),
      publicKey.subarray(2, 10),
      Buffer.alloc(64),
    ]);
    const signatureText = `untrusted comment: test signature\n${signaturePacket.toString("base64")}\n`;
    const signature = Buffer.from(signatureText).toString("base64");

    const temporary = mkdtempSync(join(tmpdir(), "preploop-release-"));
    try {
      const source = join(temporary, "source");
      const destination = join(temporary, "destination");
      const fixtures: Record<string, Array<readonly [string, Buffer]>> = {
        "PrepLoop-macOS-x64": [
          [`PrepLoop_${version}_x64.dmg`, Buffer.from("dmg-x64")],
          ["PrepLoop.app.tar.gz", Buffer.from("mac-x64")],
          ["PrepLoop.app.tar.gz.sig", Buffer.from(signature)],
        ],
        "PrepLoop-macOS-arm64": [
          [`PrepLoop_${version}_aarch64.dmg`, Buffer.from("dmg-arm64")],
          ["PrepLoop.app.tar.gz", Buffer.from("mac-arm64")],
          ["PrepLoop.app.tar.gz.sig", Buffer.from(signature)],
        ],
        "PrepLoop-Windows-x64": [
          [`PrepLoop_${version}_x64-setup.exe`, Buffer.from("exe-x64")],
          [`PrepLoop_${version}_x64-setup.exe.sig`, Buffer.from(signature)],
        ],
        "PrepLoop-Windows-arm64": [
          [`PrepLoop_${version}_arm64-setup.exe`, Buffer.from("exe-arm64")],
          [`PrepLoop_${version}_arm64-setup.exe.sig`, Buffer.from(signature)],
        ],
        "PrepLoop-Linux-x64-AppImage": [
          [`PrepLoop_${version}_amd64.AppImage`, Buffer.from("appimage-x64")],
          [`PrepLoop_${version}_amd64.AppImage.sig`, Buffer.from(signature)],
        ],
        "PrepLoop-Linux-arm64-AppImage": [
          [
            `PrepLoop_${version}_aarch64.AppImage`,
            Buffer.from("appimage-arm64"),
          ],
          [`PrepLoop_${version}_aarch64.AppImage.sig`, Buffer.from(signature)],
        ],
      };

      for (const [artifact, files] of Object.entries(fixtures)) {
        const artifactPath = join(source, artifact);
        mkdirSync(artifactPath, { recursive: true });
        for (const [name, content] of files) {
          writeFileSync(join(artifactPath, name), content);
        }
      }

      prepareReleaseAssets(
        source,
        destination,
        {
          RELEASE_TAG: `v${version}`,
          RELEASE_REPO: "utilinlabs/preploop",
        },
        ROOT,
      );

      expect(readdirSync(destination).sort()).toEqual(
        [
          `PrepLoop_${version}_x64.dmg`,
          `PrepLoop_${version}_aarch64.dmg`,
          `PrepLoop_${version}_x64-setup.exe`,
          `PrepLoop_${version}_arm64-setup.exe`,
          `PrepLoop_${version}_amd64.AppImage`,
          `PrepLoop_${version}_aarch64.AppImage`,
          `PrepLoop_${version}_x64.app.tar.gz`,
          `PrepLoop_${version}_aarch64.app.tar.gz`,
          "latest.json",
          "SHA256SUMS.txt",
        ].sort(),
      );

      const manifest = JSON.parse(
        readFileSync(join(destination, "latest.json"), "utf8"),
      ) as { platforms: Record<string, { url: string }> };
      expect(Object.keys(manifest.platforms)).toHaveLength(6);
      expect(manifest.platforms["darwin-x86_64"].url).toEndWith(
        `PrepLoop_${version}_x64.app.tar.gz`,
      );
      expect(manifest.platforms["darwin-aarch64"].url).toEndWith(
        `PrepLoop_${version}_aarch64.app.tar.gz`,
      );
      expect(
        readFileSync(join(destination, "SHA256SUMS.txt"), "utf8")
          .trim()
          .split("\n"),
      ).toHaveLength(9);
    } finally {
      rmSync(temporary, { recursive: true, force: true });
    }
  });
});
