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
6. workflow 生成三平台 updater 包、`.sig` 和 `latest.json`；
7. workflow 汇总 SHA-256 和 SPDX JSON SBOM，再创建 GitHub Release。

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

## 5. Tauri updater

`src-tauri/tauri.updater.conf.json` 用于在存在 updater 签名私钥时生成更新包和 `.sig`：

- `TAURI_SIGNING_PRIVATE_KEY`
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`

私钥必须离线备份并只存 GitHub Secrets；公钥已经写入 `tauri.conf.json`。应用通过本仓库
最新 Release 的 `latest.json` 检查更新，签名验证不可关闭。当前密钥的本机备份位置为
`%USERPROFILE%\.tauri\sticker-relay-updater.key`，不得提交、发送或写入日志；如果密钥
永久丢失，已安装版本将无法验证由新密钥签发的更新。

发布前必须确认：

1. `TAURI_SIGNING_PRIVATE_KEY` Secret 存在；有密码时同时配置
   `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`；
2. Windows 产出自定义命名的 `setup.exe` 和对应 `.sig`；
3. 两种 macOS 架构分别产出 `.app.tar.gz` 和对应 `.sig`；
4. `latest.json` 包含 `windows-x86_64`、`darwin-x86_64`、`darwin-aarch64`；
5. 手动 `workflow_dispatch` 预览构建通过后再推送正式 tag；
6. 正式 Release 中 `latest.json`、更新包和签名都已上传。

飞书 CLI 更新与应用更新相互独立：CLI 使用飞书官方版本和 SHA-256 机制，不依赖表情递
Release，也不会使用 Tauri updater 私钥。

## 6. SBOM 与校验

每个平台构建都会用 Syft 生成 SPDX JSON，最终 `SHA256SUMS.txt` 覆盖所有发布文件。
用户可在 PowerShell 中验证：

```powershell
Get-FileHash .\sticker-relay-0.4.0-windows-x64-setup.exe -Algorithm SHA256
```

macOS：

```bash
shasum -a 256 sticker-relay-0.4.0-macos-apple-silicon.dmg
```

结果应与 Release 中 `SHA256SUMS.txt` 完全一致。
