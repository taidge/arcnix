# 0.1 验证记录

## Arcnix 更名验证（2026-11-13）

实际工作区为 `D:\Works\arcnix`，`cargo metadata --no-deps --format-version 1` 返回的产物目录为 `D:\Works\arcnix\target`。两个包为 `arcnix-core`、`arcnix-desktop`，桌面程序为 `arcnix.exe`，Windows 产品名称和命令行帮助均为 Arcnix。

工作区 103 项 Rust 测试全部通过，包含显式启用的回环 SSH/SFTP、主机指纹、密钥认证、终端、资源、日志及包管理测试；格式检查、构建和严格 Clippy 通过。导航脚本的 6 组实际 WebView 验证及服务器树回归通过，覆盖改名后的资源加载、SSH 连接、终端、SFTP、双主题及拖动排序。图标透明度检查改为检查四角透明及图案非空，避免把图案内部的固定像素误认为背景。

源码、资源和文档已检查旧名称残留；日志和汇总位于 `artifacts/rename-validation`，改名前的文件备份与历史验证产物保留在忽略的 `artifacts` 目录。

## 上传下载进度与速度（2026-11-13）

完整工作区测试（含显式启用的真实回环 SSH/SFTP 测试）103 项通过，格式、桌面构建及严格 Clippy 通过。新增覆盖混合目录上传／下载中跳过项推进逻辑进度但不增加实际字节、零字节文件、冲突等待不参与计速、停滞速度归零、超大计数、完成／取消边界。

`python tools/transfer_progress_smoke.py` 的 5 组真实 WebView 验证通过。延迟固定的回环 SFTP 服务完成 6 MiB 上传和下载，观察中间进度、可访问进度值、速度及有限 ETA，结束后检查 100%／平均速度与实际字节；下载 SHA256 一致。冲突等待时隐藏速度／ETA，跳过后为零字节和零平均速度。取消后统计冻结，重试完整重新传输；1000×700 窗口无横向溢出。截图和结果位于 `artifacts/transfer-progress-ui`。

全部文件及认证材料均为隔离 fixture 数据，未连接真实服务器。速度表示 SSH/SFTP 传输的文件内容速率，不是网卡线速；现有协议、落盘和网络条件会影响真实服务器表现。

## 文件名焦点框修正

移除文件名与目录展开按钮的输入框式焦点轮廓；鼠标选择保留整行高亮，键盘导航将细焦点线放在整行。构建、格式检查与 18 组无需 SSH 的实际桌面回归通过，检查 Ctrl 多选后文件名无 outline／shadow、鼠标行无 outline，以及 Tab 后整行有约 1 CSS px 焦点线。WebView 的 DPI 缩放使计算值约为 0.98px，断言使用合理容差。截图为 `artifacts/ui-redesign/file-focus-selection.png`，记录为 `artifacts/file-focus-ui.log`。

## 分段路径、资源管理器选择、目录树与文件信息

2026-11-13：完整 93 项 Rust 测试通过（核心 34、密钥 7、日志 1、资源 6、包管理 2、SFTP 12、命令 3、桌面 28），构建、格式及严格 Clippy 通过。新增驱动器／UNC 根目录与 POSIX 分段路径测试，以及嵌套选择去重、同名前缀目录不误判的传输测试。

`python tools/ui_smoke.py --packages --legacy-host-key --isolated-crypto-runtime` 的 27 组实际 WebView 检查通过，新增验证：普通单击替换选择，Ctrl 增减、Shift 范围、Ctrl+A／Ctrl+空格／Shift+方向键；异步展开两级目录与收起后的不可见选择清理；Ctrl+L 聚焦、Escape 不提交、Enter 跳转；点击上级路径段跳转；Windows 剪贴板实测中文完整路径与远程 POSIX 路径。

两栏详细列表／平铺图标独立切换，模式跨当前会话内目录跳转保留。检查选中项类型、精确字节数、完整时间、完整路径和远程权限；展开的 SFTP 子项重命名仍位于原父目录，并能实际下载到本地。原有传输冲突、软件、日志、资源与设置回归通过。系统命令仍为隔离 fixture 响应，未操作真实服务器。

截图位于 `artifacts/ui-redesign/files-icon-grid.png`、`file-details-info.png`、`file-tree-expanded.png`、`explorer-files.png`；日志为 `artifacts/browser-*.log`。并发 Rust 集成测试曾暴露 fixture 枚举期间临时文件消失的竞态，修复为忽略该条已消失项目后全套通过；不是忽略真实目录或权限错误。

