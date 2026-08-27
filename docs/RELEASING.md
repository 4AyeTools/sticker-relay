# 发布、签名与更新

## 1. GitHub 仓库准备

首次公开和每次发布前，都应人工确认 `git status` 中没有
`src-tauri/resources/lark-cli.exe`、`target/`、真实表情、日志、Cookie、Token、证书或
私钥。推荐开启：

- GitHub Actions；
- Branch protection，要求 `CI` 通过；
- GitHub Security Advisories；
- Dependabot；
- Release provenance/attestation（可作为下一步增强）。

## 2. 发布流程

1. 同步 `package.json`、`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json` 版本；
2. 更新 `CHANGELOG.md`；
3. 合并并等待 CI；
4. 创建 `vX.Y.Z` tag 并推送；
5. `Release` workflow 构建 Windows NSIS、Windows Portable ZIP、macOS Intel/Apple
   Silicon App/DMG；
6. workflow 汇总 SHA-256 和 SPDX JSON SBOM，再创建 GitHub Release。

Windows 使用 `src-tauri/nsis/installer.nsi` 自定义模板，将全新安装目录固定为英文
`StickerRelay`，同时保留中文产品展示名。升级 `@tauri-apps/cli` 时应对照对应版本的
Tauri 上游 NSIS 模板同步差异，再重新验证安装、升级和卸载流程。

手动运行 `workflow_dispatch` 只生成预览 Artifact，不创建正式 Release。

## 3. Windows Authenticode（可选）

将代码签名证书 PFX 转为 base64 后配置为 GitHub Actions Secrets：

- `WINDOWS_CERTIFICATE_BASE64`
- `WINDOWS_CERTIFICATE_PASSWORD`

流水线只在 runner 临时目录解码证书，将其导入当前用户证书库，分别签名主程序和 NSIS
安装包，然后移除证书。不要把 PFX、密码或 thumbprint 写入仓库。

没有证书时仍会产出未签名版本，Windows SmartScreen 可能显示陌生发布者警告。

## 4. macOS Developer ID 与 notarization（可选）

按 Tauri 官方 GitHub Actions 约定配置：

- `APPLE_CERTIFICATE`：Developer ID Application 证书的 base64；
- `APPLE_CERTIFICATE_PASSWORD`；
- `APPLE_SIGNING_IDENTITY`；
- `APPLE_ID`；
- `APPLE_PASSWORD`：Apple app-specific password；
- `APPLE_TEAM_ID`。

配置后 `tauri-apps/tauri-action` 会进行签名和 notarization。没有这些 Secrets 时，CI
仍可构建未签名 App/DMG，用于开源测试，但首次打开会受到 Gatekeeper 限制。

## 5. Tauri updater（已提前铺设，可选启用）

`src-tauri/tauri.updater.conf.json` 用于在存在 updater 签名私钥时生成更新包和 `.sig`：

- `TAURI_SIGNING_PRIVATE_KEY`
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`

私钥必须离线生成并只存 GitHub Secrets；公钥可以公开。当前仓库地址和最终发布域名尚未
确定，所以应用内 updater 插件与 endpoint 默认没有启用，避免编译进错误或占位 URL。
仓库创建后再完成以下步骤：

1. 保存 updater 公钥；
2. 添加 `tauri-plugin-updater`；
3. 把 endpoint 指向本仓库 Release 的 `latest.json`；
4. 在 UI 中增加“检查应用更新”，明确展示版本、发行说明和重启行为；
5. 让 Release workflow 汇总各平台签名并生成 `latest.json`；
6. 先在预发布通道验证回滚与坏包处理，再对稳定版开放。

飞书 CLI 更新与应用更新相互独立：CLI 使用飞书官方版本和 SHA-256 机制，不依赖表情递
Release，也不会使用 Tauri updater 私钥。

## 6. SBOM 与校验

每个平台构建都会用 Syft 生成 SPDX JSON，最终 `SHA256SUMS.txt` 覆盖所有发布文件。
用户可在 PowerShell 中验证：

```powershell
Get-FileHash .\sticker-relay-0.3.0-windows-x64-setup.exe -Algorithm SHA256
```

macOS：

```bash
shasum -a 256 sticker-relay-0.3.0-macos-apple-silicon.dmg
```

结果应与 Release 中 `SHA256SUMS.txt` 完全一致。
