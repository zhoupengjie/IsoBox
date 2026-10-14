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
| 三、安装程序 | NSIS 图形和静默安装、保护设置页、捆绑 7-Zip、系统卸载入口、卸载时保留聊天记录 | 完成 |
| 四、发布 | CI、开源许可证、代码签名、文档、试用 | **下一步** |

测试现状：单元测试 16 个（`cargo test --workspace`），Windows 沙盒端到端测试 112 项（从运行安装包开始，到运行卸载程序结束），全部通过。另有界面巡检：在沙盒里用默认选择点完安装和卸载的每一页并截图（`run.ps1 -UiTour`）。

在原开发机上用旧的 deploy.ps1 部署并日常使用：系统盘的库里装着微信和 QQ，D 盘受保护。**还没在真机上实测的**：

- 安装包本身：真机有 UAC，沙盒里没有。重点看完成页“打开 IsoBox”是否以普通权限启动界面（任务管理器里 isobox-gui.exe 的“提升”列应为“否”）
- 在原开发机上从 deploy.ps1 部署的版本升级：应识别为已安装（只更新程序文件），删掉旧的 add-app.ps1、remove-app.ps1 和开始菜单里的“IsoBox 添加软件”“IsoBox 移除软件”
- 界面里点“卸载”打开卸载程序；静默更新时托盘关闭后能否重新打开
- 从界面发起的管理员操作（UAC 路径）、托盘图标的显示（阶段二遗留）

## 3. 新机器环境搭建

需要 Windows 11 专业版、企业版或专业工作站版（家庭版能编译和运行，但没有 Windows 沙盒，跑不了端到端测试）。

1. **Visual Studio 2022 生成工具**，勾选“使用 C++ 的桌面开发”，并确认包含 **Windows 11 SDK**（缺 SDK 时链接会报找不到 kernel32.lib）：
   ```powershell
   winget install Microsoft.VisualStudio.2022.BuildTools --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
   ```
2. **Rust**（MSVC 工具链，项目用 edition 2024）：`winget install Rustlang.Rustup`
3. **7-Zip**，装在默认位置 `C:\Program Files\7-Zip`：`winget install 7zip.7zip`。打包时把它的 7z.exe、7z.dll 捆绑进安装包，测试时用它做测试用的安装包
4. **Windows 沙盒**：在“启用或关闭 Windows 功能”里勾选 Windows 沙盒，重启
5. **NSIS 不用装**：`installer\build.ps1` 找不到时自动下载 3.13 绿色版到 `target\tools` 并核对 SHA-256
6. 可选：**Node.js**（只用于在浏览器里预览界面）、**gh**（GitHub 命令行）

然后：

```powershell
git clone https://github.com/zhoupengjie/IsoBox.git
cd IsoBox
cargo build --release --workspace     # 第一次要下载编译 Tauri，几分钟
cargo test --workspace
powershell -ExecutionPolicy Bypass -File installer\build.ps1      # 生成 target\installer\IsoBox-<版本>-setup.exe
powershell -ExecutionPolicy Bypass -File tests\sandbox\run.ps1    # 端到端测试，编译好之后约 3 分钟，退出码 0 表示全部通过
```

在本机安装：双击 `target\installer\IsoBox-<版本>-setup.exe`，或在管理员终端里静默安装：

```powershell
.\target\installer\IsoBox-0.1.0-setup.exe /S                       # 自动保护所有内置非系统盘
.\target\installer\IsoBox-0.1.0-setup.exe /S /PROTECT=D:\;E:\ /EXCHANGE=D:\Swap
```

已经装过的再运行一次就是更新：保留配置、库和软件，不重新建账户。之后从开始菜单的 IsoBox 打开界面。

## 4. 代码结构

Cargo 工作区，两个包：根目录的 `isobox`（核心库和三个命令行程序）、`gui/` 的 `isobox-gui`（Tauri）。安装程序是独立的 NSIS 脚本。

