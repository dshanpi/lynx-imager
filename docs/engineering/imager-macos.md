# macOS Apple Silicon 构建与手动安装候选

本轮为 `0.1.0-rc.6` macOS 预发行候选。只适用于 **macOS 26.0+ Apple Silicon**；本机验证版本为 26.6.2，未验证 Intel Mac。
本轮 Homebrew 工具库的 Mach-O 最低系统为 26.0，因此包的 Info.plist 同步限制为 26.0；不会沿用误导性的默认 10.13。
如需支持 macOS 15 等旧系统，须重新构建所有对应依赖并实际验证，不能仅修改 Info.plist 或 Mach-O 版本号。
Apple Developer ID / 公证未配置，因此不得称为公证正式版。现有 Windows/Linux OTA 公钥及 `updates/latest.json` 不变。
新建的 macOS 手动分发密钥仅用于独立文件校验，不代替 Apple 签名，也不自动成为 OTA 信任根。
私钥存放在仓库外的本机发布配置目录，禁止提交、打包或上传。Release 仅附公钥与分离签名。

## 构建输入

- macOS arm64、Xcode Command Line Tools、Rust、Node.js、Python 3（仅构建脚本，应用不依赖 Python）。
- Homebrew：`libusb pkg-config dfu-util openssl@3 zstd tinyxml2`。
- OpenOCD：由 `npm run build` 按仓库锁定的 0.12.0-7 下载并校验。
- OpenixCLI：公开仓库 `100askTeam/OpenixCLI`，提交 `4f869200b72d8fecbf6d420326495ff6baa42810`。
- OpenRockCLI：公开仓库 `dshanpi/openrockcli`，提交 `c081a0987c0b4dde02fe104894c7c3d8989b9368`。
- UUU：`nxp-imx/mfgtools` 的 `uuu_1.5.243/uuu_mac_arm`；SHA-256 `7f2ce10d5e738af635b339696709e2c3df4984801cc6c3764cd1c207c7aac9b0`。同标签的 LICENSE 一起保存。

在 `.verification/openixcli` 构建 `cargo build --release --locked`；在 `.verification/openrockcli` 构建
`make CC=clang CFLAGS='-O2 -Wall -include sys/time.h'` 并运行相同参数的 `make test`。
该头文件参数补足当前上游 macOS 的 gettimeofday 声明，不改写来源代码。
UUU 文件和许可证放在 `.verification/uuu/uuu_mac_arm`、`.verification/uuu/LICENSE`。

```sh
npm ci
python3 scripts/stage-imager-macos-runtime.py --sources .verification
npm run typecheck
npm run test:imager
npm run check:ipc-contracts
npm run check:ui-constraints
npm run check:ui-scale
npm run validate:package
cargo test --manifest-path src-tauri/Cargo.toml --lib --locked
npx tauri build --bundles app --config '{"bundle":{"createUpdaterArtifacts":false}}'
python3 scripts/package-macos-manual.py \
  --app 'src-tauri/target/release/bundle/macos/LYNX Imager.app' \
  --version 0.1.0-rc.6 --output-dir release/macos-rc6
```

输出目录必须是新目录，脚本拒绝覆盖已有安装包。打包脚本检查 Imager bundle ID、版本、arm64、非系统绝对依赖、签名、只读挂载及主程序摘要。
资源整理脚本保留源/目标摘要及许可证；只对复制后的 Mach-O 重定位并做 ad-hoc 签名。
构建时禁止生成无法用原 OTA 私钥签名的更新资产；不是关闭包内权限或验签功能。

## 安装与验证

1. 先校验下载目录内的 `SHA256SUMS.txt`：`shasum -a 256 -c SHA256SUMS.txt`。
2. 用仓库脚本及随包公钥核验 DMG 分离签名：
   `node scripts/verify-macos-manual-signature.mjs <文件.dmg> <文件.dmg.sig> <macos-manual.pub>`。
3. 打开 DMG，将 **LYNX Imager.app** 拖入 Applications。不要覆盖正在烧录的进程。
4. 当前包未公证，macOS 可能阻止启动。仅在你确认来源后按 macOS 的“隐私与安全性”流程处理；不要全局关闭 Gatekeeper。
5. 打开后确认版本、目标选择、系统镜像、设备连接、高级工作台、AI 设置和任务记录。
6. 无实体板卡测试时，不得声称烧录/物理回读/冷启动验证通过；页面模拟成功只验证交互。

## 正式发布还需要

Apple Developer Program 账号授权签发的 Developer ID Application、notarytool 公证凭据、实际干净系统安装/升级及对应设备验收。
补齐后签名所有嵌套代码，公证并 staple，重新打包/签名/校验。不能把本地自签或 macOS 分发密钥标成 Apple 证书。
未来接入 macOS OTA 须单独完成客户端平台支持、信任配置和升级验收，不能悄悄替换现有 Windows/Linux 密钥。
