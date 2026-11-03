# リリースの手順

保守する人向けの手順です。使う人向けの説明は [README.ja.md](../README.ja.md) にあります。

## 版を上げる

版は `Cargo.toml` の `version` で決まり、Info.plist の版にもそのまま入ります。Sparkle はこの版で新旧を比べるので、リリースのたびに上げます。

## リリースする

版を上げたコミットを `main` に入れてから、その版のタグを push します。

```
git tag v<version>
git push origin v<version>
```

タグを push すると、`.github/workflows/release.yml` が次のことを行います。

1. タグと `Cargo.toml` の版が一致することを確かめる（違えば止まる）
2. Developer ID の署名でビルドし、公証と staple を行う
3. GitHub のリリース（タグ `v<version>`）を作り、`atme-<version>.zip` を置く
4. `gh-pages` の `appcast.xml` に新しい版を足す
5. [dominion525/homebrew-tap](https://github.com/dominion525/homebrew-tap) の `Casks/atme.rb` の `version` と `sha256` を書き換える

3 より後で失敗したときも、リリースは残ります。遅れるのは、失敗した手順の分だけです。

## Secrets

ワークフローは、リポジトリの Secrets にある次の値を使います。

| Secret                     | 内容                                                                                   |
| -------------------------- | -------------------------------------------------------------------------------------- |
| `BUILD_CERTIFICATE_BASE64` | Developer ID Application の証明書を .p12 に書き出し、base64 にしたもの                 |
| `P12_PASSWORD`             | その .p12 の書き出しのパスワード                                                       |
| `KEYCHAIN_PASSWORD`        | ワークフローの中で証明書を入れる、一時的なキーチェーンのパスワード（どんな値でもよい） |
| `NOTARY_KEY_ID`            | 公証に使う App Store Connect の API キーの Key ID                                      |
| `NOTARY_ISSUER_ID`         | その API キーの Issuer ID                                                              |
| `NOTARY_KEY_P8_BASE64`     | その API キーの .p8 を base64 にしたもの                                               |
| `SPARKLE_PRIVATE_KEY`      | 更新に署名する Sparkle の秘密鍵（`generate_keys --account atme -x` で書き出したもの）  |
| `HOMEBREW_TAP_TOKEN`       | dominion525/homebrew-tap の Contents に書き込める fine-grained token                   |

Secrets は、登録したあとに値を読み出せません。登録し直せるよう、元の値は別の場所に控えておきます。

## 手元で配るものを作る

ワークフローと同じものを、手元でも作れます。

```
script/release
```

`script/release` は、Developer ID の署名でビルドし、公証と staple を行い、`target/dist/<version>/` に `atme-<version>.zip` と `appcast.xml` を作ります。未コミットの変更があるときと、公証が通らなかったときは止まります。

使う値は次のとおりです。環境変数で渡さなかったものは、キーチェーンから読みます。

| 環境変数                                             | 内容                                                                       | 渡さないとき                                       |
| ---------------------------------------------------- | -------------------------------------------------------------------------- | -------------------------------------------------- |
| `NOTARY_KEY_P8`、`NOTARY_KEY_ID`、`NOTARY_ISSUER_ID` | 公証に使う App Store Connect の API キーのファイル、Key ID、Issuer ID      | `NOTARY_KEYCHAIN_PROFILE` のプロファイルで公証する |
| `NOTARY_KEYCHAIN_PROFILE`                            | `xcrun notarytool store-credentials` で保存したプロファイル                | `notarytool`                                       |
| `ATME_SIGN_IDENTITY`                                 | 署名に使う証明書                                                           | `Developer ID Application`                         |
| `SPARKLE_PRIVATE_KEY`                                | 更新に署名する Sparkle の秘密鍵                                            | キーチェーンにある account `atme` の鍵             |
| `ATME_APPCAST`                                       | 今配っている `appcast.xml`。渡すと、前の版の項目を残したまま新しい版を足す | 新しい版だけの `appcast.xml` を作る                |

## 配る先

zip は GitHub のリリースに置き、`appcast.xml` は `gh-pages` ブランチに置きます。`gh-pages` は <https://atme.dominion525.com> で配信され、atme はここの `appcast.xml` を読んで新しい版を知ります。Homebrew の cask は、GitHub のリリースの zip を指します。

## Sparkle の鍵

更新の zip には、Sparkle の EdDSA の鍵で署名します。対になる公開鍵は `script/bundle` が Info.plist の `SUPublicEDKey` に入れていて、atme はこの公開鍵で署名を確かめます。秘密鍵を失うと、すでに使っている人に新しい版を届けられなくなるので、書き出したものを安全な場所に控えておきます。

鍵は、Sparkle の `generate_keys` で扱います。`generate_keys` は、`script/bundle` がダウンロードした Sparkle の `bin/` にあります。

| コマンド                                 | すること                                 |
| ---------------------------------------- | ---------------------------------------- |
| `generate_keys --account atme`           | 鍵を作る（あれば公開鍵を表示する）       |
| `generate_keys --account atme -x <file>` | 秘密鍵をファイルに書き出す               |
| `generate_keys --account atme -f <file>` | 書き出した秘密鍵をキーチェーンに読み込む |

ほかの Mac から手元でリリースするときは、控えた秘密鍵を `-f` でその Mac のキーチェーンに読み込むか、`SPARKLE_PRIVATE_KEY` で渡します。
