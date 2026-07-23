#!/usr/bin/env node
// Screenshot every bee visual scene at every time point, then either compare the results against
// the committed baselines or replace them.
//
// Invoked by `cargo xtask viz-snapshot` / `viz-update`, which builds the WASM app and starts the
// static server this script points at. Runnable by hand too — see `--help`.
//
// The scene list is NOT duplicated here. The page publishes it on `data-bee-manifest`, so adding a
// scene means adding a `Scene` in xtask/viz-web/src/scenes.rs and nothing else.
//
// Output is one JSON object on stdout; progress goes to stderr so the two never mix. Exit 0 when
// every scene matched, 1 when any did not.

import { mkdir, readFile, writeFile, access } from "node:fs/promises";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

import pixelmatch from "pixelmatch";
import { PNG } from "pngjs";
import puppeteer from "puppeteer";

const HERE = dirname(fileURLToPath(import.meta.url));

// A fixed viewport, because the browser window is what decides how many columns ratzilla's backend
// hands ratatui. Scenes are drawn at the buffer's top-left and the screenshot is clipped to their
// own cell rect, so the viewport only has to be comfortably larger than the biggest scene.
const VIEWPORT = { width: 1000, height: 600, deviceScaleFactor: 1 };

// Per-channel tolerance passed to pixelmatch. Low but not zero: identical inputs render identically,
// while a font rasterizer that shifts one anti-aliased edge by a single level should not be reported
// as a visual change.
const THRESHOLD = 0.05;

// Below this fraction of differing pixels a scene still passes. Set to zero — the pipeline exists to
// notice change, and `viz-update` is one command away when the change is intended.
const MAX_DIFF_RATIO = 0;

function usage() {
  return `snapshot.mjs — screenshot bee's visual scenes

  --mode <compare|update>   compare against baselines (default) or overwrite them
  --base-url <url>          where the built app is served (required)
  --baseline-dir <path>     committed baseline PNGs      (default ../baselines)
  --out-dir <path>          where actual/diff PNGs land  (default ../shots)
  --scene <name>            restrict to one scene (repeatable)
  --help
`;
}

function parseArgs(argv) {
  const args = {
    mode: "compare",
    baseUrl: null,
    baselineDir: join(HERE, "..", "baselines"),
    outDir: join(HERE, "..", "shots"),
    scenes: [],
  };
  for (let i = 0; i < argv.length; i++) {
    const next = () => {
      const v = argv[++i];
      if (v === undefined) throw new Error(`${argv[i - 1]} needs a value`);
      return v;
    };
    switch (argv[i]) {
      case "--mode": args.mode = next(); break;
      case "--base-url": args.baseUrl = next(); break;
      case "--baseline-dir": args.baselineDir = next(); break;
      case "--out-dir": args.outDir = next(); break;
      case "--scene": args.scenes.push(next()); break;
      case "--help": case "-h": process.stdout.write(usage()); process.exit(0); break;
      default: throw new Error(`unknown argument ${argv[i]} (try --help)`);
    }
  }
  if (!args.baseUrl) throw new Error("--base-url is required");
  if (args.mode !== "compare" && args.mode !== "update") {
    throw new Error(`--mode must be compare or update, got ${args.mode}`);
  }
  return args;
}

const exists = (p) => access(p).then(() => true, () => false);
const log = (msg) => process.stderr.write(`${msg}\n`);

async function main() {
  const args = parseArgs(process.argv.slice(2));
  await mkdir(args.baselineDir, { recursive: true });
  await mkdir(args.outDir, { recursive: true });

  const browser = await puppeteer.launch({
    headless: true,
    args: [
      "--no-sandbox",
      "--hide-scrollbars",
      // Determinism knobs. Hinting and subpixel positioning are the two font-rasterizer inputs that
      // vary with the host; sRGB pins the color transform so a wide-gamut display profile cannot
      // shift every pixel by a level or two.
      "--font-render-hinting=none",
      "--disable-font-subpixel-positioning",
      "--force-color-profile=srgb",
      "--disable-lcd-text",
    ],
  });

  const results = [];
  try {
    const page = await browser.newPage();
    await page.setViewport(VIEWPORT);
    page.on("pageerror", (e) => log(`  ! page error: ${e.message}`));
    page.on("console", (m) => {
      if (m.type() === "error") log(`  ! console: ${m.text()}`);
    });

    await page.goto(`${args.baseUrl}/index.html?mode=snapshot`, {
      waitUntil: "networkidle0",
    });

    // The bundled webfont must be resolved before anything measures a cell: ratzilla's DomBackend
    // sizes its grid from `getBoundingClientRect()` on a probe span, and a fallback font would size
    // it differently. This is the whole reason rendering is deferred to `bee_render` rather than
    // done during page load.
    await page.evaluate(() => document.fonts.ready);
    await page.waitForFunction(
      () => document.body.dataset.beeLoaded === "1" && !!globalThis.wasmBindings?.bee_render,
      { timeout: 30_000 },
    );

    const manifest = JSON.parse(
      await page.evaluate(() => document.body.dataset.beeManifest),
    );
    const wanted = args.scenes.length
      ? manifest.filter((s) => args.scenes.includes(s.name))
      : manifest;
    if (args.scenes.length && wanted.length !== args.scenes.length) {
      const known = manifest.map((s) => s.name).join(", ");
      throw new Error(`no such scene among: ${known}`);
    }
    log(`${wanted.length} scenes, ${wanted.reduce((n, s) => n + s.times.length, 0)} screenshots`);

    for (const scene of wanted) {
      for (const t of scene.times) {
        results.push(await capture(page, args, scene, t));
      }
    }
  } finally {
    await browser.close();
  }

  const failed = results.filter((r) => r.status !== "ok" && r.status !== "written");
  const report = {
    mode: args.mode,
    total: results.length,
    passed: results.length - failed.length,
    failed: failed.length,
    baselineDir: args.baselineDir,
    outDir: args.outDir,
    results,
  };
  process.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
  process.exit(failed.length === 0 ? 0 : 1);
}

