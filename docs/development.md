# Arcnix

Rust + Glory + Sapid 桌面服务器管理工作台。通过 SSH 管理多台服务器，提供文件管理、系统软件、资源概览、系统日志和 Terminal 五个模块。Windows 桌面为当前验收平台；资源监控先显示实时数值，图表后续实现。

界面默认使用英语，可从左侧底部 **Settings** 切换为简体中文。站点名、分组名、文件路径和远程日志正文保留原文。

## 项目图标

图标源文件为 [`arcnix.svg`](../crates/arcnix-desktop/assets/arcnix.svg)，画布为 **512×512**，背景完全透明。四条单层服务器分别倾斜 ±45° 围成菱形，连接中央汇聚节点，表达多台服务器的统一管理。导航栏随主题使用同源的 `arcnix-dark.svg`／`arcnix-light.svg`，Windows 程序文件、窗口与任务栏使用通用蓝绿色版本的多尺寸 ICO。

修改 SVG 后运行 `python tools/generate_app_icon.py` 更新深浅主题 SVG、PNG 预览与 ICO（需要 `CairoSVG` 和 `Pillow`）；常规 Cargo 构建使用已生成的 ICO，无需 Python。

## 页面导航

最左侧为固定图标导航栏：Logo 暂时仅作标识，点击不执行操作；**Servers** 打开服务器列表，底部提供 **Settings**。应用默认进入服务器页，左侧第二列随当前页面显示服务器列表或设置分类，右侧显示具体内容。

- Servers 未选择服务器时显示选择提示；选择但未连接时显示服务器详情及连接入口。SSH 连接成功后才显示 Files、Software、Resources、System logs 和 Terminal 操作标签。断开连接后回到服务器详情。
- 主菜单不提供通知按钮；操作提示中的“查看操作日志”仍可查看本次会话最近 100 条记录，记录不跨重启保存。
- 设置页左侧提供 General、Appearance、Files 和 Monitoring 分类。切换全局页面保留当前 SSH 连接及终端会话，资源自动采样仅在对应服务器页面可见时进行。

Servers 侧栏按文件树组织：Group 是可独立展开、折叠的文件夹，组内服务器作为子项，未分组服务器直接显示在根目录。顶部的“+”在当前组中新建服务器，文件夹图标新建 Group，“…”提供根目录新建、导入和全部展开／折叠操作。搜索按组名、服务器名称、地址或用户名匹配，并临时展开匹配组；清空搜索后恢复原来的折叠状态。

悬停或聚焦 Group、服务器时，右侧显示“…”按钮。右键 Group、服务器或树内空白处会调用同一套菜单，菜单定位在鼠标旁并自动避开窗口边缘。Shift+F10 打开键盘菜单，上下方向键移动菜单焦点，Escape 关闭；树内上下键移动焦点，左右键展开／折叠分组。菜单将两种操作分开：**Dissolve group (keep servers)**／解散分组保留全部服务器并移到根目录；**Delete group & servers**／删除分组及服务器会列出受影响服务器，确认后删除其本地配置，不操作远程主机和文件。两种操作均保留确认对话框。

Group 标题行支持鼠标拖动排序，蓝色插入线标明放置位置；顺序会写入配置并在重启后恢复。搜索期间暂时禁用排序，清空搜索即可继续拖动。展开箭头、文件夹与服务器图标均使用 SVG，统一居中并放大显示。

## 启动

源码构建需要 Rust 1.99+、Windows MSVC 构建工具、原生 Windows Perl（如 Strawberry Perl）和 NASM；运行界面需要 Microsoft Edge WebView2 Runtime。

SSH 使用源码编译并静态链接的 OpenSSL，支持 OpenSSH Ed25519 私钥（包括口令加密的私钥）及 RSA 私钥。运行应用不需要安装 OpenSSL、Perl、NASM，也不调用系统 `ssh.exe`／`sftp.exe`。首次构建会编译 OpenSSL，因此耗时较长。

升级后仍使用已有的主机信任记录。即使默认协商的主机密钥类型改变，也会先重新协商并核对原来的指纹；所有密钥都不匹配时，在发送认证信息前拒绝连接。

