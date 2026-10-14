# IsoBox

让不信任的 Windows 桌面软件以独立的本地账户运行：读不到你的文件，窗口照常显示在你的桌面上，麦克风和摄像头正常可用。不装驱动，只用 Windows 自带的账户和 NTFS 权限，卸载时全部撤销。

起因是想用微信、QQ、网盘，又不想让它们扫描整个硬盘。微信 4 和 QQNT 已实测可用。

## 状态

开发中，还没有正式发布。核心、图形界面和安装程序已完成，正在准备发布：CI、代码签名、试用。详见 [ROADMAP.md](ROADMAP.md)，接手开发请先读 [HANDOFF.md](HANDOFF.md)。

## 安装和使用

需要 Windows 10 或 11（64 位），用你日常的管理员账户安装。

1. 从 [Releases](https://github.com/zhoupengjie/IsoBox/releases) 下载 `IsoBox-<版本>-setup.exe` 并运行（发布前可以从 CI 的构建产物里下载）
2. 按向导选择要保护的硬盘和交换文件夹
3. 打开 IsoBox，点“添加软件”，选择微信、QQ 等软件的官方安装包

卸载在 Windows 设置的“应用”里，可以选择保留聊天记录。详细说明见 [使用说明](docs/user-guide.md)。

## 它能防住什么

| 能防住 | 防不住 |
|---|---|
| 读取你的用户目录（桌面、文档、AppData） | 同一个桌面上的截屏、键盘记录、读写剪贴板等界面层攻击 |
| 读写受保护的硬盘和文件夹 | 没有保护的位置，例如你在 C 盘根目录下自己建的文件夹 |
| 写入库和交换文件夹以外的位置 | 插着的 U 盘、存储卡（FAT32、exFAT 没有权限控制） |
| 安装服务和驱动、绕过 IsoBox 开机自启 | 网络访问；强制要求管理员权限的安装包；已经拿到管理员权限的恶意程序 |

它针对的是会窥探文件的普通软件，不是专门针对你的恶意程序。完整的说明和安全问题的报告方式见 [SECURITY.md](SECURITY.md)。

## 从源码构建

需要 VS 2022 生成工具（C++ 桌面开发和 Windows 11 SDK）、Rust、7-Zip。

```powershell
cargo test --workspace
powershell -ExecutionPolicy Bypass -File installer\build.ps1   # 生成 target\installer\IsoBox-<版本>-setup.exe
```

端到端测试在 Windows 沙盒里运行：`powershell -ExecutionPolicy Bypass -File tests\sandbox\run.ps1`。

## 许可证

IsoBox 是自由软件，以 [GNU 通用公共许可证第 3 版](LICENSE)或（由你选择）任何更新的版本发布：可以自由使用、修改和再分发，但分发时（包括修改后的版本）必须同样以 GPL 提供源代码。IsoBox 不提供任何担保。

Copyright (C) 2026 Pengjie Zhou

安装包捆绑的 7-Zip 使用 GNU LGPL，编译进程序的 Rust 库的许可证见安装目录里的 `licenses\THIRD-PARTY-NOTICES.txt`。
