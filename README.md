# PCL Region Unlock

解除 PCL CE 在受限地区对「Minecraft 下载、补全与整合包安装」的封锁。不修改启动器源码，不需要重新编译启动器。

> **使用范围**：本工具仅用于恢复因区域限制而受影响的正版用户下载功能，不提供、不鼓励任何盗版或破解行为。

## 工作原理

启动器在受限地区会禁用下载相关功能。本插件是一个 `dwmapi.dll` 代理，随启动器加载后把地区判定读取的 Windows 信号改写为其他地区，使判定放行。

只影响地区判定，不改动启动器文件，也不改动下载源与更新逻辑。

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

## 许可证

本项目使用 [GNU General Public License v3.0](LICENSE) 授权。