## Files 双击进入文件夹

桌面构建、格式检查和严格 Clippy 通过。`python tools/ui_smoke.py --packages --legacy-host-key --isolated-crypto-runtime` 的 21 组实际 WebView 回归通过，覆盖本地／远程文件夹前的 `+`、单击只选中、双击进入、Enter 进入、本地行空白处双击进入、多选控件与双击文件保留选中。原有上传下载、冲突处理、应用设置及系统模块回归通过。

Glory 0.7.0 的 Wry Activate 不包含点击次数，因此在应用 bootstrap 中将浏览器原生单击／双击转发至现有 Rust 选择／激活动作，使用系统双击时序；键盘激活保持原生。路径和文件业务状态由 Rust 管理。截图为 `artifacts/ui-redesign/folder-double-click.png`，记录为 `artifacts/file-navigation-ui.log`。没有新增远程命令，测试使用隔离回环服务器。

## 主机密钥算法切换与已有信任记录

通过对配置中主机进行不带认证的握手核对，确认新默认 Ed25519 指纹与报警一致，而 RSA SHA-2 协商得到的指纹仍与已有信任记录完全相同。没有读取用户私钥、发送密码或改写其主机信任。

`PendingConnection::open_for_known_host` 在共享 15 秒期限内尝试受支持的现代主机密钥类型，只返回与原 SHA256 指纹匹配的连接，再进入认证。RSA 使用 SHA-2 签名；不为匹配旧记录额外启用 SHA-1／DSA。所有密钥都不匹配时，拒绝连接且保留原记录。

90 项完整 Rust 测试、格式、桌面构建和严格 Clippy 通过。新增测试覆盖默认算法变化后保留 RSA 指纹并用 Ed25519 客户端密钥认证、默认指纹本就匹配的连接，以及服务器密钥不匹配时服务端认证审计没有收到任何凭据。

`python tools/ui_smoke.py --packages --legacy-host-key --isolated-crypto-runtime` 的 19 组实际桌面回归通过，复现“保存 RSA、默认协商 Ed25519”的升级场景，连接、文件传输和系统模块均可使用。最终独立配置保留原 RSA 信任记录。记录为 `artifacts/host-key-*.log`。同时修复操作日志初始消息与下一条日志缺少换行。

## Windows Ed25519 与内置 OpenSSL

旧 WinCNG 测试程序使用临时生成的 OpenSSH Ed25519 密钥连接本机 fixture，复现 `[Session(-1)] unknown error`。启用 `ssh2` 的 `openssl-on-win32` 与 `vendored-openssl` 后，同类认证与文件往返通过。OpenSSL 源码版本由 Cargo.lock 固定为 3.6.3，静态库在实际 workspace 的 target 下构建。

全部 87 项 Rust 测试通过，包含新增的 4 项密钥专项：普通 Ed25519 的命令与 SFTP 文件往返、口令加密 Ed25519、空／错误口令拒绝且不回显秘密、未授权密钥的服务器拒绝提示。原有密码与 RSA PEM 认证、SSH 主机指纹、SFTP、系统操作与后台任务回归通过。

专项测试程序在 PATH 仅含 Windows System32 的环境下通过，导入表没有 libssl／libcrypto DLL。所有认证使用 fixture 临时生成的密钥，没有读取真实私钥或连接真实服务器。

复现命令（先启动 `python tools/sftp_fixture.py --ready artifacts/fixture.json`）：

```powershell
$env:ARCNIX_TEST_FIXTURE = (Resolve-Path artifacts/fixture.json).Path
cargo test -p arcnix-core --test key_auth_integration --locked -- --ignored
```

MSVC 源码构建需要原生 Windows Perl；本轮将经官方 SHA256 校验的便携 Strawberry Perl 放在忽略的 `artifacts/tooling` 中，通过进程级 `OPENSSL_SRC_PERL` 使用，没有安装到系统或修改全局环境。运行程序不需要这些构建工具。上游 OpenSSL 静态库未携带其 C 调试 PDB，MSVC 链接产生 LNK4099 提示，不影响认证功能或静态链接结果。

