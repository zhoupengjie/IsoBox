# IsoBox 交接文档

给在另一台机器上接手的人（或 Claude 会话）。先读这份，再读 [ROADMAP.md](ROADMAP.md)。

## 1. 项目是什么

IsoBox 让不信任的 Windows 桌面软件（起因是微信、QQ、网盘这类会扫描磁盘的国产软件）以一个**独立的本地标准账户**运行：

- 软件读不到你的用户目录和受保护的硬盘，写不了库和交换文件夹以外的地方
- 窗口照常出现在你的桌面上，麦克风、摄像头、剪贴板正常可用
- 不装驱动，只用 Windows 自带的账户和 NTFS 权限；卸载一条命令清干净

已知边界：同一个桌面，防不住截屏、键盘记录、读剪贴板；FAT/exFAT 的 U 盘和存储卡防不住；强制要求管理员权限的安装包装不进隔离账户。

## 2. 当前进度

| 阶段 | 内容 | 状态 |
|---|---|---|
| 一、核心重构 | 库、自动识别硬盘、权限备份和还原、固定安装位置、迁移、沙盒自动测试 | 完成 |
| 二、图形界面 | Tauri 主程序（软件、库、保护状态、设置、添加软件向导）和托盘 | 完成 |
| 三、安装程序 | NSIS 图形和静默安装、捆绑 7-Zip、系统卸载入口 | **下一步** |
| 四、发布 | CI、开源许可证、代码签名、文档、试用 | 未开始 |

测试现状：单元测试 12 个（`cargo test --workspace`），Windows 沙盒端到端测试 90 项，全部通过。

在原开发机上已经实际部署并日常使用：系统盘的库里装着微信和 QQ，D 盘受保护，界面和托盘可用。**还没在真机上实测的**：从界面发起的管理员操作（UAC 路径。后端命令和界面流程分别测过，但没连起来在真机上点过），以及托盘图标的显示。

## 3. 新机器环境搭建

需要 Windows 11 专业版、企业版或专业工作站版（家庭版能编译和运行，但没有 Windows 沙盒，跑不了端到端测试）。

1. **Visual Studio 2022 生成工具**，勾选“使用 C++ 的桌面开发”，并确认包含 **Windows 11 SDK**（缺 SDK 时链接会报找不到 kernel32.lib）：
   ```powershell
   winget install Microsoft.VisualStudio.2022.BuildTools --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
   ```
2. **Rust**（MSVC 工具链，项目用 edition 2024）：`winget install Rustlang.Rustup`
3. **7-Zip**，装在默认位置 `C:\Program Files\7-Zip`：`winget install 7zip.7zip`
4. **Windows 沙盒**：在“启用或关闭 Windows 功能”里勾选 Windows 沙盒，重启
5. 可选：**Node.js**（只用于在浏览器里预览界面）、**gh**（GitHub 命令行）

然后：

```powershell
git clone https://github.com/zhoupengjie/IsoBox.git
cd IsoBox
cargo build --release --workspace     # 第一次要下载编译 Tauri，几分钟
cargo test --workspace
powershell -ExecutionPolicy Bypass -File tests\sandbox\run.ps1   # 端到端测试，约 1 分钟，退出码 0 表示全部通过
```

在本机安装（管理员终端）：

```powershell
.\deploy.ps1                                            # 编译并装到 C:\Program Files\IsoBox
& "C:\Program Files\IsoBox\isobox-setup.exe" install    # 预演
& "C:\Program Files\IsoBox\isobox-setup.exe" install --apply
```

之后从开始菜单的 IsoBox 打开界面。

## 4. 代码结构

Cargo 工作区，两个包：根目录的 `isobox`（核心库和三个命令行程序）、`gui/` 的 `isobox-gui`（Tauri）。

