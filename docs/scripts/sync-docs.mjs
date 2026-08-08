import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const docsRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const contentRoot = resolve(docsRoot, "src/content/docs");

const pages = [
  {
    source: "CLI.md",
    output: "cli.md",
    title: "CLI",
    description: "Install and use the rel CLI for captures, attached pages, proxies, and persistent browser sessions.",
  },
  {
    source: "MCP.md",
    output: "mcp.md",
    title: "MCP server",
    description: "Connect MCP clients to REL's focused browser tools over the bundled stdio adapter.",
  },
  {
    source: "RPC.md",
    output: "rpc.md",
    title: "RPC v1",
    description: "The supported loopback HTTP contract for controlling REL from agents and other local clients.",
  },
  {
    source: "SDK.md",
    output: "sdk.md",
    title: "Rust SDK",
    description: "Use the typed rel-client Rust crate for every public REL RPC v1 operation.",
  },
];

const siteLinks = new Map([
  ["CLI.md", "/cli/"],
  ["MCP.md", "/mcp/"],
  ["RPC.md", "/rpc/"],
  ["SDK.md", "/sdk/"],
]);

function rewriteLinks(markdown) {
  return markdown.replace(
    /\((CLI|MCP|RPC|SDK)\.md(#[^)]+)?\)/g,
    (_, name, hash = "") => `(${siteLinks.get(`${name}.md`)}${hash})`,
  );
}

await Promise.all([
  rm(resolve(contentRoot, "internals/services.md"), { force: true }),
  rm(resolve(contentRoot, "internals/architecture.md"), { force: true }),
]);

for (const page of pages) {
  const sourcePath = resolve(docsRoot, page.source);
  const outputPath = resolve(contentRoot, page.output);
  const source = await readFile(sourcePath, "utf8");
  const body = rewriteLinks(source.replace(/^# .+\n+/, ""));
  const frontmatter = [
    "---",
    `title: ${page.title}`,
    `description: ${page.description}`,
    "editUrl: false",
    "---",
    "",
  ].join("\n");

  await mkdir(dirname(outputPath), { recursive: true });
  await writeFile(outputPath, `${frontmatter}${body}`, "utf8");
}

console.log(`Synced ${pages.length} public documentation pages.`);
