# リリースの手順

保守する人向けの手順です。使う人向けの説明は [README.ja.md](../README.ja.md) にあります。

## 版を上げる

版は `Cargo.toml` の `version` で決まり、Info.plist の版にもそのまま入ります。Sparkle はこの版で新旧を比べるので、リリースのたびに上げます。

## 配るものを作る

```
script/release
```

`script/release` は、Developer ID の署名でビルドし、公証と staple を行い、`target/dist/` に `atme-<version>.zip` と `appcast.xml` を作ります。未コミットの変更があるときと、公証が通らなかったときは止まります。

使う資格情報は次のとおりです。

| 資格情報                  | 内容                                                        | 既定                       |
| ------------------------- | ----------------------------------------------------------- | -------------------------- |
| `NOTARY_KEYCHAIN_PROFILE` | `xcrun notarytool store-credentials` で保存したプロファイル | `notarytool`               |
| `ATME_SIGN_IDENTITY`      | 署名に使う証明書                                            | `Developer ID Application` |
| Sparkle の EdDSA の鍵     | キーチェーンにある account `atme` の鍵                      |                            |

## 配る

zip は GitHub のリリース（タグ `v<version>`）に置き、`appcast.xml` は `gh-pages` ブランチに置きます。`gh-pages` は <https://atme.dominion525.com> で配信され、atme はここの `appcast.xml` を読んで新しい版を知ります。

## Sparkle の鍵

更新の zip には、Sparkle の EdDSA の鍵で署名します。対になる公開鍵は `script/bundle` が Info.plist の `SUPublicEDKey` に入れていて、atme はこの公開鍵で署名を確かめます。秘密鍵を失うと、すでに使っている人に新しい版を届けられなくなるので、書き出したものを安全な場所に控えておきます。

鍵は、Sparkle の `generate_keys` で扱います。`generate_keys` は、`script/bundle` がダウンロードした Sparkle の `bin/` にあります。

| コマンド                                 | すること                                 |
| ---------------------------------------- | ---------------------------------------- |
| `generate_keys --account atme`           | 鍵を作る（あれば公開鍵を表示する）       |
| `generate_keys --account atme -x <file>` | 秘密鍵をファイルに書き出す               |
| `generate_keys --account atme -f <file>` | 書き出した秘密鍵をキーチェーンに読み込む |

ほかの Mac からリリースするときは、控えた秘密鍵を `-f` でその Mac のキーチェーンに読み込みます。
