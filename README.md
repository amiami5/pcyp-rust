# pcyp-rust

Windows 用の PeerCast YP ブラウザです。Rust だけで書いています (画面は eframe/egui)。

イエローページ (YP) からチャンネルの一覧を取ってきて表示し、ダブルクリックでプレイヤーを起動して再生します。
再生には、手元か LAN 内で動いている PeerCast (PeerCast YT、PeerCastStation) を使います。
画面は pcyplite に似せています。

## 主な機能

- **一覧**: 複数の YP から並列に取得します。1 つの YP が失敗しても、ほかの YP の一覧は表示します
- **表示**: 名前と説明の 2 行表示と 1 行表示、列の表示と非表示、並べ替え、検索
- **タブ**: お気に入り / すべて / YP ごと / 無視 (無視のタブは初期設定では隠しています)
- **フィルター**: 正規表現で、お気に入り・無視・色分け・通知を設定できます。当たったフィルターの名前と YP 名は、名前の行の右端に出ます
- **通知**: お気に入りの配信が始まると、Windows の通知を出します。通知の「再生」ボタンで、そのまま再生できます
- **再生**: 種類 (FLV、MKV など) ごとにプレイヤーと引数を設定できます
- **タスクトレイ**: 最小化や閉じるボタンでトレイに格納できます。起動時から格納しておくこともできます
- **PeerCast との連携**: 接続中のチャンネルの表示・停止・再接続 (JSON-RPC)、PeerCast 本体の自動起動
- **ブラウザ**: コンタクト URL、チャット、統計を開きます (http と https の URL だけ)
- **自動更新**: 2 分以上の間隔で設定できます (初期値 5 分)。手動の更新は、前回から 30 秒以上空けます

## 動作環境

- Windows 10 / 11
- PeerCast YT (C++ 版、Rust 版) または PeerCastStation。同じ PC でも、LAN の別の PC でもかまいません
- 再生用のプレイヤー (mpv、MPC-BE、MPC-HC、VLC、PeerstPlayer など)

## ビルド

