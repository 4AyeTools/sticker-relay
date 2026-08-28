# 表情递 StickerRelay

把自己收藏的微信表情采集到本地，再通过飞书官方 CLI 发给自己。发送完成后，在飞书中
打开图片并点击高亮的 **「＋ 添加表情」** 即可收藏。

表情递是本地优先的开源桌面工具：表情库、迁移状态和微信会话都保存在本机；项目
不提供中转服务器，也不收集遥测。

> [!IMPORTANT]
> 本项目不是微信、腾讯、飞书、字节跳动或 Lark 的官方项目。微信采集依赖网页版文件
> 传输助手的非公开接口，可能随时因官方调整、风控或账号状态而失效。请仅迁移自己有权
> 使用的内容，并遵守相关平台条款。

## 能做什么

- 扫码登录网页版微信文件传输助手，持续监听并采集用户重新发送的收藏表情；
- 图片仅落到本地表情库，按 SHA-256 去重，支持自定义目录、导出 ZIP 和批量删除；
- 关闭时可保留或退出微信登录，Windows 使用 DPAPI，macOS 使用 Keychain；
- 无需命令行，在界面中完成飞书授权、检查连接和批量发送；
- 飞书 CLI 按需下载、校验、安装和独立更新，不再塞进主安装包；
- 启动后自动检查表情递新版本，也可手动检查、查看更新说明并在应用内下载安装；
- Windows 安装版与便携版，macOS Intel/Apple Silicon 的 App/DMG 分发基础设施；
- Release 自动生成 updater 签名、`latest.json`、SHA-256 校验和与 SPDX JSON SBOM。

## 使用流程

1. 打开表情递，使用微信扫码。
2. 在手机微信中把“收藏的表情”发送给文件传输助手。
3. 首次使用飞书时，点击“下载官方组件”。应用会选择当前平台的官方 CLI，并验证
   官方 SHA-256。
4. 点击“连接飞书”，浏览器会打开飞书官方授权页。
5. 点击“发送待发送表情”，图片会通过飞书官方 CLI 发给当前账号。
6. 在飞书中打开图片，点击 **「＋ 添加表情」**。

飞书目前没有面向普通用户开放的“批量加入个人表情收藏”API，因此最后一步仍需在
飞书客户端中逐张确认。表情递自动化的是采集、去重、上传和发给自己。

## 下载与平台支持

| 平台 | 产物 | 状态 |
| --- | --- | --- |
| Windows x64 | NSIS 安装包、Portable ZIP | 主要支持 |
| Windows ARM64 | CI/组件逻辑已适配 | 待更多设备验证 |
| macOS Apple Silicon | `.app`、`.dmg` | Beta，CI 真机编译 |
| macOS Intel | `.app`、`.dmg` | Beta，CI 真机编译 |
| Linux | 暂不分发 | 微信安全存储尚未实现 |

macOS 在代码层面可以兼容：Tauri、React、HTTP 采集和飞书 CLI 都有 Darwin 构建，
微信会话也改用 Keychain。由于开发机是 Windows，macOS 的最终打包、签名与冒烟测试
由 GitHub Actions 的 macOS runner 完成；未签名构建首次打开时可能受到 Gatekeeper
提示，正式发布前建议配置 Apple Developer ID 和 notarization。

## 应用更新

表情递使用 Tauri 官方 Updater 检查
`https://github.com/4AyeTools/sticker-relay/releases/latest/download/latest.json`。启动后每
12 小时静默检查一次，标题区也提供手动检查入口。发现新版本后会展示发行说明、下载
进度和失败重试；检查 GitHub 时如果遇到瞬时连接失败，会自动重试并显示中文
错误说明。更新包通过独立的 Tauri 签名验证后才会安装。

下载安装前会先保存微信会话。更新应用不会清理本地表情库、迁移记录或独立安装的飞书
CLI。`0.3.1` 及更早版本没有内置 updater，因此需要手动安装 `0.4.0` 一次；之后才可在
应用内持续更新。Windows Authenticode 和 Apple Developer ID 仍属于可选的系统信任
签名，没有配置时可能继续出现 SmartScreen 或 Gatekeeper 提示。

## 飞书 CLI 组件

默认安装包不包含 `lark-cli`。应用会从 `@larksuite/cli` 的官方 npm 元数据获取最新
版本和校验清单，优先从 npm 国内镜像下载对应资产，失败后回退到 GitHub Release。
无论来源如何，最终文件都必须匹配同一份官方 SHA-256，否则不会安装。

