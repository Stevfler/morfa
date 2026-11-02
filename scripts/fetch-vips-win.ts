// Кладёт libvips (официальная сборка build-win64-mxe, вариант web-static) в src-tauri/vendor/vips-win,
// откуда src-tauri/tauri.windows.conf.json забирает её в ресурсы установщика (<resource_dir>/bin, см. sidecar.rs).
// Запускается из beforeBuildCommand Windows-сборки — и локально (`bun run tauri build` на винде), и в CI.
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readdirSync, rmSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const VER = "8.18.7";
const SHA = "a736af61d3b46499760efa606e469cb14e060445b1f6fd36a1fa2e75009e1197";
const URL = `https://github.com/libvips/build-win64-mxe/releases/download/v${VER}/vips-dev-x64-web-${VER}-static.zip`;
const OUT = join(import.meta.dir, "..", "src-tauri", "vendor", "vips-win");
const FILES = ["vips.exe", "vipsheader.exe", "libvips-42.dll"];
const STAMP = join(OUT, `.v${VER}`);

if (FILES.every((f) => existsSync(join(OUT, f))) && existsSync(STAMP)) {
  console.log(`libvips ${VER} already in ${OUT}`);
  process.exit(0);
}

const res = await fetch(URL);
if (!res.ok) throw new Error(`download failed: ${res.status} ${URL}`);
const zip = new Uint8Array(await res.arrayBuffer());
const sha = createHash("sha256").update(zip).digest("hex");
if (sha !== SHA) throw new Error(`sha256 mismatch: ${sha}`);

const tmp = mkdtempSync(join(tmpdir(), "vips-"));
const zipPath = join(tmp, "v.zip");
await Bun.write(zipPath, zip);
// На Windows 10+ есть bsdtar (умеет zip), на Linux/CI — unzip.
const cmd = process.platform === "win32" ? ["tar", "-xf", zipPath, "-C", tmp] : ["unzip", "-q", zipPath, "-d", tmp];
const p = Bun.spawnSync(cmd, { stdout: "inherit", stderr: "inherit" });
if (p.exitCode !== 0) throw new Error(`${cmd[0]} failed`);

const top = readdirSync(tmp).find((d) => statSync(join(tmp, d)).isDirectory());
if (!top) throw new Error("unexpected archive layout");
rmSync(OUT, { recursive: true, force: true });
mkdirSync(OUT, { recursive: true });
for (const f of FILES) copyFileSync(join(tmp, top, "bin", f), join(OUT, f));
await Bun.write(STAMP, "");
rmSync(tmp, { recursive: true, force: true });
console.log(`libvips ${VER} -> ${OUT}`);
