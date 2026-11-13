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
cargo run --locked -p arcnix-desktop --bin arcnix
```

To use a specific configuration, append `-- --config ./my-config.json`.

## Development and releases

CI checks Rustfmt, typos, Clippy, tests, documentation and crate packaging. It covers the Windows workspace and Linux core library on Rust 1.99. Matching `v*` tags trigger publication to crates.io.

[Development guide](docs/development.md) · [Release setup](docs/releasing.md) · [Changelog](CHANGELOG.md)

Licensed under Apache-2.0. Bundled third-party assets retain their own licenses.