| 位置 | 作用 |
|---|---|
| `src/bin/setup.rs` → `isobox-setup.exe` | 管理员工具：install / uninstall（默认预演，`--apply` 执行）、library、add、remove-app、move-app、migrate、protect、set-exchange、backup-acl、restore-acl、drives。全局选项 `--log <文件>`、`--non-interactive` 供界面隐藏调用 |
| `src/bin/launch.rs` → `isobox.exe` | 普通权限命令行：run、list、status --json、preview、shortcut、theme-sync 等 |
| `src/bin/gui_launch.rs` → `isobox-launch.exe` | 快捷方式背后的无窗口启动器；`--watch-theme` 是主题监视进程 |
| `gui/` → `isobox-gui.exe` | 图形界面和托盘。`gui/ui/` 是纯 HTML/CSS/JS（无打包工具），`gui/src/elevate.rs` 负责 UAC 调用 |
| `src/status.rs` | 汇总所有状态（界面和 `status --json` 共用） |
| `src/apps.rs` | 添加、移除、移动软件 |
| `src/library.rs` | 库：`<盘>:\Program Files\IsoBox`，标记文件 `library.toml`，`apps\` 目录 |
| `src/acl.rs` | 锁目录、读写 SDDL、检查禁止规则、备份还原 |
| `src/drives.rs` | 识别盘：固定/可移动/网络、文件系统、是否 USB |
| `src/unpack.rs` | 7-Zip 拆包、安装包类型识别、安装前预览 |
| `src/launch.rs` | 以隔离账户身份启动（CreateProcessWithLogonW） |
| `src/theme.rs` | 深浅色和强调色同步、事件驱动的监视 |
| `src/shortcut.rs` | 快捷方式（PowerShell 写、COM 读） |
| `src/config.rs` | 配置；修改配置时保留注释，写入前先解析验证 |
| `src/appcontainer.rs` | AppContainer 模式（保留，但微信、QQ 跑不了，见第 6 节） |
| `tests/sandbox/` | 沙盒端到端测试：`run.ps1` 在宿主机运行，`run-in-sandbox.ps1` 在沙盒里执行 |
| `examples/probe.rs` | 测试用探测程序：以隔离账户身份尝试各种读写 |
| `tools/` | 开始菜单“添加软件”“移除软件”用的 PowerShell 脚本（阶段三可能被界面取代） |
| `deploy.ps1` | 开发用部署脚本（阶段三会被安装程序取代） |
| `isobox.toml` | 配置模板。实际生效的是安装目录里那份 |

## 5. 不要破坏的设计

- **安装位置固定**为 `%ProgramFiles%\IsoBox`，只有管理员能写（isobox-setup 以管理员身份运行，目录可写就等于提权漏洞）。会改系统的命令通过 `paths::require_installed()` 检查；开发时设 `ISOBOX_DEV=1` 跳过。
- **程序在哪个目录就把那里当安装目录**（`paths::install_dir()` = exe 所在目录），所以四个 exe 和 `isobox.toml` 必须在同一个目录。
- **库的权限**：根目录关闭继承，管理员和 SYSTEM 完全控制、Users 只读；`apps\` 再加隔离账户修改权限（QQ 要写自己的 versions 目录）。根目录关闭继承也保证硬盘根上的禁止规则不会传进库。
- **隔离账户**：名字来自配置（默认 `isobox`），备注固定为 `isobox sandbox account`，卸载只认这个备注，并拒绝当前登录账户的 SID。随机 32 位密码存在当前用户的凭据管理器，目标名 `isobox:<账户名>`。
- **保护**：在盘根或文件夹上加 `(OI)(CI)F` 的禁止规则，加之前先备份权限。卸载时扫描所有盘根和配置里的位置，凡是有该 SID 显式禁止规则的都撤销。**顺序：先撤销权限、删配置文件，最后才删账户**，否则留下“未知 SID”。
- **交换文件夹**是显式允许，排在继承来的禁止之前，所以放在受保护的盘里也能用。
- **界面永远以普通权限运行**；管理员操作用 `ShellExecuteExW("runas")` 隐藏调用 `isobox-setup --log <临时文件> --non-interactive ...`，结束后读日志显示。
- **主题同步**：把当前用户 HKCU 里的主题值以隔离账户身份用 reg.exe 写过去，再广播 `WM_SETTINGCHANGE("ImmersiveColorSet")`。监视用 `RegNotifyChangeKeyValue` 事件驱动，命名互斥量 `Local\IsoBoxThemeWatcher` 保证只有一个；托盘运行时由托盘线程负责。
- **静态链接 C 运行库**（`.cargo/config.toml`）：不然在没装 VC++ 运行库的电脑上打不开，沙盒测试就是这样发现的。

## 6. 踩过的坑

- **Claude 桌面应用的包虚拟化**：从 Claude 桌面应用的会话里启动的进程继承它的 MSIX 包环境，写 `HKCU\Software\Classes` 和 AppData 会被重定向到它自己的目录。测试要通过 WMI（`Invoke-CimMethod Win32_Process -MethodName Create`）在包外启动，沙盒测试脚本已经这样做。
- **AppContainer 跑不了 Chromium 系软件**（微信 4、QQNT）：容器里不能创建全局命名管道，Crashpad 和 Chromium 的进程通信都依赖它。所以默认用账户模式。
- **Windows 沙盒方案被否决的原因**：每次会话的内置用户密码随机，DPAPI 主密钥解不开，登录凭据丢失；网卡 MAC 每次变化且无法固定（Hyper-V 防 MAC 欺骗），微信每次都要短信验证。实验发现自建固定密码用户的 SID 每次相同，可以绕过 DPAPI 问题，但 MAC 问题无解。
- **icacls 把完全禁止显示为 `(N)`**，不是 `DENY`；继承来的显示为 `(I)(OI)(CI)(N)`。
- **PowerShell 文本匹配默认不区分大小写**：路径里有 IsoBox 时，匹配 `isobox` 会误中，要匹配 `\isobox:`。
- **CreateProcessWithLogonW 的命令行最长 1024 字符**，所以主题同步分批执行。
- **Rust 的 `std::process::Command` 会让子进程继承所有可继承句柄**，常驻的子进程会占住调用者的输出管道。拉起监视进程时改用 `CreateProcessW` 并关闭继承。
- **写入相同的注册表值不会触发** `RegNotifyChangeKeyValue`。
- **Windows PowerShell 5.1 把无 BOM 的 UTF-8 脚本当 ANSI 读**：含中文的 `.ps1`（`deploy.ps1`、`tools\*.ps1`）必须存为 UTF-8 带 BOM；测试脚本保持纯 ASCII。
- **`net user` 遇到超过 14 个字符的密码会交互确认**，脚本里用 `NetUserAdd` 或 `New-LocalUser`。
- **删除正被某个进程当作当前目录的文件夹会失败**（os error 32），迁移时遇到过，现已改为只提示。
- **7-Zip 制作自解压包时**，`-sfx` 要给模块的完整路径。
- **PowerShell 里 `R` 是 Invoke-History 的别名**，别把辅助函数命名为 R。
- **Windows 沙盒同时只能开一个**，测试前要关掉其他沙盒。

## 7. 下一步：阶段三（安装程序）

建议做法：

1. `cargo install tauri-cli --version "^2" --locked`，在 `gui/tauri.conf.json` 里打开 `bundle`，目标 `nsis`，`installMode: perMachine`，固定安装到 `%ProgramFiles%\IsoBox`（隐藏目录选择页）。
2. 把 `isobox.exe`、`isobox-setup.exe`、`isobox-launch.exe` 作为资源打进安装包，**必须放在安装目录根上**，和 `isobox-gui.exe` 同一层（见第 5 节）。
3. 用 NSIS 安装钩子完成 `deploy.ps1` 现在做的事：锁定安装目录权限、创建开始菜单项、首次安装时复制配置模板。
4. 安装完成后的初始化（建账户、选择要保护的盘、建库）：可以在安装向导里加自定义页，也可以交给界面首次启动的向导。界面在没有隔离账户时已经会显示“立即安装”，可以从这里扩展。
5. 卸载钩子：先运行 `isobox-setup uninstall --apply`（考虑“保留聊天记录”选项），再删文件。卸载程序只能删它装的文件，**不能整个目录删**，因为系统盘的库就在安装目录里。
6. 捆绑 7-Zip：`7z.exe`、`7z.dll` 加 LGPL 许可证，放在安装目录的子目录里，`unpack::DEFAULT_7Z` 改为优先用捆绑的。精简版 `7zr.exe` 解不开 NSIS，不能用。
7. 静默安装 `/S`：用默认值（自动保护所有内置非系统盘、交换文件夹在下载目录）。
8. 把沙盒测试改为“运行安装包 → 运行卸载程序”，替换现在的 deploy 步骤。

阶段四的注意事项：GitHub Actions 的 Windows 运行器不支持 Windows 沙盒，CI 里可以直接在运行器上（它本身有管理员权限）跑安装和卸载测试。

## 8. 常用命令

```powershell
# 状态（普通权限）
& "C:\Program Files\IsoBox\isobox.exe" status
& "C:\Program Files\IsoBox\isobox.exe" preview <安装包>          # 类型、能否拆包、主程序候选
& "C:\Program Files\IsoBox\isobox-setup.exe" drives
& "C:\Program Files\IsoBox\isobox-setup.exe" library list

# 管理员
& "C:\Program Files\IsoBox\isobox-setup.exe" add <英文名> <安装包> --library D --display <显示名>
& "C:\Program Files\IsoBox\isobox-setup.exe" move-app <英文名> --to D
& "C:\Program Files\IsoBox\isobox-setup.exe" protect add D:\
& "C:\Program Files\IsoBox\isobox-setup.exe" uninstall --apply

# 在浏览器里预览界面（不用编译 Tauri）
& "C:\Program Files\IsoBox\isobox.exe" status --json --icons   # 把输出按 gui/ui-dev/mock.js 的格式存成 gui/ui-dev/data.js
node gui/ui-dev/serve.js                                        # 打开 http://localhost:5174/ui-dev/
```

`gui/ui-dev/data.js` 含本机账户 SID 和路径，已在 `.gitignore` 里，不要提交。
