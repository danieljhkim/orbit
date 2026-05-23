# @orbit-tools/cli

npm proxy for the [Orbit](https://github.com/danieljhkim/orbit) CLI.

On install, downloads the matching prebuilt `orbit` binary from
[GitHub Releases](https://github.com/danieljhkim/orbit/releases), authenticates
the signed `orbit-checksums.txt` with the package-pinned release public key,
verifies the archive SHA-256, and exposes it as the `orbit` command.

## Usage

```bash
# Install globally
npm install -g @orbit-tools/cli
orbit --version

# One-shot via npx (used by the orbit Claude plugin)
npx -y @orbit-tools/cli mcp serve
```

All arguments are forwarded to the native `orbit` binary.

## Supported platforms

- macOS arm64 / x64
- Linux arm64 / x64

Windows is not currently published. Use WSL or build from source.

## Environment variables

| Variable | Effect |
|---|---|
| `ORBIT_BINARY` | Path to a local `orbit` binary; bypasses download and trusts that path as the binary source. |
| `ORBIT_BINARY_VERSION` | Override the release tag to install (e.g. `v0.3.1`); this changes the release-selection trust boundary but still requires a valid Orbit checksum signature. |
| `ORBIT_RELEASE_PUBLIC_KEY_FILE` | Test/operations override for the trusted checksum-signing public key; changing it trusts that key for release authenticity. |
| `ORBIT_SKIP_DOWNLOAD=1` | Skip postinstall download (lazy install on first run still works). |

## License

MIT.
