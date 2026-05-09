# AtCoder-Kit

AtCoder-Kit は、[AtCoder](https://atcoder.jp/) 向けに作成された CLI ツールです。
Rust で書かれており、AtCoder のコンテストに参加するための様々な機能を提供します。

## なぜ AtCoder-Kit なのか
従来の CLI ツール（例えば、atcoder-cli や online-judge-tools）は、最近の AtCoder の仕様変更や、言語アップデートにより動作しなくなってしまいました。<br>
それらを解決するために、またモダンで使いやすいツールを提供するために、AtCoder-Kit を開発しました。

## 主な機能
- AtCoder への ログイン・ログアウト
- コンテストのダウンロード
- コードのテスト・提出
- コードテンプレートの管理
- （その他鋭意制作中・・・）

## インストール方法
[GitHub のリリースページ](https://github.com/Yu-yu0202/AtCoder-Kit/releases/latest) から最新のバージョンをダウンロードし、PATH に追加してください。（詳細な手順に関しては省略します）

また、cargo でインストールすることもできます。
```shell
cargo install --git https://github.com/Yu-yu0202/AtCoder-Kit.git
```

## 動作環境
### OS
- Windows 10 以降（動作確認済み: Windows 11 Pro 25H2)
- Linux (動作確認済み: WSL2 上の Debian 13 trixie)
- macOS (未確認)

### Rust（cargo でインストールする場合）
- Rust 1.80 以降

動作確認済み:
- rustc 1.92.0 / cargo 1.92.0
- rustc 1.97.0-nightly / cargo 1.97.0-nightly

### その他
- Linux の場合、libssl-dev のインストールが必要
- インターネット接続
- AtCoder アカウント

## 使い方
AtCoder-Kit の基本的な使い方については、[こちら](./docs/ja/01-使い方.md) を参照してください。

## ライセンス(License)
このプロジェクトは MIT License の下でライセンスされています。詳細は LICENSE ファイルを参照してください。<br>
This project is licensed under the MIT License. See the LICENSE file for details.

## 貢献
貢献は大歓迎です！小さなバグの報告や機能提案、コードの改善など、どんな形でも構いません。<br>
ただし、メンテナーも人間であることを忘れないでください。特に以下の項目に従ってください。
- 適切な言葉使いでコミュニケーションを取ること
- 高圧的・攻撃的な態度を取らないこと
- 建設的な議論を心がけること