确保 `perl` 和 `nasm` 可从构建终端调用；便携版 Perl 也可以通过 `OPENSSL_SRC_PERL` 指定其 `perl.exe` 完整路径。Git for Windows 自带的 Perl 不适用于这里的 MSVC OpenSSL 构建。不要设置 `OPENSSL_NO_VENDOR` 或 `LIBSSH2_SYS_USE_PKG_CONFIG` 来改用系统库；构建后的程序应检查导入表没有 `libssl`／`libcrypto` DLL 依赖。

```powershell
cargo run --locked -p arcnix-desktop --bin arcnix
```

使用独立配置文件：

```powershell
cargo run --locked -p arcnix-desktop --bin arcnix -- --config ./my-config.json
```

界面和状态逻辑由 Rust 编写，Glory Wry 使用系统 WebView 作为桌面渲染宿主；无需 Node、前端打包、Web 服务器或远程 Agent。使用发布的 Glory `0.7.0` 和 Sapid `0.7.1`，不依赖机器上的其他源码仓库。

## 使用

1. 点击侧栏“新建服务器”，在模态对话框填写地址、端口、用户名，明确选择认证方式和分组。每个站点悬停时出现“…”菜单，编辑／删除针对该站点，删除前会显示确认。分组操作也在对应条目的菜单中；“新建分组”和“导入 SSH 配置”各自打开独立对话框。
2. 在 Servers 中选择左侧服务器后，点击详情卡片或右上角“连接服务器”，在连接对话框中输入密码或私钥口令。右上角统一显示当前服务器、SSH 状态和连接／取消／断开操作，所有模块共享；服务器“…”菜单也保留入口。没有站点时可从服务器列表添加。第一次连接先通过可信渠道核对 SSH 指纹，再选择保存信任或仅本次信任。已保存主机的指纹变化会拒绝认证。
3. 文件列表单击只选中当前项，Ctrl 单击增减选择，Shift 单击按当前排序选择范围，支持 Ctrl+A、Ctrl+空格、Shift+方向键。双击或 Enter 进入文件夹；详细列表中的 `›`／`⌄` 按需展开、收起子目录。路径栏按目录分段，点击任意一段可直接跳转；编辑图标或 Ctrl+L 输入完整路径，Enter 跳转、Escape 取消，复制图标将当前完整路径写入系统剪贴板。点击列标题排序，“筛选”按名称筛选。文件传输递归包含空目录，不跟随符号链接。
4. 各文件栏的“…”菜单提供新建文件夹和重命名，输入表单统一使用对话框。重命名需要选中一个项目，默认拒绝覆盖已有目标。操作失败会在对话框中显示原因，成功后才关闭。
5. 底部在“传输／操作日志”标签间切换，可收起。传输队列支持进度、取消、重试、批量重试和状态筛选。暂停会阻止后续任务开始，当前任务继续运行。服务器资源统一在顶部“资源概览”查看。
6. 拖动文件双栏之间的细分隔条调整宽度，松开后保存到当前服务器的本地配置；不同服务器使用各自比例，重启后恢复。窗口变窄时临时保证最小栏宽，不修改保存的比例。分隔条可通过 Tab 聚焦，上／下方向键调整，Home 恢复均分。

新建／编辑站点保存后，侧栏自动展开其所属分组并清空搜索，保证该站点可见；从某个分组中新建时默认继承该组。创建其他站点不会中断已有的 SSH 会话。

每个文件栏右上角可独立切换 **Detailed list / Icon grid**，当前工作会话中切换目录会保留展示模式。两种模式共用选择、双击、上传下载操作；详细列表显示目录层级，图标模式平铺当前目录。底部显示选中项的名称、类型、文件大小（含精确字节数）、修改时间与时区、完整路径；远程项同时显示可用的权限位。多选时汇总选中项数量与文件大小，不递归扫描文件夹大小。

## 应用设置与本地文件浏览

左侧底部 **Settings** 打开独立设置页：