[Rust](https://www.rust-lang.org/ja/tools/install) (stable、1.93 以上) を入れてから、次を実行します。

```
git clone https://github.com/amiami5/pcyp-rust.git
cd pcyp-rust
cargo build --release
```

`target\release\pcyp-rust.exe` ができます。この exe を好きなフォルダに置いて使います。

## はじめに設定すること

起動してツールバーの「設定」を開きます。

1. **PeerCast タブ**: 「アドレス」に PeerCast の `ホスト:ポート` を入れます。
   初期値は `127.0.0.1:7144` です。LAN の別の PC なら `192.168.1.10:7144` のように書きます。
   「接続を確認」を押すと、つながるかどうかと、PeerCast の種類がわかります。
2. **プレイヤー タブ**: 使うプレイヤーの exe と引数を設定します。
   初期値は `mpv.exe` (PATH の通った mpv) です。「雛形から追加」で、よく使うプレイヤーの設定を追加できます。
3. **YP タブ**: 使う YP を選びます。初期値は SP、平成、P@、YPv6、Event YP です。追加や削除もできます。

## 使い方

| 操作 | 動き |
|---|---|
| 行をダブルクリック / Enter | 再生する |
| 右クリック | 再生、コンタクト URL・チャット・統計を開く、コピー、お気に入り・無視に追加、PeerCast の再接続・停止 |
| 列の見出しをクリック | 並べ替え (もう一度押すと逆順) |
| F5 / 「更新」 | すべての YP を取り直す |
| Ctrl + F | 検索欄へ移る |
| ↑ / ↓ | 選ぶ行を動かす |

お気に入りは、行を右クリックして「お気に入りに追加」で登録できます。
細かい条件 (ジャンルで絞る、特定の語を除くなど) は、ツールバーの「フィルター」で設定します。

## 再生の仕組み

プレイヤーには、PeerCast の次の URL を渡します。PeerCast は `tip` (トラッカー) を手がかりに中継を始めます。

```
http://127.0.0.1:7144/stream/<チャンネル ID><拡張子>?tip=<トラッカー>
```

設定の PeerCast タブで、`/pls/<ID>?tip=...` (プレイリスト) に切り替えられます。
「自由に書く」を選ぶと、`$BASE/stream/$ID$EXT?tip=$TIP` のような雛形で URL を好きな形にできます。

一覧の取得と再生には、PeerCast の認証は要りません (LAN の別の PC の PeerCast でも同じです)。
「接続中のチャンネル」「停止」「再接続」を LAN の別の PC の PeerCast YT に対して使うときだけ、
PeerCast の管理画面のパスワードを設定に入れてください。

### プレイヤーの引数

引数の雛形では、次の置き換えが使えます。`"` で囲むと、空白を含む値も 1 つの引数になります。

| 置き換え | 中身 |
|---|---|
| `$URL` | 再生用の URL (PeerCast タブで選んだ形) |
| `$STREAM` / `$PLS` | `/stream/` / `/pls/` の URL |
| `$NAME` | チャンネル名 |
| `$ID` | チャンネル ID |
| `$TIP` | トラッカー |
| `$CONTACT` | コンタクト URL |
| `$TYPE` `$GENRE` `$DESC` `$COMMENT` `$BITRATE` | 種類、ジャンル、詳細、コメント、ビットレート |

pcyplite 形式の `<stream/>` `<channelname/>` `<contact/>` も使えます。

例 (mpv): `--force-media-title="$NAME" "$URL"`

## 設定ファイル

exe と同じフォルダに保存します (ポータブル版)。フォルダごと別の場所へ移しても、そのまま使えます。

| ファイル | 中身 |
|---|---|
| `pcyp-rust.json` | YP、PeerCast、プレイヤー、通知、表示などの設定 |
| `channelFilters.json` | お気に入り・無視・色分けのフィルター。peercast-yt の `channelFilters.json` と同じ形です |

プレイヤーや PeerCast の exe を、この exe のフォルダの下に置いた場合は、相対パスで保存します。

設定が消えないように、次のようにしています。

- 保存するたびに、1 つ前の内容を `.bak` (例 `pcyp-rust.json.bak`) に残します
- 設定のファイルが読めなかったときは、消さずに `.broken-日時` の名前で残し、`.bak` から読み直します。
  起動したときに、画面の上でそのことを知らせます
- 保存はディスクまで書き込んでから入れ替えるので、途中で電源が切れても、本体か `.bak` のどちらかが残ります

## 開発

```
cargo test
```

テストでは、実際の YP にはつなぎません (固定の行と、ローカルの小さな HTTP サーバーを使います)。

実際の PeerCast につなぐテストは、ふだんは飛ばします。アドレスを環境変数で渡したときだけ動きます。

```
PCYP_TEST_PEERCAST=ホスト:ポート cargo test live_peercast -- --ignored --nocapture
```

`PCYP_TEST_PEERCAST_PASS` にパスワードを入れると、`getChannels` も確かめます。

### ソースの構成

| ファイル | 内容 |
|---|---|
| `src/main.rs` | 起動、フォント、二重起動の防止 |
| `src/app.rs` | 画面 (一覧、設定、フィルター、ログなど) |
| `src/chandir.rs` | `index.txt` の解析 |
| `src/fetch.rs` | YP からの取得 (HTTP) |
| `src/worker.rs` | 取得のスレッド、更新の間隔、新着と通知の判定 |
| `src/filter.rs` | フィルター |
| `src/player.rs` | 再生の URL、プレイヤーとブラウザの起動 |
| `src/peercast.rs` | PeerCast との連携 (JSON-RPC、本体の起動) |
| `src/config.rs` | 設定と保存 |
| `src/win.rs` | ウィンドウの表示と非表示、タスクトレイ、通知 |

## ライセンス

[MIT License](LICENSE)
