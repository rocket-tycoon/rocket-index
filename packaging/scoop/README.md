# Scoop Bucket for RocketIndex

This directory contains the Scoop manifest template for RocketIndex.

## Setup Instructions

### 1. Create the Scoop Bucket Repository

Create a new GitHub repository: `rocket-tycoon/scoop-bucket`

Initialize it with:
```bash
mkdir scoop-bucket
cd scoop-bucket
git init
mkdir bucket
```

### 2. Copy the Manifest

Copy `rocketindex.json.template` to the bucket repo as `bucket/rocketindex.json`.

The CI workflow will automatically update the version and SHA256 hashes on each release.

### 3. Add GitHub Secret

Add a secret to the `rocket-index` repository:
- Name: `SCOOP_BUCKET_TOKEN`
- Value: A GitHub Personal Access Token with `repo` scope for `rocket-tycoon/scoop-bucket`

### 4. Test Installation

Once set up, users can install with:

```powershell
# Add the bucket (one-time)
scoop bucket add rocket-tycoon https://github.com/rocket-tycoon/scoop-bucket

# Install RocketIndex
scoop install rocketindex

# Verify
rkt --version
```

## Manifest Structure

The manifest (`rocketindex.json`) follows Scoop's [App Manifests](https://github.com/ScoopInstaller/Scoop/wiki/App-Manifests) specification:

- `version`: Automatically updated by CI
- `architecture.64bit.url`: Points to the Windows release artifact
- `architecture.64bit.hash`: SHA256 hash, updated by CI
- `bin`: Exposes `rkt.exe` and `rocketindex-lsp.exe` on PATH
- `checkver`: Enables `scoop update` to detect new versions
- `autoupdate`: Template for automatic manifest updates

## Manual Update

If you need to manually update the manifest:

1. Download the Windows release zip
2. Calculate SHA256: `Get-FileHash rocketindex-vX.Y.Z-x86_64-pc-windows-msvc.zip`
3. Update `version` and `hash` in the manifest
4. Commit and push to `scoop-bucket`

## Troubleshooting

### "Couldn't find manifest for 'rocketindex'"
Ensure the bucket is added: `scoop bucket list`

### Hash mismatch
The manifest hash doesn't match the downloaded file. This usually means the CI failed to update the manifest. Check the release workflow logs.

### Permission denied
Run PowerShell as Administrator, or ensure Scoop is installed in user mode.