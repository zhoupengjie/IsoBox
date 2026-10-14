# IsoBox

让不信任的 Windows 桌面软件以独立的本地账户运行：读不到你的文件，窗口照常显示在你的桌面上，麦克风和摄像头正常可用。不装驱动，只用 Windows 自带的账户和 NTFS 权限，卸载一条命令清干净。

起因是想用微信、QQ、网盘，又不想让它们扫描整个硬盘。微信 4 和 QQNT 已实测可用。

## 状态

开发中，还没有正式发布。阶段一（核心）、阶段二（图形界面）和阶段三（安装程序）已完成，下一步是发布：CI、许可证、代码签名。详见 [ROADMAP.md](ROADMAP.md)，接手开发请先读 [HANDOFF.md](HANDOFF.md)。

## 它能防住什么

| 能防住 | 防不住 |
|---|---|
| 读取你的用户目录（桌面、文档、AppData） | 同一个桌面上的截屏、键盘记录、读取剪贴板 |
| 读写受保护的硬盘和文件夹 | 插着的 U 盘、存储卡（FAT32、exFAT 没有权限控制） |
| 写入库和交换文件夹以外的位置 | 强制要求管理员权限的安装包 |
| 安装服务和驱动、绕过 IsoBox 开机自启 | 已经拿到管理员权限的恶意程序 |

## 从源码运行

需要 Windows 11、VS 2022 生成工具（C++ 桌面开发和 Windows 11 SDK）、Rust、7-Zip。

```powershell
cargo test --workspace
powershell -ExecutionPolicy Bypass -File installer\build.ps1   # 生成 target\installer\IsoBox-<版本>-setup.exe
```

运行生成的安装包：它会把 IsoBox 装到 `C:\Program Files\IsoBox`，创建隔离账户，并让你选择要保护的硬盘。然后从开始菜单打开 IsoBox，点“添加软件”。卸载在 Windows 设置的“应用”里，可以选择保留聊天记录。

许可证尚未确定。
