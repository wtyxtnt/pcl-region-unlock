# PCL Region Unlock

解除 PCL CE 在受限地区对「Minecraft 下载、补全与整合包安装」的封锁。不修改启动器源码，不需要重新编译启动器。

## 工作原理

PCL CE 通过 `PCL.Core/App/Region/RegionPolicy.cs` 判断所在地区，命中后禁用下载相关功能。判定读取四个 Windows 信号，本插件把这四个信号改写掉：

| 信号 | 来源 | 改写为 |
|---|---|---|
| 国家/地区代码 | `GetUserDefaultGeoName` | `US` |
| 设备区域 | 注册表 `Control Panel\DeviceRegion` | 不可读 |
| 用户区域 | `GetUserDefaultLocaleName` | `en-US` |
| 首选 UI 语言 | `GetUserPreferredUILanguages` | `en-US` |

四个信号改写后，`RegionPolicy.Evaluate` 走 `geo-other` 分支返回 `Other`，`IsRestricted` 为 `false`，下载、补全、整合包安装放行。

实现方式是 `dwmapi.dll` 代理：

- `dwmapi.dll` 不在 KnownDLLs 中，且被启动器以 `[LibraryImport("dwmapi.dll")]` 按名加载（`PCL.Core/Utils/OS/WindowInterop.cs`），窗口初始化时即触发，早于任何下载操作。
- 代理导出 `DwmIsCompositionEnabled`、`DwmExtendFrameIntoClientArea` 等函数，全部转发给 `System32` 的真实实现，窗口效果不受影响。
- 首次转发调用时安装四个钩子（函数入口写入绝对跳转）。不在 `DllMain` 中做加载器操作，避免锁死。
- 补丁不引用启动器的类型名、方法名、RVA 或 IL 偏移，因此启动器版本更新后无需适配。
- 加载时置位 `SEM_FAILCRITICALERRORS`，静音加载器的模态错误框。

只影响区域判定，不改动下载源、鉴权、更新逻辑，也不改动启动器文件。

## 支持的版本

**仅在 PCL CE 2.15.1 上实测通过。** 其他版本请自行测试，出问题请开 issue 并附上启动器版本号与 `%TEMP%\pcl-region-unlock.log`。

## 安装与卸载

把 `dwmapi.dll` 放到启动器 exe 所在目录即可。删除该文件即卸载，无注册表写入。

## 构建

需要 Rust（`x86_64-pc-windows-msvc`）：

```powershell
.\build.ps1                                   # 只构建
```

产物：`native\target\release\dwmapi.dll`

## AI 生成声明

本项目的部分代码由 AI 生成。


## 问题反馈

请提交 issue，附上：

- 启动器版本号
- 弹窗或报错的原文

## 注
- 本项目经测试无法在PCL使用 适用范围仅限于PCL CE。