| 位置 | 作用 |
|---|---|
| `src/bin/setup.rs` → `isobox-setup.exe` | 管理员工具：install / uninstall（默认预演，`--apply` 执行）、library、add、remove-app、move-app、migrate、protect、set-exchange、backup-acl、restore-acl、drives。install 可选 `--protect`/`--no-protect`/`--exchange`，可重复运行；uninstall 可选 `--keep-data`；隐藏命令 `installer-info` 给安装程序用。全局选项 `--log <文件>`、`--non-interactive` 供界面隐藏调用，`--utf16` 供安装程序显示输出 |
| `src/bin/launch.rs` → `isobox.exe` | 普通权限命令行：run、list、status --json、preview、shortcut、theme-sync 等 |
| `src/bin/gui_launch.rs` → `isobox-launch.exe` | 快捷方式背后的无窗口启动器；`--watch-theme` 是主题监视进程 |
| `gui/` → `isobox-gui.exe` | 图形界面和托盘。`gui/ui/` 是纯 HTML/CSS/JS（无打包工具），`gui/src/elevate.rs` 负责 UAC 调用和启动卸载程序 |
| `installer/isobox.nsi` | 安装和卸载程序（NSIS，中文界面）。文件头注释列出了所有命令行参数和退出码 |
| `installer/build.ps1` | 生成安装包：编译、准备文件、捆绑 7-Zip、调用 makensis；`-Preview` 另外生成只看界面、不改系统的预览版 |
| `installer/notice.txt` | 安装向导“使用前须知”页的内容 |
| `src/status.rs` | 汇总所有状态（界面和 `status --json` 共用） |
| `src/apps.rs` | 添加、移除、移动软件 |
| `src/library.rs` | 库：`<盘>:\Program Files\IsoBox`，标记文件 `library.toml`，`apps\` 目录 |
| `src/acl.rs` | 锁目录、读写 SDDL、检查禁止规则、备份还原 |
| `src/drives.rs` | 识别盘：固定/可移动/网络、文件系统、是否 USB |
| `src/unpack.rs` | 7-Zip 拆包、安装包类型识别、安装前预览；`sevenzip()` 优先用安装目录里捆绑的 7-Zip |
| `src/cmd.rs` | 运行 reg、icacls、robocopy 等系统命令，把它们的 GBK 输出转成 UTF-8 再打印 |
| `src/launch.rs` | 以隔离账户身份启动（CreateProcessWithLogonW） |
| `src/theme.rs` | 深浅色和强调色同步、事件驱动的监视 |
| `src/shortcut.rs` | 快捷方式（PowerShell 写、COM 读） |
| `src/config.rs` | 配置；修改配置时保留注释，写入前先解析验证；`salvage()` 在配置损坏时尽量读出账户名，保证还能卸载 |
| `src/appcontainer.rs` | AppContainer 模式（保留，但微信、QQ 跑不了，见第 6 节） |
| `tests/sandbox/` | 沙盒测试：`run.ps1` 在宿主机运行；`run-in-sandbox.ps1` 是端到端测试，`ui-tour.ps1` 是界面巡检（`-UiTour`），都在沙盒里执行 |
| `examples/probe.rs` | 测试用探测程序：以隔离账户身份尝试各种读写 |
| `isobox.toml` | 配置模板。实际生效的是安装目录里那份 |

阶段二的 `deploy.ps1` 和 `tools\add-app.ps1`、`tools\remove-app.ps1` 已删除，分别被安装程序和界面取代。

没有用 Tauri 自带的 NSIS 打包（`gui/tauri.conf.json` 的 `bundle.active` 仍是 false），因为它满足不了这几点：不显示目录选择、固定装到 Program Files\IsoBox；安装向导里的保护设置页；更新时绝不能运行旧版的卸载（Tauri 的模板升级时会先卸载旧版，对 IsoBox 来说等于删掉隔离账户和聊天记录）；卸载时只删自己的文件。自己的脚本更短，也更好测。

版本号写在三处，发布时一起改：`Cargo.toml`、`gui/Cargo.toml`、`gui/tauri.conf.json`。安装包的版本取自根目录的 `Cargo.toml`。

## 5. 不要破坏的设计

- **安装位置固定**为 `%ProgramFiles%\IsoBox`，只有管理员能写（isobox-setup 以管理员身份运行，目录可写就等于提权漏洞）。会改系统的命令通过 `paths::require_installed()` 检查；开发时设 `ISOBOX_DEV=1` 跳过。安装程序忽略 `/D`，不显示目录选择。
- **程序在哪个目录就把那里当安装目录**（`paths::install_dir()` = exe 所在目录），所以四个 exe 和 `isobox.toml` 必须在同一个目录，捆绑的 7-Zip 在它下面的 `7-Zip\`。
- **安装程序**：安装目录里有不认识的文件就拒绝安装（退出码 2），因为接下来要锁定这个目录的权限；先锁权限再放文件；`isobox.toml` 已存在时绝不覆盖；隔离账户已经能用（`installer-info` 报 `account=ready`）就只更新文件，不再初始化。更新时只结束 isobox-gui 和 isobox-launch，隔离软件照常运行。
- **安装程序以管理员身份运行，界面不能跟着提权**：完成页通过 `explorer.exe` 启动界面，让它以普通权限运行。
- **卸载程序的顺序**：先运行 `isobox-setup uninstall --apply`，成功后才删程序文件。失败时默认停下、保留 IsoBox（退出码 3），用户确认后才强制删除，这时保留 `backup\`。只删安装程序放进来的文件，**不能整个目录删**，因为系统盘的库就在安装目录里。
- **保留聊天记录**（`uninstall --keep-data`，卸载程序默认勾选，静默卸载默认保留，`/DELETEDATA` 才删）：在撤销交换文件夹授权之后、删除个人文件夹之前，把隔离账户的文档、桌面、下载、图片、视频、音乐里的文件移到交换文件夹的“隔离账户的文件”，再重设权限和所有者。这一步失败时不再继续删除个人文件夹和账户。
- **install 可以重复运行**：账户已存在（且备注对、密码在当前账户的凭据管理器里）就跳过创建；已有禁止规则的位置不再备份，因为那时的备份里已经带着禁止规则，用它还原不掉。
- **安装和日常使用必须是同一个管理员账户**：隔离账户的密码、快捷方式、交换文件夹都跟着运行 isobox-setup 的账户走。`install` 发现运行者和桌面上登录的不是同一个人（标准用户在 UAC 里输了别的管理员密码）就拒绝。
- **交换文件夹**是显式允许，排在继承来的禁止之前，所以放在受保护的盘里也能用。它不能是整盘、用户目录、系统目录或它们的上级，也不能在安装目录里（`check_exchange_dir`）。
- **库的权限**：根目录关闭继承，管理员和 SYSTEM 完全控制、Users 只读；`apps\` 再加隔离账户修改权限（QQ 要写自己的 versions 目录）。根目录关闭继承也保证硬盘根上的禁止规则不会传进库。
- **隔离账户**：名字来自配置（默认 `isobox`），备注固定为 `isobox sandbox account`，卸载只认这个备注，并拒绝当前登录账户的 SID。随机 32 位密码存在当前用户的凭据管理器，目标名 `isobox:<账户名>`。
- **保护**：在盘根或文件夹上加 `(OI)(CI)F` 的禁止规则，加之前先备份权限。卸载时扫描所有盘根和配置里的位置，凡是有该 SID 显式禁止规则的都撤销。**顺序：先撤销权限、删配置文件，最后才删账户**，否则留下“未知 SID”。配置文件损坏时卸载照样进行（`Config::salvage`）。
- **界面永远以普通权限运行**；管理员操作用 `ShellExecuteExW("runas")` 隐藏调用 `isobox-setup --log <临时文件> --non-interactive ...`，结束后读日志显示。装了安装程序时，“卸载”按钮打开系统的卸载程序，然后界面退出。
- **isobox-setup 的输出都是 UTF-8**：系统命令一律经 `cmd::run_print` 收集转码，不能让子进程直接继承标准输出（会混进 GBK，`--utf16` 模式下还会被当成 UTF-8 解码）。
- **主题同步**：把当前用户 HKCU 里的主题值以隔离账户身份用 reg.exe 写过去，再广播 `WM_SETTINGCHANGE("ImmersiveColorSet")`。监视用 `RegNotifyChangeKeyValue` 事件驱动，命名互斥量 `Local\IsoBoxThemeWatcher` 保证只有一个；托盘运行时由托盘线程负责。
- **静态链接 C 运行库**（`.cargo/config.toml`）：不然在没装 VC++ 运行库的电脑上打不开，沙盒测试就是这样发现的。`cargo` 按当前目录找这个配置，所以要在项目目录里编译。

## 6. 踩过的坑

- **Claude 桌面应用的包虚拟化**：从 Claude 桌面应用的会话里启动的进程继承它的 MSIX 包环境，写 `HKCU\Software\Classes` 和 AppData 会被重定向到它自己的目录。测试要通过 WMI（`Invoke-CimMethod Win32_Process -MethodName Create`）在包外启动，沙盒测试脚本已经这样做。
- **AppContainer 跑不了 Chromium 系软件**（微信 4、QQNT）：容器里不能创建全局命名管道，Crashpad 和 Chromium 的进程通信都依赖它。所以默认用账户模式。
- **Windows 沙盒方案被否决的原因**：每次会话的内置用户密码随机，DPAPI 主密钥解不开，登录凭据丢失；网卡 MAC 每次变化且无法固定（Hyper-V 防 MAC 欺骗），微信每次都要短信验证。实验发现自建固定密码用户的 SID 每次相同，可以绕过 DPAPI 问题，但 MAC 问题无解。
- **icacls 把完全禁止显示为 `(N)`**，不是 `DENY`；继承来的显示为 `(I)(OI)(CI)(N)`。
- **PowerShell 文本匹配默认不区分大小写**：路径里有 IsoBox 时，匹配 `isobox` 会误中，要匹配 `\isobox:`。
- **CreateProcessWithLogonW 的命令行最长 1024 字符**，所以主题同步分批执行。
- **Rust 的 `std::process::Command` 会让子进程继承所有可继承句柄**，常驻的子进程会占住调用者的输出管道。拉起监视进程时改用 `CreateProcessW` 并关闭继承。
- **写入相同的注册表值不会触发** `RegNotifyChangeKeyValue`。
- **Windows PowerShell 5.1 把无 BOM 的 UTF-8 当 ANSI 读**：含中文的 `.ps1`（`installer\build.ps1`）必须存为 UTF-8 带 BOM；测试脚本保持纯 ASCII。读文件也一样：`Get-Content` 读配置要加 `-Encoding UTF8`，否则中文注释被读成乱码，写回去时会吞掉换行，把下一行 `sandbox_user = ...` 变成注释（沙盒测试的迁移步骤就这样坏过）。
- **NSIS 和中文**：`FileRead` 只认 ANSI，读 isobox-setup 的输出要用 UTF-16LE 文件加 `FileReadUTF16LE`（会跳过 BOM）；`nsExec::ExecToLog` 能识别带 BOM 的 UTF-16LE 输出，所以 isobox-setup 有 `--utf16`。脚本本身是 UTF-8，`build.ps1` 传 `/INPUTCHARSET UTF8`。
- **NSIS 变量名会吞掉后面的字母**：nsDialogs 坐标写 `"$2u"` 会被当成变量 `$2u`，要写成 `"$2$Unit"`（`$Unit` 是字符串 u）。
- **NSIS 卸载程序会把自己复制到 %TEMP% 再运行**（NSIS 3.13 的进程名是 `Un`），原进程立即退出。测试里要等这个进程结束。
- **自动点 NSIS 的按钮**：UI 自动化的 InvokePattern 和 `BM_CLICK` 在自定义页上都不可靠（保护设置页点了没反应，离开函数根本没被调用）。界面巡检改为给向导窗口发 `WM_COMMAND`（wParam 是按钮 ID：1 下一步、2 取消、3 上一步）。
- **调试 NSIS 页面**：`installer\build.ps1 -Preview` 生成的预览版以普通权限运行、不改系统，可以直接在宿主机上点；要看离开函数走到哪一步，临时在脚本里用 `FileWriteUTF16LE` 往 `$TEMP` 写日志。
- **沙盒测试运行时别在宿主机上读 `result.txt`**：读的同时沙盒里的 `Add-Content` 会失败，结果行丢失。等 `done.flag` 出现再看。
- **`net user` 遇到超过 14 个字符的密码会交互确认**，脚本里用 `NetUserAdd` 或 `New-LocalUser`。
- **删除正被某个进程当作当前目录的文件夹会失败**（os error 32），迁移时遇到过，现已改为只提示。
- **7-Zip 制作自解压包时**，`-sfx` 要给模块的完整路径。
- **PowerShell 里 `R` 是 Invoke-History 的别名**，别把辅助函数命名为 R。
- **Windows 沙盒同时只能开一个**，测试前要关掉其他沙盒。

## 7. 下一步：阶段四（发布）

1. **先在真机上走一遍安装包**（见第 2 节“还没在真机上实测的”），尤其是原开发机从 deploy.ps1 版本升级。
2. **CI**（GitHub Actions）：编译、单元测试、生成安装包并作为构建产物上传。Windows 运行器不支持 Windows 沙盒，但它本身有管理员权限，可以直接在运行器上跑 `IsoBox-setup.exe /S` → 检查 → `uninstall.exe /S`，大部分 `run-in-sandbox.ps1` 的检查可以复用（运行器是一次性的）。运行器上没有 7-Zip 就 `choco install 7zip` 或 `winget`。
3. **开源许可证**：捆绑的 7-Zip 是 LGPL（7z.dll，unRAR 部分另有限制），安装包里已附 7-Zip 的 License.txt；IsoBox 自己的许可证待定，README 也要更新。
4. **代码签名**：优先申请 SignPath Foundation（开源项目免费）。要签的是四个 exe、安装包和卸载程序（卸载程序由 makensis 生成，签名需要用 `!uninstfinalize`）。
5. **说明文档**：能防住什么、防不住什么（安装向导的须知页 `installer/notice.txt` 可以作为底稿）、常见问题。
6. 找几位朋友在不同电脑上试用：Windows 10、只有一个盘、下载文件夹被移到别的盘、有外接硬盘等情况。

## 8. 常用命令

```powershell
# 打包和测试
powershell -ExecutionPolicy Bypass -File installer\build.ps1 [-NoBuild] [-Preview]
powershell -ExecutionPolicy Bypass -File tests\sandbox\run.ps1             # 端到端测试
powershell -ExecutionPolicy Bypass -File tests\sandbox\run.ps1 -UiTour     # 截取安装、卸载每一页，存到 target\sandbox-test\logs\shots

