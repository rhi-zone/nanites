# Scaffolding Templates

Standard files for new Rust monorepos in the rhi ecosystem.

## Usage

Copy files to your new repo and replace placeholders:

```bash
# Copy all scaffolding files
cp -r scaffolding/. ~/git/new-project/

# Replace placeholders
sed -i 's/PROJECT_NAME/your-project/g' ~/git/new-project/flake.nix
sed -i 's/PROJECT_DESCRIPTION/Your description/g' ~/git/new-project/flake.nix
sed -i 's/PROJECT_NAME/your-project/g' ~/git/new-project/docs/package.json
```

## Files Included

| File | Purpose |
|------|---------|
| `.cargo/config.toml` | Target bloat reduction, mold linker hint |
| `.envrc` | direnv + nix-direnv integration |
| `.gitignore` | Standard ignores for Rust + Nix + Node |
| `.githooks/pre-commit` | fmt → clippy (fast checks first) |
| `.github/workflows/ci.yml` | CI: fmt, clippy, build, test |
| `.github/workflows/deploy-docs.yml` | VitePress docs to GitHub Pages |
| `flake.nix` | Nix dev shell with Rust + mold + bun |
| `docs/package.json` | VitePress with mermaid plugin |

## Placeholders

- `PROJECT_NAME` - lowercase project name (e.g., `interconnect`)
- `PROJECT_DESCRIPTION` - short description

## Additional Setup

After copying, you'll also need:

1. `Cargo.toml` - workspace manifest
2. `crates/` - your crate(s)
3. `docs/.vitepress/config.ts` - VitePress config
4. `docs/index.md` - docs home page
5. `CLAUDE.md` - project-specific Claude instructions
