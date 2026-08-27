# 参与贡献

感谢帮助改进表情递。提交代码前，请先确认问题可以在最新代码复现，并避免提交任何
真实 Cookie、Token、二维码、聊天内容或私人表情。

## 开发环境

通用依赖：

- Node.js 20+
- Rust stable
- Tauri 2 所需系统依赖

Windows 还需要 Visual Studio 2022 Build Tools、Windows SDK 和 WebView2 Runtime。
macOS 需要当前受支持的 Xcode Command Line Tools。

```bash
npm ci
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
npm run tauri dev
```

飞书 CLI 不应提交到 Git。开发机可把官方二进制放到
`src-tauri/resources/lark-cli.exe`（Windows）或 `src-tauri/resources/lark-cli`
（macOS），也可以直接在应用内点击下载。两者都已被 `.gitignore` 排除。

## 提交要求

- 一个 Pull Request 聚焦一个可审查的问题；
- UI 变更请附截图，网络或 Rust 变更请说明失败路径；
- 新增 Tauri 权限时必须解释必要性，避免恢复 `core:default` 或 `opener:default`；
- 新增下载源必须使用 HTTPS、域名白名单和内容完整性校验；
- Windows 与 macOS 分支逻辑应分别有测试或 CI 证据；
- 运行 `npm run build`、`cargo fmt --check`、`cargo clippy` 和 `cargo test`。

贡献默认按项目的 Apache-2.0 许可证授权。第三方代码必须保留原许可证与署名。
