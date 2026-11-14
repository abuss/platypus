# GitHub Actions: Release Binaries

This GitHub Actions workflow automatically builds and releases cross-platform binaries for plat when you push a git tag.

## Quick Start

### 1. Create a Release Tag
```bash
git tag v0.2.0
git push origin v0.2.0
```

That's it! GitHub Actions will:
- ✅ Build for **Linux x86_64** (Ubuntu)
- ✅ Build for **macOS x86_64** (Intel)
- ✅ Build for **macOS aarch64** (Apple Silicon)
- ✅ Run tests on each platform
- ✅ Strip binaries (remove debug symbols)
- ✅ Generate SHA256 checksums
- ✅ Create a GitHub Release with all binaries attached

### 2. Monitor the Build
- Go to GitHub → Actions tab → watch the workflow run
- Each platform builds in parallel (~3–5 min per target)

### 3. Download Binaries
After the workflow completes:
- Go to Releases → latest version
- Download `plat-linux-x86_64`, `plat-macos-x86_64`, or `plat-macos-aarch64`
- Verify checksums against `SHA256SUMS` file

## What's Included

| File | Platform | Arch |
|------|----------|------|
| `plat-linux-x86_64` | Linux | x86_64 (Intel/AMD) |
| `plat-macos-x86_64` | macOS | x86_64 (Intel) |
| `plat-macos-aarch64` | macOS | aarch64 (M1/M2/M3) |
| `SHA256SUMS` | All | Checksum verification |

## Features

- **Caching**: Cargo dependencies cached per OS to speed up builds
- **Tests**: Full test suite runs before binary creation (no broken releases)
- **Stripping**: Debug symbols removed to keep binaries small
- **Checksums**: SHA256 hashes for integrity verification
- **Permissions**: Workflow has write-access to create releases automatically

## Skipping the Workflow

If you push a tag but don't want a release:
```bash
git tag --delete v0.2.0
git push origin :v0.2.0
```

Or use a different prefix (workflow only triggers on `v*` tags):
```bash
git tag test-0.2.0  # Won't trigger the workflow
```

## Troubleshooting

**Build fails?**
- Check the workflow logs (GitHub → Actions → click the failed job)
- Common issues: Rust version, missing system dependencies
- The workflow uses `dtolnay/rust-toolchain@stable` (latest stable Rust)

**Binary won't run on my Mac?**
- Make sure you're on the matching architecture:
  - `uname -m` → `x86_64` → use `plat-macos-x86_64`
  - `uname -m` → `arm64` → use `plat-macos-aarch64`
- Mark the binary as executable: `chmod +x plat-macos-*`

**Checksums don't match?**
- Re-download (partial transfers can corrupt files)
- Check `SHA256SUMS` with: `sha256sum -c SHA256SUMS`

## Future Enhancements

- Add Windows builds (requires `x86_64-pc-windows-msvc` target)
- Add ARM Linux support (e.g., `aarch64-unknown-linux-gnu`)
- Auto-upload to Homebrew or other package managers
- Create `.deb` and `.rpm` packages for Linux
