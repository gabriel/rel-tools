import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";

export default defineConfig({
  site: "https://docs.rel.me",
  integrations: [
    starlight({
      title: "rel",
      description: "Documentation for REL, a persistent Chromium browser built for agents.",
      favicon: "/rel-mark.svg",
      logo: {
        src: "./public/rel-mark.svg",
        alt: "",
      },
      customCss: ["./src/styles/custom.css"],
      head: [
        {
          tag: "meta",
          attrs: { property: "og:image", content: "https://docs.rel.me/og.png" },
        },
        {
          tag: "meta",
          attrs: { name: "twitter:card", content: "summary_large_image" },
        },
        {
          tag: "meta",
          attrs: { name: "twitter:image", content: "https://docs.rel.me/og.png" },
        },
        {
          tag: "meta",
          attrs: { name: "theme-color", content: "#09090b" },
        },
      ],
      sidebar: [
        {
          label: "Start here",
          items: [
            { label: "Overview", slug: "index" },
            { label: "CLI", slug: "cli" },
          ],
        },
        {
          label: "Agent access",
          items: [
            { label: "MCP server", slug: "mcp" },
            { label: "RPC v1", slug: "rpc" },
            { label: "Rust SDK", slug: "sdk" },
          ],
        },
      ],
    }),
  ],
});
