# Releases

## GitHub Release binaries

Only pushing a matching `v*` tag runs `.github/workflows/release.yml`. Ordinary
pushes, pull requests and manually running the normal CI workflow do not run the
release matrix or publish assets. The tag must match every workspace package and
internal dependency version. The normal CI checks must pass before release builds.

| Platform | Native runner | Rust target | Archive |
| --- | --- | --- | --- |
| macOS ARM64 | `macos-15` | `aarch64-apple-darwin` | `arcnix-vVERSION-macos-arm64.tar.gz` |
| Windows AMD64 | `windows-2025` | `x86_64-pc-windows-msvc` | `arcnix-vVERSION-windows-amd64.zip` |
| Linux ARM64 | `ubuntu-24.04-arm` | `aarch64-unknown-linux-gnu` | `arcnix-vVERSION-linux-arm64.tar.gz` |
| Linux AMD64 | `ubuntu-24.04` | `x86_64-unknown-linux-gnu` | `arcnix-vVERSION-linux-amd64.tar.gz` |

Each native runner tests the workspace, builds with Rust **1.99.0** in release
mode, and packages the binary, READMEs and licenses. All four builds must succeed
before archives and `SHA256SUMS` are attached to a GitHub Release. Tags containing
a prerelease suffix produce a prerelease. Rerunning the release job replaces assets
on the existing release. GitHub assets do not depend on crates.io publication.

`tools/package_release.py` reads Cargo metadata to locate the actual workspace
target directory. It does not change Cargo home, the build directory or TEMP.

## Platform requirements

- **Windows:** MSVC build tools, native Windows Perl and NASM for building;
  WebView2 Runtime for running. The archive contains `arcnix.exe`.
- **macOS:** Xcode Command Line Tools for building; macOS 13+ on Apple Silicon
  for running. Extract and move `Arcnix.app` to Applications. CI creates an ad-hoc
  signature; the app is not Developer ID signed or notarized.
- **Linux:** native builds on Ubuntu 24.04 with `build-essential`, `pkg-config`,
  `perl`, `libssl-dev`, `libgtk-3-dev` and `libwebkit2gtk-4.1-dev`. Archives contain
  the `arcnix` executable and require a graphical desktop, glibc 2.39+, GTK 3 and
  WebKitGTK 4.1 at runtime. On Ubuntu 24.04, install `libgtk-3-0t64` and
  `libwebkit2gtk-4.1-0`. These are not fully static or AppImage packages.

Native file selection and clipboard integration currently have Windows-only
implementations. Building other platforms does not establish full UI parity.

References: [GitHub runner platforms](https://docs.github.com/en/actions/reference/runners/github-hosted-runners),
[Wry Linux dependencies](https://github.com/tauri-apps/wry#linux-dependencies).

## Publish a version

1. Update `workspace.package.version` and the `arcnix-core` dependency version in
   the root `Cargo.toml`; update `Cargo.lock` and `CHANGELOG.md`.
2. Commit and push the changes to `main` and wait for CI to pass.
3. Create and push a new matching tag, for example:

   ```sh
   git tag v0.1.2
   git push origin v0.1.2
   ```

The example requires first updating the manifests to `0.1.2`. Existing release
tags retain their original workflow and are not moved by CI configuration changes.
No release is triggered by editing these workflows.

Before pushing a tag, validate its version locally:

```sh
python tools/check_release.py v0.1.2
```

Rustfmt uses **nightly-2026-11-04** to honor the project's existing unstable
formatting settings. Compilation, Clippy, tests and publishing use **Rust 1.99.0**.

## Optional crates.io publication

crates.io publication remains **disabled by default** after the stopped release.
To resume it for future tags, set the repository Actions variable
`PUBLISH_CRATES_IO` to exactly `true`. The separate publish job uses the existing
organization Actions secret `CRATES_TOKEN` as `CARGO_REGISTRY_TOKEN` and runs
`cargo publish --workspace --locked` on Windows. The token must allow publishing
`arcnix-core` and `arcnix`, including creating new crate names. Verify the account
email and resolve any deleted-name reservation before enabling this job. Never
put a token in the repository or release commands.

## 发布说明

- 仅推送匹配工作区版本的 `v*` tag 时运行四个平台的原生测试、release 构建和打包；普通提交、PR 和手动运行普通 CI 不运行版本构建。
- ARM 指 ARM64；macOS 为 `.app` 压缩包，Windows 为 ZIP，Linux 为两种架构的 tar.gz，全部成功后发布 GitHub Release 附件和 `SHA256SUMS`。
- macOS 目前仅临时签名，未经公证；Linux 包依赖系统 GTK 3、WebKitGTK 4.1 和 glibc 2.39+。原生文件选择、剪贴板当前仍以 Windows 为实现平台。
- crates.io 发布默认关闭，未来需要恢复时再将 Actions 变量 `PUBLISH_CRATES_IO` 设为 `true`；GitHub 附件发布不依赖 crates.io。
- 已存在的 tag 不会被移动；本次配置更新不触发发布。新版本先更新版本与锁文件，普通 CI 通过后再推送新 tag。
