# assets

- `icon.svg`：アプリのアイコン（`script/bundle` で `atme.icns` にする）
- `menubar/idle.svg`：メニューバーの絵。`script/bundle` が、これを 30 度ずつ回した取得中の絵も作る

## 「@」の出どころ

どちらの「@」も、[Comfortaa](https://github.com/alexeiva/comfortaa) の Bold の字形を SVG のパスにしたもの。

- Copyright 2011 The Comfortaa Project Authors
- SIL Open Font License, Version 1.1

## 「@」の作り直し方

Comfortaa は可変フォント（`Comfortaa[wght].ttf`）なので、fontconfig の名前付きインスタンス（Bold）を選ばせて描く。macOS の rsvg-convert は既定で CoreText からフォントを探すので、`PANGOCAIRO_BACKEND=fc` で fontconfig を使わせる。

1. Google Fonts のリポジトリの `ofl/comfortaa/Comfortaa[wght].ttf` を、ほかのフォントを置かないディレクトリに入れ、そのディレクトリだけを読む `fonts.conf` を用意する
2. `font-family="Comfortaa" font-weight="bold"` の `<text>` で「@」を書いた SVG を、`PANGOCAIRO_BACKEND=fc FONTCONFIG_FILE=fonts.conf rsvg-convert -f svg` でパスにする
3. できたパスの範囲を測り、`icon.svg`（1024 四方の中央）と `menubar/idle.svg`（36 四方の中央）に収まるよう `transform` を付ける
