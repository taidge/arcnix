# OpenSSH 站点导入

核心 API：`ssh_config::import_file(&Path) -> Result<ImportReport>` 和 `parse_config(&str) -> Result<ImportReport>`。报告包含 `servers: Vec<ServerProfile>`、`warnings: Vec<String>`，不会自动保存配置，也不会发起网络连接或执行配置中的命令。

支持明确的 `Host` 别名、多别名、通配符默认块和 `!` 排除模式；主机匹配不区分 ASCII 大小写。依照文件顺序应用首个匹配值，支持 `HostName`、`User`、`Port`、`IdentityFile`。通配符本身不会生成站点，重复别名只导入一次。缺少 User 时采用当前本机用户名，默认端口 22，默认远程路径 `.`。

支持引号、注释、`Key=value`，保留普通 Windows 路径中的反斜杠。IdentityFile 可展开当前用户的 `~`、`%d`、`%h`、`%n`、`%r`、`%p`、`%%`；不展开环境变量或执行命令。存在多个 IdentityFile 时只选首个，后续可在服务器编辑器修改。密钥文件只记录路径，导入时不打开它。

包含 Include 或 Match 的文件整体跳过并显示警告，以免遗漏跳板机或条件规则。有效匹配的 ProxyJump、ProxyCommand、CertificateFile、IdentityAgent、RemoteCommand、LocalCommand（非 none）会使对应站点被跳过，绝不会自动绕过代理直接连接。其他选项不导入并在报告中列出，连接采用 Arcnix 自身的设置和指纹信任机制；导入不能完全替代 OpenSSH 的全部配置行为。

文件限制为 UTF-8、1 MiB、512 个明确别名和 8192 条选项。无效站点单独跳过并报告原因，语法损坏返回错误。导入的密码／私钥口令为空，不能从配置恢复秘密。