下载域名是代码中的安全白名单，但版本号和文件名不是固定值：应用读取官方 npm 的
最新版本，再根据当前应用的编译目标（系统与 CPU 架构）动态选择 `windows-amd64`、
`windows-arm64`、`darwin-amd64` 或 `darwin-arm64`。当前不支持 32 位 x86 和 Linux
自动安装。

当前官方资产覆盖：

- Windows：`amd64`、`arm64`
- macOS：`darwin-amd64`、`darwin-arm64`

CLI 源码及许可证见 [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md)。本地旧版本内置
CLI 仍可被识别，点击“安全更新”后会迁移到独立的持久组件目录。Windows 默认位于
`%LOCALAPPDATA%\com.ayecode.wechatfeishustickers-components\lark-cli\`，不随应用数据
清理而删除；macOS 默认位于
`~/Library/Application Support/com.ayecode.wechatfeishustickers-components/lark-cli/`。

启动时按以下顺序查找 CLI：表情递管理的持久组件、旧版/开发版兼容路径、系统
`PATH`。找到非空文件后还会实际执行 `lark-cli --version`，只有命令成功才判定为已安装；
随后执行 `auth status --json` 单独判断飞书授权状态。因此“安装了 CLI”和“已经连接
飞书”是两个不同状态。

从 `lark-cli 1.0.92` 开始，首次连接还需要先完成一次应用配置。表情递会识别
`not_configured` 状态并自动打开官方配置页；配置完成后再自动进入账号授权页，后续
连接只需执行账号授权。

## 卸载与数据保留

- Windows 全新安装默认使用纯英文目录 `%LOCALAPPDATA%\StickerRelay\`；应用界面、快捷方式
  和卸载列表仍显示“表情递”。从旧版“咻咻搬”升级时保留用户原安装目录，避免强制移动；
- 普通卸载保留设置、微信登录、飞书组件和表情库，重新安装后可继续使用；
- Windows 卸载时勾选清理选项，会删除设置、微信登录和默认表情库；
- 已下载的飞书 CLI 组件会保留在独立目录，重新安装后可直接复用，无需再次下载；
- 用户自定义的表情库目录不会由卸载器删除，但重新安装后需要再次选择；
- 飞书 CLI 的用户级 `.lark-cli` 授权目录不会删除，以免影响其他飞书开发工具。

## 隐私与安全

- 完整隐私边界：[PRIVACY.md](PRIVACY.md)
- 漏洞报告方式：[SECURITY.md](SECURITY.md)
- 第三方许可证：[THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md)

项目采用严格 CSP，Tauri 前端仅有事件监听、窗口控制和必要的文件对话框权限。飞书
授权 URL 由 Rust 后端验证为官方 HTTPS 域名后再交给系统浏览器，前端没有任意 opener
权限。

## 本地开发

需要 Node.js 20+、Rust stable 和 Tauri 2 的系统依赖。

```bash
npm ci
npm run tauri dev
```

验证：

```bash
npm run build
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
```

Windows 打包：

```powershell
npm run tauri build -- --bundles nsis --features custom-protocol
```

macOS 打包：

```bash
npm run tauri build -- --bundles app,dmg --features custom-protocol
```

详细发布、签名和自动更新准备见 [docs/RELEASING.md](docs/RELEASING.md)。

## 技术结构

```text
React UI
  ├─ AppUpdater.tsx：检查、下载、签名验证、安装与重启
  └─ tauriBridge.ts：类型化 invoke/event
       └─ Rust Commands
            ├─ wechat.rs：扫码、会话、消息轮询、图片下载
            ├─ store.rs：本地表情库、去重、迁移、删除、导出
            ├─ feishu.rs：CLI 生命周期、OAuth、上传和发送
            └─ secure_storage.rs：DPAPI / macOS Keychain
```

后端使用 `Arc<tokio::sync::Mutex<_>>` 共享会话和存储状态。React 不直接接触 Cookie、
磁盘路径或子进程；所有高权限操作都由显式 Tauri Command 完成。

## 参与贡献与路线

开发规则见 [CONTRIBUTING.md](CONTRIBUTING.md)，变更记录见 [CHANGELOG.md](CHANGELOG.md)。
后续方向包括：飞书消息结果重试队列、表情格式/尺寸预检、导入恢复、跨设备清单、更多
目标平台适配，以及国内更新源与稳定版/预览版更新通道。

## 许可证

表情递代码使用 [Apache License 2.0](LICENSE)。第三方组件按各自许可证授权。项目名称
与图标不授予微信、飞书等第三方商标使用权。
