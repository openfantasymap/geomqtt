import { build } from "esbuild";
import { mkdirSync, copyFileSync } from "node:fs";

mkdirSync("public", { recursive: true });

await build({
  entryPoints: ["src/app.ts"],
  outfile: "public/app.js",
  bundle: true,
  format: "esm",
  target: "es2020",
  minify: true,
  sourcemap: true,
  external: ["maplibre-gl"],
  logLevel: "info",
});

copyFileSync("index.html", "public/index.html");