桌面构建、格式检查和严格 Clippy 通过。`python tools/ui_smoke.py --packages --isolated-crypto-runtime` 的 19 组实际 WebView 检查通过：桌面子进程 PATH 仅含 Windows System32／Windows，使用生成的 OpenSSH Ed25519 私钥完成连接、SFTP 双向冲突处理、资源、日志和软件模块回归。系统操作仍为 fixture 固定响应，没有运行实际 APT。最终 `arcnix.exe` 导入表没有 OpenSSL／libssh2 DLL；记录位于 `artifacts/ed25519-runtime-dependencies.txt`，测试与构建日志为 `artifacts/ed25519-*.log`。

2026-11-12，Windows x64，Rust 1.99.1。实际工作区产物目录：`D:\Works\arcnix\target`。

## 自动化验证

- 工作区构建通过。
- `cargo fmt --all -- --check` 通过。
- `cargo clippy --workspace --all-targets --locked -- -D warnings` 通过。
- 15 个常规测试通过：配置持久化、错误配置保护、分组、主机信任、本地文件、传输取消、路径约束、旧会话响应和取消状态。
- 6 个真实回环集成测试通过：5 个 SSH/SFTP 后端测试及 1 个桌面后台任务通道测试。验证密码和未加密 RSA PEM 私钥认证、指纹不符拒绝、中文路径、空文件、SHA256 往返一致、拒绝覆盖、取消与临时文件清理、命令输出与退出码、跨服务器传输拒绝、旧 epoch 拒绝和断开。

集成测试服务仅监听本机，使用临时目录与独立生成的密钥。远程 shell 返回固定测试结果，未据此宣称验证了真实 Linux 主机的信息采集输出。

## 实际桌面验证

使用独立配置 `artifacts/ui-smoke/config.json` 启动编译后的 `arcnix.exe`，不使用用户服务器或凭据：

- 系统 WebView2 窗口启动、Glory/Sapid 组件渲染与可访问性树正常。
- 双栏文件列表、分组/站点侧栏、传输队列、日志和状态栏显示正常。
- 通过实际控件填写名称、主机和用户名，保存服务器后确认 JSON 文件落盘。
- 通过“打开”进入本地子目录，实际目录列表和配置中的上次本地目录同步更新。
- 验证后修正搜索框下的多余留白，并将工作区和侧栏滚动限制在各自区域，状态栏保持可见。

SSH 认证和传输通过真实后端与任务通道集成测试验证；未在桌面 UI 中输入认证秘密。非 Windows 平台、其他私钥格式、真实远程主机兼容性和网络中断恢复仍需后续环境验收。功能边界详见 README。

## 同日后续改进验收

自动回归共 42 项通过（含显式启用的回环集成测试）：核心 21 项、真实 SSH/SFTP 7 项、桌面 14 项。其中桌面包括 1 个真实后台任务通道测试，另有 2 个不依赖 SSH 服务的并发／结束事件测试。

新增验证覆盖：

- 中文嵌套目录与空目录递归上传下载，逐文件 SHA256 一致、累计进度单调、已有目录不合并、取消后保留已完成文件并清理文件级临时文件。
- OpenSSH 配置顺序与模式匹配、代理拒绝、编码与大小限制、不同密钥身份保留、导入事务性更新。
- 筛选排序、目录置前、重试标识隔离、目录请求序列拒绝过期结果。
- 阻塞 SSH 握手期间本地浏览仍可完成；失效会话传输必定结束；通过测试服务主动断开连接，验证 ConnectionLost 事件和后续传输失败。

最终工作区构建、格式检查和严格 Clippy 通过。已重新启动新版默认配置窗口，进程响应正常；通过实际窗口可访问性树确认 SSH 导入、双栏筛选排序／全选、队列暂停和批量重试控件存在。

本轮桌面自动化点击遇到系统错误 `DwmFlush ... desktop composition is disabled`，恢复后重试返回 `coordinate input geometry is unavailable`。因此未完成新版控件的点击／截图验收；交互行为目前以 Rust 测试和真实任务通道集成为证据，不将可访问性树检查等同于完整视觉验收。旧窗口确认未连接且队列为空后停止并替换程序；新版已保持运行。

## 界面原语重构验收

系统窗口自动化仍报告坐标不可用，因此改用独立测试进程的本地 WebView2 CDP 接口进行真实渲染和交互验收。`tools/ui_smoke.py` 为此创建了独立配置、本地测试目录与虚构服务器，并在结束后关闭测试进程；未连接真实服务器或输入凭据。

