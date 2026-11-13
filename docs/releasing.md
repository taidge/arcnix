# Releases

## One-time setup

The GitHub repository is `taidge/arcnix`. Configure a crates.io API token in its
Actions secret **CARGO_REGISTRY_TOKEN**. The token must be permitted to publish
`arcnix-core` and `arcnix-desktop`; an initial release also needs permission to
create these crate names. Verify the crates.io account's email before publishing.
Never put the token in the repository or release commands.

[crates.io publishing documentation](https://doc.rust-lang.org/cargo/reference/publishing.html)

## Publish a version

1. Update `workspace.package.version` and the `arcnix-core` dependency version in
   the root `Cargo.toml`; update `Cargo.lock` and `CHANGELOG.md`.
2. Commit and push the changes to `main` and wait for CI to pass.
3. Create and push a matching tag, for example:

   ```sh
   git tag v0.1.0
   git push origin v0.1.0
   ```

`release.yml` validates the tag and all workspace package versions, reruns CI,
then runs `cargo publish --workspace --locked` on Windows. Cargo publishes the
dependency crate before the desktop crate. Prerelease tags such as `v0.2.0-beta.1`
are accepted when the manifest has exactly that version. Ordinary pushes never
publish crates. No release tag is created by repository setup.

Before pushing a tag, check locally without uploading:

```sh
python tools/check_release.py v0.1.0
cargo publish --workspace --locked --dry-run
```

Rustfmt uses **nightly-2026-11-04** to honor the project's existing unstable
formatting settings. Compilation, Clippy, tests and publishing use **Rust 1.99.0**.

## 发布说明

- 首次发布前，在 GitHub 仓库的 Actions secrets 中设置 `CARGO_REGISTRY_TOKEN`，并确认 crates.io 邮箱已验证、令牌允许发布这两个 crate。
- 更新工作区版本、内部依赖版本、锁文件及更新记录，提交至 `main`。
- CI 通过后推送同版本 tag（如 `v0.1.0`），发布流程会重新检查后发布至 crates.io。
- `--dry-run` 仅检查打包，不上传。版本不一致时发布会被阻止；日常提交不会触发发布。
