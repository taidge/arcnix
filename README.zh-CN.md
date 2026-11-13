# Arcnix

[English](README.md) | 简体中文

使用 Rust、Glory 和 Sapid 构建的桌面服务器管理工作台，通过 SSH 连接，无需在服务器安装 Agent。目前以 Windows 桌面为验证平台。

- 按可排序的分组整理服务器，支持导入 SSH 配置。
- 双栏 SFTP 文件管理、传输队列和文件冲突处理。
- 交互式 SSH 终端、软件管理、资源监控和系统日志。
- 中英文界面，支持跟随系统主题（默认）、深色和浅色。
- 密码、私钥口令及 sudo 密码不写入本地配置。

## 运行

构建需要 **Rust 1.99+**、Windows MSVC 构建工具、原生 Windows Perl（如 Strawberry Perl）和 NASM；运行需要 WebView2 Runtime。SSH 使用静态链接的 OpenSSL，无需系统 SSH 或 OpenSSL。

```powershell
cargo run --locked -p arcnix-desktop --bin arcnix
```

使用指定配置文件：在命令末尾添加 `-- --config ./my-config.json`。

## 开发与发布

CI 检查 Rustfmt、typos、Clippy、测试、文档和 crate 打包，覆盖 Rust 1.99 的 Windows 工作区及 Linux 核心库。推送匹配工作区版本的 `v*` tag 可触发 crates.io 发布。

[开发说明](docs/development.md) · [发布流程与令牌配置](docs/releasing.md) · [更新记录](CHANGELOG.md)

Apache-2.0 许可证；内置第三方资源保留各自许可证。
