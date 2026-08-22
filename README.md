# BYO Haptics Joy-Con Bridge

Version `0.1.0`

Joy-ConをBYO Hapticsで振動させるためのWindowsアプリです。

- Supported Bridge API: `0.1.0`
- API contract: [BYO Haptics Joy-Con Bridge API](https://github.com/byohaptics/byo-haptics/blob/main/docs/joycon-bridge-contract.md)

## 使い方

1. WindowsのBluetooth設定でJoy-Con (L)とJoy-Con (R)を接続します。
2. `Joy-Con Bridge`を開きます。
3. `接続を確認`を押します。
4. 左右が両方とも`検出済み`になったら、`振動を開始`を押します。
5. 画面上部が`待機中`に変わります。BYO Haptics側が接続すると`振動中`になり、準備完了です。

通常は設定ファイルを用意する必要はありません。

画面下部には`振動の測定と最適化`と`動作ログ`の欄があります。測定と最適化は必要な人だけが使う機能で、通常は実行する必要はありません。動作ログは問題が起きたときに自動で開きます。

動作ログや測定結果のファイルは`%LOCALAPPDATA%\BYO Haptics\Joy-Con Bridge`に保存されます。

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

Assisted-by: OpenAI Codex, Claude Code

Licensed under the [MIT License](LICENSE).
