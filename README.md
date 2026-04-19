<div align="center">
  <img src="assets/logo.png" alt="Donut Browser Logo" width="150">
  <h1>Donut Browser</h1>
  <strong>Open Source Anti-Detect Browser</strong>
  <br>
  <a href="https://donutbrowser.com">donutbrowser.com</a>
</div>
<br>

<p align="center">
  <a style="text-decoration: none;" href="https://github.com/zhom/donutbrowser/releases/latest" target="_blank"><img alt="GitHub release" src="https://img.shields.io/github/v/release/zhom/donutbrowser">
  </a>
  <a style="text-decoration: none;" href="https://github.com/zhom/donutbrowser/issues" target="_blank">
    <img src="https://img.shields.io/badge/PRs-welcome-brightgreen.svg?style=flat" alt="PRs Welcome">
  </a>
  <a style="text-decoration: none;" href="https://github.com/zhom/donutbrowser/blob/main/LICENSE" target="_blank">
    <img src="https://img.shields.io/badge/license-AGPL--3.0-blue.svg" alt="License">
  </a>
  <a href="https://app.fossa.com/projects/git%2Bgithub.com%2Fzhom%2Fdonutbrowser?ref=badge_shield&issueType=security" alt="FOSSA Status">
    <img src="https://app.fossa.com/api/projects/git%2Bgithub.com%2Fzhom%2Fdonutbrowser.svg?type=shield&issueType=security" alt="FOSSA Security Status"/>
  </a>
  <a style="text-decoration: none;" href="https://github.com/zhom/donutbrowser/network/members" target="_blank">
    <img src="https://img.shields.io/github/forks/zhom/donutbrowser?style=social" alt="GitHub forks">
  </a>
  <a style="text-decoration: none;" href="https://github.com/zhom/donutbrowser/releases" target="_blank">
    <img src="https://img.shields.io/github/downloads/zhom/donutbrowser/total" alt="Downloads">
  </a>
</p>

<img alt="Donut Browser Preview" src="assets/donut-preview.png" />

## Features