- **General / Language**：英语／简体中文，实时切换。
- **Appearance / Theme**：Follow system（跟随系统，默认）／Dark／Light，实时应用。跟随系统时，系统深浅主题变化会自动更新界面、图标及终端配色；已有配置中明确保存的 Dark／Light 选择继续保留。
- **Files / Hidden files**：显示或隐藏本地、远程列表中的点号文件。
- **Monitoring / Resource refresh interval**：3／5／10／30 秒，仅在资源页打开时自动采集。

设置自动保存在当前本地配置中，重新启动后恢复；每台服务器的文件栏比例和已记忆的传输冲突选择仍独立保存。

选择私钥认证后，**Private key file** 旁的 **Browse…** 打开 Windows 原生文件选择窗口。支持无扩展名和中文路径，默认定位到当前路径所在文件夹（或本机 `.ssh` 目录），取消时保留原值。选择器只回填路径，不读取私钥内容，仍允许手动输入。原生对话框使用 [Windows Common Item Dialog](https://learn.microsoft.com/en-us/windows/win32/shell/common-file-dialog)。

## 文件冲突

文件管理底部的 **Transfers** 队列显示每项上传／下载的进度条、百分比、已处理／总大小、实时速度、耗时及预计剩余时间。速度采用最近约 3 秒的实际文件内容传输量计算（B/s、KiB/s、MiB/s），不包含 SSH/SFTP 协议开销，也不等同于服务器网卡总流量；等待用户处理冲突时暂停计速，持续无数据时速度回落为零。速度尚未稳定或正在扫描目录时，剩余时间显示 `—`。

目录中的跳过项计入处理进度，但不计入实际传输量；队列另行显示实际传输量，结束后保留平均速度和耗时。耗时包含等待时间，平均速度排除等待冲突处理的时间。零字节文件正常完成后显示 100%，文件内容复制完成但仍在发布／落盘时显示“正在完成传输”，成功确认后才到 100%。取消或失败保留当前进度，重试使用新的统计，不提供断点续传。

上传和下载遇到同名普通文件时，显示源文件与已有目标的路径、大小、修改时间，默认选择跳过。可以明确选择覆盖，并指定仅此文件、本次批量传输、当前连接或以后连接此服务器；上传／下载规则分别记忆。服务器“…”菜单中的“重置文件冲突选择”恢复询问。

同名目录会安全合并，并逐个处理内部文件冲突；文件／目录类型冲突和符号链接会拒绝。取消或失败保留已完成文件。覆盖先写同目录临时文件，完成后替换，复制失败／取消不会先删除已有目标。远程覆盖保留已有权限位；SFTP v3 无法直接替换时使用经过路径转义和类型检查的 Linux `mv -fT`，服务器不支持时保留原目标并报告错误。下载在 Windows 使用原子文件替换。当前不提供自动重命名、大小／时间条件覆盖或续传。

默认配置存放在当前用户应用配置目录，可通过 `--config` 指定独立文件。配置保存站点、分组、主机指纹、私钥路径、上次本地目录，以及按服务器隔离的分栏比例和明确记忆的冲突规则，不保存密码、私钥口令或 sudo 密码。修改站点的地址、端口或用户名会清除原冲突规则。配置损坏时保留原文件并禁止覆盖。删除分组后，原有站点回到未分组。

Windows 默认配置路径为 `%APPDATA%\arcnix\Arcnix\config\config.json`。配置格式保持版本 1，已有配置文件可以复制到该位置，或通过 `arcnix --config <path>` 直接使用。

## Terminal

连接服务器后点击顶部 **Terminal**，自动打开交互式 SSH 终端。支持 ANSI 颜色、UTF-8、键盘输入、Ctrl+C、窗口大小同步和最近 5000 行滚动历史；切换页面、语言或返回文件管理会保留当前终端会话与显示内容。工具栏提供复制选中文字、粘贴、清空显示、重新启动和关闭终端的图标按钮。

Ctrl+C 在有选中文字时复制，否则向远程程序发送中断；Ctrl+Shift+C 复制，Ctrl+Shift+V 粘贴。清空仅清除当前显示；关闭终端不影响文件管理与全局 SSH 连接，断开服务器会同时关闭终端并清理显示。

终端通过内置 SSH 库申请 `xterm-256color` PTY，复用当前连接的认证信息与已核对的主机指纹建立独立传输连接，因此不会被文件上传下载阻塞。认证信息仅保留在当前连接的进程内存中，断开后清理，不写入配置；终端输入输出不写入应用操作日志。远程 OSC 52 剪贴板写入及链接激活已禁用。

界面状态、连接和交互桥接由 Rust 管理，终端字符渲染使用随程序本地打包的 xterm.js，不需要 Node、网络 CDN 或系统 SSH 客户端。第三方版本、来源、校验值与 MIT 许可证位于 `crates/arcnix-desktop/assets/vendor`。当前每个活动服务器工作区提供一个终端；全屏程序在真实 Linux 主机上的兼容性仍需实际环境验证。

## 系统软件

1. 连接服务器，切换“系统软件”；应用识别发行版、包管理器和管理员权限，并读取已安装软件。
2. 输入软件包名搜索，点击目标包的“安装…”查看 APT 模拟结果和具体执行命令。检查目标服务器及预览后，点击“确认执行”才会安装。
3. root 会话直接执行；其他账户通过 sudo 提权。管理员密码留空时使用 `sudo -n`，需要密码时通过本次 SSH 命令的 stdin 传递，不进入命令字符串或配置；输出中的原样凭据会被过滤。
4. “更新软件源索引”对应 `apt-get update`，同样需要确认；不会执行系统整体升级。
5. 任务输出实时显示，结束后显示退出码并刷新软件包状态。执行期间锁定当前服务器与断开入口，已有文件传输先完成，后续文件传输等待系统任务结束。

软件管理只依赖 SSH，服务器未启用 SFTP 时仍可使用；只有文件管理不可用。当前完整支持 APT，DNF、YUM、APK、Pacman 仅识别并报告支持状态。暂不提供软件卸载、整机升级、后台任务托管或自动回滚；网络中断或超时后不能保证远程安装已经停止，需重连查询实际安装状态。请在任务期间保持应用与 SSH 连接。

搜索最多展示 200 项，已安装列表最多展示 1000 项。软件命令采用类型化操作生成，不接受任意 shell 输入；安装先验证精确包名和模拟结果，使用 `--no-remove` 拒绝涉及移除现有包的方案。参见 [APT 操作与产品规划](server-workbench.md)。

文件传输使用 SFTP，复用同一条 SSH 连接，认证后自动打开 SFTP 子系统，不需要单独连接。文件区显示 SFTP 的可用状态；服务器未启用或账号不可访问 SFTP 时，右上角仍显示 SSH 已连接，其他 SSH 模块继续可用。当前不支持 FTP／FTPS 独立连接。

## 资源概览与系统日志

- 资源概览通过 SSH 读取 Linux `/proc` 和 `df`，显示 CPU 使用率、核心数、内存与交换空间、负载、运行时间、分区容量及网卡收发速率／累计流量，不需要远程 Agent。
- CPU 和网络速率根据两次远程采样差值计算；第一次采样显示等待，无法读取的指标显示不可用及原因。仅在资源页面自动采集，默认约 3 秒一次；安装任务期间暂停自动采集。
- 系统日志支持 journalctl，以及当前账户可读的 syslog、messages、auth.log、kern.log 和 APT history.log。自动发现可用来源，提供服务单元、时间范围、级别筛选和本地关键词筛选；文件来源只提供文本筛选。
- 日志使用有界快照，可手动刷新或开启自动刷新。权限不足时显示原因，不自动提权；没有 SFTP 仍可读取资源和日志。当前不提供持续 `journalctl -f`、历史翻页、日志写入或服务控制。

采样与日志命令依据 [Linux proc 文档](https://www.kernel.org/doc/html/latest/filesystems/proc.html) 和 [journalctl 官方手册](https://www.freedesktop.org/software/systemd/man/latest/journalctl.html)。

## 第一版边界

- 可保存多台服务器；一个窗口同一时间连接一台服务器，传输串行排队。切换服务器会取消当前传输和排队任务。
- 支持普通文件和目录多选上传／下载；递归预扫描最多 64 层、100,000 项。符号链接／特殊文件会导致整棵目录在预扫描时失败，不会静默跳过。符号链接传输、拖放、续传、同步、远程编辑和交互式终端尚未实现。
- 文件名需要有效 Unicode；由于 Windows SSH 库的路径转换，远程包含反斜杠的路径会明确拒绝。
- 新文件下载通过同目录临时文件和硬链接发布，目标文件系统需支持硬链接（例如 NTFS）；不支持时会失败并保留已有目标。覆盖下载在 Windows 使用替换重命名。新文件上传依赖服务器遵守 SFTP 禁止覆盖重命名语义；已有文件的覆盖另有上述安全发布流程。
- 私钥算法和文件格式以 libssh2 的平台支持为准；已验证 Windows 上未加密 RSA PEM 私钥。口令入口已实现，其他密钥格式尚未逐一验证。
- 本地目录重命名目前仅支持 Windows；其他桌面平台尚未运行验收。

## 开发与验证

```powershell
cargo metadata --no-deps --format-version 1
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

导航、主题及本地 SSH 端到端验证（使用隔离配置，不访问真实服务器）：

```powershell
python tools/navigation_smoke.py
python tools/server_tree_smoke.py
python tools/system_theme_smoke.py
```

该验证需要 `artifacts/python` 中的 Playwright、Paramiko，以及 CairoSVG、Pillow。它会自行启动和关闭回环测试服务，并在 `artifacts/navigation-redesign` 保存截图与结果。可用 `--exe <path>` 指定其他已构建程序。

真实回环 SSH/SFTP 集成测试需要可选 Python 测试依赖，仅安装在项目忽略的 `artifacts/python` 目录：

```powershell
python -m pip install --target artifacts/python paramiko
python tools/sftp_fixture.py --ready artifacts/fixture.json
```

保持测试服务运行，在另一终端中执行：

```powershell
$fixture = Get-Content artifacts/fixture.json | ConvertFrom-Json
$env:ARCNIX_TEST_SSH_ADDR = $fixture.address
$env:ARCNIX_TEST_SSH_KEY = $fixture.client_key
cargo test --locked -p arcnix-core --test sftp_integration -- --ignored
cargo test --locked -p arcnix-desktop -- --ignored
cargo test --workspace --locked -- --include-ignored
```

测试服务仅监听 `127.0.0.1`，使用隔离临时目录、每次生成的密钥和固定测试账户 `arcnix / arcnix-test`。它只提供固定测试命令结果，不会执行传入的本机 shell 命令。验证包括真实握手、指纹拒绝、密码／私钥认证、中文路径、上传下载字节一致性、空文件、覆盖保护和取消清理。不要把这个测试服务用于生产。

功能规划见 [docs/v0.1-plan.md](v0.1-plan.md)。
改进记录见 [CHANGELOG.md](CHANGELOG.md)，OpenSSH 导入边界见 [docs/ssh-config-import.md](ssh-config-import.md)。

## 目录

```text
crates/arcnix-core       配置、领域类型、本地文件、SSH/SFTP
crates/arcnix-desktop    Rust 桌面 UI、后台任务与事件桥
tools/sftp_fixture.py   仅用于回环测试的 SSH/SFTP 服务
```

构建遵循调用者环境和项目 Cargo 配置；产物路径以本工作区 `cargo metadata` 的 `target_directory` 为准。

## 界面回归

```powershell
python -m pip install --target artifacts/python playwright
python tools/ui_smoke.py
python tools/ui_smoke.py --packages
```

测试直接连接独立测试进程的本机 WebView2 调试接口，不下载浏览器，不使用默认配置或真实服务器。默认覆盖菜单、模态、表单、文件和标签；`--packages` 需要保持回环 SSH fixture 运行，使用自动生成的测试私钥连接本机固定响应服务，覆盖资源二次采样、日志筛选与权限提示、自动刷新、断开后清空，以及软件查询、预览确认、任务输出和状态刷新。不会在本机或真实服务器执行 apt。截图位于 `artifacts/ui-redesign`。测试结束时关闭调试进程，普通应用启动不启用调试端口。界面原语约定见 [docs/ui-design.md](ui-design.md)。
