/**
 * Render the README artwork without contacting any service or opening a repository.
 *
 * SVG only (Node.js): node scripts/renderReadmeMedia.mjs --svg-only
 * All formats: node scripts/renderReadmeMedia.mjs
 *
 * PNG/GIF rendering requires an existing Playwright installation, its Chromium
 * browser, and ffmpeg on PATH. ENOGRAM_PLAYWRIGHT_MODULE may point to an existing
 * Playwright module entry point; ENOGRAM_CHROMIUM may point to its browser.
 * No packages or browsers are installed by this script.
 */
import { mkdir, mkdtemp, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { spawn } from "node:child_process";

const outputDir = fileURLToPath(new URL("../docs/media/", import.meta.url));
const width = 1200;
const height = 420;
const fps = 12;
const seconds = 9;
const frameCount = fps * seconds;
const clamp = (value) => Math.max(0, Math.min(1, value));
const smooth = (value) => { const t = clamp(value); return t * t * (3 - 2 * t); };
const number = (value) => Number(value.toFixed(3));
const alpha = (value) => number(clamp(value));
const text = (x, y, label, extra = "") => `<text x="${x}" y="${y}" ${extra}>${label}</text>`;

function makeSvg(time = 2.2) {
  const select = smooth((time - 0.7) / 0.8);
  const staged = smooth((time - 3.0) / 0.65);
  const committed = smooth((time - 5.8) / 0.65);
  const reset = smooth((time - 8.0) / 0.8);
  const selection = select * (1 - reset);
  const index = staged * (1 - reset);
  const commit = committed * (1 - reset);
  const selectLabel = 1 - index;
  const stageLabel = index - commit;
  const pulse = Math.sin((time / seconds) * Math.PI * 2);
  const step = (x, label, count, active) => `
    <rect x="${x}" y="278" width="116" height="38" rx="19" fill="#b7a0ff" fill-opacity="${alpha(active * 0.13)}" stroke="#b7a0ff" stroke-opacity="${alpha(0.15 + active * 0.5)}"/>
    <circle cx="${x + 20}" cy="297" r="8" fill="#b7a0ff" fill-opacity="${alpha(0.13 + active * 0.7)}"/>
    ${text(x + 20, 300, count, `text-anchor="middle" fill="${active > 0.5 ? "#171222" : "#b8aacd"}" font-size="9" font-weight="700"`)}
    ${text(x + 37, 302, label, `fill="#e4dbfa" font-size="13" font-weight="500"`)}
  `;
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}" role="img" aria-labelledby="heroTitle heroDescription">
  <title id="heroTitle">Enogram — see the history, shape the next commit</title>
  <desc id="heroDescription">Illustrative Windows Git workspace using an example repository. A parent-linked history graph sits beside a diff with a removed line and an added line selected together. The workflow selects changes, stages them in the index, and commits the prepared index. This artwork is not a product screenshot.</desc>
  <defs>
    <linearGradient id="canvas" x1="0" y1="0" x2="1" y2="1"><stop stop-color="#0c1019"/><stop offset="0.54" stop-color="#100f1c"/><stop offset="1" stop-color="#0c141b"/></linearGradient>
    <radialGradient id="violet"><stop stop-color="#8960df" stop-opacity="0.25"/><stop offset="1" stop-color="#8960df" stop-opacity="0"/></radialGradient>
    <radialGradient id="teal"><stop stop-color="#3ba9a0" stop-opacity="0.17"/><stop offset="1" stop-color="#3ba9a0" stop-opacity="0"/></radialGradient>
    <linearGradient id="glass" x1="0" y1="0" x2="1" y2="1"><stop stop-color="#292238" stop-opacity="0.94"/><stop offset="0.48" stop-color="#171824" stop-opacity="0.96"/><stop offset="1" stop-color="#14212a" stop-opacity="0.96"/></linearGradient>
    <linearGradient id="border" x1="0" y1="0" x2="1" y2="1"><stop stop-color="#c5b3fa" stop-opacity="0.5"/><stop offset="0.45" stop-color="#82829c" stop-opacity="0.12"/><stop offset="1" stop-color="#8de5da" stop-opacity="0.3"/></linearGradient>
    <linearGradient id="wordmark" x1="0" y1="0" x2="0.9" y2="1"><stop stop-color="#ffffff"/><stop offset="1" stop-color="#d9cbee"/></linearGradient>
    <linearGradient id="rail" x1="0" y1="0" x2="1" y2="0"><stop stop-color="#a891ed"/><stop offset="1" stop-color="#7ed7c9"/></linearGradient>
    <filter id="shadow" x="-20%" y="-20%" width="140%" height="150%"><feDropShadow dx="0" dy="16" stdDeviation="18" flood-color="#000" flood-opacity="0.34"/></filter>
    <clipPath id="frameClip"><rect x="521" y="43" width="631" height="313" rx="17"/></clipPath>
    <clipPath id="canvasClip"><rect width="1200" height="420" rx="20"/></clipPath>
  </defs>
  <g clip-path="url(#canvasClip)" font-family="'Segoe UI', Arial, sans-serif">
    <rect width="1200" height="420" fill="url(#canvas)"/>
    <ellipse cx="636" cy="66" rx="480" ry="330" fill="url(#violet)"/>
    <ellipse cx="1135" cy="365" rx="414" ry="260" fill="url(#teal)"/>
    <g fill="none" stroke="#a996d9" stroke-opacity="0.075">
      <path d="M 372 -60 C 612 48 418 308 764 434"/>
      <path d="M 416 -72 C 658 54 470 305 812 440"/>
      <path d="M 454 -74 C 704 52 514 300 852 440"/>
    </g>
    <path d="M 49 365 H 1151" stroke="#c4bfd9" stroke-opacity="0.1"/>

    <g transform="translate(52 54)" fill="none" stroke="#b8a1f3" stroke-width="2" stroke-linecap="round">
      <path d="M4 3 V25 M4 14 H17 M4 3 H17 M4 25 H17"/>
      <circle cx="4" cy="3" r="3" fill="#12111d"/><circle cx="4" cy="25" r="3" fill="#12111d"/>
      <circle cx="17" cy="14" r="3" fill="#12111d" stroke="#80d4c8"/>
    </g>
    ${text(87, 73, "WINDOWS GIT WORKSPACE", 'fill="#b1abc4" font-size="11" font-weight="600" letter-spacing="2.2"')}
    ${text(49, 155, "Enogram", 'fill="url(#wordmark)" font-size="77" font-weight="650" letter-spacing="-4.6"')}
    ${text(53, 205, "See the history.", 'fill="#e2ddef" font-size="27" font-weight="400" letter-spacing="-0.5"')}
    ${text(53, 240, "Shape the next commit.", 'fill="#e2ddef" font-size="27" font-weight="400" letter-spacing="-0.5"')}
    ${step(53, "Select", "1", selectLabel)}
    <path d="M176 297 H188" stroke="#6c637e" stroke-width="1.5"/><path d="m184 294 4 3-4 3" fill="none" stroke="#6c637e" stroke-width="1.5"/>
    ${step(196, "Stage", "2", stageLabel)}
    <path d="M319 297 H331" stroke="#6c637e" stroke-width="1.5"/><path d="m327 294 4 3-4 3" fill="none" stroke="#6c637e" stroke-width="1.5"/>
    ${step(339, "Commit", "3", commit)}

    <rect x="521" y="43" width="631" height="313" rx="17" fill="url(#glass)" stroke="url(#border)" filter="url(#shadow)"/>
    <g clip-path="url(#frameClip)">
      <path d="M522 87 H1152" stroke="#c2b6dd" stroke-opacity="0.14"/>
      <path d="M747 87 V356" stroke="#bfb5d5" stroke-opacity="0.12"/>
      <rect x="748" y="88" width="403" height="268" fill="#0f141e" fill-opacity="0.88"/>
      <rect x="522" y="44" width="630" height="43" fill="#c8b5f6" fill-opacity="0.035"/>
      <g transform="translate(542 60)" fill="none" stroke="#b9a6e8" stroke-width="1.3"><path d="M0 3 H7 L10 6 H16 V16 H0 Z"/></g>
      ${text(568, 72, "example", 'fill="#e8e1f5" font-size="13" font-weight="600"')}
      <rect x="641" y="55" width="80" height="22" rx="11" fill="#b39af8" fill-opacity="0.09" stroke="#b39af8" stroke-opacity="0.19"/>
      <path d="M653 62 V71 M653 67 C659 67 660 66 660 62" fill="none" stroke="#bba6ed" stroke-width="1.2"/>
      <circle cx="653" cy="62" r="1.5" fill="#bba6ed"/><circle cx="660" cy="61" r="1.5" fill="#bba6ed"/>
      ${text(668, 70, "example", 'fill="#cbb9f3" font-size="10"')}
      <circle cx="1088" cy="66" r="3" fill="#91869f" fill-opacity="0.5"/>
      <circle cx="1104" cy="66" r="3" fill="#91869f" fill-opacity="0.5"/>
      <circle cx="1120" cy="66" r="3" fill="#91869f" fill-opacity="0.5"/>

      ${text(543, 115, "PARENT-LINKED HISTORY", 'fill="#a9a0bb" font-size="9" font-weight="600" letter-spacing="1.2"')}
      <rect x="535" y="137" width="199" height="39" rx="7" fill="#b39af8" fill-opacity="${alpha(0.055 + commit * 0.075)}"/>
      <g fill="none" stroke-width="2.2" stroke-linecap="round">
        <path d="M558 156 V276" stroke="#a591d7"/>
        <path d="M558 196 C558 210 584 214 584 236 C584 256 558 258 558 276" stroke="#74c8bd"/>
        <path d="M558 276 V318" stroke="#a591d7" stroke-opacity="0.35" stroke-dasharray="3 5"/>
      </g>
      <circle cx="558" cy="156" r="${number(10 + commit * (2 + pulse))}" fill="#b6a0ee" fill-opacity="${alpha(0.07 + commit * 0.1)}"/>
      <g stroke="#141622" stroke-width="3">
        <circle cx="558" cy="156" r="6.5" fill="#bea8fb"/>
        <circle cx="558" cy="196" r="6" fill="#a591d7"/>
        <circle cx="584" cy="236" r="6" fill="#80d6c8"/>
        <circle cx="558" cy="276" r="6" fill="#a591d7"/>
      </g>
      ${text(603, 153, "Update example", 'fill="#e5dfee" font-size="11.5" font-weight="500"')}
      ${text(603, 168, "HEAD", 'fill="#b6a2de" font-size="9" letter-spacing="0.8"')}
      ${text(603, 200, "Merge example", 'fill="#c5bfd3" font-size="11.5"')}
      ${text(603, 240, "Refine example", 'fill="#c5bfd3" font-size="11.5"')}
      ${text(603, 280, "Create example", 'fill="#c5bfd3" font-size="11.5"')}
      ${text(543, 337, "Connected by Git parent IDs", 'fill="#928a9f" font-size="10"')}

      ${text(769, 115, "example.ts", 'fill="#d5d5e1" font-size="12" font-weight="500"')}
      <rect x="1094" y="101" width="35" height="21" rx="5" fill="#b39af8" fill-opacity="0.13"/>
      ${text(1111.5, 115, "W", `text-anchor="middle" fill="#c4b4ed" font-size="10" font-weight="600" opacity="${alpha(1 - index)}"`)}
      ${text(1111.5, 115, "S", `text-anchor="middle" fill="#86dbca" font-size="10" font-weight="600" opacity="${alpha(index)}"`)}
      <path d="M748 132 H1152" stroke="#b9bdd1" stroke-opacity="0.09"/>
      <g font-family="Consolas, 'Courier New', monospace" font-size="12">
        ${text(804, 158, "1", 'fill="#687589" text-anchor="end"')}
        ${text(840, 158, "export const example = {", 'fill="#b6bed1"')}
        <rect x="765" y="171" width="370" height="29" rx="4" fill="#c98293" fill-opacity="0.1"/>
        <rect x="765" y="202" width="370" height="29" rx="4" fill="#7cd2ac" fill-opacity="0.1"/>
        <rect x="765" y="171" width="370" height="60" rx="4" fill="none" stroke="#baa6ef" stroke-opacity="${alpha(selection * 0.52)}"/>
        ${text(804, 190, "2", 'fill="#a27e91" text-anchor="end"')}
        ${text(824, 190, "−", 'fill="#e8a2b1"')}
        ${text(840, 190, "  example: false,", 'fill="#e0a2af" xml:space="preserve"')}
        ${text(804, 221, "2", 'fill="#7da697" text-anchor="end"')}
        ${text(824, 221, "+", 'fill="#91d7b8"')}
        ${text(840, 221, "  example: true,", 'fill="#91d7b8" xml:space="preserve"')}
        ${text(804, 252, "3", 'fill="#687589" text-anchor="end"')}
        ${text(840, 252, "};", 'fill="#b6bed1"')}
      </g>
      <g stroke="#bca7f3" stroke-width="1.1">
        <rect x="773" y="181" width="10" height="10" rx="2" fill="#bca7f3" fill-opacity="${alpha(selection * 0.8)}" stroke-opacity="0.5"/>
        <rect x="773" y="212" width="10" height="10" rx="2" fill="#bca7f3" fill-opacity="${alpha(selection * 0.8)}" stroke-opacity="0.5"/>
      </g>
      <g fill="none" stroke="#16131f" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" opacity="${alpha(selection)}">
        <path d="m775 186 2 2 4-4"/><path d="m775 217 2 2 4-4"/>
      </g>
      <path d="M768 273 H1133" stroke="#b9bdd1" stroke-opacity="0.09"/>
      <rect x="768" y="285" width="365" height="39" rx="7" fill="#b39af8" fill-opacity="${alpha(0.09 * (1 - commit))}" stroke="#b39af8" stroke-opacity="${alpha(0.25 * (1 - commit))}"/>
      <rect x="768" y="285" width="365" height="39" rx="7" fill="#81d7c5" fill-opacity="${alpha(0.1 * commit)}" stroke="#81d7c5" stroke-opacity="${alpha(0.3 * commit)}"/>
      ${text(950.5, 309, "Stage selected lines", `text-anchor="middle" fill="#d5c6fb" font-size="12" font-weight="600" opacity="${alpha(selectLabel)}"`)}
      ${text(950.5, 309, "Review the prepared index", `text-anchor="middle" fill="#d5c6fb" font-size="12" font-weight="600" opacity="${alpha(stageLabel)}"`)}
      ${text(950.5, 309, "✓  Commit the prepared index", `text-anchor="middle" fill="#9de3d3" font-size="12" font-weight="600" opacity="${alpha(commit)}"`)}
      <rect x="768" y="339" width="365" height="2" rx="1" fill="#a89fc6" fill-opacity="0.1"/>
      <rect x="768" y="339" width="${number(365 * time / seconds)}" height="2" rx="1" fill="url(#rail)" fill-opacity="0.55"/>
    </g>

    <g font-size="10.5" fill="#aaa4b8" letter-spacing="0.4">
      <circle cx="56" cy="390" r="3" fill="#b9a2f0"/>
      ${text(68, 394, "FILE · HUNK · LINE")}
      <circle cx="235" cy="390" r="3" fill="#84d6c8"/>
      ${text(247, 394, "DESKTOP + LOCAL MCP")}
    </g>
    ${text(1150, 394, "Illustrative workflow · example repository", 'text-anchor="end" fill="#938ca4" font-size="10.5"')}
    <rect x="0.5" y="0.5" width="1199" height="419" rx="19.5" fill="none" stroke="#b6accb" stroke-opacity="0.12"/>
  </g>
</svg>`;
}

function run(command, args) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { stdio: "inherit", windowsHide: true });
    child.on("error", reject);
    child.on("close", (code) => code === 0 ? resolve() : reject(new Error(`${command} exited with ${code}`)));
  });
}

