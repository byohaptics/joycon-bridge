# BYO Haptics Joy-Con Bridge

Version `0.1.0`

Joy-ConをBYO Hapticsで振動させるためのWindowsアプリです。

- Supported Bridge API: `0.1.0`
- API contract: [BYO Haptics Joy-Con Bridge API](https://github.com/byohaptics/byo-haptics/blob/main/docs/joycon-bridge-contract.md)

## 使い方

1. WindowsのBluetooth設定でJoy-Con (L)とJoy-Con (R)を接続します。
2. `Joy-Con Bridge`を開きます。
3. `接続状態を確認`を押します。
4. 左右が両方とも`検出済み`になったら、`振動を開始`を押します。
5. `BYO Haptics: 接続`になれば準備完了です。

利用者が開くものは`Joy-Con Bridge`だけです。追加のプログラム、保存場所、通信設定はアプリが自動的に管理します。

`測定して最適化`は必要な人だけが使う機能です。通常は実行しなくても振動します。

## 開発

```powershell
cargo test --all-targets
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
./scripts/package.ps1
```

詳しい設計は[Bridge design](docs/design.md)を参照してください。

## 作者

[byohaptics](https://github.com/byohaptics)

Co-authored-by: Codex <codex@openai.com>

Licensed under the [MIT License](LICENSE).
