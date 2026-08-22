# @orbit-tools/cli

npm binary proxy for the [Orbit](https://github.com/danieljhkim/orbit) CLI and
the supported Orbit plugins for Claude Code, Codex, and Cursor.

On install, downloads the matching prebuilt `orbit` binary from
[GitHub Releases](https://github.com/danieljhkim/orbit/releases), authenticates
the signed `orbit-checksums.txt` with the package-pinned release trust set,
verifies the archive SHA-256, and exposes it as the `orbit` command.

The Claude Code, Codex, and Cursor Agent Plugin manifests all launch this
package as `npx -y @orbit-tools/cli@latest mcp serve`. Their plugin managers
install the appropriate manifests, shared skills, and client-specific
integration assets; users do not need to copy files from this package or the
Orbit repository. Installing `@orbit-tools/cli` by itself provides the native
CLI proxy, not the agent plugin assets.

## Usage

```bash
# Install globally
npm install -g @orbit-tools/cli
orbit --version

# One-shot via npx (used by the Orbit Claude Code, Codex, and Cursor plugins)
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
| `ORBIT_RELEASE_PUBLIC_KEY_FILE` | **Deprecated** in favor of `ORBIT_RELEASE_TRUSTED_KEYS_FILE`. Single-key override for the trusted checksum-signing public key; requires `ORBIT_RELEASE_PUBLIC_KEY_FILE_ACKNOWLEDGE_TRUST_CHANGE=1`, logs a deprecation notice when active. |
| `ORBIT_RELEASE_PUBLIC_KEY_FILE_ACKNOWLEDGE_TRUST_CHANGE=1` | Required acknowledgement that `ORBIT_RELEASE_PUBLIC_KEY_FILE` replaces the release authenticity trust root. |
| `ORBIT_RELEASE_TRUSTED_KEYS_FILE` | Preferred test/operations override for the full trusted signing-key set, including key IDs, `not_after`, and `revoked_at`; requires `ORBIT_RELEASE_TRUSTED_KEYS_FILE_ACKNOWLEDGE_TRUST_CHANGE=1` and logs when active. |
| `ORBIT_RELEASE_TRUSTED_KEYS_FILE_ACKNOWLEDGE_TRUST_CHANGE=1` | Required acknowledgement that `ORBIT_RELEASE_TRUSTED_KEYS_FILE` replaces the release authenticity trust root. |
| `ORBIT_SKIP_DOWNLOAD=1` | Skip postinstall download (lazy install on first run still works). |

## License

MIT.

<!-- ORB-10117 -->
