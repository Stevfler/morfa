// Собирает переносимый libvips для macOS из Homebrew (`brew install vips`) в src-tauri/vendor/vips-mac:
//   bin/vips, bin/vipsheader, lib/*.dylib (все не-системные зависимости), lib/vips-modules-<ver>/ (heif, jxl, poppler).
// Пути загрузки переписываются на @loader_path, файлы переподписываются ad-hoc (иначе arm64 их не запустит).
// src-tauri/tauri.macos.conf.json кладёт папку в <resource_dir>/vips; sidecar.rs ищет там bin/ и выставляет VIPSHOME,
// чтобы vips нашёл модули. Официальной переносимой сборки vips под macOS нет, поэтому берём из brew хоста:
// архитектура бандла = архитектура Homebrew (на CI macos-latest — arm64).
import { copyFileSync, chmodSync, existsSync, mkdirSync, readdirSync, realpathSync, rmSync } from "node:fs";
import { basename, dirname, join, relative } from "node:path";

const MODULES = ["vips-heif", "vips-jxl", "vips-poppler"]; // magick/openslide тянут слишком много и не нужны
const OUT = join(import.meta.dir, "..", "src-tauri", "vendor", "vips-mac");

const run = (cmd: string[]) => {
  const p = Bun.spawnSync(cmd, { stderr: "pipe" });
  if (p.exitCode !== 0) throw new Error(`${cmd.join(" ")}: ${p.stderr.toString()}`);
  return p.stdout.toString();
};

if (process.env.TAURI_ENV_ARCH && process.env.TAURI_ENV_ARCH !== (process.arch === "arm64" ? "aarch64" : "x86_64")) {
  throw new Error(`vips comes from host Homebrew (${process.arch}); can't bundle it for ${process.env.TAURI_ENV_ARCH}`);
}
const prefix = run(["brew", "--prefix", "vips"]).trim();
const ver = basename(realpathSync(prefix)); // Cellar/vips/<ver>
const modDir = readdirSync(join(prefix, "lib")).find((d) => d.startsWith("vips-modules-"));
if (!modDir) throw new Error(`no vips-modules-* in ${prefix}/lib`);

const stamp = join(dirname(OUT), `.vips-mac-v${ver}`);
if (existsSync(stamp) && existsSync(join(OUT, "bin", "vips"))) {
  console.log(`vips ${ver} already in ${OUT}`);
  process.exit(0);
}

const isSystem = (p: string) => p.startsWith("/usr/lib/") || p.startsWith("/System/");
const rpaths = (file: string) =>
  [...run(["otool", "-l", file]).matchAll(/cmd LC_RPATH\n\s+cmdsize \d+\n\s+path (.+?) \(offset/g)].map((m) =>
    m[1].replace("@loader_path", dirname(file)).replace("@executable_path", dirname(file)),
  );
/** Install names this Mach-O links against (excluding its own id). */
function deps(file: string) {
  const id = run(["otool", "-D", file]).split("\n")[1]?.trim();
  return run(["otool", "-L", file])
    .split("\n")
    .slice(1)
    .map((l) => l.trim().split(" (compatibility")[0])
    .filter((n) => n && n !== id && !isSystem(n));
}
function resolve(name: string, from: string): string {
  const cands = name.startsWith("@rpath/")
    ? [...rpaths(from), join(prefix, "..", "..", "lib"), "/opt/homebrew/lib", "/usr/local/lib"].map((r) => join(r, name.slice(7)))
    : name.startsWith("@loader_path/")
      ? [join(dirname(from), name.slice(13))]
      : [name];
  const hit = cands.find(existsSync);
  if (!hit) throw new Error(`can't resolve ${name} (needed by ${from})`);
  return realpathSync(hit);
}

rmSync(OUT, { recursive: true, force: true });
for (const d of ["bin", "lib", join("lib", modDir)]) mkdirSync(join(OUT, d), { recursive: true });

// dest path in OUT -> source file; libs are keyed by the basename of their install name
const files = new Map<string, string>();
const queue: [string, string][] = [
  ...["vips", "vipsheader"].map((b): [string, string] => [join("bin", b), realpathSync(join(prefix, "bin", b))]),
  ...MODULES.filter((m) => existsSync(join(prefix, "lib", modDir, `${m}.dylib`))).map((m): [string, string] => [
    join("lib", modDir, `${m}.dylib`),
    realpathSync(join(prefix, "lib", modDir, `${m}.dylib`)),
  ]),
];
const links = new Map<string, Map<string, string>>(); // dest -> (install name -> dest of the dep)
while (queue.length) {
  const [dest, src] = queue.pop()!;
  if (files.has(dest)) continue;
  files.set(dest, src);
  const m = new Map<string, string>();
  for (const n of deps(src)) {
    const dep = join("lib", basename(n));
    m.set(n, dep);
    if (!files.has(dep)) queue.push([dep, resolve(n, src)]);
  }
  links.set(dest, m);
}

for (const [dest, src] of files) {
  const out = join(OUT, dest);
  copyFileSync(src, out);
  chmodSync(out, 0o755);
  const args = ["install_name_tool"];
  if (dest.endsWith(".dylib")) args.push("-id", `@rpath/${basename(dest)}`);
  for (const [n, dep] of links.get(dest)!) args.push("-change", n, `@loader_path/${relative(dirname(dest), dep)}`);
  run([...args, out]);
  run(["codesign", "--force", "--sign", "-", out]);
}
for (const s of readdirSync(dirname(OUT))) if (s.startsWith(".vips-mac-v")) rmSync(join(dirname(OUT), s));
await Bun.write(stamp, "");
console.log(`vips ${ver}: ${files.size} files -> ${OUT}`);