# 安装包参数（管理员）
IsoBox-0.1.0-setup.exe /S [/PROTECT=D:\;E:\ | /PROTECT=none] [/EXCHANGE=<文件夹>] [/NOINIT]
"C:\Program Files\IsoBox\uninstall.exe" /S [/DELETEDATA]

# 状态（普通权限）
& "C:\Program Files\IsoBox\isobox.exe" status
& "C:\Program Files\IsoBox\isobox.exe" preview <安装包>          # 类型、能否拆包、主程序候选
& "C:\Program Files\IsoBox\isobox-setup.exe" drives
& "C:\Program Files\IsoBox\isobox-setup.exe" library list

# 管理员
& "C:\Program Files\IsoBox\isobox-setup.exe" add <英文名> <安装包> --library D --display <显示名>
& "C:\Program Files\IsoBox\isobox-setup.exe" move-app <英文名> --to D
& "C:\Program Files\IsoBox\isobox-setup.exe" protect add D:\
& "C:\Program Files\IsoBox\isobox-setup.exe" install --apply                 # 补完中途失败的安装
& "C:\Program Files\IsoBox\isobox-setup.exe" uninstall --apply --keep-data   # 只撤销系统修改，不删程序文件

# 在浏览器里预览界面（不用编译 Tauri）
& "C:\Program Files\IsoBox\isobox.exe" status --json --icons   # 把输出按 gui/ui-dev/mock.js 的格式存成 gui/ui-dev/data.js
node gui/ui-dev/serve.js                                        # 打开 http://localhost:5174/ui-dev/
```

`gui/ui-dev/data.js` 含本机账户 SID 和路径，已在 `.gitignore` 里，不要提交。
