// Кладёт статический ffmpeg для macOS (сборки Martin Riedl, ffmpeg.martin-riedl.de) в src-tauri/vendor/ffmpeg-mac,
// откуда src-tauri/tauri.macos.conf.json забирает его в ресурсы бандла (<resource_dir>/bin, см. sidecar.rs).
// Бинарь зависит только от системных фреймворков. Архитектура — по TAURI_ENV_ARCH (её выставляет tauri build
// для beforeBuildCommand, учитывая --target), иначе по хосту. Лицензия: GPL (libx264), как и у Windows-сборки.
// Внимание: файлы *.sha256 на сайте отдают не тот хэш — суммы ниже посчитаны по самим zip.
import { join } from "node:path";
import { fetchZip } from "./lib/fetch-zip";

const VER = "9.0.2";
const BUILDS = {
  arm64: { id: "1789931890", sha: "c8ed4c4e6978a03c485edbfe4e0a5dc2380f8a30bba5150531b31b094492d924" },
  amd64: { id: "1789931006", sha: "7c6b4125b191cbf773832dc51f424cf2b6bb7da43007d1e066f95909e47cacd4" },
};
const target = process.env.TAURI_ENV_ARCH ?? (process.arch === "x64" ? "x86_64" : "aarch64");
const arch = target === "x86_64" ? "amd64" : "arm64";
const b = BUILDS[arch];
await fetchZip({
  name: "ffmpeg-mac",
  ver: `${VER}-${arch}`, // арх в штампе: смена --target перекачивает бинарь
  sha: b.sha,
  url: `https://ffmpeg.martin-riedl.de/download/macos/${arch}/${b.id}_${VER}/ffmpeg.zip`,
  out: join(import.meta.dir, "..", "src-tauri", "vendor", "ffmpeg-mac"),
  files: ["ffmpeg"],
});