async function loadPlaywright() {
  if (process.env.ENOGRAM_PLAYWRIGHT_MODULE) {
    return import(pathToFileURL(path.resolve(process.env.ENOGRAM_PLAYWRIGHT_MODULE)).href);
  }
  try { return await import("playwright"); }
  catch { throw new Error("Use an existing Playwright installation, or set ENOGRAM_PLAYWRIGHT_MODULE to its module entry point. Use --svg-only to render without Playwright."); }
}

await mkdir(outputDir, { recursive: true });
await writeFile(path.join(outputDir, "enogram-hero.svg"), `${makeSvg().replace(/[\t ]+$/gm, "").trimEnd()}\n`);
if (!process.argv.includes("--svg-only")) {
  const { chromium } = await loadPlaywright();
  const scratchDir = await mkdtemp(path.join(tmpdir(), "enogram-readme-"));
  const browser = await chromium.launch({
    headless: true,
    ...(process.env.ENOGRAM_CHROMIUM ? { executablePath: process.env.ENOGRAM_CHROMIUM } : {}),
  });
  try {
    const page = await browser.newPage({ viewport: { width, height }, deviceScaleFactor: 1 });
    await page.setContent('<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Enogram README artwork</title><style>html,body{margin:0;width:1200px;height:420px;overflow:hidden;background:#0c1019}svg{display:block}</style></head><body></body></html>');
    const show = async (svg) => { await page.evaluate((markup) => { document.body.innerHTML = markup; }, svg); };
    await show(makeSvg());
    await page.screenshot({ path: path.join(outputDir, "enogram-hero.png") });
    for (let frame = 0; frame < frameCount; frame += 1) {
      await show(makeSvg(frame / fps));
      await page.screenshot({ path: path.join(scratchDir, `frame-${String(frame).padStart(3, "0")}.png`) });
      if (frame % (fps * 3) === 0) process.stdout.write(`Rendered ${frame}/${frameCount} frames\n`);
    }
    await run("ffmpeg", ["-hide_banner", "-loglevel", "warning", "-y", "-framerate", String(fps), "-i", path.join(scratchDir, "frame-%03d.png"), "-filter_complex", "[0:v]split[a][b];[a]palettegen=stats_mode=diff:max_colors=256[p];[b][p]paletteuse=dither=bayer:bayer_scale=3:diff_mode=rectangle", "-loop", "0", path.join(outputDir, "enogram-hero.gif")]);
  } finally {
    await browser.close();
    // Verify the absolute temporary target before recursively removing frames.
    if (path.dirname(path.resolve(scratchDir)) !== path.resolve(tmpdir()) || !path.basename(scratchDir).startsWith("enogram-readme-")) {
      throw new Error("Refusing to remove an unexpected temporary path.");
    }
    await rm(scratchDir, { recursive: true, force: true });
  }
}

for (const name of ["enogram-hero.svg", "enogram-hero.png", "enogram-hero.gif"]) {
  try { const fileStat = await stat(path.join(outputDir, name)); process.stdout.write(`${name}: ${width}×${height}, ${fileStat.size} bytes\n`); }
  catch (error) { if (error.code !== "ENOENT") throw error; }
}
