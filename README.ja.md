# atme

[English](README.md)

atme は、Slack で自分宛てに届いたメッセージを見落とさないための、macOS のメニューバーアプリです。自分へのメンションと DM を、メニューバーの一覧と通知で知らせます。@here や @channel、ユーザーグループへのメンションは対象にせず、自分個人に宛てたものだけを表示します。

## 特徴

- 表示したメッセージは、Slack で既読にするまで一覧に残ります。
- 複数のワークスペースのメッセージを、ワークスペースごとに分けて一つのメニューで見られます。
- token は Keychain にだけ保存します。通信先は、Slack の API と、アップデートの確認とダウンロードだけです。

## 動作環境

- macOS 13 以降
- Apple シリコンの Mac
- 自分で Slack App を作成できる Slack のワークスペース

## インストール

Homebrew を使っている場合は、次のコマンドでインストールできます。

```
brew install --cask dominion525/tap/atme
```

tap の名前を含めて指定しているので、tap の追加も同時に行われます。Homebrew が信頼するのは、その tap のうち atme の cask だけです。

Homebrew を使わない場合は、[リリース](https://github.com/dominion525/atme/releases)から `atme-<version>.zip` をダウンロードして展開し、`atme.app` を「アプリケーション」フォルダに移して開きます。

どちらの方法でも、インストールされるアプリは Developer ID で署名され、Apple の公証を受けています。インストールしたあとは、atme が自分でアップデートします。すぐに新しい版を確認したいときは、メニューの「アップデートを確認…」を選びます。

初めて起動したときに通知の許可を求められるので、許可します。

## ワークスペースを追加する

atme は、Slack App の user token を使って Slack のメッセージを読み取ります。Slack App は作成したワークスペースにしかインストールできないので、ワークスペースごとに Slack App を作る必要があります。

初めて起動したときは、ワークスペースがまだ無いので、「ワークスペース」のウインドウが自動で開きます。2 つ目以降を追加するときは、メニューの「ワークスペース…」からウインドウを開きます。追加は、ウインドウに表示される次の手順で進めます。

<img src="docs/images/workspaces-ja.png" width="592" alt="ワークスペースのウインドウ。Slack App を作る、Install to Workspace を押して token をコピーする、貼り付けて追加する、の 3 つの手順が並ぶ">

1. 「Slack App を作る…」を押し、ブラウザで開いた Slack の画面でワークスペースを選んで、Slack App を作成します。
2. Slack の画面で Install to Workspace を押し、表示された User OAuth Token をコピーします。
3. コピーした token をウインドウに貼り付け、「追加」を押します。

Slack App の名前と権限は、[slack-app-manifest.json](slack-app-manifest.json) の内容で設定されます。ワークスペースの管理者がアプリの作成やインストールを制限している場合は、管理者の承認が必要です。

ワークスペースを削除するときは、ウインドウの一覧で「削除」を押します。そのワークスペースの token も、Keychain から削除されます。

## 使い方

<img src="docs/images/menu-ja.png" width="453" alt="atme のメニュー。ワークスペースごとの見出しの下に未読のメンションと DM が並び、その下に「すべて既読にする」「今すぐ取得」「ワークスペース…」などが続く">

atme はメニューバーに「@」のアイコンとして常駐し、Dock には表示されません。アイコンの横の数字は、未読のメンションと DM の件数です。

メニューには、未読のメッセージがワークスペースごとに新しい順で並びます。項目を選ぶと、そのメッセージを Slack で開き、atme の一覧から消します。「すべて既読にする」を選ぶと、今の一覧をまとめて消します。どちらの操作も、Slack 側の既読の状態は変えません。

atme は、5 分ごとと、Mac がスリープから復帰したときに、新しいメッセージを取得します。すぐに取得したいときは、「今すぐ取得」を選びます。新しいメッセージが見つかると通知が届き、通知をクリックするとそのメッセージを Slack で開きます。

ログイン時に atme を起動するには、メニューの「ログイン時に起動」をオンにします。

## 表示するメッセージ

| 表示するもの                   | 表示しないもの                                  |
| ------------------------------ | ----------------------------------------------- |
| 自分へのメンション             | @here、@channel、ユーザーグループへのメンション |
| DM とグループ DM               | 自分が送ったメッセージ                          |
| フォローしているスレッドの返信 | フォローしていないスレッドの返信                |
|                                | アーカイブされたチャンネルのメッセージ          |

既読かどうかは、Slack の既読の位置で判断します。Slack でメッセージを読むと、次の取得で atme の一覧からも消えます。

## プライバシーと権限

token は Keychain にだけ保存し、設定ファイルには書き込みません。atme の通信先は、Slack の API（`slack.com`）と、[Sparkle](https://sparkle-project.org) によるアップデートの確認とダウンロードだけです。アップデートの確認では新しい版の一覧を取得するだけで、Mac の情報は送りません。アップデートを自動で確認するかどうかは、2 回目に起動したときに聞かれます。

Slack App が求める user token のスコープは、次の 10 個です。

| スコープ                                                           | 使う API                | 用途                       |
| ------------------------------------------------------------------ | ----------------------- | -------------------------- |
| `search:read`                                                      | `search.messages`       | メンションと DM を探す     |
| `users:read`                                                       | `users.info`            | 送り主の表示名を取得する   |
| `channels:read`、`groups:read`、`im:read`、`mpim:read`             | `conversations.info`    | 会話の既読の位置を読む     |
| `channels:history`、`groups:history`、`im:history`、`mpim:history` | `conversations.replies` | スレッドの既読の位置を読む |

## 設定ファイル

設定は `~/.config/atme/config.toml` に保存されます。ワークスペースの追加と削除はウインドウから行えるので、手で編集する必要はありません。

```toml
# さかのぼって探す日数（既定は 7）
days = 7

[[workspace]]
name = "acme"
```

ウインドウから追加したワークスペースの `name` には、ワークスペースの URL のサブドメインが入ります。手で書いたコメントは、ウインドウから追加や削除をしても残ります。

## ソースからビルドする

Rust と rsvg-convert（`brew install librsvg`）が必要です。

```
script/bundle
open target/release/atme.app
```

`script/bundle` は `target/release/atme.app` を作り、ad-hoc で署名します。証明書で署名するときは、`ATME_SIGN_IDENTITY` に証明書の名前を指定します。

テストは `cargo test` で実行します。macOS の Foundation を使うテストがあるので、macOS 上で実行します。

リリースの手順は [docs/releasing.md](docs/releasing.md) にあります。

## ライセンス

[MIT License](LICENSE)

アイコンの「@」は、[Comfortaa](https://github.com/alexeiva/comfortaa)（SIL Open Font License 1.1）の字形から作っています。詳しくは [assets/README.md](assets/README.md) にあります。
