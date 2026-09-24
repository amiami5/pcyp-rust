# pcyp-rust

Windows 用の PeerCast YP ブラウザです。Rust (eframe/egui) だけで書いています。

YP の `index.txt` を取ってきてチャンネル一覧を表示し、ダブルクリックでローカルの PeerCast
(PeerCast YT の C++ 版と Rust 版、PeerCastStation) を通してプレイヤーで再生します。
画面は pcyplite に似せています。

## 機能

- 複数の YP から並列に取得 (1 つが失敗しても、ほかの一覧は表示)
- 自動更新 (2 分以上の間隔)。手動更新は前回から 30 秒以上空ける
- タブ: お気に入り / すべて / YP ごと / 無視 (無視のタブは初期設定では隠す)
- 2 行表示 (pcyplite 風) と 1 行表示、列の表示と非表示、並べ替え、検索
- フィルター (正規表現。お気に入り・無視・色分け・通知)。peercast-yt の `channelFilters.json` と同じ形
- お気に入りのチャンネルが始まったら Windows の通知を出す (通知の「再生」で再生)
- タスクトレイ (最小化・閉じるで格納、起動時に格納)
- 種類 (FLV、MKV など) ごとのプレイヤーの設定。引数の雛形は `$URL` `$NAME` など。pcyplite 形式の `<stream/>` も使える
- PeerCast 本体の自動起動、JSON-RPC で接続中のチャンネルの表示・停止・再接続
- コンタクト URL、チャット、統計をブラウザで開く (http と https だけ)
- お知らせの行 (ID が全部 0) は色を変えて表示し、再生しない

## 再生の仕組み

プレイヤーに次の URL を渡します。PeerCast は `tip` を手がかりに中継を始めます。

```
http://127.0.0.1:7144/stream/<ID><拡張子>?tip=<トラッカー>
```

設定で `/pls/<ID>?tip=...` (プレイリスト) に切り替えられます。どちらも PeerCast YT と
PeerCastStation の両方で使える形です。
「自由に書く」を選ぶと、`$BASE/stream/$ID$EXT?tip=$TIP` のような雛形で URL を好きな形にできます。

PeerCast のアドレスは `ホスト:ポート` の形で 1 つの欄に入れます (例 `127.0.0.1:7144`、`192.168.1.10:7144`、`[::1]:7144`)。
LAN の別の PC の PeerCast でも、一覧の取得と再生に認証は要りません。
「接続中のチャンネル」「停止」「再接続」(JSON-RPC) は、PeerCast YT が localhost 以外からの操作にログインを求めるので、
PeerCast の管理画面のパスワードを設定に入れてください。

## ビルド

```
cargo build --release
```

`target\release\pcyp-rust.exe` ができます。

## 設定ファイル

exe と同じフォルダに置きます (ポータブル版)。

| ファイル | 内容 |
|---|---|
| `pcyp-rust.json` | 設定 (YP、PeerCast、プレイヤー、通知、表示) |
| `channelFilters.json` | フィルター |

プレイヤーや PeerCast の exe を exe のフォルダの下に置いた場合は、相対パスで保存します。

## テスト

```
cargo test
```

テストでは実際の YP にはつなぎません (固定の行とローカルの HTTP サーバーを使います)。

実際の PeerCast につなぐテストは、ふだんは飛ばします。アドレスを環境変数で渡すと動きます。

```
PCYP_TEST_PEERCAST=ホスト:ポート cargo test live_peercast -- --ignored --nocapture
```

`PCYP_TEST_PEERCAST_PASS` にパスワードを入れると、`getChannels` も確かめます。

## ライセンス

MIT License (`LICENSE`)。