8 组界面回归全部通过：常驻表单／全局编辑删除移除；悬停菜单与具体站点绑定；模态初始焦点、背景 inert 和 Escape；服务器认证选项、内联验证与保存；文件操作失败保留对话框、成功后关闭；底部单一标签面板；分组模态；SSH 导入错误处理、长列表菜单无裁剪、删除确认的目标名称；1000×700 下对话框提交区保持可见。

截图和详细检查记录在 `artifacts/ui-redesign`。已检查工作台、站点菜单、新建服务器、校验错误与窄窗口截图。本轮常规 Rust 测试 34 项通过，另 8 个需 SSH fixture 的测试按默认设置忽略；后端未在本轮修改。严格 Clippy、格式检查和工作区构建通过。

## 图标按钮统一验收

界面回归扩展为 10 组，全部通过。新增检查：导航/刷新/筛选/队列/折叠按钮文本为空，实际 SVG mask 存在；活动面板和队列状态切换时图标及可访问名称同步切换；关闭使用 Sapid CloseButton，无可见文字，悬停显示“关闭对话框”，点击正常关闭。已目视检查 `artifacts/ui-redesign/close-icon-tooltip.png`。常规 Rust 测试 34 项、严格 Clippy、格式检查及工作区构建通过；后端需独立 SSH fixture 的 8 项测试未在本轮重跑。

## 系统软件模块验收

完整启用 fixture 的工作区测试共 58 项通过：核心单元测试 28，包管理 SSH 集成 2，SFTP 集成 7，流式命令集成 3，桌面测试 18。覆盖精确包名与注入拒绝、模拟计划、root/sudo 命令生成、无 SFTP 连接、单次会话绑定计划、实时双流与退出码、大量输出尾部限制、私密 stdin、超时后会话恢复及跨字节分块凭据隐藏。

`python tools/ui_smoke.py --packages` 的 11 组界面回归通过。新增通过生成的隔离私钥连接本机 SSH fixture，验证软件页面识别和已安装列表、搜索 nginx、确认前显示模拟结果与命令、确认后流式结果及退出码、刷新为已安装、执行时断开/其他服务器选择禁用、更新索引预览及取消。配置与本地目录均为独立测试数据，测试服务不运行 shell/apt。

目视检查 `software.png`、`package-confirm.png`、`package-completed.png` 的布局。CPU/内存使用率监控未实现；软件包管理本轮未在真实 Debian/Ubuntu 系统执行 apt。构建、格式和严格 Clippy 通过。

## 资源数值与系统日志验收

完整启用回环 SSH fixture 的工作区测试共 71 项通过：核心单元测试 32，日志集成 1，资源解析／采样 6，包管理集成 2，SFTP 集成 7，流式命令集成 3，桌面测试 20。新增覆盖 CPU guest 去重、内存单位和回退估算、远程时间间隔、重启／计数器重置、缺失与截断数据、journal JSON／二进制消息、UTC 时间、命令参数拒绝及无 SFTP 的资源与日志后台通道。

14 组实际 WebView 检查通过；新增资源首个／第二个快照、CPU 30% 与网速 1 KiB/s 的差值计算、磁盘挂载路径、窄窗口两列卡片，以及 journal 服务／时间／级别／文本筛选、多行消息、权限错误、传统文件日志、自动刷新和断开后旧数据清空。界面统一检查输入框焦点无外层粗圈，仅保留 1px 边框变色。

截图在 `artifacts/ui-redesign/resources.png`、`resources-narrow.png`、`system-logs.png`、`system-logs-narrow.png` 和 `input-focus.png`。测试为真实 SSH 传输上的固定 Linux 响应，未读取真实服务器日志或运行真实 apt；不同 Linux 发行版和权限策略仍需实际环境验收。资源曲线尚未实现。

## 紧凑顶栏、分栏与文件冲突验收

工作区测试共 79 项通过：核心单元测试 33，日志 1，资源 6，包管理 2，SFTP 12，流式命令 3，桌面 22。SFTP 新增真实回环传输验证双向覆盖／跳过／取消、递归目录合并、目标类型拒绝、复制期间取消、晚出现的目标重新询问、特殊路径引用、发布失败保留原文件，以及保留远程 0644／0755 权限和设置权限失败保护。后台桥接测试验证等待用户选择时可以取消或切换会话，任务能收到结束事件，过期选择不会继续执行。

17 组实际 WebView 界面回归通过，验证顶栏标签与品牌同排、无常驻连接按钮；拖动分隔条实时改变双栏宽度，Home 恢复均分，两个服务器各自保存不同的比例，切换恢复并启动全新进程再核验本地配置。文件冲突模态实际完成跳过、覆盖、保存上传规则、后续连接沿用、下载方向仍询问、重置规则、关闭后取消；通过测试目录逐字节检查最终内容。

