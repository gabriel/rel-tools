# Rel documentation website

The public guides in this directory are the canonical source for the repository
and [docs.rel.me](https://docs.rel.me):

- `CLI.md` → `/cli/`
- `MCP.md` → `/mcp/`
- `RPC.md` → `/rpc/`
- `SDK.md` → `/sdk/`

Edit those source files, not the generated guide files under
`src/content/docs/`. `scripts/sync-docs.mjs` prepares the Markdown for
Starlight before each build.

## Local development

```sh
npm ci
npm run dev
```

Validate the production build and internal links with:

```sh
npm run check
```

The `Deploy docs` workflow publishes changes from `main`. It requires the
`CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID` repository secrets.
