# 第三方软件声明

## 飞书官方 CLI

表情递在用户主动点击“下载官方组件”时，按当前操作系统和 CPU 架构下载
[larksuite/cli](https://github.com/larksuite/cli) 的官方 Release。该二进制不会提交到
本仓库，也不再打进默认安装包。下载后必须与官方 npm 包中的 `checksums.txt` 完成
SHA-256 比对，校验不一致时不会安装或执行。

- 软件：`lark-cli`
- 当前已验证版本：`1.0.90`
- 版权所有：Copyright (c) 2026 Lark Technologies Pte. Ltd.
- 许可证：MIT
- 官方源码：https://github.com/larksuite/cli
- npm 包：https://www.npmjs.com/package/@larksuite/cli

MIT License：

> Permission is hereby granted, free of charge, to any person obtaining a copy
> of this software and associated documentation files (the "Software"), to deal
> in the Software without restriction, including without limitation the rights
> to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
> copies of the Software, and to permit persons to whom the Software is
> furnished to do so, subject to the following conditions:
>
> The above copyright notice and this permission notice shall be included in all
> copies or substantial portions of the Software.
>
> THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
> IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
> FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
> AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
> LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
> OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
> SOFTWARE.

## JavaScript 与 Rust 依赖

前端和 Rust 后端还使用了 `package-lock.json` 与 `src-tauri/Cargo.lock` 锁定的开源
依赖。每个 Release 会附带 SPDX JSON 格式的 SBOM，作为准确到具体版本的依赖及
许可证清单。依赖仍分别受其原许可证约束；表情递的 Apache-2.0 许可证不会替代这些
第三方许可证。

## Tauri NSIS 安装器模板

Windows 自定义安装器模板基于 Tauri CLI v2.11.4 的 `installer.nsi` 修改，用于将磁盘
安装目录与中文产品展示名称分离。

- 上游项目：https://github.com/tauri-apps/tauri
- 上游版本：`tauri-cli-v2.11.4`
- 许可证：Apache-2.0 OR MIT
- 修改内容：默认安装目录名改为 `StickerRelay`，并在目录选择页恢复旧版安装路径。
