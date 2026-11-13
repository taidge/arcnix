# Arcnix

English | [简体中文](README.zh-CN.md)

A friendly desktop workspace for managing servers over SSH, built with Rust, Glory and Sapid. No remote agent required. Windows is the currently validated desktop platform.

- Organize servers in sortable groups and import SSH configurations.
- Dual-pane SFTP browsing, transfer queues and file conflict handling.
- Interactive SSH terminals, software management, resource metrics and system logs.
- English and Chinese UI, with system (default), dark and light themes.
- Passwords, key passphrases and sudo passwords are never saved to configuration.

## Run

Building requires **Rust 1.99+**, Windows MSVC build tools, native Windows Perl (such as Strawberry Perl) and NASM. Running requires WebView2 Runtime. OpenSSL is linked statically; system SSH and OpenSSL are not required.

```powershell
cargo run --locked -p arcnix --bin arcnix
```

To use a specific configuration, append `-- --config ./my-config.json`.

## Development and releases

CI checks workflows, Rustfmt, typos, Clippy, tests, documentation and crate packaging. Matching `v*` tags additionally build GitHub Release archives for macOS ARM64, Windows AMD64, Linux ARM64 and AMD64, with SHA-256 checksums. crates.io publication is currently disabled; see the release guide to enable it.

[Development guide](docs/development.md) · [Release setup](docs/releasing.md) · [Changelog](CHANGELOG.md)

Licensed under Apache-2.0. Bundled third-party assets retain their own licenses.
