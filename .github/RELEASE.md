# GitHub Actions: Release Binaries

This GitHub Actions workflow automatically builds and releases cross-platform binaries for plat when you push a git tag.

## Quick Start

### Option 1: Automatic (Tag-based)
```bash
git tag v0.2.0
git push origin v0.2.0
```

### Option 2: Manual Trigger (via GitHub UI)
1. Go to **GitHub → Actions**
2. Click **Release Binaries** workflow
3. Click **Run workflow** button
4. Enter the release tag (e.g., `v0.2.0`) or leave blank for current commit
5. Click **Run workflow**

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

### 3. Release Tag (Manual Trigger Only)
If you used the manual GitHub UI trigger without specifying a tag, the release will be created for the current commit without a git tag. To tag it retroactively:
```bash
git tag v0.2.0 <commit-sha>
git push origin v0.2.0
```

### 4. Download Binaries
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

- **Dual triggers**: Push tags automatically OR use GitHub UI for manual control
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

## Manual Trigger (GitHub UI)

You can also trigger the workflow manually without pushing a tag:

1. Go to your repository on GitHub
2. Click **Actions** tab
3. Select **Release Binaries** workflow (left sidebar)
4. Click **Run workflow** button (top right)
5. Optionally enter a release tag (e.g., `v0.2.0`)
6. Click **Run workflow**

**Use cases:**
- Re-run a build for the current commit
- Build from any branch without tagging
- Test the workflow before making it official
- Rebuild binaries for a previously released version

**Note:** If you don't enter a tag, the release will be created without a git tag but with full binaries. You can manually tag the commit later if needed.

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
