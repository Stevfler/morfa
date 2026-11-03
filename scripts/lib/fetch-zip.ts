// Общий загрузчик для fetch-*-win.ts: качает zip по закреплённому URL, сверяет sha256,
// распаковывает и копирует нужные файлы из <top>/<sub>/ (или из корня архива, если sub не задан) в OUT. Идемпотентен: штамп версии
// лежит рядом (в src-tauri/vendor/.<name>-v<ver>), а не внутри OUT, чтобы не попасть в ресурсы.
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readdirSync, rmSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";

export async function fetchZip(o: { name: string; ver: string; url: string; sha: string; out: string; sub?: string; files: string[] }) {
  const stamp = join(dirname(o.out), `.${o.name}-v${o.ver}`);
  if (o.files.every((f) => existsSync(join(o.out, f))) && existsSync(stamp)) {
    console.log(`${o.name} ${o.ver} already in ${o.out}`);
    return;
  }
  const res = await fetch(o.url);
  if (!res.ok) throw new Error(`download failed: ${res.status} ${o.url}`);
  const zip = new Uint8Array(await res.arrayBuffer());
  const sha = createHash("sha256").update(zip).digest("hex");
  if (sha !== o.sha) throw new Error(`sha256 mismatch for ${o.name}: ${sha}`);

  const tmp = mkdtempSync(join(tmpdir(), `${o.name}-`));
  try {
    const zipPath = join(tmp, "a.zip");
    await Bun.write(zipPath, zip);
    // На Windows 10+ есть bsdtar (умеет zip), на Linux/CI — unzip.
    const cmd = process.platform === "win32" ? ["tar", "-xf", zipPath, "-C", tmp] : ["unzip", "-q", zipPath, "-d", tmp];
    const p = Bun.spawnSync(cmd, { stdout: "inherit", stderr: "inherit" });
    if (p.exitCode !== 0) throw new Error(`${cmd[0]} failed`);
    // sub === undefined: files lie in the archive root (no top-level folder).
    const top = o.sub === undefined ? "" : readdirSync(tmp).find((d) => statSync(join(tmp, d)).isDirectory());
    if (top === undefined) throw new Error("unexpected archive layout");
    rmSync(o.out, { recursive: true, force: true });
    mkdirSync(o.out, { recursive: true });
    for (const f of o.files) copyFileSync(join(tmp, top, o.sub ?? "", f), join(o.out, f));
    for (const s of readdirSync(dirname(o.out))) if (s.startsWith(`.${o.name}-v`)) rmSync(join(dirname(o.out), s));
    await Bun.write(stamp, "");
  } finally {
    rmSync(tmp, { recursive: true, force: true });
  }
  console.log(`${o.name} ${o.ver} -> ${o.out}`);
}