滚动条检查其真实计算样式为 6px 且没有上下箭头。目视检查 `split-panes.png` 和 `file-conflict.png`，本地路径不展示 Windows 内部前缀。所有远程文件操作仅作用于隔离 fixture 沙箱；APT 与日志仍为固定测试响应。

## 统一 SSH 连接状态验收

右上角显示当前服务器、SSH 状态和连接／取消／断开按钮；无站点时提供添加入口，已有站点但未选择时明确提示选择左侧服务器。文件区仅显示 SFTP 能力，不引入第二次连接操作。

79 项 Rust 测试与 18 组真实 WebView 检查通过，格式、构建、严格 Clippy 通过。新增通过只开放 SSH、拒绝 SFTP 子系统的隔离账号核验：全局显示 SSH 已连接，文件区显示 SFTP 不可用且下载禁用，资源数值、软件查询和系统日志仍能读取。截图为 `artifacts/ui-redesign/ssh-without-sftp.png`。

## 默认英语、应用设置、站点分组与原生路径选择验收

完整工作区 83 项 Rust 测试通过：核心 34，日志 1，资源 6，包管理 2，SFTP 12，流式命令 3，桌面 25。新增本机设置的默认英语／保存重载／非法间隔保护，以及词条模板保留用户数据、语言和源绑定同时更新的测试。格式、构建及严格 Clippy 通过。

19 组主界面回归通过，覆盖英语菜单和提示、真实回环 SSH 上切换中英文后连接与软件状态保留、资源／软件／日志及文件冲突全流程。默认语言使用英语；用户定义的中文站点名、路径和远程消息保持原文。

分组专项实际复现旧问题：JSON 已保存 Group B，但侧栏仍筛选 Group A。新版检查指定分组、继承分组、未分组、编辑移动和取消，JSON 与侧栏显示一致。Settings 专项验证深浅主题、语言、点号文件显示、采样间隔的即时生效和全新进程重载。

Windows 原生文件窗口完成真实选择和取消：仅使用项目隔离的假文件，文件名包含中文、空格且没有扩展名；选中后回填路径，取消保留原值。界面自动化坐标点击报告 geometry unavailable，采用 UI Automation 设置系统窗口的 File name 后以 Enter 确认，再以 Escape 取消；已通过界面回填结果验证，未读取真实私钥。

截图位于 `artifacts/settings-ui/settings-dark.png`、`settings-light.png`、`settings-chinese.png`；分组截图与独立脚本位于 `artifacts/group-assignment`、`tools/group_assignment_smoke.py`。系统日志和 APT 仍使用固定响应，SFTP 使用真实隔离文件传输。

## 交互式 SSH Terminal 验收（2026-11-13）

完整启用本机 SSH fixture 的工作区测试共 97 项通过：核心单元 34、认证 7、日志 1、资源 6、包管理 2、SFTP 12、流式命令 3、PTY 3、桌面 29。新增真实 PTY 验证输入、UTF-8 分块、ANSI、窗口尺寸、Ctrl+C、退出码、大量输出以及服务器拒绝 PTY；后台通道验证 SFTP 阻塞时终端仍响应、断开后旧会话输入被拒绝。格式、严格 Clippy 和桌面构建通过。

`python tools/terminal_smoke.py` 的 9 组实际 WebView 检查通过：使用隔离 Ed25519 私钥及原 RSA 主机指纹打开终端；中文复制粘贴及阻止远程 OSC 52 修改剪贴板；快速输入校验 SHA256；实际 SFTP 上传并保留终端；切换中英文后保留屏幕并继续输入；缩小窗口；超过 256 KiB 输出完整到达；shell 退出／重启保留全局 SSH；断开后销毁终端显示。图标工具栏、窄窗口布局和输出已检查截图 `artifacts/terminal-ui/terminal.png` 与 `terminal-narrow.png`。

测试仅连接本机回环服务，服务使用固定交互协议，不执行本机系统 shell；未读取真实服务器凭据、访问真实服务器或运行 apt。真实 Linux 发行版、全屏终端程序及网络断线恢复仍需实际环境验证。终端渲染使用程序内打包的 xterm.js，Rust 负责 SSH、PTY、状态及输入输出桥接；不依赖系统 SSH、Node 或 CDN。