- **Unlimited browser profiles** — each fully isolated with its own fingerprint, cookies, extensions, and data
- **Chromium & Firefox engines** — Chromium powered by [Wayfern](https://wayfern.com), Firefox powered by [Camoufox](https://camoufox.com), both with advanced fingerprint spoofing
- **Proxy support** — HTTP, HTTPS, SOCKS4, SOCKS5 per profile, with dynamic proxy URLs
- **VPN support** — WireGuard and OpenVPN configs per profile
- **Local API & MCP** — REST API and [Model Context Protocol](https://modelcontextprotocol.io) server for integration with Claude, automation tools, and custom workflows
- **Profile groups** — organize profiles and apply bulk settings
- **Import profiles** — migrate from Chrome, Firefox, Edge, Brave, or other Chromium browsers
- **Cookie & extension management** — import/export cookies, manage extensions per profile
- **Default browser** — set Donut as your default browser and choose which profile opens each link
- **Cloud sync** — sync profiles, proxies, and groups across devices (self-hostable)
- **E2E encryption** — optional end-to-end encrypted sync with a password only you know
- **Zero telemetry** — no tracking or device fingerprinting

## Install

<!-- install-links-start -->
### macOS

| | Apple Silicon | Intel |
|---|---|---|
| **DMG** | [Download](https://github.com/zhom/donutbrowser/releases/download/v0.21.1/Donut_0.21.1_aarch64.dmg) | [Download](https://github.com/zhom/donutbrowser/releases/download/v0.21.1/Donut_0.21.1_x64.dmg) |

Or install via Homebrew:

```bash
brew install --cask donut
```

### Windows

[Download Windows Installer (x64)](https://github.com/zhom/donutbrowser/releases/download/v0.21.1/Donut_0.21.1_x64-setup.exe) · [Portable (x64)](https://github.com/zhom/donutbrowser/releases/download/v0.21.1/Donut_0.21.1_x64-portable.zip)

### Linux

| Format | x86_64 | ARM64 |
|---|---|---|
| **deb** | [Download](https://github.com/zhom/donutbrowser/releases/download/v0.21.1/Donut_0.21.1_amd64.deb) | [Download](https://github.com/zhom/donutbrowser/releases/download/v0.21.1/Donut_0.21.1_arm64.deb) |
| **rpm** | [Download](https://github.com/zhom/donutbrowser/releases/download/v0.21.1/Donut-0.21.1-1.x86_64.rpm) | [Download](https://github.com/zhom/donutbrowser/releases/download/v0.21.1/Donut-0.21.1-1.aarch64.rpm) |
| **AppImage** | [Download](https://github.com/zhom/donutbrowser/releases/download/v0.21.1/Donut_0.21.1_amd64.AppImage) | [Download](https://github.com/zhom/donutbrowser/releases/download/v0.21.1/Donut_0.21.1_aarch64.AppImage) |
<!-- install-links-end -->

Or install via package manager:

```bash
curl -fsSL https://donutbrowser.com/install.sh | sh
```

<details>
<summary>Troubleshooting AppImage</summary>

If the AppImage segfaults on launch, install **libfuse2** (`sudo apt install libfuse2` / `yay -S libfuse2` / `sudo dnf install fuse-libs`), or bypass FUSE entirely:

```bash
APPIMAGE_EXTRACT_AND_RUN=1 ./Donut.Browser_x.x.x_amd64.AppImage
```

If that gives an EGL display error, try adding `WEBKIT_DISABLE_DMABUF_RENDERER=1` or `GDK_BACKEND=x11` to the command above. If issues persist, the **.deb** / **.rpm** packages are a more reliable alternative.

</details>

### Nix

```bash
nix run github:zhom/donutbrowser#release-start
```

## Self-Hosting Sync

Donut Browser supports syncing profiles, proxies, and groups across devices via a self-hosted sync server. See the [Self-Hosting Guide](docs/self-hosting-donut-sync.md) for Docker-based setup instructions.

## Development

See [CONTRIBUTING.md](CONTRIBUTING.md).

## Community

- **Issues**: [GitHub Issues](https://github.com/zhom/donutbrowser/issues)
- **Discussions**: [GitHub Discussions](https://github.com/zhom/donutbrowser/discussions)

## Star History

<a href="https://www.star-history.com/?repos=zhom%2Fdonutbrowser&type=date&legend=top-left">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/image?repos=zhom/donutbrowser&type=date&theme=dark&legend=top-left" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/image?repos=zhom/donutbrowser&type=date&legend=top-left" />
   <img alt="Star History Chart" src="https://api.star-history.com/image?repos=zhom/donutbrowser&type=date&legend=top-left" />
 </picture>
</a>

## Contributors

<!-- readme: collaborators,contributors -start -->
<table>
	<tbody>
		<tr>
            <td align="center">
                <a href="https://github.com/zhom">
                    <img src="https://avatars.githubusercontent.com/u/2717306?v=4" width="100;" alt="zhom"/>
                    <br />
                    <sub><b>zhom</b></sub>
                </a>
            </td>
            <td align="center">
                <a href="https://github.com/HassiyYT">
                    <img src="https://avatars.githubusercontent.com/u/81773493?v=4" width="100;" alt="HassiyYT"/>
                    <br />
                    <sub><b>Hassiy</b></sub>
                </a>
            </td>
            <td align="center">
                <a href="https://github.com/yb403">
                    <img src="https://avatars.githubusercontent.com/u/87396571?v=4" width="100;" alt="yb403"/>
                    <br />
                    <sub><b>yb403</b></sub>
                </a>
            </td>
            <td align="center">
                <a href="https://github.com/drunkod">
                    <img src="https://avatars.githubusercontent.com/u/9677471?v=4" width="100;" alt="drunkod"/>
                    <br />
                    <sub><b>drunkod</b></sub>
                </a>
            </td>
            <td align="center">
                <a href="https://github.com/JorySeverijnse">
                    <img src="https://avatars.githubusercontent.com/u/117462355?v=4" width="100;" alt="JorySeverijnse"/>
                    <br />
                    <sub><b>Jory Severijnse</b></sub>
                </a>
            </td>
		</tr>
	<tbody>
</table>
<!-- readme: collaborators,contributors -end -->

## Contact

Have an urgent question or want to report a security vulnerability? Send an email to [contact@donutbrowser.com](mailto:contact@donutbrowser.com).

## License

This project is licensed under the AGPL-3.0 License - see the [LICENSE](LICENSE) file for details.
