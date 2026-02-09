# voyager (voyager-vpm)

A fast, crash-safe CLI for CI/CD pipelines that builds a VPM package index from GitHub releases, managing `voyager.toml`/`voyager.lock` and generating + validating `index.json`.

## Features

- Manifest workflow: `init`, `add`, `remove`, `list`, `info`
- Fetch package manifests from GitHub Releases (`fetch`)
- Generate VPM index (`generate`)
- Validate package URLs in an index (`validate`)
- Manifest hash checks + transactional recovery (`lock`, `*.txn`)

## Installation

### Option 1: GitHub Releases

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/Limitex/voyager-vpm/releases/latest/download/voyager-installer.sh | sh
voy --help
```

### Option 2: Build from source

```bash
git clone https://github.com/Limitex/voyager-vpm.git
cd voyager-vpm
cargo build --release
./target/release/voy --help
```

## Quick Start

1. Initialize:

```bash
voy init --name "My VPM" --id "com.example.vpm" --author "Your Name" --url "https://example.github.io/vpm/index.json"
```

2. Add packages:

```bash
voy add owner/repo
voy add owner/another-repo --id com.example.vpm.custom_package
```

3. Fetch release metadata:

```bash
voy fetch
```

4. Generate index:

```bash
voy generate --output index.json
```

5. Validate URLs:

```bash
voy validate index.json
```

## Config (`voyager.toml`)

```toml
[vpm]
id = "com.example.vpm"
name = "Example VPM"
author = "Example Author"
url = "https://example.github.io/vpm/index.json"

[[packages]]
id = "com.example.vpm.some_package"
repository = "owner/repo"
```

Validation rules (summary):
- IDs must be reverse-domain style
- Each `packages[].id` must start with `<vpm.id>.` (e.g. `vpm.id = com.example.vpm` -> `packages[].id` starts with `com.example.vpm.`)
- Package IDs must be unique
- Repositories must be `owner/repo` (GitHub format)
- `url` must be `http://` or `https://`

## Upstream Release Requirements

`voy fetch` reads each configured repo's releases and downloads one asset per release
(default: `package.json`, configurable via `--asset-name` / `VOYAGER_ASSET_NAME`).

Each accepted release must satisfy:
- Asset exists
- Asset JSON is valid VPM metadata
- JSON `name` matches the configured package ID
- JSON `version` matches release tag version (`v1.2.3` -> `1.2.3`)

Example `package.json` (VPM format):

```json
{
  "name": "com.example.vpm.sample-package",
  "version": "1.0.0",
  "displayName": "Sample Package",
  "description": "A sample VPM package manifest.",
  "unity": "2022.3",
  "unityRelease": "22f1",
  "dependencies": {},
  "keywords": [],
  "author": {
    "name": "Example Author",
    "email": "author@example.com",
    "url": "https://github.com/example/sample-package"
  },
  "vpmDependencies": {
    "com.vrchat.avatar": "3.7.3"
  },
  "url": "https://github.com/example/sample-package/releases/download/v1.0.0/sample-package-1.0.0.zip",
  "license": "MIT"
}
```

## Common Commands

```bash
voy fetch --wipe              # refetch everything
voy fetch --asset-name x.json # custom asset name
voy lock --check              # verify manifest hash consistency
voy lock                      # accept intentional manual manifest edits
voy completions zsh > ~/.zsh/completions/_voy
```

Global options: `--config`, `-v/--verbose`, `-q/--quiet`, `--color`

## Environment Variables

- `VOYAGER_GITHUB_TOKEN` (recommended for rate limits)
- `VOYAGER_ASSET_NAME` (default: `package.json`)
- `VOYAGER_MAX_CONCURRENT` (`1..=50`, default: `5`)
- `VOYAGER_MAX_RETRIES` (`0..=8`, default: `3`)
- `VOYAGER_OUTPUT_PATH` (default: `index.json`)
- `NO_COLOR` (overrides `--color`)

## Development

```bash
just         # list tasks
just ci      # lint + test
just release # release checks
```

or:

```bash
cargo test
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
```

## License

MIT. See `LICENSE`.
