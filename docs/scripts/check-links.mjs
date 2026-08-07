import { access, readFile, readdir } from "node:fs/promises";
import { dirname, extname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const docsRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const distRoot = resolve(docsRoot, "dist");
const siteOrigin = "https://docs.rel.me";

async function walk(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const files = await Promise.all(
    entries.map((entry) => {
      const path = join(directory, entry.name);
      return entry.isDirectory() ? walk(path) : [path];
    }),
  );
  return files.flat();
}

async function exists(path) {
  try {
    await access(path);
    return true;
  } catch {
    return false;
  }
}

function routeForFile(path) {
  const outputPath = relative(distRoot, path).split(sep).join("/");
  if (outputPath === "index.html") return "/";
  if (outputPath.endsWith("/index.html")) return `/${outputPath.slice(0, -10)}`;
  return `/${outputPath}`;
}

async function targetForPath(pathname) {
  if (pathname === "/404/") return resolve(distRoot, "404.html");
  const cleanPath = decodeURIComponent(pathname).replace(/^\//, "");
  const direct = resolve(distRoot, cleanPath);
  if (extname(cleanPath)) return direct;
  if (pathname.endsWith("/")) return resolve(direct, "index.html");
  if (await exists(direct)) return direct;
  return resolve(direct, "index.html");
}

const htmlFiles = (await walk(distRoot)).filter((path) => path.endsWith(".html"));
const errors = [];

for (const sourcePath of htmlFiles) {
  const html = await readFile(sourcePath, "utf8");
  const sourceRoute = routeForFile(sourcePath);

  if (/href=["']\/internals\//i.test(html)) {
    errors.push(`${sourceRoute} links to private internal documentation`);
  }

  const hrefs = [...html.matchAll(/\shref=["']([^"']+)["']/g)].map((match) => match[1]);

  for (const href of new Set(hrefs)) {
    if (/^(mailto:|tel:|javascript:|data:|\/\/)/.test(href)) continue;

    const url = new URL(href, `${siteOrigin}${sourceRoute}`);
    if (url.origin !== siteOrigin) continue;

    const targetPath = await targetForPath(url.pathname);
    if (!(await exists(targetPath))) {
      errors.push(`${sourceRoute} → ${href} (missing ${relative(distRoot, targetPath)})`);
      continue;
    }

    if (url.hash && targetPath.endsWith(".html")) {
      const id = decodeURIComponent(url.hash.slice(1));
      const targetHtml = await readFile(targetPath, "utf8");
      const escapedId = id.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
      if (!new RegExp(`\\sid=["']${escapedId}["']`).test(targetHtml)) {
        errors.push(`${sourceRoute} → ${href} (missing anchor)`);
      }
    }
  }
}

if (errors.length) {
  console.error(`Found ${errors.length} broken internal link${errors.length === 1 ? "" : "s"}:`);
  errors.forEach((error) => console.error(`- ${error}`));
  process.exit(1);
}

console.log(`Checked internal links across ${htmlFiles.length} generated pages.`);
