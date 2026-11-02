// Кладёт libvips (официальная сборка build-win64-mxe, вариант web-static) в src-tauri/vendor/vips-win,
// откуда src-tauri/tauri.windows.conf.json забирает её в ресурсы установщика (<resource_dir>/bin, см. sidecar.rs).
// Запускается из beforeBuildCommand Windows-сборки — и локально (`bun run tauri build` на винде), и в CI.
import { join } from "node:path";
import { fetchZip } from "./lib/fetch-zip";

const VER = "8.18.7";
await fetchZip({
  name: "vips",
  ver: VER,
  sha: "a736af61d3b46499760efa606e469cb14e060445b1f6fd36a1fa2e75009e1197",
  url: `https://github.com/libvips/build-win64-mxe/releases/download/v${VER}/vips-dev-x64-web-${VER}-static.zip`,
  out: join(import.meta.dir, "..", "src-tauri", "vendor", "vips-win"),
  sub: "bin",
  files: ["vips.exe", "vipsheader.exe", "libvips-42.dll"],
});