async function capture(page, args, scene, t) {
  const id = `${scene.name}_${t}`;
  const token = `${scene.name}@${t}`;

  // `bee_render` draws synchronously and stamps the token, so a mismatch here means the page showed
  // us a frame other than the one we asked for — worth failing on rather than screenshotting.
  await page.evaluate((n, ms) => globalThis.wasmBindings.bee_render(n, ms), scene.name, t);
  const drawn = await page.evaluate(() => document.body.dataset.beeReady);
  if (drawn !== token) {
    return { scene: scene.name, t, id, status: "not-drawn", detail: `page reported ${drawn}` };
  }

  const clip = await page.evaluate(() => {
    const grid = document.getElementById("grid");
    const line = grid?.querySelector("pre");
    const cell = line?.querySelector("span");
    if (!cell) return null;
    const c = cell.getBoundingClientRect();
    const l = line.getBoundingClientRect();
    return {
      x: c.x,
      y: c.y,
      cw: c.width,
      ch: l.height,
      cols: Number(document.body.dataset.beeCols),
      rows: Number(document.body.dataset.beeRows),
    };
  });
  if (!clip) {
    return { scene: scene.name, t, id, status: "no-grid" };
  }

  const png = await page.screenshot({
    type: "png",
    clip: {
      x: Math.round(clip.x),
      y: Math.round(clip.y),
      width: Math.round(clip.cols * clip.cw),
      height: Math.round(clip.rows * clip.ch),
    },
  });
  const actual = Buffer.from(png);
  const baselinePath = join(args.baselineDir, `${id}.png`);

  if (args.mode === "update") {
    await writeFile(baselinePath, actual);
    log(`  = ${id}`);
    return { scene: scene.name, t, id, status: "written", baseline: baselinePath };
  }

  if (!(await exists(baselinePath))) {
    const actualPath = join(args.outDir, `${id}.actual.png`);
    await writeFile(actualPath, actual);
    log(`  ? ${id} — no baseline`);
    return { scene: scene.name, t, id, status: "missing-baseline", actual: actualPath };
  }

  const expected = PNG.sync.read(await readFile(baselinePath));
  const got = PNG.sync.read(actual);
  if (expected.width !== got.width || expected.height !== got.height) {
    const actualPath = join(args.outDir, `${id}.actual.png`);
    await writeFile(actualPath, actual);
    log(`  ✗ ${id} — size ${got.width}x${got.height}, baseline ${expected.width}x${expected.height}`);
    return {
      scene: scene.name, t, id, status: "resized", actual: actualPath,
      detail: `${got.width}x${got.height} vs ${expected.width}x${expected.height}`,
    };
  }

  const diff = new PNG({ width: got.width, height: got.height });
  const changed = pixelmatch(
    expected.data, got.data, diff.data, got.width, got.height,
    { threshold: THRESHOLD },
  );
  const ratio = changed / (got.width * got.height);
  if (ratio > MAX_DIFF_RATIO) {
    const actualPath = join(args.outDir, `${id}.actual.png`);
    const diffPath = join(args.outDir, `${id}.diff.png`);
    await writeFile(actualPath, actual);
    await writeFile(diffPath, PNG.sync.write(diff));
    log(`  ✗ ${id} — ${changed} px (${(ratio * 100).toFixed(3)}%)`);
    return {
      scene: scene.name, t, id, status: "changed",
      diffPixels: changed, diffRatio: ratio, actual: actualPath, diff: diffPath,
    };
  }

  log(`  ✓ ${id}`);
  return { scene: scene.name, t, id, status: "ok", diffPixels: changed };
}

main().catch((err) => {
  process.stdout.write(`${JSON.stringify({ error: String(err?.stack ?? err) }, null, 2)}\n`);
  process.exit(2);
});
