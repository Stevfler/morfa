// Кладёт статический ffmpeg.exe (сборка gyan.dev «essentials», GitHub-релиз GyanD/codexffmpeg)
// в src-tauri/vendor/ffmpeg-win → ресурсы установщика <resource_dir>/bin (см. sidecar.rs).
// essentials содержит libx264, libvpx (VP8/VP9), libopus, libmp3lame, libvorbis — проверено по
// README.txt и строке configure в ffmpeg.exe. Лицензия: GPL v3 (--enable-gpl, из-за x264),
// поэтому при распространении надо давать ссылку на исходники FFmpeg/gyan.dev.
// ffprobe/ffplay не нужны — берём только ffmpeg.exe (~105 МБ, zip ~115 МБ).
import { join } from "node:path";
import { fetchZip } from "./lib/fetch-zip";

const VER = "9.0.2";
await fetchZip({
  name: "ffmpeg",
  ver: VER,
  sha: "60f467265b1e312373dbcd92200c2618a74850f98d3d078e94296bb3fa2047ba",
  url: `https://github.com/GyanD/codexffmpeg/releases/download/${VER}/ffmpeg-${VER}-essentials_build.zip`,
  out: join(import.meta.dir, "..", "src-tauri", "vendor", "ffmpeg-win"),
  sub: "bin",
  files: ["ffmpeg.exe"],
});
